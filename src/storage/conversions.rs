use super::persistent_types::*;
use crate::types::{RequestCollection, RequestConfig};

/// Trait for converting UI types to persistent types
pub trait ToPersistent<T> {
    fn to_persistent(&self) -> T;
}

/// Trait for converting persistent types to UI types
pub trait FromPersistent<T> {
    fn from_persistent(persistent: T) -> Self;
}

// RequestCollection conversions
impl ToPersistent<PersistentCollection> for RequestCollection {
    fn to_persistent(&self) -> PersistentCollection {
        PersistentCollection {
            name: self.name.clone(),
            requests: self.requests.iter().map(|r| r.to_persistent()).collect(),
            metadata: CollectionMetadata {
                name: self.name.clone(),
                description: None,
                expanded: self.expanded,
            },
        }
    }
}

// SavedRequest conversions
impl ToPersistent<PersistentRequest> for RequestConfig {
    fn to_persistent(&self) -> PersistentRequest {
        PersistentRequest {
            name: self.name.clone(),
            method: self.method.to_string(),
            url: self.url.clone(),
            headers: Vec::new(),
            params: Vec::new(),
            body: String::new(),
            content_type: "application/json".to_string(),
            auth_type: "None".to_string(),
            bearer_token: None,
            basic_username: None,
            basic_password: None,
            api_key: None,
            api_key_header: None,
            collection_index: 0,
            request_index: 0,
            metadata: Some(RequestMetadata::default()),
        }
    }
}
