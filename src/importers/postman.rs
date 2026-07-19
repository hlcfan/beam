use serde_json::Value;

use crate::importers::{DetectedSource, Detector};

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
        if obj.contains_key("__export_format") || obj.get("_type").and_then(|v| v.as_str()) == Some("export") {
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
}