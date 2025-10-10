use iced::widget::{text_editor, pane_grid};
use iced::{Color, Element};
use iced::advanced::text::Highlighter;
use crate::storage::StorageManager;
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct ResponseHighlighter {
    pub content_type: String,
}

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

#[derive(Debug)]
pub struct PostmanApp {
    pub panes: pane_grid::State<PaneContent>,
    pub collections: Vec<RequestCollection>,
    pub current_request: RequestConfig,
    pub response: Option<ResponseData>,
    pub response_body_content: text_editor::Content,
    pub selected_response_tab: ResponseTab,

    pub is_loading: bool,
    pub request_start_time: Option<std::time::Instant>,
    pub current_elapsed_time: u64, // milliseconds
    
    // Environment management
    pub environments: Vec<Environment>,
    pub active_environment: Option<usize>,
    pub show_environment_popup: bool,
    pub method_menu_open: bool,
    
    // Last opened request tracking
    pub last_opened_request: Option<(usize, usize)>, // (collection_index, request_index)
    
    // Auto-save debounce management
    pub debounce_timers: HashMap<(usize, usize), Instant>, // (collection_index, request_index) -> last_change_time
    pub debounce_delay_ms: u64, // configurable delay in milliseconds
    
    // Rename modal state
    pub show_rename_modal: bool,
    pub rename_input: String,
    pub rename_target: Option<(usize, usize)>, // (collection_index, request_index)
    
    // Double-click detection state
    pub last_click_time: Option<std::time::Instant>,
    pub last_click_target: Option<(usize, usize)>, // (collection_index, request_index)
    
    // Tooltip state for environment variables
    pub show_url_tooltip: bool,
    pub tooltip_variable_name: Option<String>,
    pub tooltip_variable_value: Option<String>,
    pub tooltip_position: Option<(f32, f32)>, // (x, y) position
    
    // Storage
    pub storage_manager: Option<StorageManager>,
}

#[derive(Debug, Clone)]
pub struct RequestCollection {
    pub name: String,
    pub requests: Vec<SavedRequest>,
    pub expanded: bool,
}

#[derive(Debug, Clone)]
pub struct SavedRequest {
    pub name: String,
    pub method: HttpMethod,
    pub url: String,
}

#[derive(Debug)]
pub struct RequestConfig {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub params: Vec<(String, String)>,
    pub body: text_editor::Content,
    pub content_type: String,
    pub auth_type: AuthType,
    pub selected_tab: RequestTab,
    
    // Authentication fields
    pub bearer_token: String,
    pub basic_username: String,
    pub basic_password: String,
    pub api_key: String,
    pub api_key_header: String,
}

