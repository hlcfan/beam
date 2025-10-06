use iced::widget::{text_editor, pane_grid};
use iced::{Color, Element};
use iced::advanced::text::Highlighter;

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
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RequestTab {
    Body,
    Params,
    Headers,
    Auth,
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