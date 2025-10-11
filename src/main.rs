mod types;
mod http;
mod ui;
mod storage;

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
use std::collections::HashMap;
use serde_json;

pub fn main() -> iced::Result {
    iced::application(
            |_state: &BeamApp| String::from("Beam"),
            BeamApp::update,
            BeamApp::view,
        )
        .subscription(BeamApp::subscription)
        .window_size(Size::new(1200.0, 800.0))
        .run_with(|| (BeamApp::default(), Task::perform(async { Message::InitializeStorage }, |msg| msg)))
}

impl Default for BeamApp {
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
        panes.resize(second_split, 0.466); // 40/(40+35) = 0.533

        let collections = vec![];

        Self {
            panes,
            collections,
            current_request: RequestConfig {
                method: HttpMethod::GET,
                url: String::new(),
                headers: vec![
                    ("Content-Type".to_string(), "application/json".to_string()),
                    ("User-Agent".to_string(), "BeamApp/1.0".to_string()),
                ],
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

            // Initialize with empty environments
            environments: vec![],
            active_environment: None,
            show_environment_popup: false,
            method_menu_open: false,

            // Last opened request tracking
            last_opened_request: None,

            // Auto-save debounce management
            debounce_timers: HashMap::new(),
            debounce_delay_ms: 500, // 500ms default delay

            // Rename modal state
            show_rename_modal: false,
            rename_input: String::new(),
            rename_target: None,

            // Double-click detection state
            last_click_time: None,
            last_click_target: None,

            // Tooltip state for environment variables
            show_url_tooltip: false,
            tooltip_variable_name: None,
            tooltip_variable_value: None,
            tooltip_position: None,

            // Storage will be initialized asynchronously
            storage_manager: None,

            // Initialize spinner
            spinner: Spinner::new(),
        }
    }
}

