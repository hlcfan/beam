use crate::storage::persistent_types::RequestMetadata;
use iced::Color;
use iced::advanced::text::Highlighter;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestCollection {
    pub name: String,
    pub folder_name: String,
    pub requests: Vec<RequestConfig>,
    pub expanded: bool,
}

// impl RequestCollection {
//     fn get_new_request_path_from_collection(
//         &self,
//         collection: &RequestCollection,
//     ) -> String {
//         // If collection is empty, update the request path and save
//         // else deduce the new file name, and save
//         // Check if the path is empty (new request without a file path)
//         let base_path = self.base_path.to_str().unwrap().to_string();
//         if collection.requests.is_empty() {
//             return format!("{}/{}/{}.toml", base_path, collection.folder_name, "0001");
//         } else {
//             if let Some(last_request) = collection.requests.last() {
//                 let path = last_request.path.clone();

//                 let file_name: usize = path
//                     .file_stem()
//                     .and_then(|s| s.to_str())
//                     .unwrap_or("0001")
//                     .parse()
//                     .unwrap();

//                 let file_path = path.parent().and_then(|s| s.to_str()).unwrap_or("0001");

//                 format!(
//                   "{}/{}/{:04}.toml",
//                   base_path,
//                   file_path,
//                   file_name + 1
//                 )
//             } else {
//                 format!("{}/{}/{}.toml", base_path, collection.folder_name, "0001")
//             }
//         }
//     }
// }

#[derive(Serialize, Deserialize, Debug)]
pub struct RequestConfig {
    pub name: String,
    pub path: PathBuf,
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub params: Vec<(String, String)>,
    pub body: String,
    pub content_type: String,
    pub auth_type: AuthType,

    #[serde(default)]
    pub body_format: BodyFormat,

    // Authentication fields
    pub bearer_token: String,
    pub basic_username: String,
    pub basic_password: String,
    pub api_key: String,
    pub api_key_header: String,

    pub collection_index: usize,
    pub request_index: usize,

    #[serde(default)]
    pub metadata: Option<RequestMetadata>,

    #[serde(default)]
    pub post_request_script: Option<String>,

    #[serde(default)]
    pub last_response: Option<ResponseData>,
}

/// Serializable version of RequestConfig for storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableRequestConfig {
    pub name: Option<String>, // Add name field for identification
    pub method: HttpMethod,
    pub url: Option<String>,
    pub headers: Vec<(String, String)>,
    pub params: Vec<(String, String)>,
    pub body_format: Option<BodyFormat>,
    pub body: Option<String>,
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

    // Post-request script (optional for backward compatibility)
    #[serde(default)]
    pub post_request_script: Option<String>,

    // Last response (optional for backward compatibility)
    #[serde(default)]
    pub last_response: Option<ResponseData>,
}

impl Clone for RequestConfig {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            path: self.path.clone(),
            method: self.method.clone(),
            url: self.url.clone(),
            headers: self.headers.clone(),
            params: self.params.clone(),
            body: self.body.clone(),
            content_type: self.content_type.clone(),
            auth_type: self.auth_type.clone(),
            body_format: self.body_format,
            bearer_token: self.bearer_token.clone(),
            basic_username: self.basic_username.clone(),
            basic_password: self.basic_password.clone(),
            api_key: self.api_key.clone(),
            api_key_header: self.api_key_header.clone(),
            collection_index: self.collection_index,
            request_index: self.request_index,
            // TODO: check this
            metadata: Some(RequestMetadata::default()),
            post_request_script: self.post_request_script.clone(),
            last_response: self.last_response.clone(),
        }
    }
}

