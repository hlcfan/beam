use beam::types::{Environment, ResponseData, RequestConfig};
use std::collections::BTreeMap;
use log::{error, info};
use rquickjs::{Context, Runtime, Object, function::Func};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct ScriptExecutionResult {
    pub success: bool,
    pub error_message: Option<String>,
    pub environment_changes: BTreeMap<String, String>,
    pub test_results: Vec<TestResult>,
    pub console_output: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub error_message: Option<String>,
}

pub fn execute_post_request_script(
    script: &str,
    request: RequestConfig,
    response: ResponseData,
    environment: &Environment,
) -> ScriptExecutionResult {
    info!("Executing post-request script: {}", script);

    let mut result = ScriptExecutionResult {
        success: false,
        error_message: None,
        environment_changes: BTreeMap::new(),
        test_results: Vec::new(),
        console_output: Vec::new(),
    };

    // Basic script validation
    if script.trim().is_empty() {
        result.success = true;
        result.console_output.push("No script to execute".to_string());
        return result;
    }

    // Check for common syntax issues
    if script.contains("pm.response.json") && !script.contains("JSON.parse") {
        result.console_output.push("Hint: pm.response.json() returns a JSON string. Use JSON.parse() to convert it to an object.".to_string());
    }

    // Create a new runtime for this execution
    let runtime = match Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            result.error_message = Some(format!("Failed to create JavaScript runtime: {}", e));
            return result;
        }
    };

    // Create a new context for this execution
    let context = match Context::full(&runtime) {
        Ok(ctx) => ctx,
        Err(e) => {
            result.error_message = Some(format!("Failed to create JavaScript context: {}", e));
            return result;
        }
    };

    // Create shared state for capturing outputs (needs to be outside context.with)
    let console_output_shared = Arc::new(Mutex::new(Vec::new()));
    let env_changes_shared = Arc::new(Mutex::new(BTreeMap::new()));
    let test_results_shared = Arc::new(Mutex::new(Vec::new()));

    // Execute the script within the context
    let execution_result = context.with(|ctx| {
        // Add initial console message
        result.console_output.push("Starting script execution...".to_string());

        // Setup global objects with better error handling
        match setup_global_objects(&ctx, &request, &response, environment, 
                                   Arc::clone(&console_output_shared),
                                   Arc::clone(&env_changes_shared),
                                   Arc::clone(&test_results_shared)) {
            Ok(_) => {
                info!("Global objects setup complete");
            }
            Err(e) => {
                let setup_error = format!("Failed to setup global objects: {:?}", e);
                result.error_message = Some(setup_error.clone());
                result.console_output.push(setup_error);
                return Err(e);
            }
        }

        // Execute the script
        match ctx.eval::<(), _>(script) {
            Ok(_) => {
                result.success = true;
                Ok(())
            }
            Err(e) => {
                // Try to get more detailed error information
                let error_msg = match e.to_string() {
                    msg if !msg.is_empty() => format!("JavaScript execution error: {}", msg),
                    _ => format!("JavaScript execution error: {:?}", e)
                };
                result.error_message = Some(error_msg.clone());
                result.console_output.push(error_msg.clone());

                // Also log the script content for debugging
                error!("Script execution failed. Script content: {}", script);
                error!("Error details: {:?}", e);

                Err(e)
            }
        }
    });

    // Capture the results AFTER script execution
    let captured_console = console_output_shared.lock().unwrap().clone();
    result.console_output.extend(captured_console);
    result.environment_changes = env_changes_shared.lock().unwrap().clone();
    result.test_results = test_results_shared.lock().unwrap().clone();
    
    info!("===After script execution, environment changes: {:?}", result.environment_changes);

    if let Err(e) = execution_result {
        error!("Script execution failed: {:?}", e);
    }

    result
}

