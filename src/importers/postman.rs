use serde_json::Value;
use ulid::Ulid;

use crate::error::BeamError;
use crate::importers::{
    DetectedSource, Detector, ImportPlan, Parser, PlannedEnvironment, PlannedFolder, PlannedRequest,
};
use crate::models::{
    ApiKeyLocation, AuthConfig, BodyConfig, EnvironmentVariable, HeaderField, HttpMethod,
    QueryParamField, ScriptConfig,
};

/// Detects Postman Collection (v2.0/v2.1) and Postman Environment exports
/// from their JSON content.
pub struct PostmanDetector;

impl Detector for PostmanDetector {
    fn detect(&self, content: &str, _ext_hint: Option<&str>) -> DetectedSource {
        let trimmed = content.trim();
        if !trimmed.starts_with('{') {
            return DetectedSource::Unknown;
        }

        let root: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(_) => return DetectedSource::Unknown,
        };

        let obj = match root.as_object() {
            Some(obj) => obj,
            None => return DetectedSource::Unknown,
        };

        // An Insomnia export is identified by `__export_format` or `_type == "export"`,
        // so anything carrying those signals is not a Postman file.
        if obj.contains_key("__export_format")
            || obj.get("_type").and_then(|v| v.as_str()) == Some("export")
        {
            return DetectedSource::Unknown;
        }

        // Postman Collection v2.0 / v2.1 — explicit schema URL.
        let collection_schema = obj
            .get("info")
            .and_then(|info| info.get("schema"))
            .and_then(|v| v.as_str());
        if let Some(schema) = collection_schema.filter(|schema| {
            schema.starts_with("https://schema.getpostman.com/json/collection/v2.")
        }) {
            return DetectedSource::PostmanCollection {
                schema: schema.to_string(),
            };
        }

        // Postman Environment — explicit marker.
        if obj.get("_postman_variable_scope").and_then(|v| v.as_str()) == Some("environment") {
            return DetectedSource::PostmanEnvironment;
        }

        // Postman Environment — heuristic fallback: a `name` plus a non-empty
        // `values[]` array, with no collection-shape keys present.
        let has_name = obj.get("name").and_then(|v| v.as_str()).is_some();
        let has_non_empty_values = obj
            .get("values")
            .and_then(|v| v.as_array())
            .map(|arr| !arr.is_empty())
            .unwrap_or(false);
        if has_name
            && has_non_empty_values
            && !obj.contains_key("info")
            && !obj.contains_key("item")
        {
            return DetectedSource::PostmanEnvironment;
        }

        // Generic Postman Collection fallback — has both `info` and `item`
        // but no recognizable schema URL (already excluded above).
        if obj.contains_key("info") && obj.contains_key("item") {
            return DetectedSource::PostmanCollection {
                schema: "v2?".to_string(),
            };
        }

        DetectedSource::Unknown
    }
}

/// Parses a Postman Collection (v2.0 / v2.1) export into an [`ImportPlan`].
///
/// Pure: no I/O, no UI, no repository calls. Per the "Mapping Tables" rules
/// in the import plan, an `item` that has both sub-`item[]` and a `request`
/// block is treated as a folder (request dropped + warning).
pub struct PostmanCollectionParser;

