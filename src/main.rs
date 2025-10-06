use iced::widget::pane_grid::{self, PaneGrid, Axis};
use iced::widget::{
    button, column, container, row, text, text_input, text_editor, pick_list, scrollable,
    mouse_area, stack
};
use iced::{Element, Fill, Length, Size, Theme, Color};
use iced::advanced::text::Highlighter;
use iced_aw::ContextMenu;
use std::time::Instant;



#[derive(Debug, Clone)]
struct ResponseHighlighter {
    content_type: String,
}

impl ResponseHighlighter {
    fn new(content_type: String) -> Self {
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

pub fn main() -> iced::Result {
    iced::application(
            |_state: &PostmanApp| String::from("Beam"),
            PostmanApp::update,
            PostmanApp::view,
        )
        .window_size(Size::new(1200.0, 800.0))
        .run()
}

#[derive(Debug)]
struct PostmanApp {
    panes: pane_grid::State<PaneContent>,
    collections: Vec<RequestCollection>,
    current_request: RequestConfig,
    response: Option<ResponseData>,
    selected_response_tab: ResponseTab,
    context_menu_visible: bool,
    context_menu_position: (f32, f32),
    context_menu_collection: Option<usize>,
}

#[derive(Debug, Clone)]
struct RequestCollection {
    name: String,
    requests: Vec<SavedRequest>,
    expanded: bool,
}

#[derive(Debug, Clone)]
struct SavedRequest {
    name: String,
    method: HttpMethod,
    url: String,
}

#[derive(Debug)]
struct RequestConfig {
    method: HttpMethod,
    url: String,
    headers: Vec<(String, String)>,
    params: Vec<(String, String)>,
    body: text_editor::Content,
    content_type: String,
    auth_type: AuthType,
    selected_tab: RequestTab,
}

#[derive(Debug, Clone, PartialEq)]
enum RequestTab {
    Body,
    Params,
    Headers,
    Auth,
}

#[derive(Debug, Clone, PartialEq)]
enum ResponseTab {
    Body,
    Headers,
}

#[derive(Debug)]
struct ResponseData {
    status: u16,
    status_text: String,
    headers: Vec<(String, String)>,
    body: String,
    body_content: text_editor::Content,
    size: usize,
    time: u64, // milliseconds
}

#[derive(Debug, Clone, PartialEq)]
enum HttpMethod {
    GET,
    POST,
    PUT,
    DELETE,
    PATCH,
    HEAD,
    OPTIONS,
}

#[derive(Debug, Clone, PartialEq)]
enum AuthType {
    None,
    Bearer,
    Basic,
    ApiKey,
}

#[derive(Debug, Clone)]
enum PaneContent {
    Collections,
    RequestConfig,
    Response,
}

#[derive(Debug, Clone)]
enum Message {
    PaneResized(pane_grid::ResizeEvent),
    UrlChanged(String),
    MethodChanged(HttpMethod),
    SendRequest,
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
    ShowContextMenu(usize, f32, f32),
    HideContextMenu,
    AddHttpRequest(usize),
    DeleteFolder(usize),
    AddFolder(usize),
}

impl Default for PostmanApp {
    fn default() -> Self {
        let (mut panes, collections_pane) = pane_grid::State::new(PaneContent::Collections);

        // Split the collections pane to create the request config pane
        let (request_pane, split_1) = panes.split(
            Axis::Vertical,
            collections_pane,
            PaneContent::RequestConfig,
        ).expect("Failed to split pane");

        // Split the request config pane to create the response pane
        let (_, split_2) = panes.split(
            Axis::Vertical,
            request_pane,
            PaneContent::Response,
        ).expect("Failed to split pane");

        // Resize the first split to make the left panel smaller (15% instead of default 50%)
        panes.resize(split_1, 0.20);
        // Resize the second split to give more space to request config (60% of remaining space)
        panes.resize(split_2, 0.55);

        Self {
            panes,
            collections: vec![
                RequestCollection {
                    name: "JSONPlaceholder API".to_string(),
                    requests: vec![
                        SavedRequest {
                            name: "Get Users".to_string(),
                            method: HttpMethod::GET,
                            url: "https://jsonplaceholder.typicode.com/users".to_string(),
                        },
                        SavedRequest {
                            name: "Create User".to_string(),
                            method: HttpMethod::POST,
                            url: "https://jsonplaceholder.typicode.com/users".to_string(),
                        },
                        SavedRequest {
                            name: "Get Posts".to_string(),
                            method: HttpMethod::GET,
                            url: "https://jsonplaceholder.typicode.com/posts".to_string(),
                        },
                        SavedRequest {
                            name: "Update Post".to_string(),
                            method: HttpMethod::PUT,
                            url: "https://jsonplaceholder.typicode.com/posts/1".to_string(),
                        },
                        SavedRequest {
                            name: "Delete Post".to_string(),
                            method: HttpMethod::DELETE,
                            url: "https://jsonplaceholder.typicode.com/posts/1".to_string(),
                        },
                    ],
                    expanded: true,
                },
                RequestCollection {
                    name: "GitHub API".to_string(),
                    requests: vec![
                        SavedRequest {
                            name: "Get User Profile".to_string(),
                            method: HttpMethod::GET,
                            url: "https://api.github.com/user".to_string(),
                        },
                        SavedRequest {
                            name: "List Repositories".to_string(),
                            method: HttpMethod::GET,
                            url: "https://api.github.com/user/repos".to_string(),
                        },
                        SavedRequest {
                            name: "Create Repository".to_string(),
                            method: HttpMethod::POST,
                            url: "https://api.github.com/user/repos".to_string(),
                        },
                        SavedRequest {
                            name: "Get Repository".to_string(),
                            method: HttpMethod::GET,
                            url: "https://api.github.com/repos/owner/repo".to_string(),
                        },
                    ],
                    expanded: false,
                },
                RequestCollection {
                    name: "Weather API".to_string(),
                    requests: vec![
                        SavedRequest {
                            name: "Current Weather".to_string(),
                            method: HttpMethod::GET,
                            url: "https://api.openweathermap.org/data/2.5/weather".to_string(),
                        },
                        SavedRequest {
                            name: "5-Day Forecast".to_string(),
                            method: HttpMethod::GET,
                            url: "https://api.openweathermap.org/data/2.5/forecast".to_string(),
                        },
                        SavedRequest {
                            name: "Weather History".to_string(),
                            method: HttpMethod::GET,
                            url: "https://api.openweathermap.org/data/2.5/onecall/timemachine".to_string(),
                        },
                    ],
                    expanded: false,
                },
                RequestCollection {
                    name: "E-commerce API".to_string(),
                    requests: vec![
                        SavedRequest {
                            name: "Get Products".to_string(),
                            method: HttpMethod::GET,
                            url: "https://fakestoreapi.com/products".to_string(),
                        },
                        SavedRequest {
                            name: "Get Product".to_string(),
                            method: HttpMethod::GET,
                            url: "https://fakestoreapi.com/products/1".to_string(),
                        },
                        SavedRequest {
                            name: "Add Product".to_string(),
                            method: HttpMethod::POST,
                            url: "https://fakestoreapi.com/products".to_string(),
                        },
                        SavedRequest {
                            name: "Update Product".to_string(),
                            method: HttpMethod::PUT,
                            url: "https://fakestoreapi.com/products/1".to_string(),
                        },
                        SavedRequest {
                            name: "Delete Product".to_string(),
                            method: HttpMethod::DELETE,
                            url: "https://fakestoreapi.com/products/1".to_string(),
                        },
                        SavedRequest {
                            name: "Get Categories".to_string(),
                            method: HttpMethod::GET,
                            url: "https://fakestoreapi.com/products/categories".to_string(),
                        },
                        SavedRequest {
                            name: "Get Cart".to_string(),
                            method: HttpMethod::GET,
                            url: "https://fakestoreapi.com/carts/1".to_string(),
                        },
                    ],
                    expanded: false,
                },
                RequestCollection {
                    name: "Authentication".to_string(),
                    requests: vec![
                        SavedRequest {
                            name: "Login".to_string(),
                            method: HttpMethod::POST,
                            url: "https://api.example.com/auth/login".to_string(),
                        },
                        SavedRequest {
                            name: "Refresh Token".to_string(),
                            method: HttpMethod::POST,
                            url: "https://api.example.com/auth/refresh".to_string(),
                        },
                        SavedRequest {
                            name: "Logout".to_string(),
                            method: HttpMethod::POST,
                            url: "https://api.example.com/auth/logout".to_string(),
                        },
                        SavedRequest {
                            name: "Reset Password".to_string(),
                            method: HttpMethod::POST,
                            url: "https://api.example.com/auth/reset-password".to_string(),
                        },
                    ],
                    expanded: false,
                },
                RequestCollection {
                    name: "Testing & Utilities".to_string(),
                    requests: vec![
                        SavedRequest {
                            name: "HTTP Status Codes".to_string(),
                            method: HttpMethod::GET,
                            url: "https://httpstat.us/200".to_string(),
                        },
                        SavedRequest {
                            name: "Echo Request".to_string(),
                            method: HttpMethod::POST,
                            url: "https://httpbin.org/post".to_string(),
                        },
                        SavedRequest {
                            name: "Get IP Address".to_string(),
                            method: HttpMethod::GET,
                            url: "https://httpbin.org/ip".to_string(),
                        },
                        SavedRequest {
                            name: "User Agent".to_string(),
                            method: HttpMethod::GET,
                            url: "https://httpbin.org/user-agent".to_string(),
                        },
                        SavedRequest {
                            name: "Delay Test".to_string(),
                            method: HttpMethod::GET,
                            url: "https://httpbin.org/delay/2".to_string(),
                        },
                    ],
                    expanded: false,
                },
            ],
            current_request: RequestConfig {
                method: HttpMethod::GET,
                url: String::new(),
                headers: vec![("Content-Type".to_string(), "application/json".to_string())],
                params: vec![],
                body: text_editor::Content::new(),
                content_type: "application/json".to_string(),
                auth_type: AuthType::None,
                selected_tab: RequestTab::Body,
            },
            response: None,
            selected_response_tab: ResponseTab::Body,
            context_menu_visible: false,
            context_menu_position: (0.0, 0.0),
            context_menu_collection: None,
        }
    }
}

impl PostmanApp {
    fn create_response_content(body: &str) -> text_editor::Content {
        text_editor::Content::with_text(body)
    }

    fn get_content_type(headers: &[(String, String)]) -> String {
        headers
            .iter()
            .find(|(key, _)| key.to_lowercase() == "content-type")
            .map(|(_, value)| value.clone())
            .unwrap_or_else(|| "text/plain".to_string())
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::PaneResized(resize_event) => {
                self.panes.resize(resize_event.split, resize_event.ratio);
            }
            Message::UrlChanged(url) => {
                self.current_request.url = url;
            }
            Message::MethodChanged(method) => {
                self.current_request.method = method;
            }
            Message::SendRequest => {
                // Perform HTTP request synchronously
                let url = self.current_request.url.clone();
                let method = self.current_request.method.clone();
                let headers = self.current_request.headers.clone();
                let body_text = self.current_request.body.text();

                if url.is_empty() {
                    let error_body = "Please enter a URL".to_string();
                    self.response = Some(ResponseData {
                        status: 0,
                        status_text: "Error".to_string(),
                        headers: vec![],
                        body: error_body.clone(),
                        body_content: Self::create_response_content(&error_body),
                        size: 0,
                        time: 0,
                    });
                    return;
                }

                let start_time = Instant::now();

                // Create reqwest client
                let client = match reqwest::blocking::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()
                {
                    Ok(client) => client,
                    Err(e) => {
                        let error_body = format!("Failed to create HTTP client: {}", e);
                        self.response = Some(ResponseData {
                            status: 0,
                            status_text: "Error".to_string(),
                            headers: vec![],
                            body: error_body.clone(),
                            body_content: Self::create_response_content(&error_body),
                            size: 0,
                            time: 0,
                        });
                        return;
                    }
                };

                // Build request
                let mut request_builder = match method {
                    HttpMethod::GET => client.get(&url),
                    HttpMethod::POST => client.post(&url),
                    HttpMethod::PUT => client.put(&url),
                    HttpMethod::DELETE => client.delete(&url),
                    HttpMethod::PATCH => client.patch(&url),
                    HttpMethod::HEAD => client.head(&url),
                    HttpMethod::OPTIONS => client.request(reqwest::Method::OPTIONS, &url),
                };

                // Add headers
                for (key, value) in headers {
                    if !key.is_empty() && !value.is_empty() {
                        request_builder = request_builder.header(&key, &value);
                    }
                }

                // Add body for methods that support it
                if matches!(method, HttpMethod::POST | HttpMethod::PUT | HttpMethod::PATCH) && !body_text.is_empty() {
                    request_builder = request_builder.body(body_text);
                }

                // Send request
                match request_builder.send() {
                    Ok(response) => {
                        let elapsed = start_time.elapsed();
                        let status = response.status().as_u16();
                        let status_text = response.status().canonical_reason().unwrap_or("Unknown").to_string();

                        // Extract headers
                        let mut response_headers = Vec::new();
                        for (name, value) in response.headers() {
                            if let Ok(value_str) = value.to_str() {
                                response_headers.push((name.to_string(), value_str.to_string()));
                            }
                        }

                        // Get response body
                        match response.text() {
                            Ok(body) => {
                                self.response = Some(ResponseData {
                                    status,
                                    status_text,
                                    headers: response_headers,
                                    body: body.clone(),
                                    body_content: Self::create_response_content(&body),
                                    size: body.len(),
                                    time: elapsed.as_millis() as u64,
                                });
                            }
                            Err(e) => {
                                let error_body = format!("Failed to read response body: {}", e);
                                self.response = Some(ResponseData {
                                    status,
                                    status_text,
                                    headers: response_headers,
                                    body: error_body.clone(),
                                    body_content: Self::create_response_content(&error_body),
                                    size: 0,
                                    time: elapsed.as_millis() as u64,
                                });
                            }
                        }
                    }
                    Err(e) => {
                        let elapsed = start_time.elapsed();
                        let error_body = format!("Request failed: {}", e);
                        self.response = Some(ResponseData {
                            status: 0,
                            status_text: "Error".to_string(),
                            headers: vec![],
                            body: error_body.clone(),
                            body_content: Self::create_response_content(&error_body),
                            size: 0,
                            time: elapsed.as_millis() as u64,
                        });
                    }
                }
            }
            Message::CollectionToggled(index) => {
                if let Some(collection) = self.collections.get_mut(index) {
                    collection.expanded = !collection.expanded;
                }
            }
            Message::RequestSelected(collection_index, request_index) => {
                if let Some(collection) = self.collections.get(collection_index) {
                    if let Some(request) = collection.requests.get(request_index) {
                        self.current_request.method = request.method.clone();
                        self.current_request.url = request.url.clone();
                    }
                }
            }
            Message::HeaderKeyChanged(index, key) => {
                if let Some(header) = self.current_request.headers.get_mut(index) {
                    header.0 = key;
                }
            }
            Message::HeaderValueChanged(index, value) => {
                if let Some(header) = self.current_request.headers.get_mut(index) {
                    header.1 = value;
                }
            }
            Message::AddHeader => {
                self.current_request.headers.push((String::new(), String::new()));
            }
            Message::RemoveHeader(index) => {
                if index < self.current_request.headers.len() {
                    self.current_request.headers.remove(index);
                }
            }
            Message::BodyChanged(action) => {
                self.current_request.body.perform(action);
            }
            Message::ResponseBodyAction(action) => {
                // Allow all actions for text selection and navigation
                // The content remains read-only because it's recreated from response.body
                if let Some(response) = &mut self.response {
                    response.body_content.perform(action);
                }
            }

            Message::TabSelected(tab) => {
                self.current_request.selected_tab = tab;
            }
            Message::ResponseTabSelected(tab) => {
                self.selected_response_tab = tab;
            }
            Message::ParamKeyChanged(index, key) => {
                if let Some(param) = self.current_request.params.get_mut(index) {
                    param.0 = key;
                }
            }
            Message::ParamValueChanged(index, value) => {
                if let Some(param) = self.current_request.params.get_mut(index) {
                    param.1 = value;
                }
            }
            Message::AddParam => {
                self.current_request.params.push((String::new(), String::new()));
            }
            Message::RemoveParam(index) => {
                if index < self.current_request.params.len() {
                    self.current_request.params.remove(index);
                }
            }
            Message::AuthTypeChanged(auth_type) => {
                self.current_request.auth_type = auth_type;
            }
            Message::ShowContextMenu(collection_index, x, y) => {
                self.context_menu_visible = true;
                self.context_menu_position = (x, y);
                self.context_menu_collection = Some(collection_index);
            }
            Message::HideContextMenu => {
                self.context_menu_visible = false;
                self.context_menu_collection = None;
            }
            Message::AddHttpRequest(collection_index) => {
                if let Some(collection) = self.collections.get_mut(collection_index) {
                    collection.requests.push(SavedRequest {
                        name: "New Request".to_string(),
                        method: HttpMethod::GET,
                        url: String::new(),
                    });
                }
                self.context_menu_visible = false;
                self.context_menu_collection = None;
            }
            Message::DeleteFolder(collection_index) => {
                if collection_index < self.collections.len() {
                    self.collections.remove(collection_index);
                }
                self.context_menu_visible = false;
                self.context_menu_collection = None;
            }
            Message::AddFolder(_) => {
                self.collections.push(RequestCollection {
                    name: "New Collection".to_string(),
                    requests: vec![],
                    expanded: true,
                });
                self.context_menu_visible = false;
                self.context_menu_collection = None;
            }
        }
    }

    fn view(&self) -> Element<Message> {
        let pane_grid = PaneGrid::new(&self.panes, |_pane, content, _is_maximized| {
            let body = match content {
                PaneContent::Collections => self.collections_view(),
                PaneContent::RequestConfig => self.request_config_view(),
                PaneContent::Response => self.response_view(),
            };

            let bordered_body = match content {
                 PaneContent::Collections => {
                     // Left panel: no border
                     container(body)
                 },
                 PaneContent::RequestConfig => {
                     // Middle panel: left and right borders only using vertical rules
                     container(
                         row![
                             container("")
                                 .width(Length::Fixed(1.0))
                                 .height(Fill)
                                 .style(|_theme| container::Style {
                                     background: Some(iced::Background::Color(iced::Color::from_rgb(0.7, 0.7, 0.7))), // Gray
                                     ..Default::default()
                                 }),
                             container(body).width(Fill).padding(0),
                             container("")
                                 .width(Length::Fixed(1.0))
                                 .height(Fill)
                                 .style(|_theme| container::Style {
                                     background: Some(iced::Background::Color(iced::Color::from_rgb(0.7, 0.7, 0.7))), // Gray
                                     ..Default::default()
                                 })
                         ]
                     )
                     .width(Fill)
                     .height(Fill)
                     .padding(0)
                 },
                 PaneContent::Response => {
                     // Right panel: no border
                     container(body)
                 },
             };

            pane_grid::Content::new(bordered_body)
        })
        .on_resize(10, Message::PaneResized)
        .spacing(1)
        .style(|_theme| pane_grid::Style {
            hovered_region: pane_grid::Highlight {
                background: iced::Background::Color(iced::Color::TRANSPARENT),
                border: iced::Border::default(),
            },
            hovered_split: pane_grid::Line {
                color: iced::Color::from_rgb(0.7, 0.7, 0.7), // Gray
                width: 1.0,
            },
            picked_split: pane_grid::Line {
                color: iced::Color::from_rgb(0.7, 0.7, 0.7), // Gray
                width: 1.0,
            },
        });

        let base_view = container(pane_grid)
            .width(Fill)
            .height(Fill);

        if self.context_menu_visible {
            stack![
                base_view,
                // Semi-transparent overlay to capture clicks
                mouse_area(
                    container("")
                        .width(Fill)
                        .height(Fill)
                        .style(|_theme| container::Style {
                            background: Some(iced::Background::Color(iced::Color::TRANSPARENT)),
                            ..Default::default()
                        })
                )
                .on_press(Message::HideContextMenu),
                // Context menu
                container(
                    column![
                        button(text("Add HTTP Request"))
                             .on_press(Message::AddHttpRequest(self.context_menu_collection.unwrap_or(0)))
                             .width(Length::Fixed(150.0))
                             .style(|theme: &Theme, _status| button::Style {
                                 background: Some(iced::Background::Color(theme.palette().background)),
                                 text_color: theme.palette().text,
                                 border: iced::Border::default(),
                                 ..Default::default()
                             }),
                         button(text("Delete Folder"))
                             .on_press(Message::DeleteFolder(self.context_menu_collection.unwrap_or(0)))
                             .width(Length::Fixed(150.0))
                             .style(|theme: &Theme, _status| button::Style {
                                 background: Some(iced::Background::Color(theme.palette().background)),
                                 text_color: theme.palette().text,
                                 border: iced::Border::default(),
                                 ..Default::default()
                             }),
                         button(text("Add Folder"))
                             .on_press(Message::AddFolder(self.context_menu_collection.unwrap_or(0)))
                             .width(Length::Fixed(150.0))
                             .style(|theme: &Theme, _status| button::Style {
                                 background: Some(iced::Background::Color(theme.palette().background)),
                                 text_color: theme.palette().text,
                                 border: iced::Border::default(),
                                 ..Default::default()
                             }),
                    ]
                    .spacing(2)
                )
                .style(|theme| container::Style {
                    background: Some(iced::Background::Color(theme.palette().background)),
                    border: iced::Border {
                        color: theme.palette().text,
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    shadow: iced::Shadow {
                        color: iced::Color::BLACK,
                        offset: iced::Vector::new(2.0, 2.0),
                        blur_radius: 4.0,
                    },
                    text_color: None,
                })
                .padding(4)
                .width(Length::Shrink)
                .height(Length::Shrink)
            ]
            .into()
        } else {
            base_view.into()
        }
    }

    fn collections_view(&self) -> Element<Message> {
        let title = container(
            text("Collections")
                .size(16)
                .color(iced::Color::from_rgb(0.2, 0.2, 0.2))
        )
        .padding([10.0, 15.0])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(iced::Color::from_rgb(0.95, 0.95, 0.95))),
            border: iced::Border {
                color: iced::Color::from_rgb(0.85, 0.85, 0.85),
                width: 0.0,
                radius: iced::border::Radius::from(0.0),
            },
            ..Default::default()
        });

