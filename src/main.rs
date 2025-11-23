mod script;
use std::path::PathBuf;

use beam::types::AuthType;
use beam::types::BodyFormat;
use beam::types::Environment;
use beam::types::HttpMethod;
use beam::types::RenameTarget;
use beam::types::RequestCollection;
use beam::types::RequestConfig;
use beam::types::ResponseData;

use beam::http::*;
use beam::storage;
use beam::storage::StorageManager;
use beam::ui::CollectionPanel;
use beam::ui::RequestPanel;
use beam::ui::ResponsePanel;
use std::sync::Arc;

use beam::ui::collections;
use beam::ui::request;
use beam::ui::response;
use beam::ui::{IconName, icon};

use iced::color;
use iced::widget::pane_grid::{self, Axis, PaneGrid};
use iced::widget::{
    button, column, container, mouse_area, row, scrollable, space, stack, text,
    text_editor, text_input,
};
use iced::{Color, Element, Fill, Length, Padding, Size, Task, Theme, Vector};
use log::{error, info};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum PaneContent {
    Collections,
    RequestConfig,
    Response,
}

#[derive(Debug, Clone)]
pub enum Message {
    RequestPanel(request::Message),
    ResponsePanel(response::Message),
    CollectionPanel(collections::Message),

    PaneResized(pane_grid::ResizeEvent),
    TimerTick,

    AddEnvironment,
    DeleteEnvironment(usize),
    EnvironmentNameChanged(usize, String),
    EnvironmentDescriptionChanged(usize, String),
    VariableKeyChanged(usize, String, String), // (env_index, old_key, new_key)
    VariableValueChanged(usize, String, String), // (env_index, key, new_value)
    AddVariable(usize),
    RemoveVariable(usize, String), // (env_index, key)

    CloseEnvironmentPopup,
    EnvironmentSelected(usize),
    KeyPressed(iced::keyboard::Key),
    RequestCompleted(Result<ResponseData, String>),
    PostScriptCompleted(crate::script::ScriptExecutionResult),

    HideRenameModal,
    RenameInputChanged(String),
    ConfirmRename,

    // Storage operations
    // #[allow(dead_code)]
    // SaveCollection(usize),
    LoadCollections,
    #[allow(dead_code)]
    SaveEnvironments,
    LoadEnvironments,
    LoadActiveEnvironment,
    ActiveEnvironmentLoaded(Result<Option<String>, String>),
    LoadConfigFiles,
    #[allow(dead_code)]
    CollectionsSaved(Result<(), String>),
    CollectionsLoaded(Result<Vec<RequestCollection>, String>),
    EnvironmentsSaved(Result<(), String>),
    EnvironmentsLoadedComplete(crate::storage::PersistentEnvironments),
    #[allow(dead_code)]
    SaveInitialData,
    UpdateLastOpenedRequest(usize, usize), // (collection_index, request_index) - deferred state update
    LoadLastOpenedRequest(Result<Option<(usize, usize)>, String>),
    SaveRequestDebounced {
        collection_index: usize,
        request_index: usize,
    },
    RequestSaved(Result<(), String>),

    DoNothing,
}

#[derive(Debug)]
pub struct BeamApp {
    pub panes: pane_grid::State<PaneContent>,
    pub collections: Vec<RequestCollection>,
    pub current_request: RequestConfig,
    pub is_loading: bool,
    pub current_elapsed_time: u64,
    pub request_body_content: text_editor::Content,
    pub post_script_content: text_editor::Content,
    pub response_body_content: text_editor::Content,
    pub collection_panel: CollectionPanel,
    pub response_panel: ResponsePanel,
    pub request_panel: RequestPanel,
    pub request_start_time: Option<Instant>,

    // Environment management
    pub environments: Vec<Environment>,
    pub active_environment: Option<usize>,
    pub show_environment_popup: bool,
    pub method_menu_open: bool,

    // Last opened request tracking
    pub last_opened_request: Option<(usize, usize)>, // (collection_index, request_index)

    // Debounce channel for request saving
    pub debounce_tx: Option<mpsc::Sender<RequestConfig>>,

    // Rename modal state
    pub show_rename_modal: bool,
    pub rename_input: String,
    pub rename_target: Option<RenameTarget>, // What is being renamed

    // Storage
    #[allow(dead_code)]
    pub storage_manager: Option<StorageManager>,

    // Hover states for buttons
    pub send_button_hovered: bool,
    pub cancel_button_hovered: bool,

    // Flag to track recent undo operations
    pub just_performed_undo: bool,

    // Flag to track when Cmd+Z is being processed to prevent visual flicker
    pub processing_cmd_z: bool,
}

pub fn main() -> iced::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    iced::application(
        || {
            (
                BeamApp::default(),
                Task::perform(async { Message::LoadConfigFiles }, |msg| msg),
            )
        },
        BeamApp::update,
        BeamApp::view,
    )
    .title(|_: &BeamApp| "Beam".to_string())
    // .theme(BeamApp::theme)
    .subscription(BeamApp::subscription)
    .window_size(Size::new(1200.0, 800.0))
    .run()
}

impl Default for BeamApp {
    fn default() -> Self {
        let (mut panes, collections_pane) = pane_grid::State::new(PaneContent::Collections);

        // Split vertically to create request config pane (middle panel)
        let (request_pane, first_split) = panes
            .split(Axis::Vertical, collections_pane, PaneContent::RequestConfig)
            .unwrap();

        // Split vertically again to create response pane (right panel)
        let (_, second_split) = panes
            .split(Axis::Vertical, request_pane, PaneContent::Response)
            .unwrap();

        // Set three-panel horizontal layout ratios
        // Collections: 25%, Request Config: 40%, Response: 35%
        panes.resize(first_split, 0.25);
        panes.resize(second_split, 0.466); // 40/(40+35) = 0.533

        let collections = vec![];

        Self {
            panes,
            collections,
            is_loading: false,
            current_elapsed_time: 0,
            current_request: RequestConfig {
                name: String::new(),
                path: std::path::PathBuf::new(),
                method: HttpMethod::GET,
                url: String::new(),
                headers: vec![
                    ("Content-Type".to_string(), "application/json".to_string()),
                    ("User-Agent".to_string(), "BeamApp/1.0".to_string()),
                ],
                params: vec![],
                body: String::new(),
                content_type: String::new(),
                auth_type: AuthType::None,
                body_format: BodyFormat::default(),
                bearer_token: String::new(),
                basic_username: String::new(),
                basic_password: String::new(),
                api_key: String::new(),
                api_key_header: "X-API-Key".to_string(),
                collection_index: 0,
                request_index: 0,
                metadata: None,
                post_request_script: None,
                last_response: None,
            },
            request_body_content: text_editor::Content::new(),
            response_body_content: text_editor::Content::new(),
            post_script_content: text_editor::Content::new(),
            response_panel: ResponsePanel::new(),
            request_panel: RequestPanel::default(),
            collection_panel: CollectionPanel::new(),
            request_start_time: None,

            // Initialize with empty environments
            environments: vec![],
            active_environment: None,
            show_environment_popup: false,
            method_menu_open: false,

            // Last opened request tracking
            last_opened_request: None,

            // Debounce channel will be initialized later
            debounce_tx: None,

            // Rename modal state
            show_rename_modal: false,
            rename_input: String::new(),
            rename_target: None,

            // Storage will be initialized asynchronously
            storage_manager: None,

            // Initialize hover states
            send_button_hovered: false,
            cancel_button_hovered: false,

            // Initialize undo tracking
            just_performed_undo: false,
            processing_cmd_z: false,
        }
    }
}