impl Parser for PostmanCollectionParser {
    fn parse(&self, content: &str) -> Result<ImportPlan, BeamError> {
        let root: Value = serde_json::from_str(content).map_err(|e| BeamError::Validation {
            message: format!("invalid Postman collection JSON: {e}"),
        })?;
        let obj = root.as_object().ok_or_else(|| BeamError::Validation {
            message: "Postman collection root must be a JSON object".to_string(),
        })?;

        let workspace_name = obj
            .get("info")
            .and_then(|i| i.get("name"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "Imported Postman Collection".to_string());

        let mut plan = ImportPlan {
            workspace_name,
            folders: Vec::new(),
            requests: Vec::new(),
            environments: Vec::new(),
            warnings: Vec::new(),
            needs_new_workspace: false,
        };

        // Collection-level `auth` is the default for every descendant.
        let root_auth = parse_auth(obj.get("auth"));

        // Inline collection `variable[]` with `type == "default"` becomes a
        // single PlannedEnvironment named after the collection. (Standalone
        // env files are handled by PostmanEnvironmentParser.)
        if let Some(vars) = obj.get("variable").and_then(|v| v.as_array()) {
            let env_vars: Vec<EnvironmentVariable> = vars
                .iter()
                .filter_map(|v| {
                    let vo = v.as_object()?;
                    let typ = vo.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    if typ != "default" {
                        return None;
                    }
                    let name = vo
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string();
                    if name.is_empty() {
                        return None;
                    }
                    let value = value_to_string(vo.get("value"));
                    let enabled = !vo
                        .get("disabled")
                        .and_then(|d| d.as_bool())
                        .unwrap_or(false);
                    Some(EnvironmentVariable {
                        name,
                        value,
                        enabled,
                        description: None,
                    })
                })
                .collect();
            if !env_vars.is_empty() {
                plan.environments.push(PlannedEnvironment {
                    name: plan.workspace_name.clone(),
                    variables: env_vars,
                });
            }
        }

        if let Some(items) = obj.get("item").and_then(|v| v.as_array()) {
            for item in items {
                process_item(item, None, root_auth.clone(), &mut plan);
            }
        }

        Ok(plan)
    }
}

/// Parses a Postman Environment export (`_postman_variable_scope ==
/// "environment"` or the heuristic env shape) into an [`ImportPlan`] that
/// contains only a single [`PlannedEnvironment`] — no folders, no requests.
/// Workspace name = environment `name`.
pub struct PostmanEnvironmentParser;

impl Parser for PostmanEnvironmentParser {
    fn parse(&self, content: &str) -> Result<ImportPlan, BeamError> {
        let root: Value = serde_json::from_str(content).map_err(|e| BeamError::Validation {
            message: format!("invalid Postman environment JSON: {e}"),
        })?;
        let obj = root.as_object().ok_or_else(|| BeamError::Validation {
            message: "Postman environment root must be a JSON object".to_string(),
        })?;

        let name = obj
            .get("name")
            .and_then(|n| n.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "Imported Postman Environment".to_string());

        let mut variables = Vec::new();
        if let Some(values) = obj.get("values").and_then(|v| v.as_array()) {
            for v in values {
                let vo = match v.as_object() {
                    Some(o) => o,
                    None => continue,
                };
                let var_name = vo
                    .get("key")
                    .and_then(|k| k.as_str())
                    .unwrap_or("")
                    .to_string();
                if var_name.is_empty() {
                    continue;
                }
                let value = value_to_string(vo.get("value"));
                let enabled = vo.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true);
                // `values[].type` (e.g. "secret", "default", "string") is ignored.
                variables.push(EnvironmentVariable {
                    name: var_name,
                    value,
                    enabled,
                    description: None,
                });
            }
        }

        Ok(ImportPlan {
            workspace_name: name.clone(),
            folders: Vec::new(),
            requests: Vec::new(),
            environments: vec![PlannedEnvironment { name, variables }],
            warnings: Vec::new(),
            needs_new_workspace: false,
        })
    }
}

// ---------------------------------------------------------------------------
// Collection parsing helpers
// ---------------------------------------------------------------------------

fn next_order(plan: &ImportPlan) -> usize {
    plan.folders.len() + plan.requests.len()
}

fn process_item(
    item: &Value,
    parent_id: Option<Ulid>,
    inherited_auth: AuthConfig,
    plan: &mut ImportPlan,
) {
    let item_obj = match item.as_object() {
        Some(o) => o,
        None => return,
    };
    let name = item_obj
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unnamed")
        .to_string();

    let is_folder = item_obj.contains_key("item");
    let request_value = item_obj.get("request");
    let has_request = request_value.map(|r| !r.is_null()).unwrap_or(false);

    // Auth inheritance: a child's explicit auth overrides; `auth: null` clears.
    // Postman v2.1 places `auth` at the item level (sibling of `request`); some
    // third-party exports put auth inside the `request` object. We check both,
    // preferring the item-level definition.
    let request_auth = request_value
        .and_then(|r| r.as_object())
        .and_then(|ro| ro.get("auth"));
    let auth_source = item_obj.get("auth").or(request_auth);
    let effective_auth = match auth_source {
        None => inherited_auth.clone(),
        Some(Value::Null) => AuthConfig::None,
        Some(a) => parse_auth(Some(a)),
    };

    if is_folder {
        if has_request {
            plan.warnings.push(format!(
                "Item \"{}\" has both sub-items and a request; the request block was dropped.",
                name
            ));
        }
        let folder_id = Ulid::new();
        let order = next_order(plan);
        plan.folders.push(PlannedFolder {
            id: folder_id,
            parent_id,
            name: name.clone(),
            order,
        });
        if let Some(subs) = item_obj.get("item").and_then(|v| v.as_array()) {
            for sub in subs {
                process_item(sub, Some(folder_id), effective_auth.clone(), plan);
            }
        }
    } else if has_request {
        let id = Ulid::new();
        let order = next_order(plan);
        let planned = build_request(
            id,
            parent_id,
            name,
            order,
            item,
            effective_auth,
            &mut plan.warnings,
        );
        plan.requests.push(planned);
    } else {
        // An item with only `name` (or with an empty `item` array) is treated
        // as an empty folder so it still shows up in the workspace tree.
        let folder_id = Ulid::new();
        let order = next_order(plan);
        plan.folders.push(PlannedFolder {
            id: folder_id,
            parent_id,
            name: name.clone(),
            order,
        });
    }
}