impl BeamApp {
    fn create_response_content(body: &str) -> text_editor::Content {
        // Try to format JSON if the content is not too large (limit to 100KB)
        const MAX_JSON_FORMAT_SIZE: usize = 100 * 1024; // 100KB

        if body.len() <= MAX_JSON_FORMAT_SIZE {
            // Try to parse and format as JSON
            if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(body) {
                if let Ok(formatted_json) = serde_json::to_string_pretty(&json_value) {
                    return text_editor::Content::with_text(&formatted_json);
                }
            }
        }

        // If not JSON or too large, return original content
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

                // Emit auto-save message if we have a current request
                if let Some((collection_index, request_index)) = self.last_opened_request {
                    Task::perform(
                        async move { Message::RequestFieldChanged { collection_index, request_index, field: RequestField::Url } },
                        |msg| msg,
                    )
                } else {
                    Task::none()
                }
            }
            Message::MethodChanged(method) => {
                self.current_request.method = method;
                self.method_menu_open = false; // Close menu after selection

                // Emit auto-save message if we have a current request
                if let Some((collection_index, request_index)) = self.last_opened_request {
                    Task::perform(
                        async move { Message::RequestFieldChanged { collection_index, request_index, field: RequestField::Method } },
                        |msg| msg,
                    )
                } else {
                    Task::none()
                }
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
                // Update spinner animation
                if self.is_loading {
                    self.spinner.update();
                }
                Task::none()
            }
            Message::CollectionToggled(index) => {
                if let Some(collection) = self.collections.get_mut(index) {
                    collection.expanded = !collection.expanded;

                    // Save the collection to persist the expansion state
                    let collection = collection.clone();
                    Task::perform(
                        async move {
                            match storage::StorageManager::with_default_config().await {
                                Ok(storage_manager) => {
                                    match storage_manager.storage().save_collection(&collection).await {
                                        Ok(_) => Ok(()),
                                        Err(e) => Err(e.to_string()),
                                    }
                                }
                                Err(e) => Err(e.to_string()),
                            }
                        },
                        Message::CollectionsSaved,
                    )
                } else {
                    Task::none()
                }
            }
            Message::RequestSelected(collection_index, request_index) => {
                if let Some(collection) = self.collections.get(collection_index) {
                    if let Some(request) = collection.requests.get(request_index) {
                        let now = std::time::Instant::now();
                        let current_target = (collection_index, request_index);

                        // Check for double-click (within 500ms and same target)
                        let is_double_click = if let (Some(last_time), Some(last_target)) =
                            (self.last_click_time, self.last_click_target) {
                            last_target == current_target && now.duration_since(last_time).as_millis() < 500
                        } else {
                            false
                        };

                        // Update click tracking
                        self.last_click_time = Some(now);
                        self.last_click_target = Some(current_target);

                        if is_double_click {
                            // Double-click detected: show rename modal
                            self.show_rename_modal = true;
                            self.rename_target = Some((collection_index, request_index));
                            self.rename_input = request.name.clone();
                            Task::none()
                        } else {
                            // Single click: select the request
                            self.current_request.method = request.method.clone();
                            self.current_request.url = request.url.clone();

                            // Update the last opened request
                            self.last_opened_request = Some((collection_index, request_index));

                            // Save the last opened request to storage
                            Task::perform(
                                async move { Message::SaveLastOpenedRequest(collection_index, request_index) },
                                |msg| msg,
                            )
                        }
                    } else {
                        Task::none()
                    }
                } else {
                    Task::none()
                }
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

                // Emit auto-save message if we have a current request
                if let Some((collection_index, request_index)) = self.last_opened_request {
                    Task::perform(
                        async move { Message::RequestFieldChanged { collection_index, request_index, field: RequestField::Headers } },
                        |msg| msg,
                    )
                } else {
                    Task::none()
                }
            }
            Message::HeaderValueChanged(index, value) => {
                if let Some(header) = self.current_request.headers.get_mut(index) {
                    header.1 = value;
                }

                // Emit auto-save message if we have a current request
                if let Some((collection_index, request_index)) = self.last_opened_request {
                    Task::perform(
                        async move { Message::RequestFieldChanged { collection_index, request_index, field: RequestField::Headers } },
                        |msg| msg,
                    )
                } else {
                    Task::none()
                }
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

                // Emit auto-save message if we have a current request
                if let Some((collection_index, request_index)) = self.last_opened_request {
                    Task::perform(
                        async move { Message::RequestFieldChanged { collection_index, request_index, field: RequestField::Params } },
                        |msg| msg,
                    )
                } else {
                    Task::none()
                }
            }
            Message::ParamValueChanged(index, value) => {
                if let Some(param) = self.current_request.params.get_mut(index) {
                    param.1 = value;
                }

                // Emit auto-save message if we have a current request
                if let Some((collection_index, request_index)) = self.last_opened_request {
                    Task::perform(
                        async move { Message::RequestFieldChanged { collection_index, request_index, field: RequestField::Params } },
                        |msg| msg,
                    )
                } else {
                    Task::none()
                }
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

                // Emit auto-save message if we have a current request
                if let Some((collection_index, request_index)) = self.last_opened_request {
                    Task::perform(
                        async move { Message::RequestFieldChanged { collection_index, request_index, field: RequestField::Body } },
                        |msg| msg,
                    )
                } else {
                    Task::none()
                }
            }
            Message::ResponseBodyAction(action) => {
                self.response_body_content.perform(action);
                Task::none()
            }
            Message::AuthTypeChanged(auth_type) => {
                self.current_request.auth_type = auth_type;

                // Emit auto-save message if we have a current request
                if let Some((collection_index, request_index)) = self.last_opened_request {
                    Task::perform(
                        async move { Message::RequestFieldChanged { collection_index, request_index, field: RequestField::Auth } },
                        |msg| msg,
                    )
                } else {
                    Task::none()
                }
            }
            Message::BearerTokenChanged(token) => {
                self.current_request.bearer_token = token;

                // Emit auto-save message if we have a current request
                if let Some((collection_index, request_index)) = self.last_opened_request {
                    Task::perform(
                        async move { Message::RequestFieldChanged { collection_index, request_index, field: RequestField::Auth } },
                        |msg| msg,
                    )
                } else {
                    Task::none()
                }
            }
            Message::BasicUsernameChanged(username) => {
                self.current_request.basic_username = username;

                // Emit auto-save message if we have a current request
                if let Some((collection_index, request_index)) = self.last_opened_request {
                    Task::perform(
                        async move { Message::RequestFieldChanged { collection_index, request_index, field: RequestField::Auth } },
                        |msg| msg,
                    )
                } else {
                    Task::none()
                }
            }
            Message::BasicPasswordChanged(password) => {
                self.current_request.basic_password = password;

                // Emit auto-save message if we have a current request
                if let Some((collection_index, request_index)) = self.last_opened_request {
                    Task::perform(
                        async move { Message::RequestFieldChanged { collection_index, request_index, field: RequestField::Auth } },
                        |msg| msg,
                    )
                } else {
                    Task::none()
                }
            }
            Message::ApiKeyChanged(api_key) => {
                self.current_request.api_key = api_key;

                // Emit auto-save message if we have a current request
                if let Some((collection_index, request_index)) = self.last_opened_request {
                    Task::perform(
                        async move { Message::RequestFieldChanged { collection_index, request_index, field: RequestField::Auth } },
                        |msg| msg,
                    )
                } else {
                    Task::none()
                }
            }
            Message::ApiKeyHeaderChanged(header) => {
                self.current_request.api_key_header = header;

                // Emit auto-save message if we have a current request
                if let Some((collection_index, request_index)) = self.last_opened_request {
                    Task::perform(
                        async move { Message::RequestFieldChanged { collection_index, request_index, field: RequestField::Auth } },
                        |msg| msg,
                    )
                } else {
                    Task::none()
                }
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
            Message::KeyPressed(key) => {
                match key {
                    iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) => {
                        if self.show_environment_popup {
                            self.show_environment_popup = false;
                        } else if self.show_rename_modal {
                            self.show_rename_modal = false;
                            self.rename_input.clear();
                            self.rename_target = None;
                        }
                        Task::none()
                    }
                    _ => Task::none()
                }
            }
            Message::DoNothing => {
                Task::none()
            }
            Message::EnvironmentSelected(index) => {
                if index < self.environments.len() {
                    self.active_environment = Some(index);

                    // Save the active environment to storage
                    let environments = self.environments.clone();
                    let active_env_name = self.environments[index].name.clone();
                    Task::perform(
                        async move {
                            match storage::StorageManager::with_default_config().await {
                                Ok(storage_manager) => {
                                    if let Err(e) = storage_manager.storage().save_environments_with_active(&environments, Some(&active_env_name)).await {
                                        eprintln!("Failed to save active environment: {}", e);
                                    }
                                }
                                Err(e) => eprintln!("Failed to create storage manager: {}", e),
                            }
                            Message::DoNothing
                        },
                        |msg| msg,
                    )
                } else {
                    Task::none()
                }
            }
            Message::AddEnvironment => {
                let new_env = Environment::new(format!("Environment {}", self.environments.len() + 1));
                self.environments.push(new_env);
                // Set the newly created environment as active
                self.active_environment = Some(self.environments.len() - 1);

                // Save environments after adding a new one
                let environments = self.environments.clone();
                Task::perform(
                    async move {
                        match storage::StorageManager::with_default_config().await {
                            Ok(storage_manager) => {
                                match storage_manager.storage().save_environments(&environments).await {
                                    Ok(_) => Ok(()),
                                    Err(e) => Err(e.to_string()),
                                }
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    },
                    Message::EnvironmentsSaved,
                )
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

                    // Save environments after deleting one
                    let environments = self.environments.clone();
                    Task::perform(
                        async move {
                            match storage::StorageManager::with_default_config().await {
                                Ok(storage_manager) => {
                                    match storage_manager.storage().save_environments(&environments).await {
                                        Ok(_) => Ok(()),
                                        Err(e) => Err(e.to_string()),
                                    }
                                }
                                Err(e) => Err(e.to_string()),
                            }
                        },
                        Message::EnvironmentsSaved,
                    )
                } else {
                    Task::none()
                }
            }
            Message::EnvironmentNameChanged(env_index, name) => {
                if let Some(env) = self.environments.get_mut(env_index) {
                    env.name = name;

                    // Save environments after name change
                    let environments = self.environments.clone();
                    Task::perform(
                        async move {
                            match storage::StorageManager::with_default_config().await {
                                Ok(storage_manager) => {
                                    match storage_manager.storage().save_environments(&environments).await {
                                        Ok(_) => Ok(()),
                                        Err(e) => Err(e.to_string()),
                                    }
                                }
                                Err(e) => Err(e.to_string()),
                            }
                        },
                        Message::EnvironmentsSaved,
                    )
                } else {
                    Task::none()
                }
            }
            Message::EnvironmentDescriptionChanged(env_index, description) => {
                if let Some(env) = self.environments.get_mut(env_index) {
                    env.description = if description.is_empty() { None } else { Some(description) };

                    // Save environments after description change
                    let environments = self.environments.clone();
                    Task::perform(
                        async move {
                            match storage::StorageManager::with_default_config().await {
                                Ok(storage_manager) => {
                                    match storage_manager.storage().save_environments(&environments).await {
                                        Ok(_) => Ok(()),
                                        Err(e) => Err(e.to_string()),
                                    }
                                }
                                Err(e) => Err(e.to_string()),
                            }
                        },
                        Message::EnvironmentsSaved,
                    )
                } else {
                    Task::none()
                }
            }
            Message::VariableKeyChanged(env_index, var_index, key) => {
                if let Some(env) = self.environments.get_mut(env_index) {
                    let variables: Vec<(String, String)> = env.variables.clone().into_iter().collect();
                    if let Some((old_key, value)) = variables.get(var_index) {
                        env.variables.remove(old_key);
                        env.variables.insert(key, value.clone());
                    }

                    // Save environments after variable key change
                    let environments = self.environments.clone();
                    Task::perform(
                        async move {
                            match storage::StorageManager::with_default_config().await {
                                Ok(storage_manager) => {
                                    match storage_manager.storage().save_environments(&environments).await {
                                        Ok(_) => Ok(()),
                                        Err(e) => Err(e.to_string()),
                                    }
                                }
                                Err(e) => Err(e.to_string()),
                            }
                        },
                        Message::EnvironmentsSaved,
                    )
                } else {
                    Task::none()
                }
            }
            Message::VariableValueChanged(env_index, var_index, value) => {
                if let Some(env) = self.environments.get_mut(env_index) {
                    let variables: Vec<(String, String)> = env.variables.clone().into_iter().collect();
                    if let Some((key, _)) = variables.get(var_index) {
                        env.variables.insert(key.clone(), value);
                    }

                    // Save environments after variable value change
                    let environments = self.environments.clone();
                    Task::perform(
                        async move {
                            match storage::StorageManager::with_default_config().await {
                                Ok(storage_manager) => {
                                    match storage_manager.storage().save_environments(&environments).await {
                                        Ok(_) => Ok(()),
                                        Err(e) => Err(e.to_string()),
                                    }
                                }
                                Err(e) => Err(e.to_string()),
                            }
                        },
                        Message::EnvironmentsSaved,
                    )
                } else {
                    Task::none()
                }
            }
            Message::AddVariable(env_index) => {
                if let Some(env) = self.environments.get_mut(env_index) {
                    let var_count = env.variables.len();
                    env.add_variable(format!("variable_{}", var_count + 1), String::new());

                    // Save environments after adding variable
                    let environments = self.environments.clone();
                    Task::perform(
                        async move {
                            match storage::StorageManager::with_default_config().await {
                                Ok(storage_manager) => {
                                    match storage_manager.storage().save_environments(&environments).await {
                                        Ok(_) => Ok(()),
                                        Err(e) => Err(e.to_string()),
                                    }
                                }
                                Err(e) => Err(e.to_string()),
                            }
                        },
                        Message::EnvironmentsSaved,
                    )
                } else {
                    Task::none()
                }
            }
            Message::RemoveVariable(env_index, var_index) => {
                if let Some(env) = self.environments.get_mut(env_index) {
                    let variables: Vec<(String, String)> = env.variables.clone().into_iter().collect();
                    if let Some((key, _)) = variables.get(var_index) {
                        env.variables.remove(key);
                    }

                    // Save environments after removing variable
                    let environments = self.environments.clone();
                    Task::perform(
                        async move {
                            match storage::StorageManager::with_default_config().await {
                                Ok(storage_manager) => {
                                    match storage_manager.storage().save_environments(&environments).await {
                                        Ok(_) => Ok(()),
                                        Err(e) => Err(e.to_string()),
                                    }
                                }
                                Err(e) => Err(e.to_string()),
                            }
                        },
                        Message::EnvironmentsSaved,
                    )
                } else {
                    Task::none()
                }
            }

            Message::AddHttpRequest(collection_index) => {
                if let Some(collection) = self.collections.get_mut(collection_index) {
                    let new_request = SavedRequest {
                        name: format!("New Request {}", collection.requests.len() + 1),
                        method: HttpMethod::GET,
                        url: String::new(),
                    };
                    collection.requests.push(new_request.clone());

                    // Save only the new request, not the entire collection
                    let collection_name = collection.name.clone();
                    Task::perform(
                        async move {
                            use storage::conversions::ToPersistent;
                            let persistent_request = storage::PersistentRequest {
                                name: new_request.name,
                                method: new_request.method.to_string(),
                                url: new_request.url,
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
                                metadata: storage::persistent_types::RequestMetadata::default(),
                            };

                            match storage::StorageManager::with_default_config().await {
                                Ok(storage_manager) => {
                                    match storage_manager.storage().save_request(&collection_name, &persistent_request).await {
                                        Ok(_) => Ok(()),
                                        Err(e) => Err(e.to_string()),
                                    }
                                }
                                Err(e) => Err(e.to_string()),
                            }
                        },
                        Message::CollectionsSaved,
                    )
                } else {
                    Task::none()
                }
            }
            Message::DeleteFolder(collection_index) => {
                if collection_index < self.collections.len() {
                    self.collections.remove(collection_index);
                }

                // After deleting a folder, we don't need to save anything since the collection is removed
                Task::none()
            }
            Message::AddFolder(_collection_index) => {
                let new_collection = RequestCollection {
                    name: format!("New Collection {}", self.collections.len() + 1),
                    requests: vec![],
                    expanded: true,
                };
                self.collections.push(new_collection.clone());

                // Save the newly created collection
                Task::perform(
                    async move {
                        match storage::StorageManager::with_default_config().await {
                            Ok(storage_manager) => {
                                match storage_manager.storage().save_collection(&new_collection).await {
                                    Ok(_) => Ok(()),
                                    Err(e) => Err(e.to_string()),
                                }
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    },
                    Message::CollectionsSaved,
                )
            }
            Message::RenameFolder(collection_index) => {
                // For now, just add a number to the name as a placeholder
                // In a real app, this would open a dialog or text input
                if let Some(collection) = self.collections.get_mut(collection_index) {
                    collection.name = format!("{} (Renamed)", collection.name);

                    // Save the renamed collection
                    let collection = collection.clone();
                    Task::perform(
                        async move {
                            match storage::StorageManager::with_default_config().await {
                                Ok(storage_manager) => {
                                    match storage_manager.storage().save_collection(&collection).await {
                                        Ok(_) => Ok(()),
                                        Err(e) => Err(e.to_string()),
                                    }
                                }
                                Err(e) => Err(e.to_string()),
                            }
                        },
                        Message::CollectionsSaved,
                    )
                } else {
                    Task::none()
                }
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
            Message::RenameRequest(collection_index, request_index) => {
                // Show the rename modal with the current request name
                if let Some(collection) = self.collections.get(collection_index) {
                    if let Some(request) = collection.requests.get(request_index) {
                        self.show_rename_modal = true;
                        self.rename_input = request.name.clone();
                        self.rename_target = Some((collection_index, request_index));
                    }
                }
                Task::none()
            }
            Message::ShowRenameModal(collection_index, request_index) => {
                if let Some(collection) = self.collections.get(collection_index) {
                    if let Some(request) = collection.requests.get(request_index) {
                        self.show_rename_modal = true;
                        self.rename_input = request.name.clone();
                        self.rename_target = Some((collection_index, request_index));
                    }
                }
                Task::none()
            }
            Message::HideRenameModal => {
                self.show_rename_modal = false;
                self.rename_input.clear();
                self.rename_target = None;
                Task::none()
            }
            Message::RenameInputChanged(new_name) => {
                self.rename_input = new_name;
                Task::none()
            }
            Message::ConfirmRename => {
                if let Some((collection_index, request_index)) = self.rename_target {
                    let new_name = self.rename_input.trim().to_string();

                    // Validate the new name
                    if new_name.is_empty() {
                        // TODO: Show error message
                        return Task::none();
                    }

                    // Check for duplicate names in the same collection
                    if let Some(collection) = self.collections.get(collection_index) {
                        if collection.requests.iter().enumerate().any(|(i, req)| i != request_index && req.name == new_name) {
                            // TODO: Show error message for duplicate name
                            return Task::none();
                        }
                    }

                    // Update the request name
                    if let Some(collection) = self.collections.get_mut(collection_index) {
                        if let Some(request) = collection.requests.get_mut(request_index) {
                            let old_name = request.name.clone();
                            request.name = new_name.clone();

                            // Hide the modal
                            self.show_rename_modal = false;
                            self.rename_input.clear();
                            self.rename_target = None;

                            // Save the collection and rename the file
                            let collection_name = collection.name.clone();
                            let collection = collection.clone();

                            return Task::perform(
                                async move {
                                    match storage::StorageManager::with_default_config().await {
                                        Ok(storage_manager) => {
                                            let storage = storage_manager.storage();

                                            // Rename the request file (this already updates the file content)
                                            match storage.rename_request(&collection_name, &old_name, &new_name).await {
                                                Ok(_) => Ok(()),
                                                Err(e) => {
                                                    eprintln!("Failed to rename request file: {}", e);
                                                    Err(e.to_string())
                                                }
                                            }
                                        }
                                        Err(e) => Err(e.to_string()),
                                    }
                                },
                                Message::CollectionsSaved,
                            );
                        }
                    }
                }
                Task::none()
            }
            Message::DuplicateRequest(collection_index, request_index) => {
                if let Some(collection) = self.collections.get_mut(collection_index) {
                    if let Some(request) = collection.requests.get(request_index).cloned() {
                        let mut new_request = request;
                        new_request.name = format!("{} (Copy)", new_request.name);
                        collection.requests.push(new_request.clone());

                        // Save only the new duplicated request, not the entire collection
                        let collection_name = collection.name.clone();
                        Task::perform(
                            async move {
                                let persistent_request = storage::PersistentRequest {
                                    name: new_request.name,
                                    method: new_request.method.to_string(),
                                    url: new_request.url,
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
                                    metadata: storage::persistent_types::RequestMetadata::default(),
                                };

                                match storage::StorageManager::with_default_config().await {
                                    Ok(storage_manager) => {
                                        match storage_manager.storage().save_request(&collection_name, &persistent_request).await {
                                            Ok(_) => Ok(()),
                                            Err(e) => Err(e.to_string()),
                                        }
                                    }
                                    Err(e) => Err(e.to_string()),
                                }
                            },
                            Message::CollectionsSaved,
                        )
                    } else {
                        Task::none()
                    }
                } else {
                    Task::none()
                }
            }
            Message::DeleteRequest(collection_index, request_index) => {
                if let Some(collection) = self.collections.get_mut(collection_index) {
                    if request_index < collection.requests.len() {
                        // Get the request name before removing it
                        let request_name = collection.requests[request_index].name.clone();
                        let collection_name = collection.name.clone();

                        // Remove from in-memory collection
                        collection.requests.remove(request_index);

                        // Delete request file and save collection
                        let collection = collection.clone();
                        Task::perform(
                            async move {
                                match storage::StorageManager::with_default_config().await {
                                    Ok(storage_manager) => {
                                        let storage = storage_manager.storage();

                                        // First delete the request file from disk
                                        if let Err(e) = storage.delete_request(&collection_name, &request_name).await {
                                            eprintln!("Failed to delete request file '{}': {}", request_name, e);
                                        }

                                        // Then save the updated collection
                                        match storage.save_collection(&collection).await {
                                            Ok(_) => Ok(()),
                                            Err(e) => Err(e.to_string()),
                                        }
                                    }
                                    Err(e) => Err(e.to_string()),
                                }
                            },
                            Message::CollectionsSaved,
                        )
                    } else {
                        Task::none()
                    }
                } else {
                    Task::none()
                }
            }
            Message::ToggleMethodMenu => {
                self.method_menu_open = !self.method_menu_open;
                Task::none()
            }
            Message::CloseMethodMenu => {
                self.method_menu_open = false;
                Task::none()
            }

            // Storage operations
            Message::InitializeStorage => {
                Task::perform(
                    async {
                        match storage::StorageManager::with_default_config().await {
                            Ok(_) => Ok(()),
                            Err(e) => Err(e.to_string()),
                        }
                    },
                    Message::StorageInitialized,
                )
            }
            Message::StorageInitialized(result) => {
                match result {
                    Ok(_) => {
                        // Storage initialized successfully, now load collections and environments first
                        // Don't automatically save initial data - only create files when user explicitly saves
                        Task::batch([
                            Task::perform(async { Message::LoadCollections }, |msg| msg),
                            Task::perform(async { Message::LoadEnvironments }, |msg| msg),
                        ])
                    }
                    Err(e) => {
                        eprintln!("Failed to initialize storage: {}", e);
                        Task::none()
                    }
                }
            }
            Message::SetStorageManager => {
                // This message is no longer needed with the simplified approach
                Task::none()
            }
            Message::LoadCollections => {
                Task::perform(
                    async {
                        match storage::StorageManager::with_default_config().await {
                            Ok(storage_manager) => {
                                match storage_manager.storage().load_collections().await {
                                    Ok(collections) => Ok(collections),
                                    Err(e) => Err(e.to_string()),
                                }
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    },
                    Message::CollectionsLoaded,
                )
            }
            Message::CollectionsLoaded(result) => {
                match result {
                    Ok(collections) => {
                        if !collections.is_empty() {
                            self.collections = collections;
                        }
                        // After collections are loaded, now load the last opened request
                        Task::perform(async { Message::LoadLastOpenedRequest }, |msg| msg)
                    }
                    Err(e) => {
                        eprintln!("Failed to load collections: {}", e);
                        Task::none()
                    }
                }
            }
            Message::SaveCollection(collection_index) => {
                if let Some(collection) = self.collections.get(collection_index) {
                    let collection = collection.clone();
                    Task::perform(
                        async move {
                            match storage::StorageManager::with_default_config().await {
                                Ok(storage_manager) => {
                                    match storage_manager.storage().save_collection(&collection).await {
                                        Ok(_) => Ok(()),
                                        Err(e) => Err(e.to_string()),
                                    }
                                }
                                Err(e) => Err(e.to_string()),
                            }
                        },
                        Message::CollectionsSaved,
                    )
                } else {
                    Task::none()
                }
            }
            Message::CollectionsSaved(result) => {
                match result {
                    Ok(_) => {
                        println!("Collection saved successfully");
                    }
                    Err(e) => {
                        eprintln!("Failed to save collection: {}", e);
                    }
                }
                Task::none()
            }
            Message::LoadEnvironments => {
                Task::perform(
                    async {
                        match storage::StorageManager::with_default_config().await {
                            Ok(storage_manager) => {
                                match storage_manager.storage().load_environments().await {
                                    Ok(environments) => Ok(environments),
                                    Err(e) => Err(e.to_string()),
                                }
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    },
                    Message::EnvironmentsLoaded,
                )
            }
            Message::EnvironmentsLoaded(result) => {
                match result {
                    Ok(environments) => {
                        if !environments.is_empty() {
                            self.environments = environments;
                            // After loading environments, load the active environment
                            Task::perform(async { Message::LoadActiveEnvironment }, |msg| msg)
                        } else {
                            Task::none()
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to load environments: {}", e);
                        Task::none()
                    }
                }
            }
            Message::LoadActiveEnvironment => {
                Task::perform(
                    async {
                        match storage::StorageManager::with_default_config().await {
                            Ok(storage_manager) => {
                                match storage_manager.storage().load_active_environment().await {
                                    Ok(active_env) => Ok(active_env),
                                    Err(e) => Err(e.to_string()),
                                }
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    },
                    Message::ActiveEnvironmentLoaded,
                )
            }
            Message::ActiveEnvironmentLoaded(result) => {
                match result {
                    Ok(Some(active_env_name)) => {
                        // Find the environment by name and set it as active
                        if let Some(index) = self.environments.iter().position(|env| env.name == active_env_name) {
                            self.active_environment = Some(index);
                        }
                    }
                    Ok(None) => {
                        // No active environment saved
                        self.active_environment = None;
                    }
                    Err(e) => {
                        eprintln!("Failed to load active environment: {}", e);
                    }
                }
                Task::none()
            }
            Message::SaveInitialData => {
                // Save the default collections and environments to disk only if they don't exist
                // This ensures that the initial data is persisted on first run
                let collections = self.collections.clone();
                let environments = self.environments.clone();

                Task::perform(
                    async move {
                        match storage::StorageManager::with_default_config().await {
                            Ok(storage_manager) => {
                                let storage = storage_manager.storage();

                                // Check if environments file exists
                                let env_path = storage_manager.config().base_path.join("environments.toml");
                                if !env_path.exists() {
                                    if let Err(e) = storage.save_environments(&environments).await {
                                        eprintln!("Failed to save initial environments: {}", e);
                                    } else {
                                        println!("Initial environments saved successfully");
                                    }
                                }

                                // Check if collections exist and save them if they don't
                                for collection in &collections {
                                    let collection_path = storage_manager.config().base_path
                                        .join("collections")
                                        .join(&collection.name);
                                    if !collection_path.exists() {
                                        if let Err(e) = storage.save_collection_with_requests(collection).await {
                                            eprintln!("Failed to save initial collection '{}': {}", collection.name, e);
                                        } else {
                                            println!("Initial collection '{}' saved successfully", collection.name);
                                        }
                                    }
                                }

                                Ok(())
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    },
                    Message::CollectionsSaved,
                )
            }
            Message::SaveEnvironments => {
                let environments = self.environments.clone();
                Task::perform(
                    async move {
                        match storage::StorageManager::with_default_config().await {
                            Ok(storage_manager) => {
                                match storage_manager.storage().save_environments(&environments).await {
                                    Ok(_) => Ok(()),
                                    Err(e) => Err(e.to_string()),
                                }
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    },
                    Message::EnvironmentsSaved,
                )
            }
            Message::EnvironmentsSaved(result) => {
                match result {
                    Ok(_) => {
                        println!("Environments saved successfully");
                    }
                    Err(e) => {
                        eprintln!("Failed to save environments: {}", e);
                    }
                }
                Task::none()
            }

            // Last opened request handlers
            Message::SaveLastOpenedRequest(collection_index, request_index) => {
                Task::perform(
                    async move {
                        match storage::StorageManager::with_default_config().await {
                            Ok(storage_manager) => {
                                match storage_manager.storage().save_last_opened_request(collection_index, request_index).await {
                                    Ok(_) => Ok(()),
                                    Err(e) => Err(e.to_string()),
                                }
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    },
                    Message::LastOpenedRequestSaved,
                )
            }
            Message::LoadLastOpenedRequest => {
                Task::perform(
                    async {
                        match storage::StorageManager::with_default_config().await {
                            Ok(storage_manager) => {
                                match storage_manager.storage().load_last_opened_request().await {
                                    Ok(last_opened) => Ok(last_opened),
                                    Err(e) => Err(e.to_string()),
                                }
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    },
                    Message::LastOpenedRequestLoaded,
                )
            }
            Message::LastOpenedRequestSaved(result) => {
                match result {
                    Ok(_) => {
                        // Successfully saved last opened request
                    }
                    Err(e) => {
                        eprintln!("Failed to save last opened request: {}", e);
                    }
                }
                Task::none()
            }
            Message::LastOpenedRequestLoaded(result) => {
                match result {
                    Ok(Some((collection_index, request_index))) => {
                        println!("DEBUG: LastOpenedRequestLoaded - collection_index: {}, request_index: {}", collection_index, request_index);
                        // Restore the last opened request
                        self.last_opened_request = Some((collection_index, request_index));

                        // Automatically expand the collection containing the last opened request
                        if let Some(collection) = self.collections.get_mut(collection_index) {
                            collection.expanded = true;
                            println!("DEBUG: Automatically expanded collection '{}' containing last opened request", collection.name);
                        }

                        // Load the complete request configuration
                        let collections = self.collections.clone();
                        Task::perform(
                            async move {
                                println!("DEBUG: Loading request by indices - collection_index: {}, request_index: {}", collection_index, request_index);
                                match storage::StorageManager::with_default_config().await {
                                    Ok(storage_manager) => {
                                        match storage_manager.storage().load_request_by_indices(&collections, collection_index, request_index).await {
                                            Ok(Some(persistent_request)) => {
                                                // Convert PersistentRequest to RequestConfig
                                                use storage::conversions::FromPersistent;
                                                let request_config = RequestConfig::from_persistent(persistent_request);
                                                Ok(Some(request_config))
                                            }
                                            Ok(None) => Ok(None),
                                            Err(e) => Err(e.to_string()),
                                        }
                                    }
                                    Err(e) => Err(e.to_string()),
                                }
                            },
                            Message::RequestConfigLoaded,
                        )
                    }
                    Ok(None) => {
                        // No last opened request found
                        self.last_opened_request = None;
                        Task::none()
                    }
                    Err(e) => {
                        eprintln!("Failed to load last opened request: {}", e);
                        self.last_opened_request = None;
                        Task::none()
                    }
                }
            }
            Message::RequestConfigLoaded(result) => {
                match result {
                    Ok(Some(request_config)) => {
                        println!("DEBUG: RequestConfigLoaded - method: {:?}, url: {}", request_config.method, request_config.url);
                        // Update the current request with the loaded configuration
                        self.current_request = request_config;
                    }
                    Ok(None) => {
                        eprintln!("No request configuration found for the last opened request");
                    }
                    Err(e) => {
                        eprintln!("Failed to load request configuration: {}", e);
                    }
                }
                Task::none()
            }

            // Auto-save message handlers
            Message::RequestFieldChanged { collection_index, request_index, field: _ } => {
                // Update the debounce timer for this request
                let key = (collection_index, request_index);
                self.debounce_timers.insert(key, std::time::Instant::now());

                // Schedule a debounced save
                let delay = std::time::Duration::from_millis(self.debounce_delay_ms);
                Task::perform(
                    async move {
                        tokio::time::sleep(delay).await;
                        Message::SaveRequestDebounced { collection_index, request_index }
                    },
                    |msg| msg,
                )
            }
            Message::SaveRequestDebounced { collection_index, request_index } => {
                let key = (collection_index, request_index);

                // Check if this save is still valid (no newer changes)
                if let Some(last_change_time) = self.debounce_timers.get(&key) {
                    let elapsed = last_change_time.elapsed();
                    if elapsed >= std::time::Duration::from_millis(self.debounce_delay_ms) {
                        // Remove the timer entry and save the request
                        self.debounce_timers.remove(&key);

                        // Get collection name and request name for saving
                        if let Some(collection) = self.collections.get(collection_index) {
                            if let Some(saved_request) = collection.requests.get(request_index) {
                                let collection_name = collection.name.clone();
                                let request_name = saved_request.name.clone();
                                let current_request = self.current_request.clone();

                                Task::perform(
                                    async move {
                                        use storage::conversions::ToPersistent;
                                        let persistent_request = current_request.to_persistent_with_name(request_name);

                                        match storage::StorageManager::with_default_config().await {
                                            Ok(storage_manager) => {
                                                match storage_manager.storage().save_request(&collection_name, &persistent_request).await {
                                                    Ok(_) => Ok(()),
                                                    Err(e) => Err(e.to_string()),
                                                }
                                            }
                                            Err(e) => Err(e.to_string()),
                                        }
                                    },
                                    Message::RequestSaved,
                                )
                            } else {
                                Task::none()
                            }
                        } else {
                            Task::none()
                        }
                    } else {
                        // Too early, ignore this save request
                        Task::none()
                    }
                } else {
                    // No timer entry, ignore
                    Task::none()
                }
            }
            Message::RequestSaved(result) => {
                match result {
                    Ok(_) => {
                        println!("Request auto-saved successfully");
                    }
                    Err(e) => {
                        eprintln!("Failed to auto-save request: {}", e);
                    }
                }
                Task::none()
            }
            Message::ShowUrlTooltip(variable_name, variable_value, x, y) => {
                self.show_url_tooltip = true;
                self.tooltip_variable_name = Some(variable_name);
                self.tooltip_variable_value = Some(variable_value);
                self.tooltip_position = Some((x, y));
                Task::none()
            }
            Message::HideUrlTooltip => {
                self.show_url_tooltip = false;
                self.tooltip_variable_name = None;
                self.tooltip_variable_value = None;
                self.tooltip_position = None;
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
                color: iced::Color::from_rgb(0.9, 0.9, 0.9), // Gray
                width: 0.0,
            },
            picked_split: pane_grid::Line {
                color: iced::Color::from_rgb(0.9, 0.9, 0.9), // Gray
                width: 0.0,
            },
        });

        // Wrap the main content in a custom overlay if any popup is shown
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
        } else if self.show_rename_modal {
            // Create a custom overlay for the rename modal
            stack![
                pane_grid,
                // Semi-transparent backdrop with centered modal
                container(
                    container(self.rename_modal_view())
                        .width(400)
                        .height(200)
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
        collections_panel(&self.collections, self.last_opened_request)
    }

    fn request_config_view(&self) -> Element<'_, Message> {
        request_panel(
            &self.current_request,
            self.is_loading,
            &self.environments,
            self.active_environment,
            self.method_menu_open,
            self.show_url_tooltip,
            self.tooltip_variable_name.as_deref().unwrap_or(""),
            self.tooltip_variable_value.as_deref().unwrap_or(""),
            self.tooltip_position.unwrap_or((0.0, 0.0))
        )
    }

    fn response_view(&self) -> Element<'_, Message> {
        response_panel(
            &self.response,
            &self.response_body_content,
            self.selected_response_tab.clone(),
            self.is_loading,
            self.current_elapsed_time,
            &self.spinner,
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

    fn rename_modal_view(&self) -> Element<'_, Message> {
        let header = row![
            text("Rename Request").size(18),
            Space::with_width(Fill),
            button(text("×"))
                .on_press(Message::HideRenameModal)
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

        let input_field = text_input("Enter new name...", &self.rename_input)
            .on_input(Message::RenameInputChanged)
            .on_submit(Message::ConfirmRename)
            .padding(10)
            .size(16);

        let buttons = row![
            button(text("Cancel").size(16))
                .on_press(Message::HideRenameModal)
                .padding(10)
                .style(|theme: &Theme, status| {
                    let base = button::Style::default();
                    match status {
                        button::Status::Hovered => button::Style {
                            background: Some(iced::Background::Color(Color::from_rgb(0.9, 0.9, 0.9))),
                            text_color: Color::from_rgb(1.0, 0.0, 0.0), // Red text
                            ..base
                        },
                        _ => button::Style {
                            background: Some(iced::Background::Color(Color::from_rgb(0.8, 0.8, 0.8))),
                            text_color: Color::from_rgb(1.0, 0.0, 0.0), // Red text
                            ..base
                        },
                    }
                }),
            Space::with_width(10),
            button(text("Rename"))
                .on_press(Message::ConfirmRename)
                .padding(10)
                .style(|theme: &Theme, status| {
                    let base = button::Style::default();
                    match status {
                        button::Status::Hovered => button::Style {
                            background: Some(iced::Background::Color(Color::from_rgb(0.0, 0.6, 0.0))),
                            text_color: Color::from_rgb(1.0, 0.0, 0.0), // Red text
                            ..base
                        },
                        _ => button::Style {
                            background: Some(iced::Background::Color(Color::from_rgb(0.0, 0.5, 0.0))),
                            text_color: Color::from_rgb(1.0, 0.0, 0.0), // Red text
                            ..base
                        },
                    }
                })
        ]
        .align_y(iced::Alignment::Center);

        container(
            column![
                header,
                Space::with_height(20),
                text("Enter a new name for the request:").size(14),
                Space::with_height(10),
                input_field,
                Space::with_height(20),
                buttons
            ]
            .spacing(0)
        )
        .width(Length::Fixed(400.0))
        .height(Length::Fixed(200.0))
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
        let timer_subscription = if self.is_loading {
            iced::time::every(std::time::Duration::from_millis(100))
                .map(|_| Message::TimerTick)
        } else {
            iced::Subscription::none()
        };

        let keyboard_subscription = iced::keyboard::on_key_press(|key, _modifiers| {
            Some(Message::KeyPressed(key))
        });

        iced::Subscription::batch([timer_subscription, keyboard_subscription])
    }
}