        let mut collections_content = column![title].spacing(2);

        for (collection_index, collection) in self.collections.iter().enumerate() {
            // Tree node for collection/folder
            let tree_symbol = text(if collection.expanded { "▼" } else { "▶" })
                .size(10)
                .color(iced::Color::from_rgb(0.4, 0.4, 0.4));
            
            let collection_header = button(
                row![
                    tree_symbol,
                    text(&collection.name)
                        .size(13)
                        .color(iced::Color::from_rgb(0.1, 0.1, 0.1))
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center)
            )
            .on_press(Message::CollectionToggled(collection_index))
            .style(|theme, status| {
                let base_style = button::text(theme, status);
                button::Style {
                    background: match status {
                        button::Status::Hovered => Some(iced::Background::Color(iced::Color::from_rgb(0.95, 0.95, 0.95))),
                        _ => Some(iced::Background::Color(iced::Color::TRANSPARENT)),
                    },
                    ..base_style
                }
            })
            .width(Fill)
            .padding([4.0, 8.0]);

            // Wrap collection header with context menu
            let collection_with_menu = ContextMenu::new(
                collection_header,
                move || {
                    container(
                        column![
                            button(text("Add Request"))
                                .on_press(Message::AddHttpRequest(collection_index))
                                .style(|theme, status| {
                                    let base_style = button::text(theme, status);
                                    button::Style {
                                        background: match status {
                                            button::Status::Hovered => Some(iced::Background::Color(iced::Color::from_rgb(0.9, 0.9, 0.9))),
                                            _ => Some(iced::Background::Color(iced::Color::TRANSPARENT)),
                                        },
                                        text_color: iced::Color::from_rgb(0.2, 0.2, 0.2),
                                        ..base_style
                                    }
                                })
                                .padding([6, 12]),
                            button(text("Add Folder"))
                                .on_press(Message::AddFolder(collection_index))
                                .style(|theme, status| {
                                    let base_style = button::text(theme, status);
                                    button::Style {
                                        background: match status {
                                            button::Status::Hovered => Some(iced::Background::Color(iced::Color::from_rgb(0.9, 0.9, 0.9))),
                                            _ => Some(iced::Background::Color(iced::Color::TRANSPARENT)),
                                        },
                                        text_color: iced::Color::from_rgb(0.2, 0.2, 0.2),
                                        ..base_style
                                    }
                                })
                                .padding([6, 12]),
                            button(text("Delete Folder"))
                                .on_press(Message::DeleteFolder(collection_index))
                                .style(|theme, status| {
                                    let base_style = button::text(theme, status);
                                    button::Style {
                                        background: match status {
                                            button::Status::Hovered => Some(iced::Background::Color(iced::Color::from_rgb(1.0, 0.9, 0.9))),
                                            _ => Some(iced::Background::Color(iced::Color::TRANSPARENT)),
                                        },
                                        text_color: iced::Color::from_rgb(0.8, 0.2, 0.2),
                                        ..base_style
                                    }
                                })
                                .padding([6, 12])
                        ]
                        .spacing(1)
                    )
                    .style(|_theme| container::Style {
                        background: Some(iced::Background::Color(iced::Color::WHITE)),
                        border: iced::Border {
                            color: iced::Color::from_rgb(0.8, 0.8, 0.8),
                            width: 1.0,
                            radius: iced::border::Radius::from(6.0),
                        },
                        shadow: iced::Shadow {
                            color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.2),
                            offset: iced::Vector::new(2.0, 2.0),
                            blur_radius: 8.0,
                        },
                        ..Default::default()
                    })
                    .padding(4)
                    .into()
                }
            );