fn build_request(
    id: Ulid,
    parent_id: Option<Ulid>,
    name: String,
    order: usize,
    item: &Value,
    auth: AuthConfig,
    warnings: &mut Vec<String>,
) -> PlannedRequest {
    let scripts = parse_scripts(item, &name, warnings);
    let empty = serde_json::Map::new();
    let req = item
        .as_object()
        .and_then(|o| o.get("request"))
        .and_then(|r| r.as_object())
        .unwrap_or(&empty);

    let method_str = req.get("method").and_then(|m| m.as_str()).unwrap_or("GET");
    let method = parse_method(method_str).unwrap_or_else(|| {
        warnings.push(format!(
            "Unknown HTTP method \"{}\" in \"{}\" — defaulted to GET.",
            method_str, name
        ));
        HttpMethod::Get
    });

    let (url, query) = parse_url(req.get("url"));
    let headers = parse_headers(req.get("header"));
    let body = parse_body(req.get("body"), &name, warnings);

    PlannedRequest {
        id,
        parent_id,
        name,
        method,
        url,
        headers,
        query,
        auth,
        body,
        scripts,
        order,
    }
}

fn parse_method(s: &str) -> Option<HttpMethod> {
    match s.to_ascii_uppercase().as_str() {
        "GET" => Some(HttpMethod::Get),
        "POST" => Some(HttpMethod::Post),
        "PUT" => Some(HttpMethod::Put),
        "DELETE" => Some(HttpMethod::Delete),
        "PATCH" => Some(HttpMethod::Patch),
        "HEAD" => Some(HttpMethod::Head),
        "OPTIONS" => Some(HttpMethod::Options),
        "QUERY" => Some(HttpMethod::Query),
        _ => None,
    }
}

fn parse_url(url_value: Option<&Value>) -> (String, Vec<QueryParamField>) {
    match url_value {
        Some(Value::String(s)) => (s.clone(), Vec::new()),
        Some(Value::Object(obj)) => {
            let raw = obj
                .get("raw")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mut query = Vec::new();
            if let Some(q) = obj.get("query").and_then(|v| v.as_array()) {
                for qv in q {
                    let qobj = match qv.as_object() {
                        Some(o) => o,
                        None => continue,
                    };
                    let name = qobj
                        .get("key")
                        .and_then(|k| k.as_str())
                        .unwrap_or("")
                        .to_string();
                    if name.is_empty() {
                        continue;
                    }
                    let value = value_to_string(qobj.get("value"));
                    let disabled = qobj
                        .get("disabled")
                        .and_then(|d| d.as_bool())
                        .unwrap_or(false);
                    query.push(QueryParamField {
                        name,
                        value,
                        enabled: !disabled,
                        description: None,
                    });
                }
            }
            (raw, query)
        }
        _ => (String::new(), Vec::new()),
    }
}

fn parse_headers(headers_value: Option<&Value>) -> Vec<HeaderField> {
    let mut headers = Vec::new();
    if let Some(arr) = headers_value.and_then(|v| v.as_array()) {
        for hv in arr {
            let hobj = match hv.as_object() {
                Some(o) => o,
                None => continue,
            };
            let name = hobj
                .get("key")
                .and_then(|k| k.as_str())
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                continue;
            }
            let value = value_to_string(hobj.get("value"));
            let disabled = hobj
                .get("disabled")
                .and_then(|d| d.as_bool())
                .unwrap_or(false);
            headers.push(HeaderField {
                name,
                value,
                enabled: !disabled,
                description: None,
            });
        }
    }
    headers
}

fn parse_auth(auth: Option<&Value>) -> AuthConfig {
    match auth {
        None | Some(Value::Null) => AuthConfig::None,
        Some(a) => {
            let obj = match a.as_object() {
                Some(o) => o,
                None => return AuthConfig::None,
            };
            let typ = obj.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match typ {
                "bearer" => {
                    let token = find_auth_value(obj.get("bearer"), "token");
                    AuthConfig::Bearer { token: Some(token) }
                }
                "basic" => {
                    let username = find_auth_value(obj.get("basic"), "username");
                    let password = find_auth_value(obj.get("basic"), "password");
                    AuthConfig::Basic {
                        username: Some(username),
                        password: Some(password),
                    }
                }
                "apikey" => {
                    let key = find_auth_value(obj.get("apikey"), "key");
                    let value = find_auth_value(obj.get("apikey"), "value");
                    let location_str = find_auth_value(obj.get("apikey"), "in");
                    let location = match location_str.as_str() {
                        "query" => ApiKeyLocation::Query,
                        _ => ApiKeyLocation::Header,
                    };
                    AuthConfig::ApiKey {
                        key: Some(key),
                        value: Some(value),
                        location,
                    }
                }
                _ => AuthConfig::None,
            }
        }
    }
}

