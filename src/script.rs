use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use rquickjs::{CatchResultExt, Context, Object, Runtime, function::Func};
use serde::{Deserialize, Serialize};

use crate::models::EnvironmentVariable;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConsoleLevel {
    Log,
    Info,
    Warn,
    Error,
    Debug,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleMessage {
    pub level: ConsoleLevel,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptErrorType {
    SyntaxError,
    RuntimeError,
    Timeout,
    MemoryLimit,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnvironmentChangeKind {
    Added,
    Updated,
    Removed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentChange {
    pub kind: EnvironmentChangeKind,
    pub key: String,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptExecutionResult {
    pub success: bool,
    pub error_type: Option<ScriptErrorType>,
    pub error_message: Option<String>,
    pub failed: bool,
    pub failure_message: Option<String>,
    pub environment_changes: BTreeMap<String, String>,
    pub removed_env_keys: Vec<String>,
    pub environment_diff: Vec<EnvironmentChange>,
    pub test_results: Vec<TestResult>,
    pub console_output: Vec<ConsoleMessage>,
}

#[derive(Debug, Clone)]
pub struct ScriptRuntimeResponse {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub response_time_ms: u64,
    pub body_size_bytes: usize,
}

pub fn execute_post_request_script(
    script: &str,
    response: &ScriptRuntimeResponse,
    environment_variables: &[EnvironmentVariable],
) -> ScriptExecutionResult {
    execute_post_request_script_with_options(
        script,
        response,
        environment_variables,
        5,
        64 * 1024 * 1024,
    )
}

fn execute_post_request_script_with_options(
    script: &str,
    response: &ScriptRuntimeResponse,
    environment_variables: &[EnvironmentVariable],
    timeout_secs: u64,
    memory_limit_bytes: usize,
) -> ScriptExecutionResult {
    let mut result = ScriptExecutionResult {
        success: false,
        error_type: None,
        error_message: None,
        failed: false,
        failure_message: None,
        environment_changes: BTreeMap::new(),
        removed_env_keys: Vec::new(),
        environment_diff: Vec::new(),
        test_results: Vec::new(),
        console_output: Vec::new(),
    };

    if script.trim().is_empty() {
        result.success = true;
        return result;
    }

    let runtime = match Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            result.error_type = Some(ScriptErrorType::Unknown);
            result.error_message = Some(format!("Failed to create JavaScript runtime: {e}"));
            return result;
        }
    };

    runtime.set_memory_limit(memory_limit_bytes);
    let start_time = std::time::Instant::now();
    runtime.set_interrupt_handler(Some(Box::new(move || {
        start_time.elapsed().as_secs_f64() >= timeout_secs as f64
    })));

    let context = match Context::full(&runtime) {
        Ok(ctx) => ctx,
        Err(e) => {
            result.error_type = Some(ScriptErrorType::Unknown);
            result.error_message = Some(format!("Failed to create JavaScript context: {e}"));
            return result;
        }
    };

    let console_output_shared: Arc<Mutex<Vec<ConsoleMessage>>> = Arc::new(Mutex::new(Vec::new()));
    let env_changes_shared = Arc::new(Mutex::new(BTreeMap::new()));
    let removed_env_keys_shared = Arc::new(Mutex::new(Vec::new()));
    let test_results_shared = Arc::new(Mutex::new(Vec::new()));
    let fail_message_shared = Arc::new(Mutex::new(None::<String>));
    let error_info_shared = Arc::new(Mutex::new(None::<(ScriptErrorType, String)>));

    let execution_result = context.with(|ctx| {
        if let Err(e) = setup_global_objects(
            &ctx,
            response,
            environment_variables,
            Arc::clone(&console_output_shared),
            Arc::clone(&env_changes_shared),
            Arc::clone(&removed_env_keys_shared),
            Arc::clone(&test_results_shared),
            Arc::clone(&fail_message_shared),
        ) {
            return Err(e);
        }

        let eval_result: Result<(), rquickjs::Error> = ctx.eval::<(), _>(script);
        match eval_result {
            Ok(_) => Ok(()),
            Err(rquickjs::Error::Allocation) => {
                if let Ok(mut guard) = error_info_shared.lock() {
                    *guard = Some((
                        ScriptErrorType::MemoryLimit,
                        format!(
                            "Script exceeded memory limit ({} MB)",
                            memory_limit_bytes / (1024 * 1024)
                        ),
                    ));
                }
                Err(rquickjs::Error::Exception)
            }
            Err(e) => {
                let caught: Result<(), rquickjs::CaughtError> = Err(e).catch(&ctx);
                match caught {
                    Err(rquickjs::CaughtError::Exception(ex)) => {
                        let name = ex
                            .as_object()
                            .get::<_, Option<String>>("name")
                            .ok()
                            .flatten()
                            .unwrap_or_else(|| "Error".to_string());
                        let message = ex.message().unwrap_or_default();
                        let line_number = ex
                            .as_object()
                            .get::<_, Option<i32>>("lineNumber")
                            .ok()
                            .flatten()
                            .and_then(|n| (n > 0).then_some(n as usize));
                        let stack = ex
                            .as_object()
                            .get::<_, Option<String>>("stack")
                            .ok()
                            .flatten()
                            .filter(|s| !s.trim().is_empty());

                        let error_type = if name == "SyntaxError" {
                            ScriptErrorType::SyntaxError
                        } else if name == "InternalError"
                            && message.to_lowercase().contains("interrupted")
                        {
                            ScriptErrorType::Timeout
                        } else if name == "InternalError"
                            && message.to_lowercase().contains("out of memory")
                        {
                            ScriptErrorType::MemoryLimit
                        } else {
                            ScriptErrorType::RuntimeError
                        };

                        let mut full_message = if error_type == ScriptErrorType::SyntaxError {
                            format!("Syntax error: {message}")
                        } else if error_type == ScriptErrorType::Timeout {
                            format!(
                                "Script execution timed out (exceeded {} second limit)",
                                timeout_secs
                            )
                        } else if error_type == ScriptErrorType::MemoryLimit {
                            format!(
                                "Script exceeded memory limit ({} MB)",
                                memory_limit_bytes / (1024 * 1024)
                            )
                        } else {
                            format!("JavaScript runtime error: {name}: {message}")
                        };

                        if let Some(line) = line_number {
                            full_message.push_str(&format!("\nLine: {line}"));
                        }
                        if let Some(stack_trace) = stack {
                            full_message.push_str("\nStack Trace:\n");
                            full_message.push_str(&stack_trace);
                        }
                        if let Ok(mut guard) = error_info_shared.lock() {
                            *guard = Some((error_type, full_message));
                        }
                    }
                    Err(rquickjs::CaughtError::Value(val)) => {
                        let is_null = val.is_null();
                        let error_type = if is_null {
                            ScriptErrorType::MemoryLimit
                        } else {
                            ScriptErrorType::RuntimeError
                        };
                        let msg = if is_null {
                            format!(
                                "Script exceeded memory limit ({} MB)",
                                memory_limit_bytes / (1024 * 1024)
                            )
                        } else {
                            format!("JavaScript runtime error: {:?}", val)
                        };
                        if let Ok(mut guard) = error_info_shared.lock() {
                            *guard = Some((error_type, msg));
                        }
                    }
                    Err(rquickjs::CaughtError::Error(err)) => {
                        if let Ok(mut guard) = error_info_shared.lock() {
                            *guard = Some((
                                ScriptErrorType::RuntimeError,
                                format!("JavaScript runtime error: {err}"),
                            ));
                        }
                    }
                    Ok(_) => {}
                }
                Err(rquickjs::Error::Exception)
            }
        }
    });

    if let Ok(captured_console) = console_output_shared.lock() {
        result.console_output.extend(captured_console.clone());
    }
    if let Ok(changes) = env_changes_shared.lock() {
        result.environment_changes = changes.clone();
    }
    if let Ok(removed) = removed_env_keys_shared.lock() {
        result.removed_env_keys = removed.clone();
    }
    if let Ok(tests) = test_results_shared.lock() {
        result.test_results = tests.clone();
    }
    result.environment_diff = build_environment_diff(
        environment_variables,
        &result.environment_changes,
        &result.removed_env_keys,
    );

    let fail_message = fail_message_shared.lock().ok().and_then(|v| v.clone());
    match execution_result {
        Ok(()) => {
            if let Some(msg) = fail_message {
                result.failed = true;
                result.failure_message = Some(msg.clone());
                result.error_message = Some(format!("Script failed: {msg}"));
                result.success = false;
            } else {
                result.success = true;
            }
        }
        Err(_) => {
            if let Some(msg) = fail_message {
                result.failed = true;
                result.failure_message = Some(msg.clone());
                result.error_message = Some(format!("Script failed: {msg}"));
            } else if let Ok(mut guard) = error_info_shared.lock() {
                if let Some((error_type, error_msg)) = guard.take() {
                    result.error_type = Some(error_type);
                    result.error_message = Some(error_msg);
                }
            }
            result.success = false;
        }
    }

    result
}

fn build_environment_diff(
    initial_variables: &[EnvironmentVariable],
    environment_changes: &BTreeMap<String, String>,
    removed_env_keys: &[String],
) -> Vec<EnvironmentChange> {
    let mut original_values: BTreeMap<String, String> = BTreeMap::new();
    for var in initial_variables {
        if var.enabled {
            original_values.insert(var.name.clone(), var.value.clone());
        }
    }

    let mut diff = Vec::new();
    for (key, new_value) in environment_changes {
        if let Some(old_value) = original_values.get(key) {
            diff.push(EnvironmentChange {
                kind: EnvironmentChangeKind::Updated,
                key: key.clone(),
                old_value: Some(old_value.clone()),
                new_value: Some(new_value.clone()),
            });
        } else {
            diff.push(EnvironmentChange {
                kind: EnvironmentChangeKind::Added,
                key: key.clone(),
                old_value: None,
                new_value: Some(new_value.clone()),
            });
        }
    }

    for key in removed_env_keys {
        diff.push(EnvironmentChange {
            kind: EnvironmentChangeKind::Removed,
            key: key.clone(),
            old_value: original_values.get(key).cloned(),
            new_value: None,
        });
    }

    diff
}

fn setup_global_objects(
    ctx: &rquickjs::Ctx,
    response: &ScriptRuntimeResponse,
    environment_variables: &[EnvironmentVariable],
    console_output: Arc<Mutex<Vec<ConsoleMessage>>>,
    env_changes: Arc<Mutex<BTreeMap<String, String>>>,
    removed_env_keys: Arc<Mutex<Vec<String>>>,
    test_results: Arc<Mutex<Vec<TestResult>>>,
    fail_message: Arc<Mutex<Option<String>>>,
) -> Result<(), rquickjs::Error> {
    let response_status = response.status;
    let response_status_text = response.status_text.clone();
    let response_time = response.response_time_ms;
    let response_size = response.body_size_bytes;
    let env_vars = environment_variables.to_vec();

    let console = Object::new(ctx.clone())?;
    {
        let sink = Arc::clone(&console_output);
        let log_fn = Func::new(move |msg: String| {
            if let Ok(mut output) = sink.lock() {
                output.push(ConsoleMessage {
                    level: ConsoleLevel::Log,
                    message: msg,
                    timestamp: Utc::now(),
                });
            }
        });
        console.prop("__beamLog", log_fn)?;
    }
    {
        let sink = Arc::clone(&console_output);
        let fn_ = Func::new(move |msg: String| {
            if let Ok(mut output) = sink.lock() {
                output.push(ConsoleMessage {
                    level: ConsoleLevel::Error,
                    message: msg,
                    timestamp: Utc::now(),
                });
            }
        });
        console.prop("__beamError", fn_)?;
    }
    {
        let sink = Arc::clone(&console_output);
        let fn_ = Func::new(move |msg: String| {
            if let Ok(mut output) = sink.lock() {
                output.push(ConsoleMessage {
                    level: ConsoleLevel::Warn,
                    message: msg,
                    timestamp: Utc::now(),
                });
            }
        });
        console.prop("__beamWarn", fn_)?;
    }
    {
        let sink = Arc::clone(&console_output);
        let fn_ = Func::new(move |msg: String| {
            if let Ok(mut output) = sink.lock() {
                output.push(ConsoleMessage {
                    level: ConsoleLevel::Info,
                    message: msg,
                    timestamp: Utc::now(),
                });
            }
        });
        console.prop("__beamInfo", fn_)?;
    }
    {
        let sink = Arc::clone(&console_output);
        let fn_ = Func::new(move |msg: String| {
            if let Ok(mut output) = sink.lock() {
                output.push(ConsoleMessage {
                    level: ConsoleLevel::Debug,
                    message: msg,
                    timestamp: Utc::now(),
                });
            }
        });
        console.prop("__beamDebug", fn_)?;
    }
    ctx.globals().set("console", console)?;
    ctx.eval::<(), _>(
        r#"
        (function() {
            function formatArg(arg) {
                if (typeof arg === "string") return arg;
                if (arg === null) return "null";
                if (arg === undefined) return "undefined";
                if (typeof arg === "object") {
                    try {
                        var json = JSON.stringify(arg);
                        if (json !== undefined) return json;
                    } catch (_) {}
                }
                return String(arg);
            }
            function formatArgs(argsLike) {
                var out = [];
                for (var i = 0; i < argsLike.length; i++) out.push(formatArg(argsLike[i]));
                return out.join(" ");
            }
            console.log = function() { console.__beamLog(formatArgs(arguments)); };
            console.error = function() { console.__beamError(formatArgs(arguments)); };
            console.warn = function() { console.__beamWarn(formatArgs(arguments)); };
            console.info = function() { console.__beamInfo(formatArgs(arguments)); };
            console.debug = function() { console.__beamDebug(formatArgs(arguments)); };
        })();
    "#,
    )?;

    let pm = Object::new(ctx.clone())?;
    let env_obj = Object::new(ctx.clone())?;

    {
        let env_changes_clone = Arc::clone(&env_changes);
        let set_fn = Func::new(move |key: String, value: String| {
            if let Ok(mut changes) = env_changes_clone.lock() {
                changes.insert(key, value);
            }
        });
        env_obj.prop("__internalSet", set_fn)?;
    }
    {
        let env_changes_clone = Arc::clone(&env_changes);
        let removed_env_keys_clone = Arc::clone(&removed_env_keys);
        let env_vars_for_clear = env_vars.clone();
        let clear_fn = Func::new(move || {
            if let Ok(mut changes) = env_changes_clone.lock() {
                changes.clear();
            }
            if let Ok(mut removed) = removed_env_keys_clone.lock() {
                for var in &env_vars_for_clear {
                    if !removed.contains(&var.name) {
                        removed.push(var.name.clone());
                    }
                }
            }
        });
        env_obj.prop("__internalClear", clear_fn)?;
    }

    ctx.globals().set("__tempEnvObj", env_obj.clone())?;
    ctx.eval::<(), _>(
        r#"
        (function() {
            var envObj = __tempEnvObj;
            var internalSetFn = envObj.__internalSet;
            var internalClearFn = envObj.__internalClear;
            envObj.set = function(key, value, type) {
                var valueStr;
                if (typeof value === "string") {
                    valueStr = value;
                } else if (value === null || value === undefined) {
                    valueStr = "";
                } else if (typeof value === "object") {
                    valueStr = JSON.stringify(value);
                } else {
                    valueStr = String(value);
                }
                internalSetFn(key, valueStr);
            };
            envObj.setAll = function(obj) {
                if (obj === null || obj === undefined) return;
                var keys = Object.keys(obj);
                for (var i = 0; i < keys.length; i++) envObj.set(keys[i], obj[keys[i]]);
            };
            envObj.setIfPresent = function(key, value) {
                if (value === null || value === undefined || value === "") return;
                envObj.set(key, value);
            };
            envObj.clear = function() { internalClearFn(); };
        })();
    "#,
    )?;
    let _ = ctx.globals().remove("__tempEnvObj");
    let _ = env_obj.remove("__internalSet");
    let _ = env_obj.remove("__internalClear");

    {
        let env_vars_for_get = env_vars.clone();
        let env_changes_for_get = Arc::clone(&env_changes);
        let removed_keys_for_get = Arc::clone(&removed_env_keys);
        let get_fn = Func::new(move |key: String| -> Option<String> {
            if let Ok(removed) = removed_keys_for_get.lock() {
                if removed.contains(&key) {
                    return None;
                }
            }
            if let Ok(changes) = env_changes_for_get.lock() {
                if let Some(value) = changes.get(&key) {
                    return Some(value.clone());
                }
            }
            env_vars_for_get
                .iter()
                .find(|var| var.name == key && var.enabled)
                .map(|var| var.value.clone())
        });
        env_obj.prop("get", get_fn)?;
    }
    {
        let env_vars_for_has = env_vars.clone();
        let env_changes_for_has = Arc::clone(&env_changes);
        let removed_keys_for_has = Arc::clone(&removed_env_keys);
        let has_fn = Func::new(move |key: String| -> bool {
            if let Ok(removed) = removed_keys_for_has.lock() {
                if removed.contains(&key) {
                    return false;
                }
            }
            if let Ok(changes) = env_changes_for_has.lock() {
                if changes.contains_key(&key) {
                    return true;
                }
            }
            env_vars_for_has
                .iter()
                .any(|var| var.name == key && var.enabled)
        });
        env_obj.prop("has", has_fn)?;
    }
    {
        let env_changes_clone = Arc::clone(&env_changes);
        let removed_env_keys_clone = Arc::clone(&removed_env_keys);
        let env_vars_for_unset = env_vars.clone();
        let unset_fn = Func::new(move |key: String| {
            let mut existed_in_changes = false;
            if let Ok(mut changes) = env_changes_clone.lock() {
                existed_in_changes = changes.remove(&key).is_some();
            }
            if !existed_in_changes && env_vars_for_unset.iter().any(|v| v.name == key) {
                if let Ok(mut removed) = removed_env_keys_clone.lock() {
                    if !removed.contains(&key) {
                        removed.push(key);
                    }
                }
            }
        });
        env_obj.prop("unset", unset_fn)?;
    }
    pm.prop("environment", env_obj)?;

    let response_obj = Object::new(ctx.clone())?;
    response_obj.prop("status", response_status)?;
    response_obj.prop("statusText", response_status_text)?;
    response_obj.prop("responseTime", response_time)?;
    response_obj.prop("bodySize", response_size)?;
    response_obj.prop("__beamRawText", response.body.clone())?;

    let mut headers_map = serde_json::Map::new();
    for (header_name, header_value) in &response.headers {
        headers_map.insert(
            header_name.clone(),
            serde_json::Value::String(header_value.clone()),
        );
    }
    let headers_json = serde_json::Value::Object(headers_map).to_string();
    let headers_value = ctx.json_parse(headers_json.as_bytes())?;
    ctx.globals().set("__tempHeaders", headers_value)?;
    ctx.eval::<(), _>(
        r#"
        (function() {
            var h = __tempHeaders;
            Object.defineProperty(h, "get", {
                value: function(name) {
                    var nameLower = String(name).toLowerCase();
                    for (var key in this) {
                        if (key.toLowerCase() === nameLower) return this[key];
                    }
                    return undefined;
                },
                enumerable: false, writable: true, configurable: true
            });
            Object.defineProperty(h, "has", {
                value: function(name) {
                    var nameLower = String(name).toLowerCase();
                    for (var key in this) {
                        if (key.toLowerCase() === nameLower) return true;
                    }
                    return false;
                },
                enumerable: false, writable: true, configurable: true
            });
        })();
    "#,
    )?;
    let headers_obj: Object = ctx.globals().get("__tempHeaders")?;
    let _ = ctx.globals().remove("__tempHeaders");
    response_obj.prop("headers", headers_obj)?;
    pm.prop("response", response_obj)?;

    ctx.globals().set("pm", pm)?;
    ctx.eval::<(), _>(
        r#"
        (function() {
            var rawText = pm.response.__beamRawText || "";
            var parseAttempted = false;
            var cachedJson;
            pm.response.text = function() { return rawText; };
            function ensureJsonParsed() {
                if (parseAttempted) return cachedJson;
                parseAttempted = true;
                try {
                    cachedJson = JSON.parse(rawText);
                } catch (e) {
                    console.warn("Response body is not valid JSON: " + e + ". pm.response.json will return undefined.");
                    cachedJson = undefined;
                }
                return cachedJson;
            }
            var jsonFn = function() { return ensureJsonParsed(); };
            var proxy = new Proxy(jsonFn, {
                apply: function() { return ensureJsonParsed(); },
                get: function(target, prop) {
                    if (prop === "name" || prop === "length" || prop === "prototype") return target[prop];
                    var parsed = ensureJsonParsed();
                    if (parsed === null || parsed === undefined) return undefined;
                    return parsed[prop];
                },
                has: function(target, prop) {
                    var parsed = ensureJsonParsed();
                    if (parsed === null || parsed === undefined) return false;
                    return prop in parsed;
                },
                ownKeys: function() {
                    var parsed = ensureJsonParsed();
                    if (parsed === null || parsed === undefined) return [];
                    return Reflect.ownKeys(parsed);
                }
            });
            pm.response.json = proxy;
        })();
    "#,
    )?;

    ctx.eval::<(), _>(
        r#"
        (function() {
            pm.extract = function(sourcePath, envVarName, defaultValue) {
                var hasDefault = arguments.length >= 3;
                var value;
                var pathStr = String(sourcePath || "");
                if (pathStr === "status") {
                    value = pm.response.status;
                } else if (pathStr.toLowerCase().startsWith("header:")) {
                    var headerName = pathStr.substring(7).toLowerCase();
                    value = pm.response.headers.get(headerName);
                } else {
                    var cleanPath = pathStr;
                    if (cleanPath.indexOf("$.") === 0) cleanPath = cleanPath.substring(2);
                    else if (cleanPath.indexOf("$") === 0) cleanPath = cleanPath.substring(1);
                    var parts = cleanPath.split(".");
                    var current = pm.response.json();
                    for (var i = 0; i < parts.length; i++) {
                        var part = parts[i];
                        if (current === null || current === undefined) break;
                        if (Array.isArray(current)) {
                            var idx = parseInt(part, 10);
                            if (!isNaN(idx) && idx >= 0 && idx < current.length) current = current[idx];
                            else { current = undefined; break; }
                        } else {
                            current = current[part];
                        }
                    }
                    value = current;
                }
                if (value === undefined) {
                    if (!hasDefault) {
                        console.warn('Extract warning: "' + pathStr + '" not found; no default provided. Skipped.');
                        return undefined;
                    }
                    value = defaultValue;
                }
                pm.environment.set(envVarName, value);
                return value;
            };
        })();
    "#,
    )?;

    {
        let fail_message_clone = Arc::clone(&fail_message);
        let set_fail_fn = Func::new(move |msg: String| {
            if let Ok(mut fail) = fail_message_clone.lock() {
                *fail = Some(msg);
            }
        });
        ctx.globals().set("__pmSetFailMessage", set_fail_fn)?;
    }
    ctx.eval::<(), _>(
        r#"
        (function() {
            pm.fail = function(message) {
                var msg = message || "Script failed";
                __pmSetFailMessage(msg);
                var err = new Error(msg);
                err._isPmFail = true;
                throw err;
            };
        })();
    "#,
    )?;

    {
        let test_results_clone = Arc::clone(&test_results);
        let record_test_fn = Func::new(
            move |name: String,
                  passed: bool,
                  expected: Option<String>,
                  actual: Option<String>,
                  error_message: Option<String>| {
                if let Ok(mut results) = test_results_clone.lock() {
                    results.push(TestResult {
                        name,
                        passed,
                        expected,
                        actual,
                        error_message,
                    });
                }
            },
        );
        ctx.globals().set("__recordTest", record_test_fn)?;
    }

    ctx.eval::<(), _>(
        r#"
        (function() {
            var recordTest = __recordTest;
            function runTest(name, testFn) {
                var passed = false;
                var error = null;
                try {
                    passed = !!testFn();
                } catch (e) {
                    if (e && e._isPmFail) {
                        recordTest(name, false, null, null, e.message);
                        throw e;
                    }
                    error = String(e);
                    passed = false;
                }
                recordTest(name, passed, null, null, error);
            }
            function deepEqual(a, b) {
                if (a === b) return true;
                if (a === null || b === null || a === undefined || b === undefined) return false;
                if (typeof a !== typeof b) return false;
                if (typeof a !== "object") return false;
                var aIsArr = Array.isArray(a);
                var bIsArr = Array.isArray(b);
                if (aIsArr !== bIsArr) return false;
                if (aIsArr) {
                    if (a.length !== b.length) return false;
                    for (var i = 0; i < a.length; i++) if (!deepEqual(a[i], b[i])) return false;
                    return true;
                }
                var aKeys = Object.keys(a);
                var bKeys = Object.keys(b);
                if (aKeys.length !== bKeys.length) return false;
                for (var i = 0; i < aKeys.length; i++) {
                    var k = aKeys[i];
                    if (!b.hasOwnProperty(k)) return false;
                    if (!deepEqual(a[k], b[k])) return false;
                }
                return true;
            }
            function resolveJsonPath(path) {
                var cleanPath = String(path || "");
                if (cleanPath.indexOf("$.") === 0) cleanPath = cleanPath.substring(2);
                else if (cleanPath.indexOf("$") === 0) cleanPath = cleanPath.substring(1);
                var parts = cleanPath.split(".");
                var current = pm.response.json();
                for (var i = 0; i < parts.length; i++) {
                    var part = parts[i];
                    if (current === null || current === undefined) break;
                    if (Array.isArray(current)) {
                        var idx = parseInt(part, 10);
                        if (!isNaN(idx) && idx >= 0 && idx < current.length) current = current[idx];
                        else { current = undefined; break; }
                    } else {
                        current = current[part];
                    }
                }
                return current;
            }
            function test(name, testFn) {
                if (typeof testFn !== "function") {
                    recordTest(name, false, null, null, "Test callback must be a function");
                    return;
                }
                runTest(name, testFn);
            }
            test.status = function(expected) {
                var actual = pm.response.status;
                var passed = actual === expected;
                var msg = passed ? null : ("Expected status " + expected + " but got " + actual);
                recordTest("Status code is " + expected, passed, String(expected), String(actual), msg);
            };
            test.statusOneOf = function(expectedArray) {
                var actual = pm.response.status;
                var passed = false;
                for (var i = 0; i < expectedArray.length; i++) {
                    if (actual === expectedArray[i]) { passed = true; break; }
                }
                var list = "[" + expectedArray.join(", ") + "]";
                var msg = passed ? null : ("Expected status to be one of " + list + " but got " + actual);
                recordTest("Status code is one of " + list, passed, list, String(actual), msg);
            };
            test.json = function(path, expected, message) {
                var actual = resolveJsonPath(path);
                var passed = deepEqual(actual, expected);
                var name = message || ("JSON path " + path + " equals " + JSON.stringify(expected));
                recordTest(name, passed, JSON.stringify(expected), JSON.stringify(actual), null);
            };
            test.jsonExists = function(path) {
                var actual = resolveJsonPath(path);
                var passed = actual !== undefined;
                var msg = passed ? null : ("JSON path " + path + " does not exist");
                recordTest("JSON path " + path + " exists", passed, "exists", passed ? "exists" : "missing", msg);
            };
            test.header = function(name, expected) {
                var actual = pm.response.headers.get(name);
                var passed = actual === expected;
                var msg = passed ? null : ("Expected header " + name + " to be " + JSON.stringify(expected) + " but got " + JSON.stringify(actual));
                recordTest("Header " + name + " is " + JSON.stringify(expected), passed, JSON.stringify(expected), JSON.stringify(actual), msg);
            };
            test.responseTimeLessThan = function(ms) {
                var actual = pm.response.responseTime;
                var passed = actual < ms;
                var msg = passed ? null : ("Expected response time < " + ms + "ms but got " + actual + "ms");
                recordTest("Response time < " + ms + "ms", passed, "< " + ms + "ms", String(actual) + "ms", msg);
            };
            pm.test = test;
        })();
    "#,
    )?;
    let _ = ctx.globals().remove("__recordTest");
    Ok(())
}
