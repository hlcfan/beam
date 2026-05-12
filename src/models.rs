use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::schema::SCHEMA_VERSION_V1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemType {
    Folder,
    Request,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentScope {
    Global,
    Collection,
}

// TODO: clean up the alias

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyLocation {
    #[serde(alias = "Header")]
    Header,
    #[serde(alias = "Query")]
    Query,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceFile {
    pub schema_version: u32,
    pub workspace: WorkspaceMeta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMeta {
    pub workspace_id: Ulid,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WorkspaceFile {
    pub fn new(name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            schema_version: SCHEMA_VERSION_V1,
            workspace: WorkspaceMeta {
                workspace_id: Ulid::new(),
                name: name.into(),
                description: None,
                created_at: now,
                updated_at: now,
            },
        }
    }
}

impl Default for WorkspaceFile {
    fn default() -> Self {
        Self::new("Beam Workspace")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionFile {
    pub collection: CollectionMeta,
    #[serde(default)]
    pub items: Vec<CollectionItemRef>,
    #[serde(skip)]
    pub manifest_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionMeta {
    pub collection_id: Ulid,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionItemRef {
    pub item_id: Ulid,
    pub item_type: ItemType,
    pub name: String,
    pub order: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderFile {
    pub folder: FolderMeta,
    #[serde(default)]
    pub items: Vec<CollectionItemRef>,
    #[serde(skip)]
    pub manifest_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderMeta {
    pub folder_id: Ulid,
    pub collection_id: Ulid,
    pub parent_folder_id: Option<Ulid>,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestFile {
    pub meta: RequestMeta,
    pub request: RequestDefinition,
    // TODO: auth, body, scripts belong to the RequestDefinition instead of new fields
    pub auth: AuthConfig,
    pub body: BodyConfig,
    pub scripts: ScriptConfig,
    #[serde(skip)]
    pub file_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestMeta {
    pub request_id: Ulid,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestDefinition {
    pub method: HttpMethod,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<HeaderField>,
    #[serde(default)]
    pub query_params: Vec<QueryParamField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderField {
    pub name: String,
    pub value: String,
    pub enabled: bool,
    pub description: Option<String>,
    #[serde(default)]
    pub secret: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryParamField {
    pub name: String,
    pub value: String,
    pub enabled: bool,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthConfig {
    #[serde(alias = "None")]
    None,
    #[serde(alias = "Bearer")]
    Bearer { token: Option<String> },
    #[serde(alias = "Basic")]
    Basic {
        username: Option<String>,
        password: Option<String>,
    },
    #[serde(alias = "ApiKey")]
    ApiKey {
        key: Option<String>,
        value: Option<String>,
        location: ApiKeyLocation,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum BodyConfig {
    #[serde(alias = "None")]
    None,
    #[serde(alias = "Raw")]
    Raw {
        media_type: Option<String>,
        text: String,
    },
    #[serde(alias = "Json")]
    Json { text: String },
    #[serde(alias = "Xml")]
    Xml { text: String },
    #[serde(alias = "FormUrlEncoded")]
    FormUrlEncoded {
        #[serde(default)]
        fields: Vec<QueryParamField>,
    },
    #[serde(alias = "Multipart")]
    Multipart {
        #[serde(default)]
        fields: Vec<QueryParamField>,
    },
    #[serde(alias = "Graphql")]
    Graphql {
        query: String,
        variables_json: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ScriptConfig {
    pub post_response: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentFile {
    pub schema_version: u32,
    pub environment: EnvironmentMeta,
    #[serde(default)]
    pub variables: Vec<EnvironmentVariable>,
    #[serde(skip)]
    pub file_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentMeta {
    pub environment_id: Ulid,
    pub collection_id: Option<Ulid>,
    pub scope: EnvironmentScope,
    pub name: String,
    #[serde(default)]
    pub file_name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentVariable {
    pub name: String,
    pub value: String,
    pub enabled: bool,
    #[serde(default)]
    pub secret: bool,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalStateFile {
    pub schema_version: u32,
    // TODO: why local state is in a dedicated struct?
    pub local_state: LocalState,
    #[serde(default)]
    pub collection_environment_selection: BTreeMap<Ulid, Ulid>,
    #[serde(default)]
    pub tree_state: TreeState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalState {
    pub active_global_environment_id: Option<Ulid>,
    pub last_opened_request_id: Option<Ulid>,
    #[serde(default)]
    pub theme_name: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TreeState {
    #[serde(default)]
    pub expanded_item_ids: Vec<Ulid>,
}

impl Default for LocalStateFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION_V1,
            local_state: LocalState {
                active_global_environment_id: None,
                last_opened_request_id: None,
                theme_name: None,
                updated_at: Utc::now(),
            },
            collection_environment_selection: BTreeMap::new(),
            tree_state: TreeState::default(),
        }
    }
}

impl CollectionFile {
    pub fn with_manifest_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.manifest_path = Some(path.into());
        self
    }
}

impl FolderFile {
    pub fn with_manifest_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.manifest_path = Some(path.into());
        self
    }
}

impl RequestFile {
    pub fn with_file_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.file_path = Some(path.into());
        self
    }
}

impl EnvironmentFile {
    pub fn with_file_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.file_path = Some(path.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AuthConfig, BodyConfig, CollectionFile, CollectionMeta, EnvironmentFile, EnvironmentMeta,
        EnvironmentScope, RequestDefinition, RequestFile, RequestMeta, ScriptConfig,
    };
    use crate::schema::SCHEMA_VERSION_V1;
    use chrono::Utc;
    use std::path::PathBuf;
    use ulid::Ulid;

    #[test]
    fn auth_config_accepts_title_case_type_values() {
        let none_auth: AuthConfig = toml::from_str("type = \"None\"").expect("parse none auth");
        assert!(matches!(none_auth, AuthConfig::None));

        let bearer_auth: AuthConfig =
            toml::from_str("type = \"Bearer\"\ntoken = \"abc\"").expect("parse bearer auth");
        assert!(matches!(bearer_auth, AuthConfig::Bearer { .. }));
    }

    #[test]
    fn auth_config_accepts_title_case_api_key_location() {
        let api_key_auth: AuthConfig = toml::from_str(
            "type = \"ApiKey\"\nkey = \"X-Key\"\nvalue = \"v\"\nlocation = \"Header\"",
        )
        .expect("parse api key auth");
        assert!(matches!(api_key_auth, AuthConfig::ApiKey { .. }));
    }

    #[test]
    fn body_config_accepts_title_case_mode_values() {
        let none_body: BodyConfig = toml::from_str("mode = \"None\"").expect("parse none body");
        assert!(matches!(none_body, BodyConfig::None));

        let json_body: BodyConfig =
            toml::from_str("mode = \"Json\"\ntext = \"{}\"").expect("parse json body");
        assert!(matches!(json_body, BodyConfig::Json { .. }));
    }

    #[test]
    fn request_file_runtime_path_is_not_serialized() {
        let now = Utc::now();
        let request = RequestFile {
            meta: RequestMeta {
                request_id: Ulid::new(),
                name: "Get User".to_string(),
                description: None,
                created_at: now,
                updated_at: now,
            },
            request: RequestDefinition {
                method: super::HttpMethod::Get,
                url: "https://api.example.com/users/1".to_string(),
                headers: Vec::new(),
                query_params: Vec::new(),
            },
            auth: AuthConfig::None,
            body: BodyConfig::None,
            scripts: ScriptConfig::default(),
            file_path: Some(PathBuf::from("/tmp/request.toml")),
        };

        let encoded = toml::to_string_pretty(&request).expect("encode request");
        assert!(!encoded.contains("file_path"));
    }

    #[test]
    fn collection_and_environment_runtime_paths_are_not_serialized() {
        let now = Utc::now();
        let collection = CollectionFile {
            collection: CollectionMeta {
                collection_id: Ulid::new(),
                name: "Sample".to_string(),
                description: None,
                created_at: now,
                updated_at: now,
            },
            items: Vec::new(),
            manifest_path: Some(PathBuf::from("/tmp/collection.toml")),
        };
        let environment = EnvironmentFile {
            schema_version: SCHEMA_VERSION_V1,
            environment: EnvironmentMeta {
                environment_id: Ulid::new(),
                collection_id: None,
                scope: EnvironmentScope::Global,
                name: "Global".to_string(),
                file_name: "global.toml".to_string(),
                description: None,
                created_at: now,
                updated_at: now,
            },
            variables: Vec::new(),
            file_path: Some(PathBuf::from("/tmp/global.toml")),
        };
        let folder = super::FolderFile {
            folder: super::FolderMeta {
                folder_id: Ulid::new(),
                collection_id: Ulid::new(),
                parent_folder_id: None,
                name: "Auth".to_string(),
                description: None,
                created_at: now,
                updated_at: now,
            },
            items: Vec::new(),
            manifest_path: Some(PathBuf::from("/tmp/folder.toml")),
        };

        let collection_encoded = toml::to_string_pretty(&collection).expect("encode collection");
        let environment_encoded = toml::to_string_pretty(&environment).expect("encode environment");
        let folder_encoded = toml::to_string_pretty(&folder).expect("encode folder");

        assert!(!collection_encoded.contains("manifest_path"));
        assert!(!environment_encoded.contains("file_path"));
        assert!(!folder_encoded.contains("manifest_path"));
    }
}