impl Clone for RequestConfig {
    fn clone(&self) -> Self {
        Self {
            method: self.method.clone(),
            url: self.url.clone(),
            headers: self.headers.clone(),
            params: self.params.clone(),
            body: text_editor::Content::with_text(&self.body.text()),
            content_type: self.content_type.clone(),
            auth_type: self.auth_type.clone(),
            selected_tab: self.selected_tab.clone(),
            bearer_token: self.bearer_token.clone(),
            basic_username: self.basic_username.clone(),
            basic_password: self.basic_password.clone(),
            api_key: self.api_key.clone(),
            api_key_header: self.api_key_header.clone(),
        }
    }
}

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

    pub fn with_description(name: String, description: String) -> Self {
        Self {
            name,
            variables: std::collections::HashMap::new(),
            description: Some(description),
        }
    }

    pub fn add_variable(&mut self, key: String, value: String) {
        self.variables.insert(key, value);
    }

    pub fn get_variable(&self, key: &str) -> Option<&String> {
        self.variables.get(key)
    }

    pub fn remove_variable(&mut self, key: &str) -> Option<String> {
        self.variables.remove(key)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RequestTab {
    Body,
    Params,
    Headers,
    Auth,
    Environment,
}

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
pub enum HttpMethod {
    GET,
    POST,
    PUT,
    DELETE,
    PATCH,
    HEAD,
    OPTIONS,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AuthType {
    None,
    Bearer,
    Basic,
    ApiKey,
}

#[derive(Debug, Clone)]
pub enum PaneContent {
    Collections,
    RequestConfig,
    Response,
}

#[derive(Debug, Clone)]
pub enum Message {
    PaneResized(pane_grid::ResizeEvent),
    UrlChanged(String),
    MethodChanged(HttpMethod),

    SendRequest,
    CancelRequest,
    RequestCompleted(Result<ResponseData, String>),
    TimerTick,
    CollectionToggled(usize),
    RequestSelected(usize, usize),
    TabSelected(RequestTab),
    ResponseTabSelected(ResponseTab),
    HeaderKeyChanged(usize, String),
    HeaderValueChanged(usize, String),
    AddHeader,
    RemoveHeader(usize),
    ParamKeyChanged(usize, String),
    ParamValueChanged(usize, String),
    AddParam,
    RemoveParam(usize),
    BodyChanged(text_editor::Action),
    ResponseBodyAction(text_editor::Action),
    AuthTypeChanged(AuthType),
    BearerTokenChanged(String),
    BasicUsernameChanged(String),
    BasicPasswordChanged(String),
    ApiKeyChanged(String),
    ApiKeyHeaderChanged(String),

    // Environment management
    OpenEnvironmentPopup,
    CloseEnvironmentPopup,
    KeyPressed(iced::keyboard::Key),
    ToggleMethodMenu,
    CloseMethodMenu,
    DoNothing, // Used to prevent event propagation
    EnvironmentSelected(usize),
    AddEnvironment,
    DeleteEnvironment(usize),
    EnvironmentNameChanged(usize, String),
    EnvironmentDescriptionChanged(usize, String),
    VariableKeyChanged(usize, usize, String),
    VariableValueChanged(usize, usize, String),
    AddVariable(usize),
    RemoveVariable(usize, usize),

    AddHttpRequest(usize),
    DeleteFolder(usize),
    AddFolder(usize),
    RenameFolder(usize),
    // Request context menu actions
    SendRequestFromMenu(usize, usize),
    CopyRequestAsCurl(usize, usize),
    RenameRequest(usize, usize),
    DuplicateRequest(usize, usize),
    DeleteRequest(usize, usize),
    
    // Rename modal
    ShowRenameModal(usize, usize), // (collection_index, request_index)
    HideRenameModal,
    RenameInputChanged(String),
    ConfirmRename,
    
    // URL tooltip for environment variables
    ShowUrlTooltip(String, String, f32, f32), // (variable_name, variable_value, x, y)
    HideUrlTooltip,
    
    // Storage operations
    SaveCollection(usize),
    LoadCollections,
    SaveEnvironments,
    LoadEnvironments,
    InitializeStorage,
    StorageInitialized(Result<(), String>),
    SetStorageManager,
    CollectionsSaved(Result<(), String>),
    CollectionsLoaded(Result<Vec<RequestCollection>, String>),
    EnvironmentsSaved(Result<(), String>),
    EnvironmentsLoaded(Result<Vec<Environment>, String>),
    SaveInitialData,
    SaveLastOpenedRequest(usize, usize), // (collection_index, request_index)
    LoadLastOpenedRequest,
    LastOpenedRequestSaved(Result<(), String>),
    LastOpenedRequestLoaded(Result<Option<(usize, usize)>, String>),
    RequestConfigLoaded(Result<Option<RequestConfig>, String>),
    
    // Auto-save messages
    RequestFieldChanged {
        collection_index: usize,
        request_index: usize,
        field: RequestField,
    },
    SaveRequestDebounced {
        collection_index: usize,
        request_index: usize,
    },
    RequestSaved(Result<(), String>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum RequestField {
    Url,
    Method,
    Body,
    Headers,
    Params,
    Auth,
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