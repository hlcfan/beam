use ulid::Ulid;

use crate::error::BeamError;
use crate::models::{
    AuthConfig, BodyConfig, EnvironmentVariable, HeaderField, HttpMethod, QueryParamField,
    ScriptConfig,
};

pub use crate::error::BeamError as DetectError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPlan {
    pub workspace_name: String,
    pub folders: Vec<PlannedFolder>,
    pub requests: Vec<PlannedRequest>,
    pub environments: Vec<PlannedEnvironment>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFolder {
    pub id: Ulid,
    pub parent_id: Option<Ulid>,
    pub name: String,
    pub order: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedRequest {
    pub id: Ulid,
    pub parent_id: Option<Ulid>,
    pub name: String,
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<HeaderField>,
    pub query: Vec<QueryParamField>,
    pub auth: AuthConfig,
    pub body: BodyConfig,
    pub scripts: ScriptConfig,
    pub order: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedEnvironment {
    pub name: String,
    pub variables: Vec<EnvironmentVariable>,
}

/// Combines file-level plans into one import while retaining the boundary
/// between collections. When more than one plan contains tree items, each
/// collection becomes a top-level folder instead of flattening all of its
/// root requests into the workspace.
pub fn merge_file_plans(plans: Vec<(ImportPlan, bool)>) -> Option<(ImportPlan, bool)> {
    let tree_plan_count = plans
        .iter()
        .filter(|(plan, _)| !plan.folders.is_empty() || !plan.requests.is_empty())
        .count();
    let wrap_tree_plans = tree_plan_count > 1;
    let mut plans = plans.into_iter();
    let (mut merged, mut needs_new_workspace) = plans.next()?;

    if wrap_tree_plans && (!merged.folders.is_empty() || !merged.requests.is_empty()) {
        wrap_plan_tree(&mut merged);
    }

    for (mut plan, has_workspace) in plans {
        if has_workspace {
            needs_new_workspace = true;
        }
        if wrap_tree_plans && (!plan.folders.is_empty() || !plan.requests.is_empty()) {
            wrap_plan_tree(&mut plan);
        }
        merged.folders.extend(plan.folders);
        merged.requests.extend(plan.requests);
        merged.environments.extend(plan.environments);
        merged.warnings.extend(plan.warnings);
    }

    Some((merged, needs_new_workspace))
}

fn wrap_plan_tree(plan: &mut ImportPlan) {
    let wrapper_id = Ulid::new();
    for folder in &mut plan.folders {
        if folder.parent_id.is_none() {
            folder.parent_id = Some(wrapper_id);
        }
    }
    for request in &mut plan.requests {
        if request.parent_id.is_none() {
            request.parent_id = Some(wrapper_id);
        }
    }
    plan.folders.insert(
        0,
        PlannedFolder {
            id: wrapper_id,
            parent_id: None,
            name: plan.workspace_name.clone(),
            order: 0,
        },
    );
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectedSource {
    PostmanCollection { schema: String },
    PostmanEnvironment,
    Insomnia,
    Unknown,
}

pub trait Detector {
    fn detect(&self, content: &str, ext_hint: Option<&str>) -> DetectedSource;
}

pub trait Parser {
    fn parse(&self, content: &str) -> Result<ImportPlan, BeamError>;
}

pub mod curl;
pub mod insomnia;
pub mod postman;
pub mod scanner;

pub use curl::{CurlPlan, is_curl, parse as parse_curl};

fn strip_query(url: &str) -> String {
    let (before_fragment, fragment) = match url.split_once('#') {
        Some((before, fragment)) => (before, Some(fragment)),
        None => (url, None),
    };
    let without_query = before_fragment
        .split_once('?')
        .map_or(before_fragment, |(base, _)| base);

    match fragment {
        Some(fragment) => format!("{without_query}#{fragment}"),
        None => without_query.to_string(),
    }
}

pub const DETECTORS: &[(&str, &'static dyn Detector, &str)] = &[
    ("postman", &postman::PostmanDetector, "Postman"),
    ("insomnia", &insomnia::InsomniaDetector, "Insomnia"),
];

pub fn detect(content: &str, ext_hint: Option<&str>) -> DetectedSource {
    for (_, detector, _) in DETECTORS {
        let result = detector.detect(content, ext_hint);
        if result != DetectedSource::Unknown {
            return result;
        }
    }
    DetectedSource::Unknown
}

pub fn parser_for(source: &DetectedSource) -> Option<&'static dyn Parser> {
    match source {
        DetectedSource::PostmanCollection { .. } => Some(&postman::PostmanCollectionParser),
        DetectedSource::PostmanEnvironment => Some(&postman::PostmanEnvironmentParser),
        DetectedSource::Insomnia => Some(&insomnia::InsomniaParser),
        DetectedSource::Unknown => None,
    }
}

#[cfg(test)]
mod strip_query_tests {
    use super::strip_query;

    #[test]
    fn removes_query_and_preserves_fragment() {
        assert_eq!(
            strip_query("https://example.com/items?page=1#results"),
            "https://example.com/items#results"
        );
    }

    #[test]
    fn does_not_treat_question_mark_in_fragment_as_query() {
        assert_eq!(
            strip_query("https://example.com/items#results?view=full"),
            "https://example.com/items#results?view=full"
        );
    }

    #[test]
    fn supports_templated_and_relative_urls() {
        assert_eq!(
            strip_query("{{base_url}}/items?page=1"),
            "{{base_url}}/items"
        );
        assert_eq!(strip_query("/items?page=1#results"), "/items#results");
    }
}

/// Check if the given file content represents a workspace-level export
/// (rather than a single collection or environment that should be imported
/// into the current workspace).
///
/// The check is format-agnostic and supports both JSON resources exports and
/// Insomnia v5 YAML collections.
pub fn content_has_workspace(content: &str) -> bool {
    if insomnia::is_workspace_export(content) {
        return true;
    }
    let trimmed = content.trim();
    if !trimmed.starts_with('{') {
        return false;
    }
    let Ok(root) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return false;
    };
    let Some(obj) = root.as_object() else {
        return false;
    };
    // Insomnia exports carry a `resources[]` array containing items with
    // `_type: "workspace"`. Other formats either lack this structure or
    // have no workspace concept.
    if let Some(resources) = obj.get("resources").and_then(|r| r.as_array()) {
        if resources.iter().any(|r| {
            r.as_object()
                .and_then(|o| o.get("_type"))
                .and_then(|t| t.as_str())
                == Some("workspace")
        }) {
            return true;
        }
    }
    false
}

pub fn tag_label(source: &DetectedSource) -> &'static str {
    match source {
        DetectedSource::PostmanCollection { .. } => "Postman",
        DetectedSource::PostmanEnvironment => "Postman",
        DetectedSource::Insomnia => "Insomnia",
        DetectedSource::Unknown => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DetectedSource, ImportPlan, PlannedRequest, content_has_workspace, detect, merge_file_plans,
    };
    use crate::models::{AuthConfig, BodyConfig, HttpMethod, ScriptConfig};
    use ulid::Ulid;

    fn request_plan(name: &str) -> ImportPlan {
        ImportPlan {
            workspace_name: name.to_string(),
            folders: Vec::new(),
            requests: vec![PlannedRequest {
                id: Ulid::new(),
                parent_id: None,
                name: format!("{name} request"),
                method: HttpMethod::Get,
                url: String::new(),
                headers: Vec::new(),
                query: Vec::new(),
                auth: AuthConfig::None,
                body: BodyConfig::None,
                scripts: ScriptConfig::default(),
                order: 0,
            }],
            environments: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn merging_multiple_collection_files_keeps_each_tree_in_a_folder() {
        let (merged, needs_workspace) = merge_file_plans(vec![
            (request_plan("Collection A"), true),
            (request_plan("Collection B"), true),
        ])
        .unwrap();

        assert!(needs_workspace);
        assert_eq!(
            merged
                .folders
                .iter()
                .map(|folder| folder.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Collection A", "Collection B"]
        );
        assert_eq!(merged.requests[0].parent_id, Some(merged.folders[0].id));
        assert_eq!(merged.requests[1].parent_id, Some(merged.folders[1].id));
    }

    #[test]
    fn content_has_workspace_false_for_postman_environment() {
        let content = r#"{
            "name": "My Env",
            "_postman_variable_scope": "environment",
            "values": [{ "key": "k", "value": "v" }]
        }"#;
        assert!(!content_has_workspace(content));
    }

    #[test]
    fn content_has_workspace_false_for_postman_collection() {
        let content = r#"{
            "info": { "name": "API", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" },
            "item": []
        }"#;
        assert!(!content_has_workspace(content));
    }

    #[test]
    fn content_has_workspace_true_for_insomnia_workspace_export() {
        let content = r#"{
            "__export_format": 4,
            "_type": "export",
            "resources": [
                { "_id": "ws_1", "_type": "workspace", "name": "My WS" }
            ]
        }"#;
        assert!(content_has_workspace(content));
    }

    #[test]
    fn content_has_workspace_false_for_non_json() {
        assert!(!content_has_workspace("not json"));
    }

    #[test]
    fn content_has_workspace_false_for_json_without_resources() {
        let content = r#"{ "foo": "bar" }"#;
        assert!(!content_has_workspace(content));
    }

    #[test]
    fn detect_dispatches_postman_collection() {
        let content = r#"{
            "info": {
                "name": "Sample",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "item": []
        }"#;
        assert!(matches!(
            detect(content, None),
            DetectedSource::PostmanCollection { .. }
        ));
    }

    #[test]
    fn detect_dispatches_postman_environment() {
        let content = r#"{
            "name": "Env",
            "_postman_variable_scope": "environment",
            "values": [ { "key": "k", "value": "v" } ]
        }"#;
        assert_eq!(detect(content, None), DetectedSource::PostmanEnvironment);
    }

    #[test]
    fn detect_dispatches_insomnia() {
        let content = r#"{ "__export_format": 4, "_type": "export" }"#;
        assert_eq!(detect(content, None), DetectedSource::Insomnia);
    }

    #[test]
    fn detect_returns_unknown_for_unrecognized_content() {
        assert_eq!(detect("not json", None), DetectedSource::Unknown);
        assert_eq!(detect(r#"{ "foo": "bar" }"#, None), DetectedSource::Unknown);
    }

    #[test]
    fn detect_first_non_unknown_wins_postman_before_insomnia() {
        // Postman detector is registered first; a payload the Insomnia detector
        // would also catch (via `__export_format`) but the Postman detector
        // rejects should still resolve to `Insomnia`, and a clear Postman
        // Collection should resolve to Postman even if `_type == "export"` is
        // also present (which the Postman detector itself rejects).
        let insomnia_only = r#"{ "__export_format": 4 }"#;
        assert_eq!(detect(insomnia_only, None), DetectedSource::Insomnia);

        let postman = r#"{
            "info": {
                "name": "Sample",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "item": []
        }"#;
        assert!(matches!(
            detect(postman, None),
            DetectedSource::PostmanCollection { .. }
        ));
    }

    #[test]
    fn detected_source_postman_collection_carries_schema() {
        let src = DetectedSource::PostmanCollection {
            schema: "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
                .to_string(),
        };
        match src {
            DetectedSource::PostmanCollection { schema } => {
                assert!(schema.starts_with("https://schema.getpostman.com/json/collection/v2."));
            }
            _ => panic!("expected PostmanCollection variant"),
        }
    }

    #[test]
    fn detected_source_variants_distinct() {
        assert_ne!(DetectedSource::PostmanEnvironment, DetectedSource::Insomnia);
        assert_ne!(DetectedSource::Insomnia, DetectedSource::Unknown);
        assert_eq!(DetectedSource::Unknown, DetectedSource::Unknown);
    }
}