impl BeamApp {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::RequestPanel(view_message) => {
                match self.request_panel.update(
                    view_message,
                    &self.current_request,
                    &self.environments,
                ) {
                    request::Action::SendRequest(request_start_time) => {
                        let resolved_config =
                            self.resolve_request_config_variables(&self.current_request);

                        self.handle_send_request(resolved_config, request_start_time)
                    }
                    request::Action::CancelRequest() => {
                        // TODO: cancel request
                        self.is_loading = false;
                        Task::none()
                    }
                    request::Action::Run(task) => return task.map(Message::RequestPanel),
                    request::Action::UpdateCurrentRequest(request_config) => {
                        self.current_request = request_config.clone();

                        Self::update_editor_content(
                            &mut self.request_body_content,
                            self.current_request.body.to_string(),
                        );

                        if let Some(collection) = self
                            .collections
                            .get_mut(self.current_request.collection_index)
                        {
                            if let Some(request) = collection
                                .requests
                                .get_mut(self.current_request.request_index)
                            {
                                *request = self.current_request.clone();
                            }
                        }

                        // Send to debounce channel if available
                        if let Some(tx) = &self.debounce_tx {
                            if let Err(_) = tx.try_send(request_config) {
                                // Channel is full or closed, ignore for now
                                info!("Debounce channel is full or closed");
                            }
                        }

                        Task::none()
                    }
                    request::Action::UpdateActiveEnvironment(index) => {
                        self.active_environment = Some(index);
                        let environments = self.environments.clone();
                        let active_env_name = environments[index].name.clone();

                        tokio::spawn(async move {
                            match storage::StorageManager::with_default_config() {
                                Ok(storage_manager) => {
                                    if let Err(e) =
                                        storage_manager.storage().save_environments_with_active(
                                            &environments,
                                            Some(&active_env_name),
                                        )
                                    {
                                        error!("Failed to save active environment: {}", e);
                                    }
                                }
                                Err(e) => error!("Failed to create storage manager: {}", e),
                            }
                        });

                        Task::none()
                    }
                    request::Action::EditRequestBody(action) => {
                        self.request_body_content.perform(action);
                        self.current_request.body = self.request_body_content.text();

                        if let Some(collection) = self
                            .collections
                            .get_mut(self.current_request.collection_index)
                        {
                            if let Some(request) = collection
                                .requests
                                .get_mut(self.current_request.request_index)
                            {
                                *request = self.current_request.clone();
                            }
                        }

                        let request_to_persist = self.current_request.clone();

                        // TODO: only save if edit, not movement
                        if let Some(tx) = &self.debounce_tx {
                            if let Err(_) = tx.try_send(request_to_persist) {
                                info!("Debounce channel is full or closed");
                            }
                        }

                        Task::none()
                    }
                    request::Action::EditRequestPostRequestScript(action) => {
                        self.post_script_content.perform(action);
                        self.current_request.post_request_script =
                            Some(self.post_script_content.text());

                        if let Some(collection) = self
                            .collections
                            .get_mut(self.current_request.collection_index)
                        {
                            if let Some(request) = collection
                                .requests
                                .get_mut(self.current_request.request_index)
                            {
                                *request = self.current_request.clone();
                            }
                        }

                        let request_to_persist = self.current_request.clone();

                        // TODO: only save if edit, not movement
                        if let Some(tx) = &self.debounce_tx {
                            if let Err(_) = tx.try_send(request_to_persist) {
                                info!("Debounce channel is full or closed");
                            }
                        }

                        Task::none()
                    }
                    request::Action::FormatRequestBody(formatted_body) => {
                        let select_all_action = text_editor::Action::SelectAll;
                        self.request_body_content.perform(select_all_action);

                        let paste_action = text_editor::Action::Edit(text_editor::Edit::Paste(
                            std::sync::Arc::new(formatted_body),
                        ));
                        self.request_body_content.perform(paste_action);
                        self.current_request.body = self.request_body_content.text();

                        if let Some(collection) = self
                            .collections
                            .get_mut(self.current_request.collection_index)
                        {
                            if let Some(request) = collection
                                .requests
                                .get_mut(self.current_request.request_index)
                            {
                                *request = self.current_request.clone();
                            }
                        }

                        // Save the updated request
                        let request_to_persist = self.current_request.clone();
                        if let Some(tx) = &self.debounce_tx {
                            if let Err(_) = tx.try_send(request_to_persist) {
                                info!("Debounce channel is full or closed");
                            }
                        }

                        Task::none()
                    }
                    request::Action::OpenEnvironmentPopup => {
                        self.show_environment_popup = true;

                        Task::none()
                    }
                    request::Action::None => Task::none(),
                }
            }
            Message::ResponsePanel(view_message) => {
                match self.response_panel.update(view_message) {
                    response::Action::ResponseBodyAction(action) => {
                        self.response_body_content.perform(action);

                        Task::none()
                    }
                    response::Action::None => Task::none(),
                }
            }
            Message::CollectionPanel(view_message) => {
                match self
                    .collection_panel
                    .update(view_message, &self.collections)
                {
                    collections::Action::ToggleCollection(collection_index) => {
                        if let Some(collection) = self.collections.get_mut(collection_index) {
                            collection.expanded = !collection.expanded;

                            let col = collection.clone();

                            tokio::spawn(async move {
                                match storage::StorageManager::with_default_config() {
                                    Ok(storage_manager) => {
                                        match storage_manager.storage().save_collection(&col) {
                                            Ok(_) => Ok(()),
                                            Err(e) => Err(e.to_string()),
                                        }
                                    }
                                    Err(e) => Err(e.to_string()),
                                }
                            });
                        }

                        Task::none()
                    }
                    collections::Action::SelectRequestConfig(collection_index, request_index) => {
                        if self.last_opened_request == Some((collection_index, request_index)) {
                            return Task::none();
                        }

                        if let Some(collection) = self.collections.get(collection_index) {
                            if let Some(request_config) = collection.requests.get(request_index) {
                                self.current_request = request_config.clone();

                                Self::update_editor_content(
                                    &mut self.request_body_content,
                                    self.current_request.body.to_string(),
                                );

                                Self::update_editor_content(
                                    &mut self.post_script_content,
                                    self.current_request
                                        .post_request_script
                                        .as_deref()
                                        .unwrap_or("")
                                        .to_string(),
                                );

                                if let Some(resp) = &self.current_request.last_response {
                                    let formatted_resp = Self::format_response_content(
                                        resp.body.as_str(),
                                        self.current_request.body_format,
                                    );

                                    Self::update_editor_content(
                                        &mut self.response_body_content,
                                        formatted_resp,
                                    );
                                }

                                // Update the last opened request state and save to storage
                                self.last_opened_request = Some((collection_index, request_index));
                                // Save the last opened request asynchronously without blocking the UI
                                tokio::spawn(async move {
                                    if let Ok(storage_manager) =
                                        storage::StorageManager::with_default_config()
                                    {
                                        if let Err(e) =
                                            storage_manager.storage().save_last_opened_request(
                                                collection_index,
                                                request_index,
                                            )
                                        {
                                            error!("Failed to save last opened request: {}", e);
                                        }
                                    }
                                });
                            }
                        }

                        Task::none()
                    }
                    collections::Action::SaveRequestToCollection(request_config) => {
                        if let Some(collection) =
                            self.collections.get_mut(request_config.collection_index)
                        {
                            let mut new_req = request_config.clone();
                            // Get the new request path using the storage manager
                            if let Ok(storage_manager) =
                                storage::StorageManager::with_default_config()
                            {
                                let new_request_path = storage_manager
                                    .storage()
                                    .get_new_request_path_from_collection(collection);

                                new_req.path = PathBuf::from(new_request_path);
                            } else {
                                error!("failed to get storage manager");
                                return Task::none();
                            };

                            self.last_opened_request = Some((
                                request_config.collection_index,
                                request_config.request_index,
                            ));

                            collection.requests.push(new_req.clone());

                            self.current_request = new_req.clone();

                            tokio::spawn(async move {
                                Self::save_request(new_req);
                            });
                        }

                        Task::none()
                    }
                    collections::Action::SaveNewCollection(new_collection) => {
                        self.collections.push(new_collection.clone());

                        tokio::spawn(async move {
                            if let Ok(storage_manager) =
                                storage::StorageManager::with_default_config()
                            {
                                if let Err(e) =
                                    storage_manager.storage().save_collection(&new_collection)
                                {
                                    error!("Failed to save collection: {}", e);
                                }
                            }
                        });

                        Task::none()
                    }
                    collections::Action::SendRequest(
                        collection_index,
                        request_index,
                        request_start_time,
                    ) => {
                        if let Some(collection) = self.collections.get(collection_index) {
                            if let Some(request) = collection.requests.get(request_index) {
                                let resolved_config =
                                    self.resolve_request_config_variables(request);
                                return self
                                    .handle_send_request(resolved_config, request_start_time);
                            }
                        }

                        Task::none()
                    }
                    collections::Action::DuplicateRequest(collection_index, request_index) => {
                        if let Some(collection) = self.collections.get_mut(collection_index) {
                            if let Some(request) = collection.requests.get(request_index) {
                                let mut new_request = request.clone();

                                new_request.name = format!("{} (Copy)", new_request.name);
                                new_request.collection_index = collection_index;
                                new_request.request_index = request_index;

                                let mut path = PathBuf::new();
                                let curr_request_path = PathBuf::from(&request.path);
                                if let Some(parent) = curr_request_path.as_path().parent() {
                                    path.push(parent);
                                }

                                let mut max_number = 0;
                                if let Some(last_request) = collection.requests.last() {
                                    if let Some(filename_str) =
                                        last_request.path.file_stem().and_then(|s| s.to_str())
                                    {
                                        if let Ok(number) = filename_str.parse::<u32>() {
                                            max_number = number;
                                        }
                                    }
                                }
                                path.push(format!("{:04}.toml", max_number + 1));
                                new_request.path = path.clone();
                                let request_to_persist = new_request.clone();
                                collection.requests.push(new_request);

                                tokio::spawn(async move {
                                    Self::save_request(request_to_persist);
                                });
                            }
                        }

                        Task::none()
                    }
                    collections::Action::DeleteRequest(collection_index, request_index) => {
                        if let Some(collection) = self.collections.get_mut(collection_index) {
                            if request_index < collection.requests.len() {
                                if let Some(request) = collection.requests.get(request_index) {
                                    let request_path = request.path.clone();
                                    collection.requests.remove(request_index);

                                    // Use the storage method to delete the file
                                    tokio::spawn(async move {
                                        if let Ok(storage_manager) =
                                            StorageManager::with_default_config()
                                        {
                                            let storage = storage_manager.storage();
                                            if let Err(e) =
                                                storage.delete_request_by_path(&request_path)
                                            {
                                                error!("Failed to delete request file: {}", e);
                                            }
                                        }
                                    });

                                    Task::none()
                                } else {
                                    Task::none()
                                }
                            } else {
                                Task::none()
                            }
                        } else {
                            Task::none()
                        }
                    }
                    collections::Action::RenameRequest(collection_index, request_index) => {
                        // Show the rename modal with the current request name
                        if let Some(collection) = self.collections.get(collection_index) {
                            if let Some(request) = collection.requests.get(request_index) {
                                self.show_rename_modal = true;
                                self.rename_input = request.name.clone();
                                self.rename_target =
                                    Some(RenameTarget::Request(collection_index, request_index));
                            }
                        }

                        Task::none()
                    }
                    collections::Action::RenameCollection(collection_index) => {
                        // Show the rename modal for the folder
                        if let Some(collection) = self.collections.get(collection_index) {
                            self.show_rename_modal = true;
                            self.rename_input = collection.name.clone();
                            self.rename_target = Some(RenameTarget::Folder(collection_index));
                        }

                        Task::none()
                    }
                    collections::Action::DeleteCollection(collection_index) => {
                        if collection_index >= self.collections.len() {
                            return Task::none();
                        }

                        if let Some(collection) = self.collections.get(collection_index) {
                            if let Ok(storage_manager) =
                                storage::StorageManager::with_default_config()
                            {
                                storage_manager
                                    .storage()
                                    .delete_collection_by_folder_name(&collection.folder_name);
                            }
                        }

                        self.collections.remove(collection_index);

                        Task::none()
                    }
                    collections::Action::None => Task::none(),
                }
            }
            Message::PaneResized(event) => {
                self.panes.resize(event.split, event.ratio);
                Task::none()
            }
            Message::RequestCompleted(result) => {
                self.is_loading = false;
                self.request_start_time = None;
                match result {
                    Ok(response) => {
                        let formatted_body = Self::format_response_content(
                            &response.body,
                            self.current_request.body_format,
                        );
                        self.response_body_content
                            .perform(text_editor::Action::SelectAll);
                        self.response_body_content
                            .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                                Arc::new(formatted_body),
                            )));