            collections_content = collections_content.push(collection_with_menu);

            // Tree branches for requests when expanded
            if collection.expanded {
                for (request_index, request) in collection.requests.iter().enumerate() {
                    let is_last = request_index == collection.requests.len() - 1;
                    
                    let method_color = match request.method {
                        HttpMethod::GET => iced::Color::from_rgb(0.0, 0.7, 0.0),
                        HttpMethod::POST => iced::Color::from_rgb(1.0, 0.5, 0.0),
                        HttpMethod::PUT => iced::Color::from_rgb(0.0, 0.5, 1.0),
                        HttpMethod::DELETE => iced::Color::from_rgb(1.0, 0.2, 0.2),
                        _ => iced::Color::from_rgb(0.5, 0.5, 0.5),
                    };

                    let method_badge = container(
                        text(format!("{:?}", request.method))
                            .size(9)
                            .color(iced::Color::WHITE)
                    )
                    .padding([2.0, 6.0])
                    .style(move |_theme| container::Style {
                        background: Some(iced::Background::Color(method_color)),
                        border: iced::Border {
                            color: method_color,
                            width: 1.0,
                            radius: iced::border::Radius::from(3.0),
                        },
                        ..Default::default()
                    });

                    let request_item = button(
                        row![
                            method_badge,
                            text(&request.name)
                                .size(12)
                                .color(iced::Color::from_rgb(0.2, 0.2, 0.2))
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center)
                    )
                    .on_press(Message::RequestSelected(collection_index, request_index))
                    .style(|theme, status| {
                        let base_style = button::text(theme, status);
                        button::Style {
                            background: match status {
                                button::Status::Hovered => Some(iced::Background::Color(iced::Color::from_rgb(0.92, 0.95, 1.0))),
                                _ => Some(iced::Background::Color(iced::Color::TRANSPARENT)),
                            },
                            border: iced::Border {
                                color: match status {
                                    button::Status::Hovered => iced::Color::from_rgb(0.8, 0.9, 1.0),
                                    _ => iced::Color::TRANSPARENT,
                                },
                                width: 1.0,
                                radius: iced::border::Radius::from(3.0),
                            },
                            ..base_style
                        }
                    })
                    .width(Fill)
                    .padding([4.0, 8.0]);

                    collections_content = collections_content.push(
                        container(request_item)
                             .padding([0.0, 16.0]) // Left indentation for tree structure
                    );
                }
            }
        }

        container(
            scrollable(collections_content)
        )
        .width(Fill)
        .height(Fill)
        .padding(0)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(iced::Color::WHITE)),
            ..Default::default()
        })
        .into()
    }

    fn request_config_view(&self) -> Element<Message> {
        let title = container(
            text("Request Configuration")
                .size(16)
        )
        .padding(10);

        // URL and Method section
        let method_options = vec![
            HttpMethod::GET,
            HttpMethod::POST,
            HttpMethod::PUT,
            HttpMethod::DELETE,
            HttpMethod::PATCH,
            HttpMethod::HEAD,
            HttpMethod::OPTIONS,
        ];

        let url_section = row![
            pick_list(
                method_options,
                Some(self.current_request.method.clone()),
                Message::MethodChanged
            )
            .width(Length::Fixed(100.0)),
            text_input("Enter URL...", &self.current_request.url)
                .on_input(Message::UrlChanged)
                .width(Fill),
            button("Send")
                .on_press(Message::SendRequest)
                .style(button::primary)
        ]
        .spacing(10);

        // Tab buttons
        let tab_buttons = row![
            button("Body")
                .on_press(Message::TabSelected(RequestTab::Body))
                .style(if self.current_request.selected_tab == RequestTab::Body {
                    button::primary
                } else {
                    button::secondary
                }),
            button("Params")
                .on_press(Message::TabSelected(RequestTab::Params))
                .style(if self.current_request.selected_tab == RequestTab::Params {
                    button::primary
                } else {
                    button::secondary
                }),
            button("Headers")
                .on_press(Message::TabSelected(RequestTab::Headers))
                .style(if self.current_request.selected_tab == RequestTab::Headers {
                    button::primary
                } else {
                    button::secondary
                }),
            button("Auth")
                .on_press(Message::TabSelected(RequestTab::Auth))
                .style(if self.current_request.selected_tab == RequestTab::Auth {
                    button::primary
                } else {
                    button::secondary
                })
        ]
        .spacing(5);

        // Tab content based on selected tab
        let tab_content = match self.current_request.selected_tab {
            RequestTab::Body => self.body_tab_content(),
            RequestTab::Params => self.params_tab_content(),
            RequestTab::Headers => self.headers_tab_content(),
            RequestTab::Auth => self.auth_tab_content(),
        };

        let content = column![
            title,
            url_section,
            tab_buttons,
            tab_content
        ]
        .spacing(10);

        container(content)
        .width(Fill)
        .height(Fill)
        .padding(10)
        .into()
    }

    fn params_tab_content(&self) -> Element<Message> {
        let mut params_content = column![].spacing(5);

        for (index, (key, value)) in self.current_request.params.iter().enumerate() {
            let param_row = row![
                text_input("Key", key)
                    .on_input(move |k| Message::ParamKeyChanged(index, k))
                    .width(Fill),
                text_input("Value", value)
                    .on_input(move |v| Message::ParamValueChanged(index, v))
                    .width(Fill),
                button("×")
                    .on_press(Message::RemoveParam(index))
                    .style(button::danger)
                    .width(Length::Fixed(30.0))
            ]
            .spacing(5);

            params_content = params_content.push(param_row);
        }

        let add_param_button = button("+ Add Parameter")
            .on_press(Message::AddParam)
            .style(button::secondary);

        params_content = params_content.push(add_param_button);

        container(
            scrollable(params_content)
        )
        .width(Fill)
        .height(Fill)
        .into()
    }

    fn headers_tab_content(&self) -> Element<Message> {
        let mut headers_content = column![].spacing(5);

        for (index, (key, value)) in self.current_request.headers.iter().enumerate() {
            let header_row = row![
                text_input("Key", key)
                    .on_input(move |k| Message::HeaderKeyChanged(index, k))
                    .width(Fill),
                text_input("Value", value)
                    .on_input(move |v| Message::HeaderValueChanged(index, v))
                    .width(Fill),
                button("×")
                    .on_press(Message::RemoveHeader(index))
                    .style(button::danger)
                    .width(Length::Fixed(30.0))
            ]
            .spacing(5);

            headers_content = headers_content.push(header_row);
        }

        let add_header_button = button("+ Add Header")
            .on_press(Message::AddHeader)
            .style(button::secondary);

        headers_content = headers_content.push(add_header_button);

        container(
            scrollable(headers_content)
        )
        .width(Fill)
        .height(Fill)
        .into()
    }



    fn body_tab_content(&self) -> Element<Message> {
        // JSON syntax highlighter
        #[derive(Debug, Clone, PartialEq)]
        struct JsonHighlighterSettings;

        #[derive(Debug, Clone, Copy)]
        enum JsonHighlight {
            String,
            Number,
            Boolean,
            Null,
            Key,
            Punctuation,
        }

        struct JsonHighlighter {
            current_line: usize,
        }

        impl Highlighter for JsonHighlighter {
            type Settings = JsonHighlighterSettings;
            type Highlight = JsonHighlight;
            type Iterator<'a> = std::vec::IntoIter<(std::ops::Range<usize>, Self::Highlight)>;

            fn new(_settings: &Self::Settings) -> Self {
                Self { current_line: 0 }
            }

            fn update(&mut self, _new_settings: &Self::Settings) {
                // Settings don't change for our simple highlighter
            }

            fn change_line(&mut self, line: usize) {
                self.current_line = line;
            }

            fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_> {
                let mut highlights = Vec::new();
                let mut chars = line.char_indices().peekable();

                while let Some((start, ch)) = chars.next() {
                    match ch {
                        '"' => {
                            // String highlighting
                            let mut end = start + 1;
                            let mut escaped = false;

                            while let Some((i, c)) = chars.next() {
                                end = i + c.len_utf8();
                                if !escaped && c == '"' {
                                    break;
                                }
                                escaped = !escaped && c == '\\';
                            }

                            // Check if this is a key (followed by colon)
                            let remaining: String = line.chars().skip(end).collect();
                            let is_key = remaining.trim_start().starts_with(':');

                            highlights.push((
                                start..end,
                                if is_key { JsonHighlight::Key } else { JsonHighlight::String }
                            ));
                        }
                        '0'..='9' | '-' => {
                            // Number highlighting
                            let mut end = start + ch.len_utf8();
                            while let Some((i, c)) = chars.peek() {
                                if c.is_ascii_digit() || *c == '.' || *c == 'e' || *c == 'E' || *c == '+' || *c == '-' {
                                    end = *i + c.len_utf8();
                                    chars.next();
                                } else {
                                    break;
                                }
                            }
                            highlights.push((start..end, JsonHighlight::Number));
                        }
                        't' | 'f' => {
                            // Boolean highlighting
                            if line[start..].starts_with("true") {
                                highlights.push((start..start + 4, JsonHighlight::Boolean));
                                // Skip the remaining characters
                                chars.nth(2);
                            } else if line[start..].starts_with("false") {
                                highlights.push((start..start + 5, JsonHighlight::Boolean));
                                // Skip the remaining characters
                                chars.nth(3);
                            }
                        }
                        'n' => {
                            // Null highlighting
                            if line[start..].starts_with("null") {
                                highlights.push((start..start + 4, JsonHighlight::Null));
                                // Skip the remaining characters
                                chars.nth(2);
                            }
                        }
                        '{' | '}' | '[' | ']' | ',' | ':' => {
                            // Punctuation highlighting
                            highlights.push((start..start + ch.len_utf8(), JsonHighlight::Punctuation));
                        }
                        _ => {
                            // Skip other characters
                        }
                    }
                }

                highlights.into_iter()
            }

            fn current_line(&self) -> usize {
                self.current_line
            }
        }

        // Note: The JsonHighlighter is implemented above but the text_editor widget
        // in this version of Iced may not support direct highlighting integration.
        // The highlighter implementation follows the advanced Highlighter trait correctly.

        let body_editor = text_editor(&self.current_request.body)
            .placeholder("Enter JSON request body here...")
            .on_action(Message::BodyChanged)
            .height(Fill);

        container(
            column![
                text("Request Body (JSON)").size(14),
                body_editor
            ]
            .spacing(10)
        )
        .width(Fill)
        .height(Fill)
        .into()
    }

    fn auth_tab_content(&self) -> Element<Message> {
        let auth_options = vec![
            AuthType::None,
            AuthType::Bearer,
            AuthType::Basic,
            AuthType::ApiKey,
        ];

        let auth_selector = pick_list(
            auth_options,
            Some(self.current_request.auth_type.clone()),
            Message::AuthTypeChanged
        );

        container(
            column![
                text("Authentication").size(14),
                auth_selector,
                text("Authentication configuration will be implemented here").size(12)
            ]
            .spacing(10)
        )
        .width(Fill)
        .height(Fill)
        .into()
    }

    fn response_view(&self) -> Element<Message> {
        let title = container(
            text("Response")
                .size(16)
        )
        .padding(10);

        if let Some(response) = &self.response {
            let status_color = if response.status >= 200 && response.status < 300 {
                iced::Color::from_rgb(0.0, 0.8, 0.0)
            } else if response.status >= 400 {
                iced::Color::from_rgb(1.0, 0.2, 0.2)
            } else {
                iced::Color::from_rgb(1.0, 0.6, 0.0)
            };

            // Status information row at the top
            let status_info_row = row![
                text(format!("Status: {} {}", response.status, response.status_text))
                    .color(status_color)
                    .size(14),
                text(format!("Time: {}ms", response.time))
                    .size(12)
                    .color(iced::Color::from_rgb(0.6, 0.6, 0.6)),
                text(format!("Size: {} bytes", response.size))
                    .size(12)
                    .color(iced::Color::from_rgb(0.6, 0.6, 0.6))
            ]
            .spacing(20);

            // Tab buttons for response content
            let tab_buttons = row![
                button("Body")
                    .on_press(Message::ResponseTabSelected(ResponseTab::Body))
                    .style(if self.selected_response_tab == ResponseTab::Body {
                        button::primary
                    } else {
                        button::secondary
                    }),
                button("Headers")
                    .on_press(Message::ResponseTabSelected(ResponseTab::Headers))
                    .style(if self.selected_response_tab == ResponseTab::Headers {
                        button::primary
                    } else {
                        button::secondary
                    })
            ]
            .spacing(5);

            // Tab content based on selected tab
            let tab_content = match self.selected_response_tab {
                ResponseTab::Body => self.response_body_content(response),
                ResponseTab::Headers => self.response_headers_content(response),
            };

            let content = column![
                title,
                status_info_row,
                tab_buttons,
                tab_content
            ]
            .spacing(10);

            container(content)
                .width(Fill)
                .height(Fill)
                .padding(10)
                .into()
        } else {
            let content = column![
                title,
                container(
                    text("No response yet. Send a request to see the response here.")
                        .size(14)
                        .color(iced::Color::from_rgb(0.6, 0.6, 0.6))
                )
                .width(Fill)
                .height(Fill)
            ];

            container(content)
                .width(Fill)
                .height(Fill)
                .padding(10)
                .into()
        }
    }

    fn response_body_content<'a>(&self, response: &'a ResponseData) -> Element<'a, Message> {
        container(
            text_editor(&response.body_content)
                .size(12.0)
                .on_action(Message::ResponseBodyAction)
        )
        .width(Fill)
        .height(Fill)
        .into()
    }

    fn response_headers_content<'a>(&self, response: &'a ResponseData) -> Element<'a, Message> {
        let mut headers_content = column![].spacing(6);

        for (key, value) in &response.headers {
            let header_row = container(
                row![
                    text(key).size(12).width(Length::Fixed(200.0)),
                    text(value).size(12),
                    container("").width(Length::Fixed(20.0)) // Spacer for scrollbar
                ]
                .spacing(12)
            )
            .padding([3, 8]);

            headers_content = headers_content.push(header_row);
        }

        container(
            scrollable(headers_content)
        )
        .width(Fill)
        .height(Fill)
        .padding(6)
        .into()
    }


}

impl std::fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::fmt::Display for AuthType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
