mod types;
mod http;
mod ui;

use types::*;
use http::*;
use ui::*;

use iced::widget::pane_grid::{self, PaneGrid, Axis};
use iced::widget::{
    button, column, container, row, text, text_input, text_editor, pick_list, scrollable,
    mouse_area, stack, Space
};
use iced::{Element, Fill, Length, Size, Theme, Color, Task, Vector};
use iced_aw::ContextMenu;

pub fn main() -> iced::Result {
    iced::application(
            |_state: &PostmanApp| String::from("Beam"),
            PostmanApp::update,
            PostmanApp::view,
        )
        .subscription(PostmanApp::subscription)
        .window_size(Size::new(1200.0, 800.0))
        .run()
}

impl Default for PostmanApp {
    fn default() -> Self {
        let (mut panes, collections_pane) = pane_grid::State::new(PaneContent::Collections);

        // Split vertically to create request config pane (middle panel)
        let (request_pane, first_split) = panes.split(
            Axis::Vertical,
            collections_pane,
            PaneContent::RequestConfig,
        ).unwrap();

        // Split vertically again to create response pane (right panel)
        let (_, second_split) = panes.split(
            Axis::Vertical,
            request_pane,
            PaneContent::Response,
        ).unwrap();

        // Set three-panel horizontal layout ratios
        // Collections: 25%, Request Config: 40%, Response: 35%
        panes.resize(first_split, 0.25);
        panes.resize(second_split, 0.533); // 40/(40+35) = 0.533

        let collections = vec![
            RequestCollection {
                name: "JSONPlaceholder API".to_string(),
                requests: vec![
                    SavedRequest {
                        name: "Get All Users".to_string(),
                        method: HttpMethod::GET,
                        url: "https://jsonplaceholder.typicode.com/users".to_string(),
                    },
                    SavedRequest {
                        name: "Get Single User".to_string(),
                        method: HttpMethod::GET,
                        url: "https://jsonplaceholder.typicode.com/users/1".to_string(),
                    },
                    SavedRequest {
                        name: "Create User".to_string(),
                        method: HttpMethod::POST,
                        url: "https://jsonplaceholder.typicode.com/users".to_string(),
                    },
                    SavedRequest {
                        name: "Update User".to_string(),
                        method: HttpMethod::PUT,
                        url: "https://jsonplaceholder.typicode.com/users/1".to_string(),
                    },
                    SavedRequest {
                        name: "Patch User".to_string(),
                        method: HttpMethod::PATCH,
                        url: "https://jsonplaceholder.typicode.com/users/1".to_string(),
                    },
                    SavedRequest {
                        name: "Delete User".to_string(),
                        method: HttpMethod::DELETE,
                        url: "https://jsonplaceholder.typicode.com/users/1".to_string(),
                    },
                    SavedRequest {
                        name: "Get All Posts".to_string(),
                        method: HttpMethod::GET,
                        url: "https://jsonplaceholder.typicode.com/posts".to_string(),
                    },
                    SavedRequest {
                        name: "Get Comments".to_string(),
                        method: HttpMethod::GET,
                        url: "https://jsonplaceholder.typicode.com/comments".to_string(),
                    },
                ],
                expanded: true,
            },
            RequestCollection {
                name: "HTTPBin Testing".to_string(),
                requests: vec![
                    SavedRequest {
                        name: "Health Check".to_string(),
                        method: HttpMethod::GET,
                        url: "https://httpbin.org/status/200".to_string(),
                    },
                    SavedRequest {
                        name: "Get IP Address".to_string(),
                        method: HttpMethod::GET,
                        url: "https://httpbin.org/ip".to_string(),
                    },
                    SavedRequest {
                        name: "Get User Agent".to_string(),
                        method: HttpMethod::GET,
                        url: "https://httpbin.org/user-agent".to_string(),
                    },
                    SavedRequest {
                        name: "Get Headers".to_string(),
                        method: HttpMethod::GET,
                        url: "https://httpbin.org/headers".to_string(),
                    },
                    SavedRequest {
                        name: "Test POST Data".to_string(),
                        method: HttpMethod::POST,
                        url: "https://httpbin.org/post".to_string(),
                    },
                    SavedRequest {
                        name: "Test PUT Data".to_string(),
                        method: HttpMethod::PUT,
                        url: "https://httpbin.org/put".to_string(),
                    },
                    SavedRequest {
                        name: "Test PATCH Data".to_string(),
                        method: HttpMethod::PATCH,
                        url: "https://httpbin.org/patch".to_string(),
                    },
                    SavedRequest {
                        name: "Test DELETE".to_string(),
                        method: HttpMethod::DELETE,
                        url: "https://httpbin.org/delete".to_string(),
                    },
                    SavedRequest {
                        name: "Delay 2 Seconds".to_string(),
                        method: HttpMethod::GET,
                        url: "https://httpbin.org/delay/2".to_string(),
                    },
                    SavedRequest {
                        name: "Random UUID".to_string(),
                        method: HttpMethod::GET,
                        url: "https://httpbin.org/uuid".to_string(),
                    },
                ],
                expanded: false,
            },
            RequestCollection {
                name: "Size Testing".to_string(),
                requests: vec![
                    SavedRequest {
                        name: "Small Response (1KB)".to_string(),
                        method: HttpMethod::GET,
                        url: "https://httpbin.org/bytes/1024".to_string(),
                    },
                    SavedRequest {
                        name: "Medium Response (10KB)".to_string(),
                        method: HttpMethod::GET,
                        url: "https://httpbin.org/bytes/10240".to_string(),
                    },
                    SavedRequest {
                        name: "Large Response (100KB)".to_string(),
                        method: HttpMethod::GET,
                        url: "https://httpbin.org/bytes/102400".to_string(),
                    },
                    SavedRequest {
                        name: "Very Large Response (1MB)".to_string(),
                        method: HttpMethod::GET,
                        url: "https://httpbin.org/bytes/1048576".to_string(),
                    },
                ],
                expanded: false,
            },
            RequestCollection {
                name: "Status Code Tests".to_string(),
                requests: vec![
                    SavedRequest {
                        name: "200 OK".to_string(),
                        method: HttpMethod::GET,
                        url: "https://httpbin.org/status/200".to_string(),
                    },
                    SavedRequest {
                        name: "201 Created".to_string(),
                        method: HttpMethod::GET,
                        url: "https://httpbin.org/status/201".to_string(),
                    },
                    SavedRequest {
                        name: "400 Bad Request".to_string(),
                        method: HttpMethod::GET,
                        url: "https://httpbin.org/status/400".to_string(),
                    },
                    SavedRequest {
                        name: "401 Unauthorized".to_string(),
                        method: HttpMethod::GET,
                        url: "https://httpbin.org/status/401".to_string(),
                    },
                    SavedRequest {
                        name: "404 Not Found".to_string(),
                        method: HttpMethod::GET,
                        url: "https://httpbin.org/status/404".to_string(),
                    },
                    SavedRequest {
                        name: "500 Server Error".to_string(),
                        method: HttpMethod::GET,
                        url: "https://httpbin.org/status/500".to_string(),
                    },
                ],
                expanded: false,
            },
            RequestCollection {
                name: "Real APIs".to_string(),
                requests: vec![
                    SavedRequest {
                        name: "GitHub API - User Info".to_string(),
                        method: HttpMethod::GET,
                        url: "https://api.github.com/users/octocat".to_string(),
                    },
                    SavedRequest {
                        name: "GitHub API - Repos".to_string(),
                        method: HttpMethod::GET,
                        url: "https://api.github.com/users/octocat/repos".to_string(),
                    },
                    SavedRequest {
                        name: "REST Countries".to_string(),
                        method: HttpMethod::GET,
                        url: "https://restcountries.com/v3.1/name/canada".to_string(),
                    },
                    SavedRequest {
                        name: "Cat Facts".to_string(),
                        method: HttpMethod::GET,
                        url: "https://catfact.ninja/fact".to_string(),
                    },
                    SavedRequest {
                        name: "Random Quote".to_string(),
                        method: HttpMethod::GET,
                        url: "https://api.quotable.io/random".to_string(),
                    },
                ],
                expanded: false,
            },
        ];

        Self {
            panes,
            collections,
            current_request: RequestConfig {
                method: HttpMethod::GET,
                url: String::new(),
                headers: vec![("Content-Type".to_string(), "application/json".to_string())],
                params: vec![],
                body: text_editor::Content::new(),
                content_type: "application/json".to_string(),
                auth_type: AuthType::None,
                selected_tab: RequestTab::Body,
                bearer_token: String::new(),
                basic_username: String::new(),
                basic_password: String::new(),
                api_key: String::new(),
                api_key_header: "X-API-Key".to_string(),
            },
            response: None,
            response_body_content: text_editor::Content::new(),
            selected_response_tab: ResponseTab::Body,

            is_loading: false,
            request_start_time: None,
            current_elapsed_time: 0,
            
            // Initialize with a default environment
            environments: vec![
                {
                    let mut env = Environment::with_description(
                        "Development".to_string(),
                        "Development environment variables".to_string(),
                    );
                    // Add some sample variables for testing
                    env.add_variable("base_url".to_string(), "https://jsonplaceholder.typicode.com".to_string());
                    env.add_variable("api_version".to_string(), "v1".to_string());
                    env.add_variable("user_id".to_string(), "1".to_string());
                    env.add_variable("auth_token".to_string(), "your-auth-token-here".to_string());
                    env
                }
            ],
            active_environment: Some(0),
            show_environment_popup: false,
            method_menu_open: false,
        }
    }
}

