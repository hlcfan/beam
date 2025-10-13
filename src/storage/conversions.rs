use super::persistent_types::*;
use crate::types::{RequestCollection, SavedRequest, Environment, RequestConfig};

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
                created_at: chrono::Utc::now().to_rfc3339(),
                modified_at: chrono::Utc::now().to_rfc3339(),
                description: None,
                version: "1.0".to_string(),
                expanded: self.expanded,
            },
        }
    }
}

impl FromPersistent<PersistentCollection> for RequestCollection {
    fn from_persistent(persistent: PersistentCollection) -> Self {
        Self {
            name: persistent.name,
            requests: persistent.requests
                .into_iter()
                .map(SavedRequest::from_persistent)
                .collect(),
            expanded: persistent.metadata.expanded,
        }
    }
}

// SavedRequest conversions
impl ToPersistent<PersistentRequest> for SavedRequest {
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
            metadata: RequestMetadata::default(),
        }
    }
}

impl FromPersistent<PersistentRequest> for SavedRequest {
    fn from_persistent(persistent: PersistentRequest) -> Self {
        Self {
            name: persistent.name,
            method: match persistent.method.as_str() {
                "GET" => crate::types::HttpMethod::GET,
                "POST" => crate::types::HttpMethod::POST,
                "PUT" => crate::types::HttpMethod::PUT,
                "DELETE" => crate::types::HttpMethod::DELETE,
                "PATCH" => crate::types::HttpMethod::PATCH,
                "HEAD" => crate::types::HttpMethod::HEAD,
                "OPTIONS" => crate::types::HttpMethod::OPTIONS,
                _ => crate::types::HttpMethod::GET,
            },
            url: persistent.url,
        }
    }
}

// RequestConfig conversions
impl ToPersistent<PersistentRequest> for RequestConfig {
    fn to_persistent(&self) -> PersistentRequest {
        let auth_type = match &self.auth_type {
            crate::types::AuthType::None => "None",
            crate::types::AuthType::Bearer => "Bearer",
            crate::types::AuthType::Basic => "Basic",
            crate::types::AuthType::ApiKey => "ApiKey",
        };

        PersistentRequest {
            name: "Untitled Request".to_string(), // Default name
            method: self.method.to_string(),
            url: self.url.clone(),
            headers: self.headers.clone(),
            params: self.params.clone(),
            body: self.body.text(),
            content_type: self.content_type.clone(),
            auth_type: auth_type.to_string(),
            bearer_token: if self.bearer_token.is_empty() { None } else { Some(self.bearer_token.clone()) },
            basic_username: if self.basic_username.is_empty() { None } else { Some(self.basic_username.clone()) },
            basic_password: if self.basic_password.is_empty() { None } else { Some(self.basic_password.clone()) },
            api_key: if self.api_key.is_empty() { None } else { Some(self.api_key.clone()) },
            api_key_header: if self.api_key_header.is_empty() { None } else { Some(self.api_key_header.clone()) },
            metadata: RequestMetadata::default(),
        }
    }
}

impl FromPersistent<PersistentRequest> for RequestConfig {
    fn from_persistent(persistent: PersistentRequest) -> Self {
        let auth_type = match persistent.auth_type.as_str() {
            "Bearer" => crate::types::AuthType::Bearer,
            "Basic" => crate::types::AuthType::Basic,
            "ApiKey" => crate::types::AuthType::ApiKey,
            _ => crate::types::AuthType::None,
        };

        // Ensure default User-Agent header is present
        let mut headers = persistent.headers;
        let has_user_agent = headers.iter().any(|(key, _)| key.to_lowercase() == "user-agent");
        if !has_user_agent {
            headers.push(("User-Agent".to_string(), "BeamApp/1.0".to_string()));
        }

        Self {
            method: match persistent.method.as_str() {
                "GET" => crate::types::HttpMethod::GET,
                "POST" => crate::types::HttpMethod::POST,
                "PUT" => crate::types::HttpMethod::PUT,
                "DELETE" => crate::types::HttpMethod::DELETE,
                "PATCH" => crate::types::HttpMethod::PATCH,
                "HEAD" => crate::types::HttpMethod::HEAD,
                "OPTIONS" => crate::types::HttpMethod::OPTIONS,
                _ => crate::types::HttpMethod::GET,
            },
            url: persistent.url.clone(),
            url_content: iced::widget::text_editor::Content::with_text(&persistent.url),
            headers,
            params: persistent.params,
            body: iced::widget::text_editor::Content::with_text(&persistent.body),
            content_type: persistent.content_type,
            auth_type,
            selected_tab: crate::types::RequestTab::Body, // Default to Body tab
            bearer_token: persistent.bearer_token.unwrap_or_default(),
            basic_username: persistent.basic_username.unwrap_or_default(),
            basic_password: persistent.basic_password.unwrap_or_default(),
            api_key: persistent.api_key.unwrap_or_default(),
            api_key_header: persistent.api_key_header.unwrap_or_else(|| "X-API-Key".to_string()),
        }
    }
}

// Environment conversions
impl ToPersistent<PersistentEnvironment> for Environment {
    fn to_persistent(&self) -> PersistentEnvironment {
        PersistentEnvironment {
            name: self.name.clone(),
            variables: self.variables.clone(),
            description: self.description.clone(),
            metadata: EnvironmentMetadata::default(),
        }
    }
}

impl FromPersistent<PersistentEnvironment> for Environment {
    fn from_persistent(persistent: PersistentEnvironment) -> Self {
        Self {
            name: persistent.name,
            variables: persistent.variables,
            description: persistent.description,
        }
    }
}

// Helper function to convert a RequestConfig with a name to PersistentRequest
impl RequestConfig {
    pub fn to_persistent_with_name(&self, name: String) -> PersistentRequest {
        let mut persistent = self.to_persistent();
        persistent.name = name;
        persistent
    }
}