/// Postman `auth.<type>` is an array of `{ key, value, type }` entries; pick
/// the entry whose `key` matches `key_name` and return its `value` string.
fn find_auth_value(arr: Option<&Value>, key_name: &str) -> String {
    arr.and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter().find_map(|entry| {
                let e = entry.as_object()?;
                if e.get("key").and_then(|k| k.as_str()) == Some(key_name) {
                    Some(value_to_string(e.get("value")))
                } else {
                    None
                }
            })
        })
        .unwrap_or_default()
}

fn parse_body(body: Option<&Value>, request_name: &str, warnings: &mut Vec<String>) -> BodyConfig {
    let body_obj = match body.and_then(|b| b.as_object()) {
        Some(o) => o,
        None => return BodyConfig::None,
    };
    let mode = body_obj.get("mode").and_then(|m| m.as_str()).unwrap_or("");
    match mode {
        "raw" => {
            let raw = body_obj.get("raw").and_then(|r| r.as_str()).unwrap_or("");
            let language = body_obj
                .get("options")
                .and_then(|o| o.get("raw"))
                .and_then(|r| r.get("language"))
                .and_then(|l| l.as_str())
                .unwrap_or("");
            match language {
                "json" => BodyConfig::Json {
                    text: raw.to_string(),
                },
                "xml" => BodyConfig::Xml {
                    text: raw.to_string(),
                },
                _ => BodyConfig::Raw {
                    media_type: language_to_media_type(language),
                    text: raw.to_string(),
                },
            }
        }
        "urlencoded" => {
            let fields = parse_form_fields(body_obj.get("urlencoded"));
            BodyConfig::FormUrlEncoded { fields }
        }
        "formdata" => {
            let (fields, file_count) = parse_formdata(body_obj.get("formdata"));
            if file_count > 0 {
                warnings.push(format!(
                    "Skipped {} file-upload field(s) in \"{}\" — binary uploads unsupported in v1",
                    file_count, request_name
                ));
            }
            BodyConfig::Multipart { fields }
        }
        "graphql" => {
            let gql = body_obj.get("graphql").and_then(|g| g.as_object());
            let query = gql
                .and_then(|g| g.get("query"))
                .and_then(|q| q.as_str())
                .unwrap_or("")
                .to_string();
            let variables_value = gql
                .and_then(|g| g.get("variables"))
                .cloned()
                .unwrap_or(Value::Null);
            // Preserve `{ "query": ..., "variables": ... }` verbatim.
            let payload = serde_json::json!({ "query": query, "variables": variables_value });
            warnings.push(format!(
                "GraphQL body in \"{}\" imported as JSON — variables are not separately editable in v1.",
                request_name
            ));
            BodyConfig::Json {
                text: serde_json::to_string(&payload).unwrap_or_default(),
            }
        }
        "" => BodyConfig::None,
        other => {
            warnings.push(format!(
                "Unknown body mode \"{}\" in \"{}\" — imported as Text.",
                other, request_name
            ));
            BodyConfig::Raw {
                media_type: None,
                text: String::new(),
            }
        }
    }
}

fn language_to_media_type(language: &str) -> Option<String> {
    match language {
        "" => None,
        "text" => Some("text/plain".to_string()),
        "html" => Some("text/html".to_string()),
        "javascript" => Some("application/javascript".to_string()),
        "json" => Some("application/json".to_string()),
        "xml" => Some("application/xml".to_string()),
        other => Some(other.to_string()),
    }
}

fn parse_form_fields(arr: Option<&Value>) -> Vec<QueryParamField> {
    let mut fields = Vec::new();
    if let Some(arr) = arr.and_then(|v| v.as_array()) {
        for ev in arr {
            let eobj = match ev.as_object() {
                Some(o) => o,
                None => continue,
            };
            let name = eobj
                .get("key")
                .and_then(|k| k.as_str())
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                continue;
            }
            let value = value_to_string(eobj.get("value"));
            let disabled = eobj
                .get("disabled")
                .and_then(|d| d.as_bool())
                .unwrap_or(false);
            fields.push(QueryParamField {
                name,
                value,
                enabled: !disabled,
                description: None,
            });
        }
    }
    fields
}

fn parse_formdata(arr: Option<&Value>) -> (Vec<QueryParamField>, usize) {
    let mut fields = Vec::new();
    let mut file_count = 0;
    if let Some(arr) = arr.and_then(|v| v.as_array()) {
        for ev in arr {
            let eobj = match ev.as_object() {
                Some(o) => o,
                None => continue,
            };
            let name = eobj
                .get("key")
                .and_then(|k| k.as_str())
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                continue;
            }
            let typ = eobj.get("type").and_then(|t| t.as_str()).unwrap_or("text");
            if typ == "file" {
                file_count += 1;
                continue;
            }
            let value = value_to_string(eobj.get("value"));
            let disabled = eobj
                .get("disabled")
                .and_then(|d| d.as_bool())
                .unwrap_or(false);
            fields.push(QueryParamField {
                name,
                value,
                enabled: !disabled,
                description: None,
            });
        }
    }
    (fields, file_count)
}