impl PostmanApp {
    fn create_response_content(body: &str) -> text_editor::Content {
        text_editor::Content::with_text(body)
    }

    /// Resolves variables in the format {{variable_name}} using the active environment
    fn resolve_variables(&self, input: &str) -> String {
        if let Some(active_env_index) = self.active_environment {
            if let Some(active_env) = self.environments.get(active_env_index) {
                let mut result = input.to_string();
                
                // Use regex to find all {{variable_name}} patterns
                use regex::Regex;
                let re = Regex::new(r"\{\{([^}]+)\}\}").unwrap();
                
                // Replace each variable with its value from the active environment
                for captures in re.captures_iter(input) {
                    if let Some(var_name) = captures.get(1) {
                        let var_name = var_name.as_str().trim();
                        if let Some(var_value) = active_env.get_variable(var_name) {
                            let pattern = format!("{{{{{}}}}}", var_name);
                            result = result.replace(&pattern, var_value);
                        }
                    }
                }
                
                return result;
            }
        }
        
        // If no active environment or variable not found, return original string
        input.to_string()
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::PaneResized(event) => {
                self.panes.resize(event.split, event.ratio);
                Task::none()
            }
            Message::UrlChanged(url) => {
                self.current_request.url = url;
                Task::none()
            }
            Message::MethodChanged(method) => {
                self.current_request.method = method;
                self.method_menu_open = false; // Close menu after selection
                Task::none()
            }
            Message::SendRequest => {
                self.is_loading = true;
                self.request_start_time = Some(std::time::Instant::now());
                self.current_elapsed_time = 0;
                
                // Create a copy of the config with resolved variables
                let mut config = self.current_request.clone();
                
                // Resolve variables in URL
                config.url = self.resolve_variables(&config.url);
                
                // Resolve variables in headers
                for (key, value) in &mut config.headers {
                    *key = self.resolve_variables(key);
                    *value = self.resolve_variables(value);
                }
                
                // Resolve variables in params
                for (key, value) in &mut config.params {
                    *key = self.resolve_variables(key);
                    *value = self.resolve_variables(value);
                }
                
                // Resolve variables in body
                let body_text = config.body.text();
                let resolved_body = self.resolve_variables(&body_text);
                config.body = text_editor::Content::with_text(&resolved_body);
                
                // Resolve variables in authentication fields
                config.bearer_token = self.resolve_variables(&config.bearer_token);
                config.basic_username = self.resolve_variables(&config.basic_username);
                config.basic_password = self.resolve_variables(&config.basic_password);
                config.api_key = self.resolve_variables(&config.api_key);
                config.api_key_header = self.resolve_variables(&config.api_key_header);
                
                Task::perform(send_request(config), Message::RequestCompleted)
            }
            Message::CancelRequest => {
                self.is_loading = false;
                Task::none()
            }
            Message::RequestCompleted(result) => {
                self.is_loading = false;
                self.request_start_time = None;
                match result {
                    Ok(response) => {
                        self.response_body_content = Self::create_response_content(&response.body);
                        self.response = Some(response);
                    }
                    Err(error) => {
                        self.response_body_content = Self::create_response_content(&error);
                        self.response = Some(ResponseData {
                            status: 0,
                            status_text: "Error".to_string(),
                            headers: vec![],
                            body: error,
                            content_type: "text/plain".to_string(),
                            is_binary: false,
                            size: 0,
                            time: 0,
                        });
                    }
                }
                Task::none()
            }
            Message::TimerTick => {
                if let Some(start_time) = self.request_start_time {
                    self.current_elapsed_time = start_time.elapsed().as_millis() as u64;
                }
                Task::none()
            }
            Message::CollectionToggled(index) => {
                if let Some(collection) = self.collections.get_mut(index) {
                    collection.expanded = !collection.expanded;
                }
                Task::none()
            }
            Message::RequestSelected(collection_index, request_index) => {
                if let Some(collection) = self.collections.get(collection_index) {
                    if let Some(request) = collection.requests.get(request_index) {
                        self.current_request.method = request.method.clone();
                        self.current_request.url = request.url.clone();
                    }
                }
                Task::none()
            }
            Message::TabSelected(tab) => {
                self.current_request.selected_tab = tab;
                Task::none()
            }
            Message::ResponseTabSelected(tab) => {
                self.selected_response_tab = tab;
                Task::none()
            }
            Message::HeaderKeyChanged(index, key) => {
                if let Some(header) = self.current_request.headers.get_mut(index) {
                    header.0 = key;
                }
                Task::none()
            }
            Message::HeaderValueChanged(index, value) => {
                if let Some(header) = self.current_request.headers.get_mut(index) {
                    header.1 = value;
                }
                Task::none()
            }
            Message::AddHeader => {
                self.current_request.headers.push((String::new(), String::new()));
                Task::none()
            }
            Message::RemoveHeader(index) => {
                if index < self.current_request.headers.len() {
                    self.current_request.headers.remove(index);
                }
                Task::none()
            }
            Message::ParamKeyChanged(index, key) => {
                if let Some(param) = self.current_request.params.get_mut(index) {
                    param.0 = key;
                }
                Task::none()
            }
            Message::ParamValueChanged(index, value) => {
                if let Some(param) = self.current_request.params.get_mut(index) {
                    param.1 = value;
                }
                Task::none()
            }
            Message::AddParam => {
                self.current_request.params.push((String::new(), String::new()));
                Task::none()
            }
            Message::RemoveParam(index) => {
                if index < self.current_request.params.len() {
                    self.current_request.params.remove(index);
                }
                Task::none()
            }
            Message::BodyChanged(action) => {
                self.current_request.body.perform(action);
                Task::none()
            }
            Message::ResponseBodyAction(action) => {
                self.response_body_content.perform(action);
                Task::none()
            }
            Message::AuthTypeChanged(auth_type) => {
                self.current_request.auth_type = auth_type;
                Task::none()
            }
            Message::BearerTokenChanged(token) => {
                self.current_request.bearer_token = token;
                Task::none()
            }
            Message::BasicUsernameChanged(username) => {
                self.current_request.basic_username = username;
                Task::none()
            }
            Message::BasicPasswordChanged(password) => {
                self.current_request.basic_password = password;
                Task::none()
            }
            Message::ApiKeyChanged(api_key) => {
                self.current_request.api_key = api_key;
                Task::none()
            }
            Message::ApiKeyHeaderChanged(header) => {
                self.current_request.api_key_header = header;
                Task::none()
            }

            // Environment message handlers
            Message::OpenEnvironmentPopup => {
                self.show_environment_popup = true;
                Task::none()
            }
            Message::CloseEnvironmentPopup => {
                self.show_environment_popup = false;
                Task::none()
            }
            Message::DoNothing => {
                Task::none()
            }
            Message::EnvironmentSelected(index) => {
                if index < self.environments.len() {
                    self.active_environment = Some(index);
                }
                Task::none()
            }
            Message::AddEnvironment => {
                let new_env = Environment::new(format!("Environment {}", self.environments.len() + 1));
                self.environments.push(new_env);
                // Set the newly created environment as active
                self.active_environment = Some(self.environments.len() - 1);
                Task::none()
            }
            Message::DeleteEnvironment(index) => {
                if index < self.environments.len() && self.environments.len() > 1 {
                    self.environments.remove(index);
                    // Adjust active environment if necessary
                    if let Some(active) = self.active_environment {
                        if active == index {
                            self.active_environment = Some(0);
                        } else if active > index {
                            self.active_environment = Some(active - 1);
                        }
                    }
                }
                Task::none()
            }
            Message::EnvironmentNameChanged(env_index, name) => {
                if let Some(env) = self.environments.get_mut(env_index) {
                    env.name = name;
                }
                Task::none()
            }
            Message::EnvironmentDescriptionChanged(env_index, description) => {
                if let Some(env) = self.environments.get_mut(env_index) {
                    env.description = if description.is_empty() { None } else { Some(description) };
                }
                Task::none()
            }
            Message::VariableKeyChanged(env_index, var_index, key) => {
                if let Some(env) = self.environments.get_mut(env_index) {
                    let variables: Vec<(String, String)> = env.variables.clone().into_iter().collect();
                    if let Some((old_key, value)) = variables.get(var_index) {
                        env.variables.remove(old_key);
                        env.variables.insert(key, value.clone());
                    }
                }
                Task::none()
            }
            Message::VariableValueChanged(env_index, var_index, value) => {
                if let Some(env) = self.environments.get_mut(env_index) {
                    let variables: Vec<(String, String)> = env.variables.clone().into_iter().collect();
                    if let Some((key, _)) = variables.get(var_index) {
                        env.variables.insert(key.clone(), value);
                    }
                }
                Task::none()
            }
            Message::AddVariable(env_index) => {
                if let Some(env) = self.environments.get_mut(env_index) {
                    let var_count = env.variables.len();
                    env.add_variable(format!("variable_{}", var_count + 1), String::new());
                }
                Task::none()
            }
            Message::RemoveVariable(env_index, var_index) => {
                if let Some(env) = self.environments.get_mut(env_index) {
                    let variables: Vec<(String, String)> = env.variables.clone().into_iter().collect();
                    if let Some((key, _)) = variables.get(var_index) {
                        env.variables.remove(key);
                    }
                }
                Task::none()
            }

            Message::AddHttpRequest(collection_index) => {
                if let Some(collection) = self.collections.get_mut(collection_index) {
                    collection.requests.push(SavedRequest {
                        name: format!("New Request {}", collection.requests.len() + 1),
                        method: HttpMethod::GET,
                        url: String::new(),
                    });
                }

                Task::none()
            }
            Message::DeleteFolder(collection_index) => {
                if collection_index < self.collections.len() {
                    self.collections.remove(collection_index);
                }

                Task::none()
            }
            Message::AddFolder(_collection_index) => {
                self.collections.push(RequestCollection {
                    name: format!("New Collection {}", self.collections.len() + 1),
                    requests: vec![],
                    expanded: true,
                });

                Task::none()
            }
            Message::RenameFolder(collection_index) => {
                // For now, just add a number to the name as a placeholder
                // In a real app, this would open a dialog or text input
                if let Some(collection) = self.collections.get_mut(collection_index) {
                    collection.name = format!("{} (Renamed)", collection.name);
                }

                Task::none()
            }
            Message::SendRequestFromMenu(collection_index, request_index) => {
                if let Some(collection) = self.collections.get(collection_index) {
                    if let Some(request) = collection.requests.get(request_index) {
                        self.current_request.method = request.method.clone();
                        self.current_request.url = request.url.clone();
                        self.is_loading = true;
                        self.request_start_time = Some(std::time::Instant::now());
                        self.current_elapsed_time = 0;
                        let config = self.current_request.clone();

                        return Task::perform(send_request(config), Message::RequestCompleted);
                    }
                }

                Task::none()
            }
            Message::CopyRequestAsCurl(collection_index, request_index) => {
                if let Some(collection) = self.collections.get(collection_index) {
                    if let Some(request) = collection.requests.get(request_index) {
                        let mut temp_config = self.current_request.clone();
                        temp_config.method = request.method.clone();
                        temp_config.url = request.url.clone();
                        let curl_command = generate_curl_command(&temp_config);
                        // In a real app, you'd copy to clipboard here
                        println!("Curl command: {}", curl_command);
                    }
                }
                Task::none()
            }
            Message::RenameRequest(_collection_index, _request_index) => {
                // TODO: Implement rename functionality
                Task::none()
            }
            Message::DuplicateRequest(collection_index, request_index) => {
                if let Some(collection) = self.collections.get_mut(collection_index) {
                    if let Some(request) = collection.requests.get(request_index).cloned() {
                        let mut new_request = request;
                        new_request.name = format!("{} (Copy)", new_request.name);
                        collection.requests.push(new_request);
                    }
                }
                Task::none()
            }
            Message::DeleteRequest(collection_index, request_index) => {
                if let Some(collection) = self.collections.get_mut(collection_index) {
                    if request_index < collection.requests.len() {
                        collection.requests.remove(request_index);
                    }
                }
                Task::none()
            }
            Message::ToggleMethodMenu => {
                self.method_menu_open = !self.method_menu_open;
                Task::none()
            }
            Message::CloseMethodMenu => {
                self.method_menu_open = false;
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let pane_grid = PaneGrid::new(&self.panes, |_id, pane, _is_maximized| {
            let content = match pane {
                PaneContent::Collections => self.collections_view(),
                PaneContent::RequestConfig => self.request_config_view(),
                PaneContent::Response => self.response_view(),
            };

            container(content)
                .width(Fill)
                .height(Fill)
                .padding(0)
                .into()
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

        // Wrap the main content in a custom overlay if the environment popup is shown
        if self.show_environment_popup {
            // Create a custom overlay using stack
            stack![
                pane_grid,
                // Semi-transparent backdrop with centered popup
                container(
                    container(self.environment_popup_view())
                        .width(800)
                        .height(650)
                )
                .center_x(Fill)
                .center_y(Fill)
                .width(Fill)
                .height(Fill)
                .style(|theme| container::Style {
                    background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.5).into()),
                    ..Default::default()
                })
            ].into()
        } else {
            pane_grid.into()
        }
    }

    fn collections_view(&self) -> Element<'_, Message> {
        collections_panel(&self.collections)
    }