fn setup_global_objects(
    ctx: &rquickjs::Ctx,
    _request: &RequestConfig,
    response: &ResponseData,
    environment: &Environment,
    console_output: Arc<Mutex<Vec<String>>>,
    env_changes: Arc<Mutex<BTreeMap<String, String>>>,
    test_results: Arc<Mutex<Vec<TestResult>>>,
) -> Result<(), rquickjs::Error> {

    // Clone response data for closures
    let response_status = response.status;
    let response_status_text = response.status_text.clone();
    let response_body = response.body.clone();
    let env_vars = environment.variables.clone();

    // Create console object
    let console = Object::new(ctx.clone())?;

    // Add console.log method - simple version without Result return
    let console_output_clone = Arc::clone(&console_output);
    let log_fn = Func::new(move |msg: String| {
        let mut output = console_output_clone.lock().unwrap();
        output.push(format!("[LOG] {}", msg));
        info!("[SCRIPT LOG] {}", msg);
    });
    console.prop("log", log_fn)?;

    // Add console.error method - simple version without Result return
    let console_output_clone = Arc::clone(&console_output);
    let error_fn = Func::new(move |msg: String| {
        let mut output = console_output_clone.lock().unwrap();
        output.push(format!("[ERROR] {}", msg));
        error!("[SCRIPT ERROR] {}", msg);
    });
    console.prop("error", error_fn)?;

    // Add console object to global scope
    ctx.globals().set("console", console)?;

    // Create pm object
    let pm = Object::new(ctx.clone())?;

    // Create pm.environment object
    let env_obj = Object::new(ctx.clone())?;

    // Add pm.environment.set method - simple version
    let env_changes_clone = Arc::clone(&env_changes);
    let set_fn = Func::new(move |key: String, value: String| {
        let mut changes = env_changes_clone.lock().unwrap();
        changes.insert(key.clone(), value.clone());
        info!("===set to: {:?}", env_changes_clone); //pm.environment.set('token', "123");
    });
    env_obj.prop("set", set_fn)?;

    // Add pm.environment.get method
    let get_fn = Func::new(move |key: String| -> Option<String> {
        env_vars.get(&key).cloned()
    });
    env_obj.prop("get", get_fn)?;

    // Add pm.environment.unset method - simple version
    let env_changes_clone = Arc::clone(&env_changes);
    let unset_fn = Func::new(move |key: String| {
        let mut changes = env_changes_clone.lock().unwrap();
        changes.remove(&key);
    });
    env_obj.prop("unset", unset_fn)?;

    pm.prop("environment", env_obj)?;

    // Create pm.response object
    let response_obj = Object::new(ctx.clone())?;

    // Add response status
    response_obj.prop("status", response_status)?;
    response_obj.prop("statusText", response_status_text)?;

    // Add response.json() method - return JSON string
    let response_body_clone = response_body.clone();
    let json_fn = Func::new(move || -> String {
        match serde_json::from_str::<serde_json::Value>(&response_body_clone) {
            Ok(json_value) => json_value.to_string(),
            Err(_) => "null".to_string()
        }
    });
    response_obj.prop("json", json_fn)?;

    // Add response.text() method
    let text_fn = Func::new(move || -> String {
        response_body.clone()
    });
    response_obj.prop("text", text_fn)?;

    pm.prop("response", response_obj)?;

    // Add pm.test method for test assertions - simplified version
    let test_results_clone = Arc::clone(&test_results);
    let test_fn = Func::new(move |name: String, test_func: rquickjs::Function| {
        let test_result = match test_func.call::<_, bool>(()) {
            Ok(passed) => TestResult {
                name: name.clone(),
                passed,
                error_message: if passed { None } else { Some("Test assertion failed".to_string()) },
            },
            Err(e) => TestResult {
                name: name.clone(),
                passed: false,
                error_message: Some(format!("Test execution error: {:?}", e)),
            },
        };

        let mut results = test_results_clone.lock().unwrap();
        results.push(test_result);
    });
    pm.prop("test", test_fn)?;

    // Add pm object to global scope
    ctx.globals().set("pm", pm)?;

    Ok(())
}
