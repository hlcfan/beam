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

pub const DETECTORS: &[(&'static str, &'static dyn Detector, &'static str)] = &[];

pub fn detect(content: &str, ext_hint: Option<&str>) -> DetectedSource {
    for (_, detector, _) in DETECTORS {
        let result = detector.detect(content, ext_hint);
        if result != DetectedSource::Unknown {
            return result;
        }
    }
    DetectedSource::Unknown
}

#[cfg(test)]
mod tests {
    use super::DetectedSource;

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