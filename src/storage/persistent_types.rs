use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{Utc};

/// Persistent version of a request that can be serialized
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentRequest {
    pub name: String,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub params: Vec<(String, String)>,
    pub body: String,
    pub content_type: String,
    pub auth_type: String,

    // Authentication fields
    pub bearer_token: Option<String>,
    pub basic_username: Option<String>,
    pub basic_password: Option<String>,
    pub api_key: Option<String>,
    pub api_key_header: Option<String>,

    pub collection_index: u32,
    pub request_index: u32,

    #[serde(default)]
    pub metadata: Option<RequestMetadata>,
}

/// Metadata for a request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMetadata {
    pub created_at: String,
    pub modified_at: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
}

impl Default for RequestMetadata {
    fn default() -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            created_at: now.clone(),
            modified_at: now,
            description: None,
            tags: Vec::new(),
        }
    }
}

/// Persistent version of a collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentCollection {
    pub name: String,
    pub requests: Vec<PersistentRequest>,
    pub metadata: CollectionMetadata,
}

/// Metadata for a collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionMetadata {
    pub name: String,
    pub created_at: String,
    pub modified_at: String,
    pub description: Option<String>,
    pub version: String,
    pub expanded: bool,
}

impl Default for CollectionMetadata {
    fn default() -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            name: "New Collection".to_string(),
            created_at: now.clone(),
            modified_at: now,
            description: None,
            version: "1.0".to_string(),
            expanded: false,
        }
    }
}

/// Persistent version of environment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentEnvironment {
    pub name: String,
    pub variables: HashMap<String, String>,
    pub description: Option<String>,
    pub metadata: EnvironmentMetadata,
}

/// Metadata for an environment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentMetadata {
    pub created_at: String,
    pub modified_at: String,
}

impl Default for EnvironmentMetadata {
    fn default() -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            created_at: now.clone(),
            modified_at: now,
        }
    }
}

/// Root structure for environments file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentEnvironments {
    pub environments: Vec<PersistentEnvironment>,
    pub active_environment: Option<String>,
    pub metadata: EnvironmentsMetadata,
}

/// Metadata for the environments file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentsMetadata {
    pub version: String,
    pub modified_at: String,
}

impl Default for EnvironmentsMetadata {
    fn default() -> Self {
        Self {
            version: "1.0".to_string(),
            modified_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}
