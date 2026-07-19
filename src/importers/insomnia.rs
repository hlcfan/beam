use serde_json::Value;

use crate::importers::{DetectedSource, Detector};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_insomnia_export_format_4() {
        let content = r#"{
            "__export_format": 4,
            "__export_source": "insomnia.desktop.app",
            "resources": []
        }"#;
        assert_eq!(InsomniaDetector.detect(content, None), DetectedSource::Insomnia);
    }

    #[test]
    fn detects_insomnia_export_format_3() {
        let content = r#"{ "__export_format": 3 }"#;
        assert_eq!(InsomniaDetector.detect(content, None), DetectedSource::Insomnia);
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
        assert_eq!(InsomniaDetector.detect(content, None), DetectedSource::Insomnia);
    }

    #[test]
    fn detects_insomnia_type_export_alone() {
        let content = r#"{ "_type": "export" }"#;
        assert_eq!(InsomniaDetector.detect(content, None), DetectedSource::Insomnia);
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
}