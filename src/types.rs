use crate::storage::persistent_types::RequestMetadata;
use iced::Color;
use iced::advanced::text::Highlighter;
use iced::widget::text_editor;
use log::info;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HttpMethod {
    GET,
    POST,
    PUT,
    DELETE,
    PATCH,
    HEAD,
    OPTIONS,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AuthType {
    None,
    Bearer,
    Basic,
    ApiKey,
}

impl Default for AuthType {
    fn default() -> Self {
        AuthType::None
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RenameTarget {
    Folder(usize),         // collection_index
    Request(usize, usize), // (collection_index, request_index)
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ResponseHighlighter {
    pub content_type: String,
}

#[allow(dead_code)]
impl ResponseHighlighter {
    pub fn new(content_type: String) -> Self {
        Self { content_type }
    }
}

impl Highlighter for ResponseHighlighter {
    type Settings = ();
    type Highlight = Color;
    type Iterator<'a> = std::iter::Empty<(std::ops::Range<usize>, Self::Highlight)>;

    fn new(_settings: &Self::Settings) -> Self {
        Self {
            content_type: String::new(),
        }
    }

    fn update(&mut self, _new_settings: &Self::Settings) {}

    fn current_line(&self) -> usize {
        0
    }

    fn change_line(&mut self, _line: usize) {}

    fn highlight_line(&mut self, _text: &str) -> Self::Iterator<'_> {
        // Simple highlighting - could be expanded for JSON, XML, etc.
        std::iter::empty()
    }
}

#[derive(Debug, Clone)]
pub struct RequestCollection {
    pub name: String,
    pub requests: Vec<RequestConfig>,
    pub expanded: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RequestConfig {
    pub name: String, // Add name field for identification
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub params: Vec<(String, String)>,
    pub body: String,
    pub content_type: String,
    pub auth_type: AuthType,

    // Authentication fields
    pub bearer_token: String,
    pub basic_username: String,
    pub basic_password: String,
    pub api_key: String,
    pub api_key_header: String,

    pub collection_index: u32,
    pub request_index: u32,

    #[serde(default)]
    pub metadata: Option<RequestMetadata>,
}

/// Serializable version of RequestConfig for storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableRequestConfig {
    pub name: Option<String>, // Add name field for identification
    pub method: HttpMethod,
    pub url: Option<String>,
    pub headers: Vec<(String, String)>,
    pub params: Vec<(String, String)>,
    pub body: Option<String>, // Store as string instead of text_editor::Content
    pub content_type: Option<String>,
    pub auth_type: Option<AuthType>,

    // Authentication fields
    pub bearer_token: Option<String>,
    pub basic_username: Option<String>,
    pub basic_password: Option<String>,
    pub api_key: Option<String>,
    pub api_key_header: Option<String>,

    // Metadata field (optional for backward compatibility)
    #[serde(default)]
    pub metadata: Option<RequestMetadata>,
}

impl Clone for RequestConfig {
    fn clone(&self) -> Self {
        info!("====clone request config");
        Self {
            name: self.name.clone(),
            method: self.method.clone(),
            url: self.url.clone(),
            headers: self.headers.clone(),
            params: self.params.clone(),
            body: self.body.clone(),
            content_type: self.content_type.clone(),
            auth_type: self.auth_type.clone(),
            bearer_token: self.bearer_token.clone(),
            basic_username: self.basic_username.clone(),
            basic_password: self.basic_password.clone(),
            api_key: self.api_key.clone(),
            api_key_header: self.api_key_header.clone(),
            collection_index: self.collection_index,
            request_index: self.request_index,
            // TODO: check this
            metadata: Some(RequestMetadata::default()),
        }
    }
}

impl Default for RequestConfig {
  fn default() -> Self {
    Self {
      name: "New Request".to_string(),
      method: HttpMethod::GET,
      url: String::new(),
      headers: Vec::new(),
      params: Vec::new(),
      body: String::new(),
      content_type: String::new(),
      auth_type: AuthType::None,
      bearer_token: String::new(),
      basic_username: String::new(),
      basic_password: String::new(),
      api_key: String::new(),
      api_key_header: String::new(),
      collection_index: 0,
      request_index: 0,
      metadata: Some(RequestMetadata::default()),
    }
  }
}

/// Create from serializable format
// pub fn from_serializable(serializable: SerializableRequestConfig) -> Self {
//     Self {
//         method: serializable.method,
//         url: serializable.url.clone(),
//         headers: serializable.headers,
//         params: serializable.params,
//         body: serializable.body,
//         content_type: serializable.content_type,
//         auth_type: serializable.auth_type,
//         selected_tab: RequestTab::Body, // Default tab
//         bearer_token: serializable.bearer_token,
//         basic_username: serializable.basic_username,
//         basic_password: serializable.basic_password,
//         api_key: serializable.api_key,
//         api_key_header: serializable.api_key_header,
//         collection_index: serializable.collection_index,
//         request_index: serializable.request_index,
//     }
// }
// }

#[derive(Debug, Clone)]
pub struct Environment {
    pub name: String,
    pub variables: std::collections::HashMap<String, String>,
    pub description: Option<String>,
}

impl Environment {
    pub fn new(name: String) -> Self {
        Self {
            name,
            variables: std::collections::HashMap::new(),
            description: None,
        }
    }

    pub fn add_variable(&mut self, key: String, value: String) {
        self.variables.insert(key, value);
    }

    pub fn get_variable(&self, key: &str) -> Option<&String> {
        self.variables.get(key)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RequestTab {
    Body,
    Params,
    Headers,
    Auth,
    #[allow(dead_code)]
    Environment,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResponseTab {
    Body,
    Headers,
}

#[derive(Debug, Clone)]
pub struct ResponseData {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub content_type: String,
    pub is_binary: bool,
    pub size: usize,
    pub time: u64, // milliseconds
}

impl std::fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpMethod::GET => write!(f, "GET"),
            HttpMethod::POST => write!(f, "POST"),
            HttpMethod::PUT => write!(f, "PUT"),
            HttpMethod::DELETE => write!(f, "DELETE"),
            HttpMethod::PATCH => write!(f, "PATCH"),
            HttpMethod::HEAD => write!(f, "HEAD"),
            HttpMethod::OPTIONS => write!(f, "OPTIONS"),
        }
    }
}

impl std::fmt::Display for AuthType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthType::None => write!(f, "None"),
            AuthType::Bearer => write!(f, "Bearer Token"),
            AuthType::Basic => write!(f, "Basic Auth"),
            AuthType::ApiKey => write!(f, "API Key"),
        }
    }
}

#[derive(Debug)]
pub struct ResponsePanel {
    pub response: Option<ResponseData>,
    pub response_body_content: text_editor::Content,
    pub selected_tab: ResponseTab,
    pub is_loading: bool,
    pub current_elapsed_time: u64,
    pub spinner: crate::ui::Spinner,
}
