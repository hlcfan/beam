use std::collections::HashMap;

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

/// Detects Insomnia JSON exports.
///
/// Insomnia exports are top-level JSON objects identified by either an
/// integer `__export_format` >= 3 or a string `_type == "export"`.
pub struct InsomniaDetector;

impl Detector for InsomniaDetector {
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

        let is_insomnia_export_format = obj
            .get("__export_format")
            .and_then(|v| v.as_i64())
            .is_some_and(|n| n >= 3);
        if is_insomnia_export_format {
            return DetectedSource::Insomnia;
        }

        if obj.get("_type").and_then(|v| v.as_str()) == Some("export") {
            return DetectedSource::Insomnia;
        }

        DetectedSource::Unknown
    }
}

/// Parses an Insomnia JSON export into one or more [`ImportPlan`]s.
///
/// Insomnia exports can carry multiple `workspace` resources. The plan calls
/// for the parser to remain pure: `Parser::parse` handles the **first**
/// workspace and appends a warning for any extra workspaces (their actual
/// materialization is the runner's job in Phase 6). Two associated helpers —
/// [`iter_workspaces`](Self::iter_workspaces) and
/// [`parse_for_workspace`](Self::parse_for_workspace) — let the runner split a
/// multi-workspace file on its own.
pub struct InsomniaParser;

impl Parser for InsomniaParser {
    fn parse(&self, content: &str) -> Result<ImportPlan, BeamError> {
        let workspaces = iter_workspaces(content)?;
        if workspaces.is_empty() {
            return Err(BeamError::Validation {
                message: "Insomnia export contains no workspace resources".to_string(),
            });
        }

        // Drive multi-workspace splitting through the same code path as the
        // runner uses so the first workspace is built exactly like any other.
        let (first_id, _) = workspaces[0].clone();
        let mut plan = parse_for_workspace(content, &first_id)?;

        // Surface any additional workspaces via the standard warnings channel.
        if workspaces.len() > 1 {
            for (_, extra) in workspaces.iter().skip(1) {
                plan.warnings.push(format!(
                    "Additional Insomnia workspace \"{}\" skipped — runner handles multi-workspace splitting.",
                    extra
                ));
            }
        }

        Ok(plan)
    }
}

impl InsomniaParser {
    /// Convenience passthrough for [`iter_workspaces`].
    pub fn list_workspaces(content: &str) -> Result<Vec<(String, String)>, BeamError> {
        iter_workspaces(content)
    }

    /// Convenience passthrough for [`parse_for_workspace`].
    pub fn for_workspace(content: &str, workspace_id: &str) -> Result<ImportPlan, BeamError> {
        parse_for_workspace(content, workspace_id)
    }
}

/// Enumerates every `workspace` resource in the export as `(workspace_id, name)`.
/// Preserves the order they appear in `resources[]`.
pub fn iter_workspaces(content: &str) -> Result<Vec<(String, String)>, BeamError> {
    let root: Value = serde_json::from_str(content).map_err(|e| BeamError::Validation {
        message: format!("invalid Insomnia export JSON: {e}"),
    })?;
    let resources = root
        .get("resources")
        .and_then(|v| v.as_array())
        .ok_or_else(|| BeamError::Validation {
            message: "Insomnia export is missing a `resources[]` array".to_string(),
        })?;

    let mut workspaces = Vec::new();
    for r in resources {
        let obj = match r.as_object() {
            Some(o) => o,
            None => continue,
        };
        if obj.get("_type").and_then(|t| t.as_str()) != Some("workspace") {
            continue;
        }
        let id = obj
            .get("_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Imported Insomnia Workspace")
            .to_string();
        if !id.is_empty() {
            workspaces.push((id, name));
        }
    }
    Ok(workspaces)
}