fn parse_scripts(item: &Value, request_name: &str, warnings: &mut Vec<String>) -> ScriptConfig {
    let mut post_response: Option<String> = None;
    let events = item
        .as_object()
        .and_then(|o| o.get("event"))
        .and_then(|v| v.as_array());
    let Some(events) = events else {
        return ScriptConfig { post_response };
    };
    for ev in events {
        let eobj = match ev.as_object() {
            Some(o) => o,
            None => continue,
        };
        let listen = eobj.get("listen").and_then(|l| l.as_str()).unwrap_or("");
        let exec = eobj.get("script").and_then(|s| s.get("exec"));
        let script_text = match exec {
            Some(Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        if script_text.trim().is_empty() {
            continue;
        }
        match listen {
            "test" => {
                post_response = Some(match post_response {
                    Some(prev) => format!("{prev}\n\n{script_text}"),
                    None => script_text,
                });
            }
            "prerequest" => {
                warnings.push(format!(
                    "Pre-request script in \"{}\" was imported but will be ignored at run time (pre-request execution unsupported in v1).",
                    request_name
                ));
            }
            _ => {}
        }
    }
    ScriptConfig { post_response }
}

/// Coerces a JSON value into a string for header/query/form values:
/// strings preserved; numbers/bools stringified; null/missing → "".
fn value_to_string(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_postman_collection_v2_0() {
        let content = r#"{
            "info": {
                "_postman_id": "abc",
                "name": "Sample",
                "schema": "https://schema.getpostman.com/json/collection/v2.0.0/collection.json"
            },
            "item": []
        }"#;
        let src = PostmanDetector.detect(content, None);
        match src {
            DetectedSource::PostmanCollection { schema } => {
                assert!(schema.starts_with("https://schema.getpostman.com/json/collection/v2.0"));
            }
            other => panic!("expected PostmanCollection, got {:?}", other),
        }
    }

    #[test]
    fn detects_postman_collection_v2_1() {
        let content = r#"{
            "info": {
                "_postman_id": "abc",
                "name": "Sample",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "item": []
        }"#;
        let src = PostmanDetector.detect(content, None);
        match src {
            DetectedSource::PostmanCollection { schema } => {
                assert!(schema.starts_with("https://schema.getpostman.com/json/collection/v2.1"));
            }
            other => panic!("expected PostmanCollection, got {:?}", other),
        }
    }

    #[test]
    fn detects_postman_environment_explicit_scope() {
        let content = r#"{
            "name": "My Env",
            "_postman_variable_scope": "environment",
            "values": [
                { "key": "BASE_URL", "value": "https://example.com" }
            ]
        }"#;
        assert_eq!(
            PostmanDetector.detect(content, None),
            DetectedSource::PostmanEnvironment
        );
    }

    #[test]
    fn detects_postman_environment_heuristic_fallback() {
        let content = r#"{
            "name": "My Env",
            "values": [
                { "key": "BASE_URL", "value": "https://example.com" }
            ]
        }"#;
        assert_eq!(
            PostmanDetector.detect(content, None),
            DetectedSource::PostmanEnvironment
        );
    }

    #[test]
    fn heuristic_env_requires_non_empty_values() {
        let content = r#"{ "name": "Empty", "values": [] }"#;
        assert_eq!(
            PostmanDetector.detect(content, None),
            DetectedSource::Unknown
        );
    }

    #[test]
    fn heuristic_env_rejects_when_info_present() {
        let content = r#"{
            "name": "Tricky",
            "values": [ { "key": "k", "value": "v" } ],
            "info": { "name": "oops" }
        }"#;
        assert_eq!(
            PostmanDetector.detect(content, None),
            DetectedSource::Unknown
        );
    }

    #[test]
    fn detects_generic_collection_fallback() {
        let content = r#"{
            "info": { "name": "No Schema" },
            "item": [ { "name": "req", "request": {} } ]
        }"#;
        match PostmanDetector.detect(content, None) {
            DetectedSource::PostmanCollection { schema } => assert_eq!(schema, "v2?"),
            other => panic!("expected PostmanCollection fallback, got {:?}", other),
        }
    }

    #[test]
    fn rejects_insomnia_export_payload() {
        let content = r#"{
            "__export_format": 4,
            "info": { "name": "Tricky" },
            "item": []
        }"#;
        assert_eq!(
            PostmanDetector.detect(content, None),
            DetectedSource::Unknown
        );
    }

    #[test]
    fn unknown_for_non_json_or_missing_keys() {
        assert_eq!(
            PostmanDetector.detect("hello world", None),
            DetectedSource::Unknown
        );
        assert_eq!(
            PostmanDetector.detect(r#"{ "foo": "bar" }"#, None),
            DetectedSource::Unknown
        );
        assert_eq!(
            PostmanDetector.detect("[1, 2, 3]", None),
            DetectedSource::Unknown
        );
    }

    #[test]
    fn ignores_extension_hint_for_postman() {
        let content = r#"{
            "info": {
                "name": "Sample",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "item": []
        }"#;
        // A misleading .txt hint must not change detection.
        assert!(matches!(
            PostmanDetector.detect(content, Some("txt")),
            DetectedSource::PostmanCollection { .. }
        ));
    }

    // ---- Phase 2: parser tests ----

    fn find_folder<'a>(plan: &'a ImportPlan, name: &str) -> &'a PlannedFolder {
        plan.folders
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("missing folder \"{name}\""))
    }

    fn find_request<'a>(plan: &'a ImportPlan, name: &str) -> &'a PlannedRequest {
        plan.requests
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("missing request \"{name}\""))
    }

    #[test]
    fn parses_empty_collection_with_fallback_name() {
        let content = r#"{
            "info": { "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" },
            "item": []
        }"#;
        let plan = PostmanCollectionParser.parse(content).unwrap();
        assert_eq!(plan.workspace_name, "Imported Postman Collection");
        assert!(plan.folders.is_empty());
        assert!(plan.requests.is_empty());
        assert!(plan.environments.is_empty());
        assert!(plan.warnings.is_empty());
    }

    #[test]
    fn parses_v21_collection_with_folders_requests_bodies_and_auth() {
        let content = r#"{
            "info": {
                "name": "Pet Store",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "variable": [
                { "name": "BaseUrl", "value": "https://pet.example.com", "type": "default" },
                { "name": "Secret", "value": "topsecret", "type": "secret" }
            ],
            "auth": {
                "type": "bearer",
                "bearer": [ { "key": "token", "value": "root-token", "type": "string" } ]
            },
            "item": [
                {
                    "name": "Folder A",
                    "item": [
                        {
                            "name": "Get Pet (bearer override)",
                            "request": {
                                "method": "GET",
                                "header": [
                                    { "key": "Accept", "value": "application/json" },
                                    { "key": "X-Disabled", "value": "nope", "disabled": true }
                                ],
                                "url": {
                                    "raw": "https://pet.example.com/pets/1?include=owner",
                                    "query": [
                                        { "key": "include", "value": "owner" },
                                        { "key": "archived", "value": "true", "disabled": true }
                                    ]
                                }
                            },
                            "event": [
                                { "listen": "test", "script": { "exec": ["pm.test('ok', () => {});"] } },
                                { "listen": "prerequest", "script": { "exec": "console.log('hi');" } }
                            ]
                        },
                        {
                            "name": "Post Pet (form-urlencoded)",
                            "request": {
                                "method": "POST",
                                "url": "https://pet.example.com/pets",
                                "body": {
                                    "mode": "urlencoded",
                                    "urlencoded": [
                                        { "key": "name", "value": "Rex" },
                                        { "key": "age", "value": "3" }
                                    ]
                                }
                            }
                        },
                        {
                            "name": "Upload (multipart with file skip)",
                            "request": {
                                "method": "POST",
                                "url": "https://pet.example.com/upload",
                                "body": {
                                    "mode": "formdata",
                                    "formdata": [
                                        { "key": "label", "value": "dog", "type": "text" },
                                        { "key": "avatar", "type": "file", "src": "/tmp/dog.png" }
                                    ]
                                }
                            }
                        },
                        {
                            "name": "GraphQL pet",
                            "request": {
                                "method": "POST",
                                "url": "https://pet.example.com/graphql",
                                "body": {
                                    "mode": "graphql",
                                    "graphql": {
                                        "query": "query { pet(id: 1) { name } }",
                                        "variables": "{ \"id\": 1 }"
                                    }
                                }
                            }
                        },
                        {
                            "name": "Raw JSON pet",
                            "request": {
                                "method": "PUT",
                                "url": "https://pet.example.com/pets/1",
                                "body": {
                                    "mode": "raw",
                                    "raw": "{\"name\":\"Rex\"}",
                                    "options": { "raw": { "language": "json" } }
                                }
                            }
                        },
                        {
                            "name": "Raw XML pet",
                            "request": {
                                "method": "PATCH",
                                "url": "https://pet.example.com/pets/1",
                                "body": {
                                    "mode": "raw",
                                    "raw": "<pet/>",
                                    "options": { "raw": { "language": "xml" } }
                                }
                            }
                        },
                        {
                            "name": "Raw text pet",
                            "request": {
                                "method": "DELETE",
                                "url": "https://pet.example.com/pets/1",
                                "body": {
                                    "mode": "raw",
                                    "raw": "plain text body",
                                    "options": { "raw": { "language": "text" } }
                                }
                            }
                        }
                    ]
                },
                {
                    "name": "Folder B (basic + apikey)",
                    "auth": {
                        "type": "basic",
                        "basic": [
                            { "key": "username", "value": "u" },
                            { "key": "password", "value": "p" }
                        ]
                    },
                    "item": [
                        {
                            "name": "Get with basic",
                            "request": {
                                "method": "GET",
                                "url": "https://pet.example.com/secure"
                            }
                        },
                        {
                            "name": "Get with apikey in query",
                            "request": {
                                "method": "GET",
                                "auth": {
                                    "type": "apikey",
                                    "apikey": [
                                        { "key": "key", "value": "X-API-Key" },
                                        { "key": "value", "value": "secret123" },
                                        { "key": "in", "value": "query" }
                                    ]
                                },
                                "url": "https://pet.example.com/api"
                            }
                        },
                        {
                            "name": "Get with explicit null auth",
                            "request": {
                                "method": "GET",
                                "auth": null,
                                "url": "https://pet.example.com/open"
                            }
                        }
                    ]
                },
                {
                    "name": "Folder with request block dropped",
                    "item": [ { "name": "Inner", "request": { "method": "GET", "url": "https://pet.example.com/inner" } } ],
                    "request": { "method": "GET", "url": "https://pet.example.com/should-drop" }
                }
            ]
        }"#;

        let plan = PostmanCollectionParser.parse(content).unwrap();

        // ---- Workspace + environment (only `default`-typed variables) ----
        assert_eq!(plan.workspace_name, "Pet Store");
        assert_eq!(plan.environments.len(), 1);
        let env = &plan.environments[0];
        assert_eq!(env.name, "Pet Store");
        assert_eq!(env.variables.len(), 1);
        assert_eq!(env.variables[0].name, "BaseUrl");
        assert_eq!(env.variables[0].value, "https://pet.example.com");
        assert!(env.variables[0].enabled);

        // ---- Folder A + its 7 children ----
        let folder_a = find_folder(&plan, "Folder A");
        assert_eq!(folder_a.parent_id, None);
        let folder_a_id = folder_a.id;

        // Collection-level bearer auth must reach Folder A's first request if
        // not overridden.
        let get_pet = find_request(&plan, "Get Pet (bearer override)");
        assert_eq!(get_pet.parent_id, Some(folder_a_id));
        assert_eq!(get_pet.method, HttpMethod::Get);
        assert_eq!(get_pet.url, "https://pet.example.com/pets/1?include=owner");
        assert_eq!(
            get_pet.auth,
            AuthConfig::Bearer {
                token: Some("root-token".to_string())
            }
        );
        // Headers: Accept enabled, X-Disabled disabled.
        assert_eq!(get_pet.headers.len(), 2);
        assert_eq!(get_pet.headers[0].name, "Accept");
        assert!(get_pet.headers[0].enabled);
        assert_eq!(get_pet.headers[1].name, "X-Disabled");
        assert!(!get_pet.headers[1].enabled);
        // Query: include enabled, archived disabled.
        assert_eq!(get_pet.query.len(), 2);
        assert_eq!(get_pet.query[0].name, "include");
        assert!(get_pet.query[0].enabled);
        assert_eq!(get_pet.query[1].name, "archived");
        assert!(!get_pet.query[1].enabled);
        // Test script captured; prerequest not (only a warning).
        assert_eq!(
            get_pet.scripts.post_response.as_deref(),
            Some("pm.test('ok', () => {});")
        );

        // Form-urlencoded
        let post_pet = find_request(&plan, "Post Pet (form-urlencoded)");
        let BodyConfig::FormUrlEncoded { fields } = &post_pet.body else {
            panic!("expected FormUrlEncoded body");
        };
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "name");
        assert_eq!(fields[0].value, "Rex");

        // Multipart with skipped file + warning
        let upload = find_request(&plan, "Upload (multipart with file skip)");
        let BodyConfig::Multipart { fields } = &upload.body else {
            panic!("expected Multipart body");
        };
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "label");
        assert!(plan.warnings.iter().any(|w| {
            w.contains("Skipped 1 file-upload field(s) in \"Upload (multipart with file skip)\"")
        }));

        // GraphQL → JSON body carrying query+variables verbatim + warning
        let gql = find_request(&plan, "GraphQL pet");
        let BodyConfig::Json { text } = &gql.body else {
            panic!("expected Json body for graphql");
        };
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["query"], "query { pet(id: 1) { name } }");
        assert_eq!(parsed["variables"], "{ \"id\": 1 }");
        assert!(
            plan.warnings
                .iter()
                .any(|w| w.contains("GraphQL body in \"GraphQL pet\" imported as JSON"))
        );

        // Raw JSON / XML / Text bodies
        let raw_json = find_request(&plan, "Raw JSON pet");
        assert!(
            matches!(&raw_json.body, BodyConfig::Json { text } if text == "{\"name\":\"Rex\"}")
        );
        let raw_xml = find_request(&plan, "Raw XML pet");
        assert!(matches!(&raw_xml.body, BodyConfig::Xml { text } if text == "<pet/>"));
        let raw_text = find_request(&plan, "Raw text pet");
        assert!(
            matches!(&raw_text.body, BodyConfig::Raw { media_type, text } if media_type.as_deref() == Some("text/plain") && text == "plain text body")
        );

        // ---- Folder B inherits folder-level basic auth ----
        let folder_b = find_folder(&plan, "Folder B (basic + apikey)");
        let folder_b_id = folder_b.id;
        let basic_req = find_request(&plan, "Get with basic");
        assert_eq!(
            basic_req.auth,
            AuthConfig::Basic {
                username: Some("u".to_string()),
                password: Some("p".to_string())
            }
        );

        // Per-request apikey auth overrides folder basic.
        let apikey_req = find_request(&plan, "Get with apikey in query");
        assert_eq!(
            apikey_req.auth,
            AuthConfig::ApiKey {
                key: Some("X-API-Key".to_string()),
                value: Some("secret123".to_string()),
                location: ApiKeyLocation::Query
            }
        );

        // Explicit `auth: null` clears inheritance.
        let null_req = find_request(&plan, "Get with explicit null auth");
        assert_eq!(null_req.auth, AuthConfig::None);
        assert_eq!(null_req.parent_id, Some(folder_b_id));

        // ---- Folder that had both sub-items and a request ----
        let dropped_folder = find_folder(&plan, "Folder with request block dropped");
        assert_eq!(dropped_folder.parent_id, None);
        // Inner request still imported; outer request dropped + a warning.
        assert!(plan.requests.iter().any(|r| r.name == "Inner"));
        assert!(
            !plan
                .requests
                .iter()
                .any(|r| r.url == "https://pet.example.com/should-drop")
        );
        assert!(
            plan.warnings
                .iter()
                .any(|w| w.contains("\"Folder with request block dropped\""))
        );

        // Prerequest script warning must appear exactly once.
        assert!(
            plan.warnings
                .iter()
                .any(|w| w.contains("Pre-request script in \"Get Pet (bearer override)\""))
        );
    }

    #[test]
    fn parses_postman_environment_with_secret_and_missing_value_entries() {
        let content = r#"{
            "name": "Staging env",
            "_postman_variable_scope": "environment",
            "values": [
                { "key": "BaseUrl", "value": "https://staging.example.com", "enabled": true },
                { "key": "Secret", "value": "supersecret", "type": "secret", "enabled": true },
                { "key": "Disabled", "value": "off", "enabled": false },
                { "key": "NoValue", "type": "string", "enabled": true, "value": null },
                { "key": "NoEnabledFlag", "value": "fallback" }
            ]
        }"#;

        let plan = PostmanEnvironmentParser.parse(content).unwrap();
        assert_eq!(plan.workspace_name, "Staging env");
        assert!(plan.folders.is_empty());
        assert!(plan.requests.is_empty());
        assert_eq!(plan.environments.len(), 1);
        assert!(plan.warnings.is_empty());

        let env = &plan.environments[0];
        assert_eq!(env.name, "Staging env");
        assert_eq!(env.variables.len(), 5);

        assert_eq!(env.variables[0].name, "BaseUrl");
        assert_eq!(env.variables[0].value, "https://staging.example.com");
        assert!(env.variables[0].enabled);

        // `secret` typed variable still imported (type is ignored).
        assert_eq!(env.variables[1].name, "Secret");
        assert_eq!(env.variables[1].value, "supersecret");

        // Disabled flag respected.
        assert_eq!(env.variables[2].name, "Disabled");
        assert!(!env.variables[2].enabled);

        // null value → "" (not dropped) — the variable is still listed.
        assert_eq!(env.variables[3].name, "NoValue");
        assert_eq!(env.variables[3].value, "");

        // Missing `enabled` defaults to true.
        assert_eq!(env.variables[4].name, "NoEnabledFlag");
        assert!(env.variables[4].enabled);
    }

    #[test]
    fn postman_environment_parser_empty_values_yields_empty_env() {
        let content = r#"{ "name": "Empty", "values": [] }"#;
        let plan = PostmanEnvironmentParser.parse(content).unwrap();
        assert_eq!(plan.environments.len(), 1);
        assert!(plan.environments[0].variables.is_empty());
    }

    #[test]
    fn collection_parser_errors_on_non_object_root() {
        assert!(
            PostmanCollectionParser
                .parse("[1, 2, 3]")
                .err()
                .map(|e| matches!(e, BeamError::Validation { .. }))
                .unwrap_or(false)
        );
    }

    #[test]
    fn environment_parser_errors_on_invalid_json() {
        assert!(
            PostmanEnvironmentParser
                .parse("not json")
                .err()
                .map(|e| matches!(e, BeamError::Validation { .. }))
                .unwrap_or(false)
        );
    }
}