impl Default for RequestConfig {
    fn default() -> Self {
        Self {
            name: "New Request".to_string(),
            path: PathBuf::new(),
            method: HttpMethod::GET,
            url: String::new(),
            headers: Vec::new(),
            params: Vec::new(),
            body: String::new(),
            content_type: String::new(),
            auth_type: AuthType::None,
            body_format: BodyFormat::default(),
            bearer_token: String::new(),
            basic_username: String::new(),
            basic_password: String::new(),
            api_key: String::new(),
            api_key_header: String::new(),
            collection_index: 0,
            request_index: 0,
            metadata: Some(RequestMetadata::default()),
            post_request_script: None,
            last_response: None,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentVariable {
    pub value: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

impl EnvironmentVariable {
    pub fn new(value: String) -> Self {
        Self {
            value,
            enabled: true,
        }
    }
}

// Custom deserialization to support backward compatibility
// Old format: BTreeMap<String, String>
// New format: BTreeMap<String, EnvironmentVariable>
impl<'de> serde::Deserialize<'de> for Environment {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct EnvironmentHelper {
            name: String,
            #[serde(deserialize_with = "deserialize_variables")]
            variables: std::collections::BTreeMap<String, EnvironmentVariable>,
            description: Option<String>,
        }

        fn deserialize_variables<'de, D>(
            deserializer: D,
        ) -> Result<std::collections::BTreeMap<String, EnvironmentVariable>, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            use serde::de::{MapAccess, Visitor};
            use std::fmt;

            struct VariablesVisitor;

            impl<'de> Visitor<'de> for VariablesVisitor {
                type Value = std::collections::BTreeMap<String, EnvironmentVariable>;

                fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                    formatter.write_str("a map of variables")
                }

                fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
                where
                    M: MapAccess<'de>,
                {
                    let mut variables = std::collections::BTreeMap::new();

                    while let Some(key) = map.next_key::<String>()? {
                        // Try to deserialize as EnvironmentVariable first
                        if let Ok(var) = map.next_value::<EnvironmentVariable>() {
                            variables.insert(key, var);
                        } else {
                            // Fall back to String for backward compatibility
                            // This branch won't be reached in normal flow, so we need a different approach
                        }
                    }

                    Ok(variables)
                }
            }

            // Try deserializing as new format first
            #[derive(Deserialize)]
            #[serde(untagged)]
            enum VariableValue {
                New(EnvironmentVariable),
                Old(String),
            }

            let map =
                std::collections::BTreeMap::<String, VariableValue>::deserialize(deserializer)?;
            Ok(map
                .into_iter()
                .map(|(k, v)| {
                    let var = match v {
                        VariableValue::New(var) => var,
                        VariableValue::Old(value) => EnvironmentVariable::new(value),
                    };
                    (k, var)
                })
                .collect())
        }

        let helper = EnvironmentHelper::deserialize(deserializer)?;
        Ok(Environment {
            name: helper.name,
            variables: helper.variables,
            description: helper.description,
        })
    }
}

impl serde::Serialize for Environment {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("Environment", 3)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("variables", &self.variables)?;
        state.serialize_field("description", &self.description)?;
        state.end()
    }
}

#[derive(Debug, Clone)]
pub struct Environment {
    pub name: String,
    pub variables: std::collections::BTreeMap<String, EnvironmentVariable>,
    pub description: Option<String>,
}

impl Environment {
    pub fn new(name: String) -> Self {
        Self {
            name,
            variables: std::collections::BTreeMap::new(),
            description: None,
        }
    }

    pub fn add_variable(&mut self, key: String, value: String) {
        self.variables.insert(key, EnvironmentVariable::new(value));
    }

    /// Get variable value only if it's enabled (for request resolution)
    pub fn get_variable(&self, key: &str) -> Option<&String> {
        self.variables
            .get(key)
            .filter(|var| var.enabled)
            .map(|var| &var.value)
    }

    /// Get variable with its state (for UI display)
    pub fn get_variable_with_state(&self, key: &str) -> Option<&EnvironmentVariable> {
        self.variables.get(key)
    }

    /// Set variable enabled state
    pub fn set_variable_enabled(&mut self, key: &str, enabled: bool) {
        if let Some(var) = self.variables.get_mut(key) {
            var.enabled = enabled;
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RequestTab {
    Body,
    Params,
    Headers,
    Auth,
    PostScript,
    // #[allow(dead_code)]
    // Environment,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BodyFormat {
    None,
    Json,
    Xml,
    GraphQL,
    Text,
}

impl Default for BodyFormat {
    fn default() -> Self {
        BodyFormat::None
    }
}

impl std::fmt::Display for BodyFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BodyFormat::None => write!(f, "None"),
            BodyFormat::Json => write!(f, "JSON"),
            BodyFormat::Xml => write!(f, "XML"),
            BodyFormat::GraphQL => write!(f, "GraphQL"),
            BodyFormat::Text => write!(f, "Text"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResponseTab {
    Body,
    Headers,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseData {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    #[serde(default)]
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