                        // Update the request in the collections as well
                        if let Some(collection) = self
                            .collections
                            .get_mut(self.current_request.collection_index)
                        {
                            if let Some(request) = collection
                                .requests
                                .get_mut(self.current_request.request_index)
                            {
                                request.last_response = Some(response.clone());
                                self.current_request = request.clone();

                                let request_to_persist = request.clone();

                                tokio::spawn(async move {
                                    Self::save_request(request_to_persist);
                                });
                            }
                        }

                        // Execute post-request script if available
                        if let Some(script) = &self.current_request.post_request_script {
                            if !script.trim().is_empty() {
                                let script_clone = script.clone();
                                let request_config = self.current_request.clone();
                                let active_env = self
                                    .active_environment
                                    .and_then(|idx| self.environments.get(idx))
                                    .cloned()
                                    .unwrap_or_else(|| Environment::new("Default".to_string()));

                                return Task::perform(
                                    async move {
                                        crate::script::execute_post_request_script(
                                            &script_clone,
                                            request_config,
                                            response,
                                            &active_env,
                                        )
                                    },
                                    Message::PostScriptCompleted,
                                );
                            }
                        }
                    }
                    Err(error) => {
                        let error_response = ResponseData {
                            status: 0,
                            status_text: "Error".to_string(),
                            headers: vec![],
                            body: error.clone(),
                            content_type: "text/plain".to_string(),
                            is_binary: false,
                            size: 0,
                            time: 0,
                        };

                        // Store the error response in the current request
                        self.current_request.last_response = Some(error_response.clone());
                        Self::update_editor_content(
                            &mut self.response_body_content,
                            error.to_string(),
                        );

                        if let Some(collection) = self
                            .collections
                            .get_mut(self.current_request.collection_index)
                        {
                            if let Some(request) = collection
                                .requests
                                .get_mut(self.current_request.request_index)
                            {
                                request.last_response = Some(error_response);
                                let request_to_save = request.clone();

                                tokio::spawn(async move {
                                    Self::save_request(request_to_save);
                                });
                            }
                        }
                    }
                }

                Task::none()
            }
            Message::PostScriptCompleted(script_result) => {
                info!("Post-request script completed: {:?}", script_result);

                // Apply environment variable changes
                if let Some(active_env_idx) = self.active_environment {
                    if let Some(active_env) = self.environments.get_mut(active_env_idx) {
                        for (key, value) in script_result.environment_changes {
                            active_env.variables.insert(key, value);
                        }
                    }
                }

                // TODO: Display script execution results in UI
                // For now, just log the results
                for test_result in &script_result.test_results {
                    info!(
                        "Test '{}': {}",
                        test_result.name,
                        if test_result.passed {
                            "PASSED"
                        } else {
                            "FAILED"
                        }
                    );
                }

                for console_msg in &script_result.console_output {
                    info!("Console: {}", console_msg);
                }

                Task::none()
            }
            Message::EnvironmentSelected(index) => {
                self.active_environment = Some(index);

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
                    iced::keyboard::Key::Character(ref c) if c == "z" => {
                        // Check if this is Cmd+Z (undo) or Cmd+Shift+Z (redo)
                        // For now, we'll handle this in the subscription with modifiers
                        Task::none()
                    }
                    _ => Task::none(),
                }
            }
            Message::TimerTick => {
                if let Some(start_time) = self.request_start_time {
                    self.current_elapsed_time = start_time.elapsed().as_millis() as u64;
                }

                if self.is_loading {
                    self.response_panel.update_spinner();
                }

                Task::none()
            }
            Message::AddEnvironment => {
                let new_env =
                    Environment::new(format!("Environment {}", self.environments.len() + 1));
                self.environments.push(new_env);
                // Set the newly created environment as active
                self.active_environment = Some(self.environments.len() - 1);

                // Save environments after adding a new one
                let environments = self.environments.clone();
                Task::perform(
                    async move {
                        match storage::StorageManager::with_default_config() {
                            Ok(storage_manager) => {
                                match storage_manager.storage().save_environments(&environments) {
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
                            match storage::StorageManager::with_default_config() {
                                Ok(storage_manager) => {
                                    match storage_manager.storage().save_environments(&environments)
                                    {
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
                            match storage::StorageManager::with_default_config() {
                                Ok(storage_manager) => {
                                    match storage_manager.storage().save_environments(&environments)
                                    {
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
                    env.description = if description.is_empty() {
                        None
                    } else {
                        Some(description)
                    };

                    // Save environments after description change
                    let environments = self.environments.clone();
                    Task::perform(
                        async move {
                            match storage::StorageManager::with_default_config() {
                                Ok(storage_manager) => {
                                    match storage_manager.storage().save_environments(&environments)
                                    {
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
            Message::VariableKeyChanged(env_index, old_key, new_key) => {
                if let Some(env) = self.environments.get_mut(env_index) {
                    if let Some(value) = env.variables.remove(&old_key) {
                        env.variables.insert(new_key, value);
                    }

                    // If this is the active environment, URL updates will reflect on next render

                    // Save environments after variable key change
                    let environments = self.environments.clone();
                    Task::perform(
                        async move {
                            match storage::StorageManager::with_default_config() {
                                Ok(storage_manager) => {
                                    match storage_manager.storage().save_environments(&environments)
                                    {
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
            Message::VariableValueChanged(env_index, key, value) => {
                if let Some(env) = self.environments.get_mut(env_index) {
                    env.variables.insert(key, value);

                    // Environment variables will be applied during URL resolution
                    // No direct field updates needed as url_input component was removed

                    // Save environments after variable value change
                    let environments = self.environments.clone();
                    Task::perform(
                        async move {
                            match storage::StorageManager::with_default_config() {
                                Ok(storage_manager) => {
                                    match storage_manager.storage().save_environments(&environments)
                                    {
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

                    // Environment variables will be applied during URL resolution
                    // No direct field updates needed as url_input component was removed

                    // Save environments after adding variable
                    let environments = self.environments.clone();
                    Task::perform(
                        async move {
                            match storage::StorageManager::with_default_config() {
                                Ok(storage_manager) => {
                                    match storage_manager.storage().save_environments(&environments)
                                    {
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
            Message::RemoveVariable(env_index, key) => {
                if let Some(env) = self.environments.get_mut(env_index) {
                    env.variables.remove(&key);

                    // Environment variables will be applied during URL resolution
                    // No direct field updates needed as url_input component was removed

                    // Save environments after removing variable
                    let environments = self.environments.clone();
                    Task::perform(
                        async move {
                            match storage::StorageManager::with_default_config() {
                                Ok(storage_manager) => {
                                    match storage_manager.storage().save_environments(&environments)
                                    {
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
            // Storage operations
            Message::LoadConfigFiles => {
                // Initialize the debouncer for request update
                self.initialize_debouncer();

                Task::batch([
                    Task::perform(async { Message::LoadCollections }, |msg| msg),
                    Task::perform(async { Message::LoadEnvironments }, |msg| msg),
                ])
            }
            Message::LoadCollections => Task::perform(
                async {
                    match storage::StorageManager::with_default_config() {
                        Ok(storage_manager) => match storage_manager.storage().load_collections() {
                            Ok(collections) => Ok(collections),
                            Err(e) => Err(e.to_string()),
                        },
                        Err(e) => Err(e.to_string()),
                    }
                },
                Message::CollectionsLoaded,
            ),
            Message::CollectionsLoaded(result) => {
                match result {
                    Ok(collections) => {
                        if !collections.is_empty() {
                            self.collections = collections;

                            // Load lsast opened request after collections are loaded
                            return Task::perform(
                                async {
                                    match storage::StorageManager::with_default_config() {
                                        Ok(storage_manager) => {
                                            match storage_manager
                                                .storage()
                                                .load_last_opened_request()
                                            {
                                                Ok(last_opened) => Ok(last_opened),
                                                Err(e) => Err(e.to_string()),
                                            }
                                        }
                                        Err(e) => Err(e.to_string()),
                                    }
                                },
                                Message::LoadLastOpenedRequest,
                            );
                        }

                        Task::none()
                    }
                    Err(e) => {
                        error!("Failed to load collections: {}", e);
                        Task::none()
                    }
                }
            }
            Message::CollectionsSaved(result) => {
                match result {
                    Ok(_) => {
                        info!("Collection saved successfully");
                    }
                    Err(e) => {
                        error!("Failed to save collection: {}", e);
                    }
                }
                Task::none()
            }
            Message::LoadEnvironments => Task::perform(
                async {
                    match storage::StorageManager::with_default_config() {
                        Ok(storage_manager) => {
                            match storage_manager.storage().load_environments() {
                                Ok(persistent_envs) => persistent_envs,
                                Err(e) => {
                                    error!("Failed to load environments: {}", e);
                                    crate::storage::PersistentEnvironments {
                                        environments: Vec::new(),
                                        active_environment: None,
                                        metadata: crate::storage::EnvironmentsMetadata::default(),
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to create storage manager: {}", e);
                            crate::storage::PersistentEnvironments {
                                environments: Vec::new(),
                                active_environment: None,
                                metadata: crate::storage::EnvironmentsMetadata::default(),
                            }
                        }
                    }
                },
                Message::EnvironmentsLoadedComplete,
            ),
            Message::EnvironmentsLoadedComplete(persistent_envs) => {
                // Update environments
                self.environments = persistent_envs.environments;

                // Update active environment if specified
                if let Some(active_env_name) = persistent_envs.active_environment {
                    if let Some(index) = self
                        .environments
                        .iter()
                        .position(|env| env.name == active_env_name)
                    {
                        self.active_environment = Some(index);
                    }
                }

                Task::none()
            }
            Message::LoadActiveEnvironment => Task::perform(
                async {
                    match storage::StorageManager::with_default_config() {
                        Ok(storage_manager) => {
                            match storage_manager.storage().load_active_environment() {
                                Ok(active_env) => Ok(active_env),
                                Err(e) => Err(e.to_string()),
                            }
                        }
                        Err(e) => Err(e.to_string()),
                    }
                },
                Message::ActiveEnvironmentLoaded,
            ),
            Message::ActiveEnvironmentLoaded(result) => {
                match result {
                    Ok(Some(active_env_name)) => {
                        // Find the environment by name and set it as active
                        if let Some(index) = self
                            .environments
                            .iter()
                            .position(|env| env.name == active_env_name)
                        {
                            self.active_environment = Some(index);

                            // Environment variables will be applied during URL resolution
                            // No direct field updates needed as url_input component was removed
                        }
                    }
                    Ok(None) => {
                        // No active environment saved
                        self.active_environment = None;
                    }
                    Err(e) => {
                        error!("Failed to load active environment: {}", e);
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
                        match storage::StorageManager::with_default_config() {
                            Ok(storage_manager) => {
                                let storage = storage_manager.storage();

                                // Check if environments file exists
                                let env_path =
                                    storage_manager.config().base_path.join("environments.toml");
                                if !env_path.exists() {
                                    if let Err(e) = storage.save_environments(&environments) {
                                        error!("Failed to save initial environments: {}", e);
                                    } else {
                                        info!("Initial environments saved successfully");
                                    }
                                }

                                // Check if collections exist and save them if they don't
                                for collection in &collections {
                                    let collection_path = storage_manager
                                        .config()
                                        .base_path
                                        .join("collections")
                                        .join(&collection.name);
                                    if !collection_path.exists() {
                                        if let Err(e) =
                                            storage.save_collection_with_requests(collection)
                                        {
                                            error!(
                                                "Failed to save initial collection '{}': {}",
                                                collection.name, e
                                            );
                                        } else {
                                            info!(
                                                "Initial collection '{}' saved successfully",
                                                collection.name
                                            );
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
                        match storage::StorageManager::with_default_config() {
                            Ok(storage_manager) => {
                                match storage_manager.storage().save_environments(&environments) {
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
                        info!("Environments saved successfully");
                    }
                    Err(e) => {
                        error!("Failed to save environments: {}", e);
                    }
                }
                Task::none()
            }
            Message::UpdateLastOpenedRequest(collection_index, request_index) => {
                // Update the last opened request state and save to storage
                self.last_opened_request = Some((collection_index, request_index));
                // Save the last opened request asynchronously without blocking the UI
                tokio::spawn(async move {
                    if let Ok(storage_manager) = storage::StorageManager::with_default_config() {
                        if let Err(e) = storage_manager
                            .storage()
                            .save_last_opened_request(collection_index, request_index)
                        {
                            error!("Failed to save last opened request: {}", e);
                        }
                    }
                });

                Task::none()
            }
            Message::LoadLastOpenedRequest(result) => {
                match result {
                    Ok(Some((collection_index, request_index))) => {
                        if let Some(collection) = self.collections.get_mut(collection_index) {
                            collection.expanded = true;

                            if let Some(request_config) = collection.requests.get(request_index) {
                                self.last_opened_request = Some((collection_index, request_index));

                                self.current_request = request_config.clone();
                                Self::update_editor_content(
                                    &mut self.request_body_content,
                                    self.current_request.body.to_string(),
                                );

                                Self::update_editor_content(
                                    &mut self.post_script_content,
                                    self.current_request
                                        .post_request_script
                                        .as_deref()
                                        .unwrap_or("")
                                        .to_string(),
                                );

                                if let Some(resp) = &self.current_request.last_response {
                                    // TODO: move the response body content update in a new message
                                    // so it doesn't block the UI loading
                                    let formatted_resp = Self::format_response_content(
                                        resp.body.as_str(),
                                        self.current_request.body_format,
                                    );

                                    Self::update_editor_content(
                                        &mut self.response_body_content,
                                        formatted_resp.to_string(),
                                    );
                                }
                            }
                        } else {
                            error!("===no collections");
                        }

                        Task::none()
                    }
                    Ok(None) => {
                        // No last opened request found
                        self.last_opened_request = None;
                        Task::none()
                    }
                    Err(e) => {
                        error!("Failed to load last opened request: {}", e);
                        self.last_opened_request = None;
                        Task::none()
                    }
                }
            }
            // Message::RequestConfigLoaded(result) => {
            //     match result {
            //         Ok(Some(request_config)) => {
            //             info!(
            //                 "DEBUG: RequestConfigLoaded - method: {:?}, url: {}",
            //                 request_config.method, request_config.url
            //             );
            //             // Update the current request with the loaded configuration
            //             let current_request = request_config.clone();
            //             info!("====1: ");
            //             // Sync the request body content with the loaded body
            //             self.request_panel
            //                 .set_body_content(current_request.body.clone());
            //             info!("====2: ");
            //             // Update the URL string with the loaded URL
            //             self.url = current_request.url.clone();
            //             info!("====3: ");
            //             // Environment variables applied to URL input are handled during rendering
            //             self.current_request = current_request;
            //             info!("===request load successed");

            //             return Task::perform(
            //                 async move {
            //                     Message::UpdateLastOpenedRequest(
            //                         request_config.collection_index as usize,
            //                         request_config.request_index as usize,
            //                     )
            //                 },
            //                 |msg| msg,
            //             );
            //         }
            //         Ok(None) => {
            //             error!("No request configuration found for the last opened request");
            //         }
            //         Err(e) => {
            //             error!("Failed to load request configuration: {}", e);
            //         }
            //     }

            //     Task::none()
            // }
            // Auto-save message handlers
            Message::SaveRequestDebounced {
                collection_index,
                request_index,
            } => {
                info!(
                    "=== SaveRequestDebounced - collection_index: {}, request_index: {}",
                    collection_index, request_index
                );
                let key = (collection_index, request_index);

                // Check if this save is still valid (no newer changes)
                // if let Some(last_change_time) = self.debounce_timers.get(&key) {
                //     info!("====1");
                //     let elapsed = last_change_time.elapsed();
                //     if elapsed >= std::time::Duration::from_millis(self.debounce_delay_ms) {
                //         // Remove the timer entry and save the request
                //         self.debounce_timers.remove(&key);
                //         info!("====2");

                //         // Get collection name and request name for saving
                //         if let Some(collection) = self.collections.get(collection_index) {
                //             info!("====3");
                //             if let Some(saved_request) = collection.requests.get(request_index) {
                //                 info!("====4");
                //                 let collection_name = collection.name.clone();
                //                 info!("====5");
                //                 let request_name = saved_request.name.clone();
                //                 // TODO: persistent request config
                //                 // let serializable_request = self.current_request.to_serializable(request_name.clone());
                //                 // info!("===start perform auto save request");
                //                 // Task::perform(
                //                 //     async move {
                //                 //         match storage::StorageManager::with_default_config().await {
                //                 //             Ok(storage_manager) => {
                //                 //                 info!("===done perform auto save request");

                //                 //                 match storage_manager
                //                 //                     .storage()
                //                 //                     .save_serializable_request(
                //                 //                         &collection_name,
                //                 //                         &request_name,
                //                 //                         &serializable_request,
                //                 //                     )
                //                 //                     .await
                //                 //                 {
                //                 //                     Ok(_) => Ok(()),
                //                 //                     Err(e) => Err(e.to_string()),
                //                 //                 }
                //                 //             }
                //                 //             Err(e) => Err(e.to_string()),
                //                 //         }
                //                 //     },
                //                 //     Message::RequestSaved,
                //                 // )
                //                 Task::none()
                //             } else {
                //                 Task::none()
                //             }
                //         } else {
                //             Task::none()
                //         }
                //     } else {
                //         // Too early, ignore this save request
                //         Task::none()
                //     }
                // } else {
                // No timer entry, ignore
                //     Task::none()
                // }
                Task::none()
            }
            Message::RequestSaved(result) => {
                match result {
                    Ok(_) => {
                        info!("Request auto-saved successfully");
                    }
                    Err(e) => {
                        error!("Failed to auto-save request: {}", e);
                    }
                }
                Task::none()
            }
            Message::CloseEnvironmentPopup => {
                self.show_environment_popup = false;

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
                if let Some(rename_target) = &self.rename_target {
                    let new_name = self.rename_input.trim().to_string();

                    // Validate the new name
                    if new_name.is_empty() {
                        // TODO: Show error message
                        return Task::none();
                    }
                    match rename_target {
                        RenameTarget::Request(collection_index, request_index) => {
                            let collection_index = *collection_index;
                            let request_index = *request_index;

                            // Check for duplicate names in the same collection
                            if let Some(collection) = self.collections.get(collection_index) {
                                if collection
                                    .requests
                                    .iter()
                                    .enumerate()
                                    .any(|(i, req)| i != request_index && req.name == new_name)
                                {
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

                                    // Save the collection and rename the file (non-blocking)
                                    let collection_name = collection.name.clone();

                                    tokio::spawn(async move {
                                        if let Ok(storage_manager) =
                                            storage::StorageManager::with_default_config()
                                        {
                                            let storage = storage_manager.storage();
                                            if let Err(e) = storage.rename_request(
                                                &collection_name,
                                                &old_name,
                                                &new_name,
                                            ) {
                                                eprintln!("Failed to rename request file: {}", e);
                                            }
                                        }
                                    });

                                    return Task::none();
                                }
                            }
                        }
                        RenameTarget::Folder(collection_index) => {
                            let collection_index = *collection_index;

                            // Check for duplicate folder names
                            if self
                                .collections
                                .iter()
                                .enumerate()
                                .any(|(i, col)| i != collection_index && col.name == new_name)
                            {
                                // TODO: Show error message for duplicate name
                                return Task::none();
                            }

                            // Update the folder name
                            if let Some(collection) = self.collections.get_mut(collection_index) {
                                let old_name = collection.name.clone();
                                collection.name = new_name.clone();

                                // Hide the modal
                                self.show_rename_modal = false;
                                self.rename_input.clear();
                                self.rename_target = None;

                                // Rename the collection folder (non-blocking)
                                tokio::spawn(async move {
                                    if let Ok(storage_manager) =
                                        storage::StorageManager::with_default_config()
                                    {
                                        let storage = storage_manager.storage();
                                        if let Err(e) =
                                            storage.rename_collection(&old_name, &new_name)
                                        {
                                            eprintln!("Failed to rename collection folder: {}", e);
                                        }
                                    }
                                });

                                return Task::none();
                            }
                        }
                    }
                }

                Task::none()
            }
            Message::DoNothing => Task::none(),
        }
    }

    fn view(&self) -> Element<'_, Message> {
        info!("=== Rendering main view ===");
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
                mouse_area(
                    container(
                        mouse_area(
                            container(self.environment_popup_view())
                                .width(800)
                                .height(650)
                        )
                        .on_press(Message::DoNothing)
                    )
                    .center_x(Fill)
                    .center_y(Fill)
                    .width(Fill)
                    .height(Fill)
                    .style(|_theme| container::Style {
                        background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.5).into()),
                        ..Default::default()
                    })
                )
                .on_press(Message::DoNothing)
                .on_scroll(|_| Message::DoNothing)
            ]
            .into()
        } else if self.show_rename_modal {
            // Create a custom overlay for the rename modal
            stack![
                pane_grid,
                // Semi-transparent backdrop with centered modal
                container(container(self.rename_modal_view()).width(400).height(200))
                    .center_x(Fill)
                    .center_y(Fill)
                    .width(Fill)
                    .height(Fill)
                    .style(|_theme| container::Style {
                        background: Some(Color::from_rgba(0.25, 0.25, 0.25, 0.7).into()),
                        ..Default::default()
                    })
            ]
            .into()
        } else {
            pane_grid.into()
        }
    }

    fn update_editor_content(editor_content: &mut text_editor::Content, content: String) {
        editor_content.perform(text_editor::Action::SelectAll);
        editor_content.perform(text_editor::Action::Edit(text_editor::Edit::Paste(
            Arc::new(content),
        )));
    }

    fn format_response_content(body: &str, body_format: BodyFormat) -> String {
        const MAX_JSON_FORMAT_SIZE: usize = 100 * 1024; // 100KB

        // Only format if body format is JSON
        if body_format != BodyFormat::Json {
            // Not JSON format, return as-is
            return body.to_string();
        }

        // Try to format JSON if the content is not too large
        if body.len() <= MAX_JSON_FORMAT_SIZE {
            if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(body) {
                if let Ok(formatted_json) = serde_json::to_string_pretty(&json_value) {
                    return formatted_json;
                }
            }
        }

        // If formatting fails or content is too large, return original
        body.to_string()
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

    /// Resolves all variables in a RequestConfig and returns a new resolved config
    fn resolve_request_config_variables(&self, config: &RequestConfig) -> RequestConfig {
        let mut resolved_config = config.clone();

        info!("resolve variables");
        // Resolve variables in URL
        resolved_config.url = self.resolve_variables(&resolved_config.url);
        info!("DEBUG: Resolved URL: {}", resolved_config.url);

        // Resolve variables in headers
        for (key, value) in &mut resolved_config.headers {
            *key = self.resolve_variables(key);
            *value = self.resolve_variables(value);
        }
        info!("DEBUG: Resolved Headers");

        // Resolve variables in params
        for (key, value) in &mut resolved_config.params {
            *key = self.resolve_variables(key);
            *value = self.resolve_variables(value);
        }
        info!("DEBUG: Resolved Params");

        // Resolve variables in body
        resolved_config.body = self.resolve_variables(&resolved_config.body);
        info!("DEBUG: Resolved Body");

        // Resolve variables in authentication fields
        resolved_config.bearer_token = self.resolve_variables(&resolved_config.bearer_token);
        resolved_config.basic_username = self.resolve_variables(&resolved_config.basic_username);
        resolved_config.basic_password = self.resolve_variables(&resolved_config.basic_password);
        resolved_config.api_key = self.resolve_variables(&resolved_config.api_key);
        resolved_config.api_key_header = self.resolve_variables(&resolved_config.api_key_header);

        resolved_config
    }

    /// Handles sending a request with the provided resolved config
    fn handle_send_request(
        &mut self,
        config: RequestConfig,
        request_start_time: Instant,
    ) -> Task<Message> {
        self.is_loading = true;
        self.request_start_time = Some(request_start_time);

        if let Some((collection_index, request_index)) = self.last_opened_request {
            if collection_index != config.collection_index || request_index != config.request_index
            {
                self.last_opened_request = Some((config.collection_index, config.request_index));
            }
        }

        Task::perform(send_request(config), Message::RequestCompleted)
    }
    fn collections_view(&self) -> Element<'_, Message> {
        // collections_panel(&self.collections, self.last_opened_request)
        self.collection_panel
            .view(&self.collections, self.last_opened_request)
            .map(Message::CollectionPanel)
    }

    fn request_config_view(&self) -> Element<'_, Message> {
        self.request_panel
            .view(
                &self.current_request,
                &self.request_body_content,
                &self.post_script_content,
                self.is_loading,
                &self.environments,
                self.active_environment,
            )
            .map(Message::RequestPanel)
    }

    fn response_view(&self) -> Element<'_, Message> {
        self.response_panel
            .view(
                &self.current_request.last_response,
                &self.response_body_content,
                self.is_loading,
                self.current_elapsed_time,
            )
            .map(Message::ResponsePanel)
    }

    fn environment_popup_view(&self) -> Element<'_, Message> {
        // Header with title and close button
        let close_button = button(
            container(
                icon(IconName::Close)
                    .size(20)
                    .color(Color::from_rgb(0.5, 0.5, 0.5)),
            )
            .align_x(iced::alignment::Horizontal::Center)
            .align_y(iced::alignment::Vertical::Center)
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .padding(Padding::from(6.0))
        .on_press(Message::CloseEnvironmentPopup)
        .width(32)
        .height(32)
        .style(|_theme: &Theme, status| {
            let base = button::Style::default();
            match status {
                button::Status::Hovered | button::Status::Pressed => button::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(0.9, 0.9, 0.9))),
                    border: iced::Border {
                        radius: 6.0.into(),
                        ..Default::default()
                    },
                    ..base
                },
                _ => button::Style {
                    background: Some(iced::Background::Color(Color::TRANSPARENT)),
                    border: iced::Border {
                        radius: 6.0.into(),
                        ..Default::default()
                    },
                    ..base
                },
            }
        });

        let header = row![
            text("Environments").size(16).color(Color::from_rgb(0.3, 0.3, 0.3)),
            space().width(Fill),
            close_button
        ]
        .align_y(iced::Alignment::Center);

        let mut sidebar = column![].spacing(8);
        
        sidebar = sidebar.push(space().height(10));
        
        // New Environment button at the top
        let new_env_button = button(
            row![
                icon(IconName::Add).size(14).color(Color::WHITE),
                space().width(8),
                text("New Environment").size(14)
            ]
            .align_y(iced::Alignment::Center)
        )
        .on_press(Message::AddEnvironment)
        .width(Fill)
        .padding([10, 16])
        .style(|_theme, status| {
            match status {
                button::Status::Hovered => button::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(0.1, 0.1, 0.1))),
                    text_color: Color::WHITE,
                    border: iced::Border {
                        radius: 6.0.into(),
                        ..Default::default()
                    },
                    ..button::Style::default()
                },
                _ => button::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(0.2, 0.2, 0.2))),
                    text_color: Color::WHITE,
                    border: iced::Border {
                        radius: 6.0.into(),
                        ..Default::default()
                    },
                    ..button::Style::default()
                },
            }
        });
        
        sidebar = sidebar.push(new_env_button);
        sidebar = sidebar.push(space().height(10));

        // Environment list
        for (idx, env) in self.environments.iter().enumerate() {
            let is_active = self.active_environment == Some(idx);
            let var_count = env.variables.len();
            
            let env_item = button(
                column![
                    row![
                        text(&env.name)
                            .size(14)
                            .color(if is_active { 
                                Color::from_rgb(0.1, 0.1, 0.1) 
                            } else { 
                                Color::from_rgb(0.3, 0.3, 0.3) 
                            }),
                        space().width(Fill),
                        if is_active {
                            container(
                                text("Active")
                                    .size(10)
                                    .color(Color::WHITE)
                            )
                            .padding([2, 6])
                            .style(|_theme: &Theme| container::Style {
                                background: Some(iced::Background::Color(Color::from_rgb(0.1, 0.1, 0.1))),
                                border: iced::Border {
                                    radius: 4.0.into(),
                                    ..Default::default()
                                },
                                ..Default::default()
                            })
                        } else {
                            container(text(""))
                        }
                    ]
                    .align_y(iced::Alignment::Center),
                    space().height(2),
                    text(format!("{} variables", var_count))
                        .size(12)
                        .color(Color::from_rgb(0.6, 0.6, 0.6))
                ]
                .spacing(0)
            )
            .on_press(Message::EnvironmentSelected(idx))
            .width(Fill)
            .padding([10, 12])
            .style(move |_theme, status| {
                match status {
                    button::Status::Hovered => button::Style {
                        background: Some(iced::Background::Color(
                            if is_active {
                                Color::from_rgb(0.92, 0.92, 0.92)
                            } else {
                                Color::from_rgb(0.97, 0.97, 0.97)
                            }
                        )),
                        border: iced::Border {
                            radius: 6.0.into(),
                            ..Default::default()
                        },
                        ..button::Style::default()
                    },
                    _ => button::Style {
                        background: Some(iced::Background::Color(
                            if is_active {
                                Color::from_rgb(0.95, 0.95, 0.95)
                            } else {
                                Color::TRANSPARENT
                            }
                        )),
                        border: iced::Border {
                            radius: 6.0.into(),
                            ..Default::default()
                        },
                        ..button::Style::default()
                    },
                }
            });
            
            sidebar = sidebar.push(env_item);
        }

        let sidebar_container = container(
            scrollable(sidebar)
                .height(Fill)
        )
        .width(240)
        .height(Fill)
        .padding(12)
        .style(|_theme: &Theme| container::Style {
            border: iced::Border {
                color: Color::from_rgb(0.9, 0.9, 0.9),
                width: 0.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        });

        // RIGHT PANEL - Variables display
        let right_panel = if let Some(active_idx) = self.active_environment {
            if let Some(active_env) = self.environments.get(active_idx) {
                let mut panel_content = column![].spacing(12);

                // Environment name input with Active badge
                let env_header = row![
                    text_input("", &active_env.name)
                        .on_input(move |input| {
                            Message::EnvironmentNameChanged(active_idx, input)
                        })
                        .padding(8)
                        .size(18)
                        .width(Length::FillPortion(3))
                        .style(|_theme, status| {
                            let (border_color, border_width) = match status {
                                text_input::Status::Focused { .. } => (Color::from_rgb(0.7, 0.7, 0.7), 1.0),
                                _ => (Color::from_rgb(0.9, 0.9, 0.9), 1.0),
                            };
                            text_input::Style {
                                background: iced::Background::Color(Color::TRANSPARENT),
                                border: iced::Border {
                                    color: border_color,
                                    width: border_width,
                                    radius: 4.0.into(),
                                },
                                icon: Color::from_rgb(0.5, 0.5, 0.5),
                                placeholder: Color::from_rgb(0.7, 0.7, 0.7),
                                value: Color::from_rgb(0.1, 0.1, 0.1),
                                selection: Color::from_rgb(0.7, 0.85, 1.0),
                            }
                        }),
                    space().width(10),
                    container(
                        text("Active")
                            .size(12)
                            .color(Color::WHITE)
                    )
                    .padding([4, 10])
                    .style(|_theme: &Theme| container::Style {
                        background: Some(iced::Background::Color(Color::from_rgb(0.1, 0.1, 0.1))),
                        border: iced::Border {
                            radius: 12.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    space().width(Fill)
                ]
                .align_y(iced::Alignment::Center);

                panel_content = panel_content.push(env_header);
                
                // Description
                panel_content = panel_content.push(
                    text("Define variables that can be used across your requests")
                        .size(13)
                        .color(Color::from_rgb(0.5, 0.5, 0.5))
                );

                panel_content = panel_content.push(space().height(10));

                // Variables section header
                let enabled_count = active_env.variables.len();
                let variables_header = row![
                    text("Variables").size(14),
                    space().width(10),
                    container(
                        text(format!("{} enabled", enabled_count))
                            .size(12)
                            .color(Color::from_rgb(0.5, 0.5, 0.5))
                    )
                    .padding([2, 8])
                    .style(|_theme: &Theme| container::Style {
                        background: Some(iced::Background::Color(Color::from_rgb(0.95, 0.95, 0.95))),
                        border: iced::Border {
                            radius: 10.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    space().width(Fill),
                    text("Show Values").size(13).color(Color::from_rgb(0.5, 0.5, 0.5))
                ]
                .align_y(iced::Alignment::Center);

                panel_content = panel_content.push(variables_header);
                panel_content = panel_content.push(space().height(8));

                // Variables table - create a column to hold header and rows
                let mut table_content = column![].spacing(0);

                // Variables table header
                let table_header = container(
                    row![
                        container(text("Key").size(12).color(Color::from_rgb(0.3, 0.3, 0.3)).font(iced::Font {
                            weight: iced::font::Weight::Bold,
                            ..Default::default()
                        }))
                            .width(Length::FillPortion(1))
                            .padding([8, 8]),
                        container(text("Value").size(12).color(Color::from_rgb(0.3, 0.3, 0.3)).font(iced::Font {
                            weight: iced::font::Weight::Bold,
                            ..Default::default()
                        }))
                            .width(Length::FillPortion(1))
                            .padding([8, 8]),
                        container(text("").width(40)) // Delete button column
                    ]
                    .spacing(10)
                )
                .padding([8, 8])
                .style(|_theme: &Theme| container::Style {
                    background: Some(iced::Background::Color(Color::TRANSPARENT)),
                    border: iced::Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: 0.0.into(),
                    },
                    ..Default::default()
                });

                table_content = table_content.push(table_header);

                // Add separator after header
                if !active_env.variables.is_empty() {
                    table_content = table_content.push(
                        container(space())
                            .width(Length::Fill)
                            .height(1)
                            .style(|_theme: &Theme| container::Style {
                                background: Some(iced::Background::Color(Color::from_rgb(0.9, 0.9, 0.9))),
                                ..Default::default()
                            })
                    );
                }

                // Variables rows
                for (i, (key, value)) in active_env.variables.iter().enumerate() {
                    if i > 0 {
                        // Add separator between rows
                        table_content = table_content.push(
                            container(space())
                                .width(Length::Fill)
                                .height(1)
                                .style(|_theme: &Theme| container::Style {
                                    background: Some(iced::Background::Color(Color::from_rgb(0.9, 0.9, 0.9))),
                                    ..Default::default()
                                })
                        );
                    }
                    let key_clone = key.clone();
                    let key_clone2 = key.clone();
                    let key_clone3 = key.clone();
                    
                    let delete_button = button(
                        container(
                            icon(IconName::Close)
                                .size(14)
                                .color(Color::from_rgb(0.8, 0.3, 0.3)),
                        )
                        .align_x(iced::alignment::Horizontal::Center)
                        .align_y(iced::alignment::Vertical::Center)
                        .width(Length::Fill)
                        .height(Length::Fill),
                    )
                    .on_press(Message::RemoveVariable(active_idx, key_clone3))
                    .width(32)
                    .height(32)
                    .style(|_theme: &Theme, status| {
                        let base = button::Style::default();
                        match status {
                            button::Status::Hovered | button::Status::Pressed => button::Style {
                                background: Some(iced::Background::Color(Color::from_rgb(0.98, 0.95, 0.95))),
                                border: iced::Border {
                                    radius: 4.0.into(),
                                    ..Default::default()
                                },
                                ..base
                            },
                            _ => button::Style {
                                background: Some(iced::Background::Color(Color::TRANSPARENT)),
                                ..base
                            },
                        }
                    });

                    let variable_row = container(
                        row![
                            text_input("", key)
                                .on_input(move |input| Message::VariableKeyChanged(
                                    active_idx,
                                    key_clone.clone(),
                                    input
                                ))
                                .padding(8)
                                .size(13)
                                .width(Length::FillPortion(1))
                                .style(|_theme, status| {
                                    let (border_color, border_width) = match status {
                                        text_input::Status::Focused { .. } => (Color::from_rgb(0.7, 0.7, 0.7), 1.0),
                                        _ => (Color::from_rgb(0.9, 0.9, 0.9), 1.0),
                                    };
                                    text_input::Style {
                                        background: iced::Background::Color(Color::TRANSPARENT),
                                        border: iced::Border {
                                            color: border_color,
                                            width: border_width,
                                            radius: 4.0.into(),
                                        },
                                        icon: Color::from_rgb(0.5, 0.5, 0.5),
                                        placeholder: Color::from_rgb(0.7, 0.7, 0.7),
                                        value: Color::from_rgb(0.1, 0.1, 0.1),
                                        selection: Color::from_rgb(0.7, 0.85, 1.0),
                                    }
                                }),
                            text_input("", value)
                                .on_input(move |input| Message::VariableValueChanged(
                                    active_idx,
                                    key_clone2.clone(),
                                    input
                                ))
                                .padding(8)
                                .size(13)
                                .width(Length::FillPortion(1))
                                .style(|_theme, status| {
                                    let (border_color, border_width) = match status {
                                        text_input::Status::Focused { .. } => (Color::from_rgb(0.7, 0.7, 0.7), 1.0),
                                        _ => (Color::from_rgb(0.9, 0.9, 0.9), 1.0),
                                    };
                                    text_input::Style {
                                        background: iced::Background::Color(Color::TRANSPARENT),
                                        border: iced::Border {
                                            color: border_color,
                                            width: border_width,
                                            radius: 4.0.into(),
                                        },
                                        icon: Color::from_rgb(0.5, 0.5, 0.5),
                                        placeholder: Color::from_rgb(0.7, 0.7, 0.7),
                                        value: Color::from_rgb(0.1, 0.1, 0.1),
                                        selection: Color::from_rgb(0.7, 0.85, 1.0),
                                    }
                                }),
                            container(delete_button)
                                .width(40)
                                .align_x(iced::alignment::Horizontal::Center)
                        ]
                        .spacing(10)
                        .align_y(iced::Alignment::Center)
                    )
                    .padding([8, 8])
                    .style(|_theme: &Theme| container::Style {
                        background: Some(iced::Background::Color(Color::TRANSPARENT)),
                        border: iced::Border {
                            color: Color::TRANSPARENT,
                            width: 0.0,
                            radius: 0.0.into(),
                        },
                        ..Default::default()
                    });

                    table_content = table_content.push(variable_row);
                }

                // Wrap the table in a container with rounded border
                let table_container = container(table_content)
                    .style(|_theme: &Theme| container::Style {
                        background: Some(iced::Background::Color(Color::WHITE)),
                        border: iced::Border {
                            color: Color::from_rgb(0.9, 0.9, 0.9),
                            width: 1.0,
                            radius: 8.0.into(),
                        },
                        ..Default::default()
                    });

                panel_content = panel_content.push(table_container);

                // Add Variable button
                panel_content = panel_content.push(space().height(8));
                panel_content = panel_content.push(
                    button(
                        row![
                            icon(IconName::Add).size(14),
                            space().width(6),
                            text("Add Variable").size(13)
                        ]
                        .align_y(iced::Alignment::Center)
                    )
                    .on_press(Message::AddVariable(active_idx))
                    .padding([8, 12])
                    .style(|_theme, status| {
                        match status {
                            button::Status::Hovered => button::Style {
                                background: Some(iced::Background::Color(Color::from_rgb(0.96, 0.96, 0.96))),
                                text_color: Color::from_rgb(0.3, 0.3, 0.3),
                                border: iced::Border {
                                    color: Color::from_rgb(0.85, 0.85, 0.85),
                                    width: 1.0,
                                    radius: 6.0.into(),
                                },
                                ..button::Style::default()
                            },
                            _ => button::Style {
                                background: Some(iced::Background::Color(Color::WHITE)),
                                text_color: Color::from_rgb(0.3, 0.3, 0.3),
                                border: iced::Border {
                                    color: Color::from_rgb(0.9, 0.9, 0.9),
                                    width: 1.0,
                                    radius: 6.0.into(),
                                },
                                ..button::Style::default()
                            },
                        }
                    })
                );

                // Using Variables section
                panel_content = panel_content.push(space().height(20));
                panel_content = panel_content.push(
                    container(
                        column![
                            text("Using Variables").size(14),
                            space().height(6),
                            text("Reference variables in your requests using the syntax: {{variable_name}}")
                                .size(12)
                                .color(Color::from_rgb(0.5, 0.5, 0.5)),
                            space().height(4),
                            text("Example: {{base_url}}/api/users")
                                .size(12)
                                .color(Color::from_rgb(0.5, 0.5, 0.5))
                        ]
                        .spacing(0)
                    )
                    .padding(12)
                    .style(|_theme: &Theme| container::Style {
                        background: Some(iced::Background::Color(Color::from_rgb(0.97, 0.97, 0.98))),
                        border: iced::Border {
                            color: Color::from_rgb(0.9, 0.9, 0.92),
                            width: 1.0,
                            radius: 6.0.into(),
                        },
                        ..Default::default()
                    })
                );

                // Environment management section
                panel_content = panel_content.push(space().height(20));
                panel_content = panel_content.push(
                    container(
                        column![
                            text("Environment Settings").size(14),
                            space().height(10),
                            text("Description").size(12).color(Color::from_rgb(0.5, 0.5, 0.5)),
                            space().height(4),
                            text_input(
                                "Environment description",
                                active_env.description.as_deref().unwrap_or(""),
                            )
                            .on_input(move |input| {
                                Message::EnvironmentDescriptionChanged(active_idx, input)
                            })
                            .padding(8)
                            .size(13),
                            if self.environments.len() > 1 {
                                column![
                                    space().height(15),
                                    button(
                                        row![
                                            icon(IconName::Close).size(14).color(Color::from_rgb(0.8, 0.3, 0.3)),
                                            space().width(6),
                                            text("Delete Environment").size(13)
                                        ]
                                        .align_y(iced::Alignment::Center)
                                    )
                                    .on_press(Message::DeleteEnvironment(active_idx))
                                    .padding([8, 12])
                                    .style(|_theme, status| {
                                        match status {
                                            button::Status::Hovered => button::Style {
                                                background: Some(iced::Background::Color(Color::from_rgb(0.95, 0.3, 0.3))),
                                                text_color: Color::WHITE,
                                                border: iced::Border {
                                                    radius: 6.0.into(),
                                                    ..Default::default()
                                                },
                                                ..button::Style::default()
                                            },
                                            _ => button::Style {
                                                background: Some(iced::Background::Color(Color::from_rgb(0.98, 0.95, 0.95))),
                                                text_color: Color::from_rgb(0.8, 0.3, 0.3),
                                                border: iced::Border {
                                                    color: Color::from_rgb(0.95, 0.85, 0.85),
                                                    width: 1.0,
                                                    radius: 6.0.into(),
                                                },
                                                ..button::Style::default()
                                            },
                                        }
                                    })
                                ]
                            } else {
                                column![]
                            }
                        ]
                        .spacing(0)
                    )
                    .padding(12)
                    .style(|_theme: &Theme| container::Style {
                        background: Some(iced::Background::Color(Color::from_rgb(0.98, 0.98, 0.98))),
                        border: iced::Border {
                            color: Color::from_rgb(0.92, 0.92, 0.92),
                            width: 1.0,
                            radius: 6.0.into(),
                        },
                        ..Default::default()
                    })
                );

                scrollable(panel_content)
                    .height(Fill)
                    .direction(scrollable::Direction::Vertical(
                        scrollable::Scrollbar::new()
                            .width(0)
                            .scroller_width(0)
                    ))
            } else {
                scrollable(
                    column![
                        text("No environment selected").size(14).color(Color::from_rgb(0.5, 0.5, 0.5))
                    ]
                )
                .height(Fill)
                .direction(scrollable::Direction::Vertical(
                    scrollable::Scrollbar::new()
                        .width(0)
                        .scroller_width(0)
                ))
            }
        } else {
            scrollable(
                column![
                    text("No environment selected").size(14).color(Color::from_rgb(0.5, 0.5, 0.5)),
                    space().height(10),
                    text("Select an environment from the sidebar or create a new one")
                        .size(13)
                        .color(Color::from_rgb(0.6, 0.6, 0.6))
                ]
            )
            .height(Fill)
            .direction(scrollable::Direction::Vertical(
                scrollable::Scrollbar::new()
                    .width(0)
                    .scroller_width(0)
            ))
        };

        let right_panel_container = container(right_panel)
            .width(Fill)
            .height(Fill)
            .padding(12);

        // Main layout: header + two-panel content
        let main_content = row![
            sidebar_container,
            container(
                column![text("")]
                    .width(1)
            )
            .height(Fill)
            .style(|_theme: &Theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.9, 0.9, 0.9))),
                ..Default::default()
            }),
            right_panel_container
        ]
        .spacing(0)
        .height(Fill);

        container(
            column![
                header,
                main_content
            ]
            .spacing(0)
            .height(Fill),
        )
        .width(Length::Fixed(1000.0))
        .height(Length::Fixed(650.0))
        .padding(20)
        .style(|_theme: &Theme| container::Style {
            background: Some(iced::Background::Color(Color::WHITE)),
            border: iced::Border {
                color: Color::from_rgb(0.85, 0.85, 0.85),
                width: 1.0,
                radius: 8.0.into(),
            },
            shadow: iced::Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.15),
                offset: Vector::new(0.0, 4.0),
                blur_radius: 20.0,
            },
            ..Default::default()
        })
        .into()
    }

    fn rename_modal_view(&self) -> Element<'_, Message> {
        let (title, description) = match &self.rename_target {
            Some(RenameTarget::Folder(_)) => ("Rename Folder", "Enter a new name for the folder:"),
            Some(RenameTarget::Request(_, _)) => {
                ("Rename Request", "Enter a new name for the request:")
            }
            None => ("Rename", "Enter a new name:"),
        };

        let header =
            row![text(title).size(18), space().width(Fill),].align_y(iced::Alignment::Center);

        let input_field = text_input("Enter new name...", &self.rename_input)
            .on_input(Message::RenameInputChanged)
            .on_submit(Message::ConfirmRename)
            .padding(10)
            .size(16);

        let buttons = container(
            row![
                button(text("Cancel").size(16))
                    .on_press(Message::HideRenameModal)
                    .padding(10)
                    .style(|_theme: &Theme, status| {
                        let base = button::Style::default();
                        match status {
                            button::Status::Hovered => button::Style {
                                background: Some(iced::Background::Color(color!(0xe4e4e7))),
                                border: iced::Border {
                                    color: color!(0xa1a1aa),
                                    width: 0.0,
                                    radius: 8.0.into(),
                                },
                                text_color: color!(0x18181b),
                                snap: true,
                                ..base
                            },
                            _ => button::Style {
                                background: Some(iced::Background::Color(Color::WHITE)),
                                border: iced::Border {
                                    color: color!(0xe4e4e7),
                                    width: 1.0,
                                    radius: 8.0.into(),
                                },
                                text_color: color!(0x3f3f46),
                                snap: true,
                                ..base
                            },
                        }
                    }),
                space().width(10),
                button(text("Rename").size(16))
                    .on_press(Message::ConfirmRename)
                    .padding(10)
                    .style(|_theme: &Theme, status| {
                        let base = button::Style::default();
                        match status {
                            button::Status::Hovered => button::Style {
                                background: Some(iced::Background::Color(color!(0x4f46e5))),
                                border: iced::Border {
                                    color: color!(0x818cf8),
                                    width: 0.0,
                                    radius: 8.0.into(),
                                },
                                text_color: Color::WHITE,
                                snap: true,
                                ..base
                            },
                            _ => button::Style {
                                background: Some(iced::Background::Color(color!(0x818cf8))),
                                border: iced::Border {
                                    color: color!(0xc7d2fe),
                                    width: 0.0,
                                    radius: 8.0.into(),
                                },
                                text_color: Color::WHITE,
                                snap: true,
                                ..base
                            },
                        }
                    }),
            ]
            .align_y(iced::Alignment::Center),
        )
        .width(Fill)
        .align_x(iced::Alignment::End);

        container(column![
            header,
            space().height(10),
            text(description).size(14),
            space().height(10),
            input_field,
            space().height(10),
            buttons,
        ])
        .padding(20)
        .style(|_theme: &Theme| container::Style {
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
            snap: true,
            ..Default::default()
        })
        .into()
    }

    fn subscription(&self) -> iced::Subscription<Message> {
        let timer_subscription = if self.is_loading {
            iced::time::every(std::time::Duration::from_millis(100)).map(|_| Message::TimerTick)
        } else {
            iced::Subscription::none()
        };

        let keyboard_subscription = iced::event::listen_with(|event, _status, _id| {
            match event {
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key, ..
                }) => {
                    // Forward all key presses to the KeyPressed handler
                    Some(Message::KeyPressed(key.clone()))
                }
                _ => None,
            }
        });

        iced::Subscription::batch([timer_subscription, keyboard_subscription])
    }

    fn initialize_debouncer(&mut self) {
        let (debounce_tx, mut debounce_rx) = mpsc::channel::<RequestConfig>(10);
        self.debounce_tx = Some(debounce_tx);

        // Start the debouncer task
        tokio::spawn(async move {
            let duration = Duration::from_millis(500);
            let mut last_request: Option<RequestConfig> = None;

            loop {
                match tokio::time::timeout(duration, debounce_rx.recv()).await {
                    Ok(Some(request_config)) => {
                        // Received a new request, store it and continue waiting
                        last_request = Some(request_config);
                        info!("Debouncer received request update");
                    }
                    Ok(None) => {
                        // Channel closed, save any pending request and exit
                        if let Some(request) = last_request {
                            Self::save_request(request);
                        }
                        info!("Debounce channel closed");
                        break;
                    }
                    Err(_) => {
                        // Timeout occurred, save the last request if any
                        if let Some(request) = last_request.take() {
                            info!("Debounce save request");
                            Self::save_request(request);
                        }
                    }
                }
            }
        });
    }

    fn save_request(request_config: RequestConfig) {
        match storage::StorageManager::with_default_config() {
            Ok(storage_manager) => {
                info!("===request auto saved (debounced)");

                if let Err(e) = storage_manager
                    .storage()
                    .save_request_by_path(&request_config)
                {
                    error!("Failed to save request: {}", e);
                }
            }
            Err(e) => {
                error!("Failed to create storage manager: {}", e);
            }
        }
    }

    fn theme(&self) -> Theme {
        // Catppuccin Mocha color palette
        let base = Color::from_rgb(0.118, 0.118, 0.180); // #1e1e2e
        let text = Color::from_rgb(0.804, 0.839, 0.957); // #cdd6f4
        let primary = Color::from_rgb(0.537, 0.706, 0.980); // #89b4fa (blue)
        let success = Color::from_rgb(0.651, 0.890, 0.631); // #a6e3a1 (green)
        let danger = Color::from_rgb(0.953, 0.545, 0.659); // #f38ba8 (red)
        let warning = Color::from_rgb(0.980, 0.706, 0.529); // #fab387 (peach)

        Theme::custom(
            "Catppuccin Mocha".to_string(),
            iced::theme::Palette {
                background: base,
                text,
                primary,
                success,
                danger,
                warning,
            },
        )
    }
}