    fn request_config_view(&self) -> Element<'_, Message> {
        request_panel(&self.current_request, self.is_loading, &self.environments, self.active_environment, self.method_menu_open)
    }

    fn response_view(&self) -> Element<'_, Message> {
        response_panel(
            &self.response,
            &self.response_body_content,
            self.selected_response_tab.clone(),
            self.is_loading,
            self.current_elapsed_time,
        )
    }

    fn environment_popup_view(&self) -> Element<'_, Message> {
        // Fixed header with title and close button
        let header = row![
            text("Environment Manager").size(20),
            Space::with_width(Fill),
            button(text("×"))
                .on_press(Message::CloseEnvironmentPopup)
                .style(|theme: &Theme, status| {
                    let base = button::Style::default();
                    match status {
                        button::Status::Hovered => button::Style {
                            background: Some(iced::Background::Color(Color::from_rgb(0.9, 0.2, 0.2))),
                            text_color: Color::WHITE,
                            ..base
                        },
                        _ => button::Style {
                            background: Some(iced::Background::Color(Color::from_rgb(0.8, 0.0, 0.0))),
                            text_color: Color::WHITE,
                            ..base
                        },
                    }
                })
        ]
        .align_y(iced::Alignment::Center);

        // Scrollable content area
        let mut content = column![];

        // Environment selector
        if !self.environments.is_empty() {
            let env_names: Vec<String> = self.environments.iter().map(|env| env.name.clone()).collect();
            let selected_env = self.active_environment.and_then(|idx| env_names.get(idx).cloned());

            let env_selector = column![
                text("Active Environment"),
                row![
                    pick_list(
                        env_names,
                        selected_env,
                        |selected| {
                            if let Some(index) = self.environments.iter().position(|env| env.name == selected) {
                                Message::EnvironmentSelected(index)
                            } else {
                                Message::EnvironmentSelected(0)
                            }
                        }
                    )
                    .width(Length::FillPortion(2)),
                    Space::with_width(10),
                    button(text("Add Environment"))
                        .on_press(Message::AddEnvironment)
                        .style(|theme, status| {
                            let base = button::Style::default();
                            match status {
                                button::Status::Hovered => button::Style {
                                    background: Some(iced::Background::Color(Color::from_rgb(0.9, 0.9, 0.9))),
                                    ..base
                                },
                                _ => base,
                            }
                        })
                        .width(Length::FillPortion(1)),
                ]
                .align_y(iced::Alignment::Center),
            ]
            .spacing(5);

            content = content.push(env_selector);
            content = content.push(Space::with_height(15));

            // Show variables for the active environment
            if let Some(active_idx) = self.active_environment {
                if let Some(active_env) = self.environments.get(active_idx) {
                    content = content.push(text("Environment Variables").size(16));
                    content = content.push(Space::with_height(10));

                    // Variables header
                    let variables_header = row![
                        text("Variable Name").width(Length::FillPortion(1)),
                        text("Value").width(Length::FillPortion(1)),
                        text("").width(50) // For delete button
                    ]
                    .spacing(10);
                    content = content.push(variables_header);

                    // Variables list
                    let variables: Vec<(String, String)> = active_env.variables.clone().into_iter().collect();
                    for (var_index, (key, value)) in variables.iter().enumerate() {
                        let variable_row = row![
                            text_input("Variable name", key)
                                .on_input(move |input| Message::VariableKeyChanged(active_idx, var_index, input))
                                .width(Length::FillPortion(1)),
                            text_input("Variable value", value)
                                .on_input(move |input| Message::VariableValueChanged(active_idx, var_index, input))
                                .width(Length::FillPortion(1)),
                            button(text("×"))
                                .on_press(Message::RemoveVariable(active_idx, var_index))
                                .width(50)
                        ]
                        .spacing(10)
                        .align_y(iced::Alignment::Center);

                        content = content.push(variable_row);
                    }

                    // Add variable button
                    content = content.push(
                        button(text("+ Add Variable").size(14))
                            .on_press(Message::AddVariable(active_idx))
                            .padding([8, 16])
                            .style(|theme, status| {
                                match status {
                                    button::Status::Hovered => button::Style {
                                        background: Some(iced::Background::Color(Color::from_rgb(0.2, 0.5, 0.9))),
                                        text_color: Color::WHITE,
                                        border: iced::Border {
                                            color: Color::from_rgb(0.1, 0.4, 0.8),
                                            width: 1.0,
                                            radius: 4.0.into(),
                                        },
                                        ..button::Style::default()
                                    },
                                    _ => button::Style {
                                        background: Some(iced::Background::Color(Color::from_rgb(0.3, 0.6, 1.0))),
                                        text_color: Color::WHITE,
                                        border: iced::Border {
                                            color: Color::from_rgb(0.2, 0.5, 0.9),
                                            width: 1.0,
                                            radius: 4.0.into(),
                                        },
                                        ..button::Style::default()
                                    },
                                }
                            })
                    );

                    // Environment name editing
                    content = content.push(Space::with_height(15));
                    content = content.push(text("Environment Name"));
                    content = content.push(
                        text_input("Environment name", &active_env.name)
                            .on_input(move |input| Message::EnvironmentNameChanged(active_idx, input))
                            .width(Fill)
                    );

                    // Environment description
                    content = content.push(Space::with_height(10));
                    content = content.push(text("Description"));
                    content = content.push(
                        text_input("Environment description", active_env.description.as_deref().unwrap_or(""))
                            .on_input(move |input| Message::EnvironmentDescriptionChanged(active_idx, input))
                            .width(Fill)
                    );

                    // Delete environment button (only if more than one environment exists)
                    if self.environments.len() > 1 {
                        content = content.push(Space::with_height(15));
                        content = content.push(
                            button(text("Delete Environment"))
                                .on_press(Message::DeleteEnvironment(active_idx))
                                .style(|theme, status| {
                                    let base = button::Style::default();
                                    match status {
                                        button::Status::Hovered => button::Style {
                                            background: Some(iced::Background::Color(Color::from_rgb(0.9, 0.2, 0.2))),
                                            text_color: Color::WHITE,
                                            ..base
                                        },
                                        _ => button::Style {
                                            background: Some(iced::Background::Color(Color::from_rgb(0.8, 0.0, 0.0))),
                                            text_color: Color::WHITE,
                                            ..base
                                        },
                                    }
                                })
                        );
                    }
                }
            }
        } else {
            content = content.push(text("No environments available"));
            content = content.push(
                button(text("Add Environment"))
                    .on_press(Message::AddEnvironment)
            );
        }

        container(
            column![
                header,
                Space::with_height(20),
                scrollable(content.spacing(10))
                    .height(Length::Fixed(580.0)) // Reduced height to account for header
            ]
            .spacing(0)
        )
        .width(Length::Fixed(800.0))
        .padding(20)
        .style(|theme: &Theme| container::Style {
            background: Some(iced::Background::Color(Color::WHITE)),
            border: iced::Border {
                color: Color::from_rgb(0.7, 0.7, 0.7),
                width: 1.0,
                radius: 8.0.into(),
            },
            shadow: iced::Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.3),
                offset: Vector::new(0.0, 4.0),
                blur_radius: 10.0,
            },
            ..Default::default()
        })
        .into()
    }

    fn subscription(&self) -> iced::Subscription<Message> {
        if self.is_loading {
            iced::time::every(std::time::Duration::from_millis(100))
                .map(|_| Message::TimerTick)
        } else {
            iced::Subscription::none()
        }
    }
}