/// Builds an [`ImportPlan`] containing only the resources that belong (directly
/// or transitively through `parentId`) to the given workspace.
pub fn parse_for_workspace(content: &str, workspace_id: &str) -> Result<ImportPlan, BeamError> {
    let root: Value = serde_json::from_str(content).map_err(|e| BeamError::Validation {
        message: format!("invalid Insomnia export JSON: {e}"),
    })?;
    let resources = root
        .get("resources")
        .and_then(|v| v.as_array())
        .ok_or_else(|| BeamError::Validation {
            message: "Insomnia export is missing a `resources[]` array".to_string(),
        })?;

    // Index every resource by `_id` so we can resolve parent chains.
    let mut by_id: HashMap<String, &Value> = HashMap::new();
    for r in resources {
        if let Some(id) = r
            .as_object()
            .and_then(|o| o.get("_id"))
            .and_then(|v| v.as_str())
        {
            by_id.insert(id.to_string(), r);
        }
    }

    let workspace_name = by_id
        .get(workspace_id)
        .and_then(|r| r.as_object())
        .and_then(|o| o.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Imported Insomnia Workspace".to_string());

    let mut plan = ImportPlan {
        workspace_name: workspace_name.clone(),
        folders: Vec::new(),
        requests: Vec::new(),
        environments: Vec::new(),
        warnings: Vec::new(),
        needs_new_workspace: true,
    };

    // First pass — collect folder+request+env resources belonging to this
    // workspace via their parent chain. Resource order within each parent
    // follows the `metaSortNum` Insomnia tracks (clamped to i64); absent
    // values default to zero. We track children grouped by parentId.
    let mut folder_rows: Vec<&Value> = Vec::new();
    let mut request_rows: Vec<&Value> = Vec::new();
    let mut env_rows: Vec<&Value> = Vec::new();

    for r in resources {
        let obj = match r.as_object() {
            Some(o) => o,
            None => continue,
        };
        let Some(parent_id) = obj.get("parentId").and_then(|v| v.as_str()) else {
            continue;
        };
        // Sort key = `_type` plus parent — only descendants of this workspace.
        if !belongs_to_workspace(parent_id, workspace_id, &by_id) {
            continue;
        }
        match obj.get("_type").and_then(|t| t.as_str()).unwrap_or("") {
            "folder" => folder_rows.push(r),
            "request" => request_rows.push(r),
            "environment" => env_rows.push(r),
            "cookiejar" | "api_spec" | "proto_channel" | "unit_test" | "unit_test_suite" => {
                let kind = obj.get("_type").and_then(|t| t.as_str()).unwrap_or("");
                let name = obj.get("name").and_then(|n| n.as_str()).unwrap_or("");
                plan.warnings.push(format!(
                    "Skipped \"{}\" ({}) — unsupported in v1.",
                    name, kind
                ));
            }
            _ => {}
        }
    }

    // Stable order for requests and environments: by `metaSortNum`.
    request_rows.sort_by_key(|r| sort_num(r));
    env_rows.sort_by_key(|r| sort_num(r));

    // Folders need a parent-first traversal so that a child's `parentId` can
    // resolve to an already-allocated Ulid. We do a recursive walk starting
    // at the workspace root, then through every folder, sorting each parent's
    // children by `metaSortNum`.
    let mut id_to_ulid: HashMap<String, Ulid> = HashMap::new();
    // The workspace itself is the implicit root.
    id_to_ulid.insert(workspace_id.to_string(), Ulid::nil());

    let mut next_order = 0usize;
    process_folders(
        &folder_rows,
        workspace_id,
        Some(Ulid::nil()),
        &mut plan,
        &mut id_to_ulid,
        &mut next_order,
    );

    for r in &request_rows {
        let obj = r.as_object().unwrap();
        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unnamed")
            .to_string();
        let parent_id = obj
            .get("parentId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let parent_ulid = id_to_ulid.get(&parent_id).copied();
        let parent_filter = parent_ulid.filter(|&u| u != Ulid::nil());

        let method_str = obj.get("method").and_then(|m| m.as_str()).unwrap_or("GET");
        let method = parse_method(method_str).unwrap_or_else(|| {
            plan.warnings.push(format!(
                "Unknown HTTP method \"{}\" in \"{}\" — defaulted to GET.",
                method_str, name
            ));
            HttpMethod::Get
        });

        let (url, query) = parse_url(obj.get("url"));
        let headers = parse_headers(obj.get("headers"));
        let auth = parse_authentication(obj.get("authentication"), &name, &mut plan.warnings);
        let body = parse_body(obj.get("body"), &name, &mut plan.warnings);
        let scripts = parse_scripts(obj.get("scripts"));

        let id = Ulid::new();
        plan.requests.push(PlannedRequest {
            id,
            parent_id: parent_filter,
            name,
            method,
            url,
            headers,
            query,
            auth,
            body,
            scripts,
            order: next_order,
        });
        next_order += 1;
    }

    // Environments → one PlannedEnvironment per top-level environment resource.
    // Folder-scoped environments (parentId == folder) are not supported in v1
    // because Beam keeps only global environments — skip with a warning.
    for r in &env_rows {
        let obj = r.as_object().unwrap();
        let parent_id = obj
            .get("parentId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let parent_is_folder = folder_rows.iter().any(|f| {
            f.as_object()
                .and_then(|o| o.get("_id"))
                .and_then(|v| v.as_str())
                == Some(parent_id.as_str())
        });
        if parent_is_folder {
            let name = obj.get("name").and_then(|n| n.as_str()).unwrap_or("");
            plan.warnings.push(format!(
                "Folder-scoped environment \"{}\" skipped — only global environments supported in v1.",
                name
            ));
            continue;
        }

        let name = obj
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or(&plan.workspace_name)
            .to_string();
        let variables = parse_environment_data(obj.get("data"));
        plan.environments
            .push(PlannedEnvironment { name, variables });
    }

    Ok(plan)
}

/// Recursively allocates Ulids for folders in parent-first order, sorting each
/// parent's children by `metaSortNum`. Children whose `parentId` matches the
/// workspace root get `parent_id == None` since the workspace is not a Beam folder.
fn process_folders(
    folder_rows: &[&Value],
    parent_id: &str,
    parent_ulid: Option<Ulid>,
    plan: &mut ImportPlan,
    id_to_ulid: &mut HashMap<String, Ulid>,
    next_order: &mut usize,
) {
    let mut children: Vec<&Value> = folder_rows
        .iter()
        .copied()
        .filter(|r| {
            r.as_object()
                .and_then(|o| o.get("parentId"))
                .and_then(|v| v.as_str())
                == Some(parent_id)
        })
        .collect();
    children.sort_by_key(|r| sort_num(r));

    let parent_filter = parent_ulid.filter(|&u| u != Ulid::nil());

    for r in children {
        let obj = r.as_object().unwrap();
        let id = obj
            .get("_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unnamed")
            .to_string();

        let new_id = Ulid::new();
        id_to_ulid.insert(id.clone(), new_id);
        plan.folders.push(PlannedFolder {
            id: new_id,
            parent_id: parent_filter,
            name,
            order: *next_order,
        });
        *next_order += 1;

        process_folders(folder_rows, &id, Some(new_id), plan, id_to_ulid, next_order);
    }
}

/// Walks `parentId` links until reaching (or passing) the target workspace.
/// Returns true if the chain terminates at `workspace_id`.
fn belongs_to_workspace(
    start_id: &str,
    workspace_id: &str,
    by_id: &HashMap<String, &Value>,
) -> bool {
    let mut cursor = start_id.to_string();
    let mut guard = 0u32;
    while !cursor.is_empty() && cursor != workspace_id {
        guard += 1;
        if guard > 1024 {
            return false;
        }
        let Some(resource) = by_id.get(&cursor) else {
            return false;
        };
        let next = resource
            .as_object()
            .and_then(|o| o.get("parentId"))
            .and_then(|v| v.as_str());
        match next {
            Some(parent) => {
                if parent == workspace_id {
                    return true;
                }
                cursor = parent.to_string();
            }
            None => return false,
        }
    }
    cursor == workspace_id
}

fn sort_num(resource: &Value) -> i64 {
    resource
        .as_object()
        .and_then(|o| o.get("metaSortNum"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
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
            let text = obj
                .get("text")
                .or_else(|| obj.get("raw"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mut query = Vec::new();
            if let Some(params) = obj.get("parameters").and_then(|v| v.as_array()) {
                for p in params {
                    let pobj = match p.as_object() {
                        Some(o) => o,
                        None => continue,
                    };
                    let name = pobj
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string();
                    if name.is_empty() {
                        continue;
                    }
                    let value = value_to_string(pobj.get("value"));
                    // Insomnia marks disabled params by `"disabled": true`.
                    let disabled = pobj
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
            (text, query)
        }
        None => (String::new(), Vec::new()),
        _ => (String::new(), Vec::new()),
    }
}

fn parse_headers(headers_value: Option<&Value>) -> Vec<HeaderField> {
    let mut headers = Vec::new();
    let Some(arr) = headers_value.and_then(|v| v.as_array()) else {
        return headers;
    };
    for h in arr {
        let hobj = match h.as_object() {
            Some(o) => o,
            None => continue,
        };
        let name = hobj
            .get("name")
            .and_then(|n| n.as_str())
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
    headers
}

fn parse_authentication(
    auth: Option<&Value>,
    request_name: &str,
    warnings: &mut Vec<String>,
) -> AuthConfig {
    let Some(auth) = auth else {
        return AuthConfig::None;
    };
    let obj = match auth.as_object() {
        Some(o) => o,
        None => return AuthConfig::None,
    };
    let typ = obj.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match typ {
        "bearer" => {
            let token = value_to_string(obj.get("token"));
            AuthConfig::Bearer { token: Some(token) }
        }
        "basic" => {
            let username = value_to_string(obj.get("username"));
            let password = value_to_string(obj.get("password"));
            AuthConfig::Basic {
                username: Some(username),
                password: Some(password),
            }
        }
        "apikey" => {
            let key = value_to_string(obj.get("key"));
            let value = value_to_string(obj.get("value"));
            let location_str = obj.get("in").and_then(|v| v.as_str()).unwrap_or("header");
            let location = if location_str == "query" {
                ApiKeyLocation::Query
            } else {
                ApiKeyLocation::Header
            };
            AuthConfig::ApiKey {
                key: Some(key),
                value: Some(value),
                location,
            }
        }
        other => {
            warnings.push(format!(
                "Unsupported auth type \"{}\" in \"{}\" imported as no-auth.",
                other, request_name
            ));
            AuthConfig::None
        }
    }
}

fn parse_body(body: Option<&Value>, request_name: &str, warnings: &mut Vec<String>) -> BodyConfig {
    let Some(body) = body else {
        return BodyConfig::None;
    };
    let obj = match body.as_object() {
        Some(o) => o,
        None => return BodyConfig::None,
    };
    let mime = obj.get("mimeType").and_then(|m| m.as_str()).unwrap_or("");

    match mime {
        "application/x-www-form-urlencoded" => {
            let fields = parse_body_params(obj.get("params"), request_name, warnings, false);
            BodyConfig::FormUrlEncoded { fields }
        }
        "multipart/form-data" => {
            let fields = parse_body_params(obj.get("params"), request_name, warnings, true);
            BodyConfig::Multipart { fields }
        }
        "" => BodyConfig::None,
        "application/json" | "application/graphql" => {
            let text = obj.get("text").and_then(|t| t.as_str()).unwrap_or("");
            BodyConfig::Json {
                text: text.to_string(),
            }
        }
        "application/xml" | "text/xml" => {
            let text = obj.get("text").and_then(|t| t.as_str()).unwrap_or("");
            BodyConfig::Xml {
                text: text.to_string(),
            }
        }
        other => {
            let text = obj.get("text").and_then(|t| t.as_str()).unwrap_or("");
            BodyConfig::Raw {
                media_type: Some(other.to_string()),
                text: text.to_string(),
            }
        }
    }
}

fn parse_body_params(
    params: Option<&Value>,
    request_name: &str,
    warnings: &mut Vec<String>,
    multipart: bool,
) -> Vec<QueryParamField> {
    let mut fields = Vec::new();
    let Some(arr) = params.and_then(|v| v.as_array()) else {
        return fields;
    };
    let mut skipped_files = 0usize;
    for p in arr {
        let pobj = match p.as_object() {
            Some(o) => o,
            None => continue,
        };
        let name = pobj
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            continue;
        }
        // File uploads in Insomnia use `type: "file"` with `fileName` and
        // optionally `filePath`. We can't replicate binary uploads in v1.
        let is_file = pobj.get("type").and_then(|t| t.as_str()) == Some("file");
        if multipart && is_file {
            skipped_files += 1;
            continue;
        }
        let value = value_to_string(pobj.get("value"));
        let disabled = pobj
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

    if multipart && skipped_files > 0 {
        warnings.push(format!(
            "Skipped {} file-upload param(s) in \"{}\" — binary uploads unsupported in v1.",
            skipped_files, request_name
        ));
    }

    fields
}

fn parse_environment_data(data: Option<&Value>) -> Vec<EnvironmentVariable> {
    let mut vars = Vec::new();
    let Some(obj) = data.and_then(|v| v.as_object()) else {
        return vars;
    };
    for (name, val) in obj {
        let value = value_to_string(Some(val));
        vars.push(EnvironmentVariable {
            name: name.clone(),
            value,
            enabled: true,
            description: None,
        });
    }
    vars
}

fn parse_scripts(scripts: Option<&Value>) -> ScriptConfig {
    // Insomnia keeps after-response scripts under `scripts.afterResponse[]`.
    let Some(scripts) = scripts.and_then(|s| s.as_object()) else {
        return ScriptConfig {
            post_response: None,
        };
    };
    let after = scripts
        .get("afterResponse")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        });
    ScriptConfig {
        post_response: after.filter(|s| !s.is_empty()),
    }
}

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
    fn detects_insomnia_export_format_4() {
        let content = r#"{
            "__export_format": 4,
            "__export_source": "insomnia.desktop.app",
            "resources": []
        }"#;
        assert_eq!(
            InsomniaDetector.detect(content, None),
            DetectedSource::Insomnia
        );
    }

    #[test]
    fn detects_insomnia_export_format_3() {
        let content = r#"{ "__export_format": 3 }"#;
        assert_eq!(
            InsomniaDetector.detect(content, None),
            DetectedSource::Insomnia
        );
    }

    #[test]
    fn rejects_insomnia_export_format_below_3() {
        let content = r#"{ "__export_format": 2 }"#;
        assert_eq!(
            InsomniaDetector.detect(content, None),
            DetectedSource::Unknown
        );
    }

    #[test]
    fn detects_insomnia_type_export() {
        let content = r#"{
            "_type": "export",
            "__export_format": 4
        }"#;
        assert_eq!(
            InsomniaDetector.detect(content, None),
            DetectedSource::Insomnia
        );
    }

    #[test]
    fn detects_insomnia_type_export_alone() {
        let content = r#"{ "_type": "export" }"#;
        assert_eq!(
            InsomniaDetector.detect(content, None),
            DetectedSource::Insomnia
        );
    }

    #[test]
    fn unknown_for_non_json() {
        assert_eq!(
            InsomniaDetector.detect("curl https://example.com", None),
            DetectedSource::Unknown
        );
        assert_eq!(
            InsomniaDetector.detect(r#"{ "foo": "bar" }"#, None),
            DetectedSource::Unknown
        );
    }

    #[test]
    fn unknown_for_json_array() {
        assert_eq!(
            InsomniaDetector.detect("[1, 2, 3]", None),
            DetectedSource::Unknown
        );
    }

    // ---- Phase 3: parser tests ----

    const SINGLE_WORKSPACE_FIXTURE: &str = r#"{
        "_type": "export",
        "__export_format": 4,
        "__export_source": "insomnia.desktop.app",
        "resources": [
            {
                "_id": "ws1",
                "_type": "workspace",
                "name": "Petstore",
                "parentId": null,
                "metaSortNum": 0
            },
            {
                "_id": "fld_auth",
                "_type": "folder",
                "name": "Auth",
                "parentId": "ws1",
                "metaSortNum": 10
            },
            {
                "_id": "fld_login",
                "_type": "folder",
                "name": "Login",
                "parentId": "fld_auth",
                "metaSortNum": 5
            },
            {
                "_id": "req_json",
                "_type": "request",
                "name": "Create pet (JSON)",
                "method": "POST",
                "url": {
                    "text": "https://api.example.com/pets",
                    "parameters": [
                        { "name": "include", "value": "owner" },
                        { "name": "archived", "value": "true", "disabled": true }
                    ]
                },
                "headers": [
                    { "name": "Content-Type", "value": "application/json" },
                    { "name": "Accept", "value": "application/json" }
                ],
                "body": {
                    "mimeType": "application/json",
                    "text": "{\"name\":\"Rex\"}"
                },
                "parentId": "ws1",
                "metaSortNum": 20
            },
            {
                "_id": "req_form",
                "_type": "request",
                "name": "Submit form",
                "method": "POST",
                "url": { "text": "https://api.example.com/forms" },
                "headers": [
                    { "name": "Content-Type", "value": "application/x-www-form-urlencoded" }
                ],
                "body": {
                    "mimeType": "application/x-www-form-urlencoded",
                    "params": [
                        { "name": "name", "value": "Rex" },
                        { "name": "age", "value": "3" },
                        { "name": "archived", "value": "1", "disabled": true }
                    ]
                },
                "parentId": "fld_auth",
                "metaSortNum": 30
            },
            {
                "_id": "req_bearer",
                "_type": "request",
                "name": "Get me (bearer)",
                "method": "GET",
                "url": { "text": "https://api.example.com/me" },
                "headers": [],
                "authentication": { "type": "bearer", "token": "tok123" },
                "parentId": "fld_login",
                "metaSortNum": 40
            },
            {
                "_id": "env_base",
                "_type": "environment",
                "name": "Base",
                "data": {
                    "BASE_URL": "https://api.example.com",
                    "TIMEOUT": "30"
                },
                "parentId": "ws1",
                "metaSortNum": 50
            },
            {
                "_id": "jar1",
                "_type": "cookiejar",
                "name": "Cookies",
                "parentId": "ws1",
                "metaSortNum": 60
            }
        ]
    }"#;

    #[test]
    fn parses_single_workspace_export() {
        let plan = InsomniaParser.parse(SINGLE_WORKSPACE_FIXTURE).unwrap();
        assert_eq!(plan.workspace_name, "Petstore");
        assert_eq!(plan.folders.len(), 2);
        assert_eq!(plan.requests.len(), 3);
        assert_eq!(plan.environments.len(), 1);

        // Folders: Auth (top-level) and Login (nested inside Auth).
        let auth = find_folder(&plan, "Auth");
        assert_eq!(auth.parent_id, None);
        let login = find_folder(&plan, "Login");
        assert_eq!(login.parent_id, Some(auth.id));

        // JSON request at workspace root with query params.
        let json_req = find_request(&plan, "Create pet (JSON)");
        assert_eq!(json_req.method, HttpMethod::Post);
        assert_eq!(json_req.parent_id, None);
        assert_eq!(json_req.url, "https://api.example.com/pets");
        assert_eq!(json_req.query.len(), 2);
        assert_eq!(json_req.query[0].name, "include");
        assert!(json_req.query[0].enabled);
        assert_eq!(json_req.query[1].name, "archived");
        assert!(!json_req.query[1].enabled);
        assert_eq!(json_req.headers.len(), 2);
        let BodyConfig::Json { text } = &json_req.body else {
            panic!("expected Json body");
        };
        assert_eq!(text, "{\"name\":\"Rex\"}");

        // Form-urlencoded request inside the Auth folder.
        let form_req = find_request(&plan, "Submit form");
        assert_eq!(form_req.method, HttpMethod::Post);
        assert_eq!(form_req.parent_id, Some(auth.id));
        let BodyConfig::FormUrlEncoded { fields } = &form_req.body else {
            panic!("expected FormUrlEncoded");
        };
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].name, "name");
        assert_eq!(fields[0].value, "Rex");
        assert!(!fields[2].enabled);

        // Bearer-authed request inside Login folder.
        let bearer_req = find_request(&plan, "Get me (bearer)");
        assert_eq!(bearer_req.method, HttpMethod::Get);
        assert_eq!(bearer_req.parent_id, Some(login.id));
        assert_eq!(
            bearer_req.auth,
            AuthConfig::Bearer {
                token: Some("tok123".to_string())
            }
        );

        // Environment with two variables.
        let env = &plan.environments[0];
        assert_eq!(env.name, "Base");
        assert_eq!(env.variables.len(), 2);
        assert_eq!(env.variables[0].name, "BASE_URL");
        assert_eq!(env.variables[0].value, "https://api.example.com");
        assert!(env.variables[0].enabled);

        // Cookiejar skipped with a warning.
        assert!(
            plan.warnings
                .iter()
                .any(|w| w.contains("Skipped \"Cookies\" (cookiejar)"))
        );
    }

    const MULTI_WORKSPACE_FIXTURE: &str = r#"{
        "_type": "export",
        "__export_format": 4,
        "resources": [
            {
                "_id": "ws_a",
                "_type": "workspace",
                "name": "Workspace A",
                "parentId": null,
                "metaSortNum": 0
            },
            {
                "_id": "req_a",
                "_type": "request",
                "name": "A request",
                "method": "GET",
                "url": { "text": "https://a.example.com" },
                "parentId": "ws_a",
                "metaSortNum": 10
            },
            {
                "_id": "env_a",
                "_type": "environment",
                "name": "Env A",
                "data": { "A": "alpha" },
                "parentId": "ws_a",
                "metaSortNum": 20
            },
            {
                "_id": "ws_b",
                "_type": "workspace",
                "name": "Workspace B",
                "parentId": null,
                "metaSortNum": 100
            },
            {
                "_id": "req_b",
                "_type": "request",
                "name": "B request",
                "method": "GET",
                "url": { "text": "https://b.example.com" },
                "parentId": "ws_b",
                "metaSortNum": 10
            },
            {
                "_id": "env_b",
                "_type": "environment",
                "name": "Env B",
                "data": { "B": "beta" },
                "parentId": "ws_b",
                "metaSortNum": 20
            }
        ]
    }"#;

    #[test]
    fn iter_workspaces_lists_all_workspaces_in_order() {
        let workspaces = iter_workspaces(MULTI_WORKSPACE_FIXTURE).unwrap();
        assert_eq!(workspaces.len(), 2);
        assert_eq!(workspaces[0].0, "ws_a");
        assert_eq!(workspaces[0].1, "Workspace A");
        assert_eq!(workspaces[1].0, "ws_b");
        assert_eq!(workspaces[1].1, "Workspace B");
    }

    #[test]
    fn parser_defaults_to_first_workspace_and_warns_about_others() {
        let plan = InsomniaParser.parse(MULTI_WORKSPACE_FIXTURE).unwrap();
        assert_eq!(plan.workspace_name, "Workspace A");
        assert_eq!(plan.requests.len(), 1);
        assert_eq!(plan.requests[0].name, "A request");
        assert_eq!(plan.environments.len(), 1);
        assert_eq!(plan.environments[0].name, "Env A");
        assert!(plan.warnings.iter().any(|w| w.contains("Workspace B")));
    }

    #[test]
    fn parse_for_workspace_isolates_second_workspace() {
        let plan = parse_for_workspace(MULTI_WORKSPACE_FIXTURE, "ws_b").unwrap();
        assert_eq!(plan.workspace_name, "Workspace B");
        // Only request B is included — request A is filtered out.
        assert_eq!(plan.requests.len(), 1);
        assert_eq!(plan.requests[0].name, "B request");
        assert_eq!(plan.environments.len(), 1);
        assert_eq!(plan.environments[0].name, "Env B");
        assert_eq!(plan.environments[0].variables[0].name, "B");
        assert_eq!(plan.environments[0].variables[0].value, "beta");
    }

    #[test]
    fn errors_on_empty_workspace_set() {
        let content = r#"{ "_type": "export", "__export_format": 4, "resources": [] }"#;
        let err = InsomniaParser.parse(content).unwrap_err();
        assert!(matches!(err, BeamError::Validation { .. }));
    }

    #[test]
    fn unknown_method_falls_back_to_get_with_warning() {
        let content = r#"{
            "_type": "export",
            "__export_format": 4,
            "resources": [
                { "_id": "ws1", "_type": "workspace", "name": "W", "parentId": null },
                {
                    "_id": "req1",
                    "_type": "request",
                    "name": "weird-method",
                    "method": "BREW",
                    "url": { "text": "https://example.com/coffee" },
                    "parentId": "ws1"
                }
            ]
        }"#;
        let plan = InsomniaParser.parse(content).unwrap();
        let r = find_request(&plan, "weird-method");
        assert_eq!(r.method, HttpMethod::Get);
        assert!(plan.warnings.iter().any(|w| w.contains("BREW")));
    }

    #[test]
    fn unknown_auth_type_becomes_noauth_with_warning_and_header_fallback() {
        let content = r#"{
            "_type": "export",
            "__export_format": 4,
            "resources": [
                { "_id": "ws1", "_type": "workspace", "name": "W", "parentId": null },
                {
                    "_id": "req1",
                    "_type": "request",
                    "name": "hawk",
                    "method": "GET",
                    "url": { "text": "https://api.example.com/hawk" },
                    "headers": [
                        { "name": "Authorization", "value": "Hawk id=abc" }
                    ],
                    "authentication": { "type": "hawk", "id": "abc" },
                    "parentId": "ws1"
                }
            ]
        }"#;
        let plan = InsomniaParser.parse(content).unwrap();
        let r = find_request(&plan, "hawk");
        assert_eq!(r.auth, AuthConfig::None);
        assert_eq!(r.headers.len(), 1);
        assert_eq!(r.headers[0].name, "Authorization");
        assert_eq!(r.headers[0].value, "Hawk id=abc");
        assert!(plan.warnings.iter().any(|w| w.contains("hawk")));
    }

    #[test]
    fn multipart_file_upload_param_skipped_with_warning() {
        let content = r#"{
            "_type": "export",
            "__export_format": 4,
            "resources": [
                { "_id": "ws1", "_type": "workspace", "name": "W", "parentId": null },
                {
                    "_id": "req1",
                    "_type": "request",
                    "name": "upload",
                    "method": "POST",
                    "url": { "text": "https://api.example.com/upload" },
                    "body": {
                        "mimeType": "multipart/form-data",
                        "params": [
                            { "name": "label", "value": "dog" },
                            { "name": "file", "type": "file", "fileName": "dog.png" }
                        ]
                    },
                    "parentId": "ws1"
                }
            ]
        }"#;
        let plan = InsomniaParser.parse(content).unwrap();
        let r = find_request(&plan, "upload");
        let BodyConfig::Multipart { fields } = &r.body else {
            panic!("expected Multipart");
        };
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "label");
        assert!(plan.warnings.iter().any(|w| w.contains("upload")));
    }

    #[test]
    fn xml_body_detected_via_mimetype() {
        let content = r#"{
            "_type": "export",
            "__export_format": 4,
            "resources": [
                { "_id": "ws1", "_type": "workspace", "name": "W", "parentId": null },
                {
                    "_id": "req1",
                    "_type": "request",
                    "name": "xml-pet",
                    "method": "PUT",
                    "url": { "text": "https://api.example.com/pets/1" },
                    "body": { "mimeType": "application/xml", "text": "<pet/>" },
                    "parentId": "ws1"
                }
            ]
        }"#;
        let plan = InsomniaParser.parse(content).unwrap();
        let r = find_request(&plan, "xml-pet");
        assert!(matches!(&r.body, BodyConfig::Xml { text } if text == "<pet/>"));
    }

    #[test]
    fn detects_insomnia_via_module_level_detect_fn() {
        let result = super::super::detect(SINGLE_WORKSPACE_FIXTURE, None);
        assert_eq!(result, DetectedSource::Insomnia);
    }

    #[test]
    fn folder_scoped_env_skipped_with_warning() {
        let content = r#"{
            "_type": "export",
            "__export_format": 4,
            "resources": [
                { "_id": "ws1", "_type": "workspace", "name": "W", "parentId": null, "metaSortNum": 0 },
                { "_id": "fld1", "_type": "folder", "name": "F", "parentId": "ws1", "metaSortNum": 5 },
                {
                    "_id": "env_f",
                    "_type": "environment",
                    "name": "FolderEnv",
                    "data": { "x": "1" },
                    "parentId": "fld1",
                    "metaSortNum": 10
                }
            ]
        }"#;
        let plan = InsomniaParser.parse(content).unwrap();
        assert!(plan.environments.is_empty());
        assert!(
            plan.warnings
                .iter()
                .any(|w| w.contains("Folder-scoped environment \"FolderEnv\""))
        );
    }
}
