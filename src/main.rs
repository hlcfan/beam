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
use beam::ui::EnvironmentPanel;
use beam::ui::RequestPanel;
use beam::ui::ResponsePanel;
use std::sync::Arc;

use beam::ui::collections;
use beam::ui::environment;
use beam::ui::request;
use beam::ui::response;
use beam::ui::undoable_editor;

use iced::color;
use iced::widget::pane_grid::{self, Axis, PaneGrid};
use iced::widget::{
    button, column, container, mouse_area, operation, row, space, stack, text, text_editor,
    text_input,
};
use iced::{Color, Element, Fill, Size, Task, Theme, Vector};
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
    EnvironmentPanel(environment::Message),

    PaneResized(pane_grid::ResizeEvent),
    TimerTick,

    ModifiersChanged(iced::keyboard::Modifiers),
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
    pub environment_panel: EnvironmentPanel,
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

    // Track keyboard modifiers
    pub modifiers: iced::keyboard::Modifiers,
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
            environment_panel: EnvironmentPanel::new(),
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
            modifiers: iced::keyboard::Modifiers::default(),
        }
    }
}

impl BeamApp {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers;
                Task::none()
            }
            Message::RequestPanel(view_message) => {
                match self.request_panel.update(
                    view_message,
                    &self.current_request,
                    &self.environments,
                    &mut self.request_body_content,
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
                        // If the user hasn't made any edits yet, ensure the URL baseline is synced.
                        // This prevents undo from clearing the entire input on the first edit.
                        if !self.request_panel.url_input.has_history() {
                            self.request_panel
                                .url_input
                                .set_value(request_config.url.clone());
                        }
                        self.current_request = request_config.clone();

                        if self.request_body_content.text() != self.current_request.body {
                            Self::update_editor_content(
                                &mut self.request_body_content,
                                self.current_request.body.to_string(),
                            );
                        }

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
                        let mut request = self.current_request.clone();
                        request.post_request_script = Some(self.post_script_content.text());

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

                        if let Some(tx) = &self.debounce_tx {
                            if let Err(_) = tx.try_send(request_to_persist) {
                                info!("Debounce channel is full or closed");
                            }
                        }

                        Task::none()
                    }
                    request::Action::Focus(id) => {
                        return iced::widget::operation::focus(id)
                            .map(|_: ()| Message::RequestPanel(request::Message::DoNothing));
                    }
                    request::Action::SearchNext(focus_id) => {
                        self.perform_search(true, Some(focus_id))
                    }
                    request::Action::SearchPrevious(focus_id) => {
                        self.perform_search(false, Some(focus_id))
                    }
                    request::Action::SubmitSearch(focus_id) => {
                        let is_previous = self.modifiers.shift();
                        self.perform_search(!is_previous, Some(focus_id))
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
                match self
                    .response_panel
                    .update(view_message, &self.current_request.last_response)
                {
                    response::Action::ResponseBodyAction(action) => {
                        self.response_body_content.perform(action);

                        Task::none()
                    }
                    response::Action::FormatResponseBody(formatted_body) => {
                        let select_all_action = text_editor::Action::SelectAll;
                        self.response_body_content.perform(select_all_action);

                        let paste_action = text_editor::Action::Edit(text_editor::Edit::Paste(
                            std::sync::Arc::new(formatted_body),
                        ));
                        self.response_body_content.perform(paste_action);

                        Task::none()
                    }
                    response::Action::SearchNext(focus_id) => {
                        self.perform_response_search(true, Some(focus_id))
                    }
                    response::Action::SearchPrevious(focus_id) => {
                        self.perform_response_search(false, Some(focus_id))
                    }
                    response::Action::SubmitSearch(focus_id) => {
                        let is_previous = self.modifiers.shift();
                        self.perform_response_search(!is_previous, Some(focus_id))
                    }
                    response::Action::Focus(id) => iced::widget::operation::focus(id)
                        .map(|_: ()| Message::ResponsePanel(response::Message::DoNothing)),
                    response::Action::Run(task) => task.map(Message::ResponsePanel),
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
                                self.request_panel
                                    .reset_undo_histories(&self.current_request.url, &self.current_request.body);

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
                                new_request.request_index = collection.requests.len();

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

                        Self::update_editor_content(
                            &mut self.response_body_content,
                            formatted_body,
                        );

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
                            if let Some(var) = active_env.variables.get_mut(&key) {
                                var.value = value;
                            } else {
                                active_env.add_variable(key, value);
                            }
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
            Message::EnvironmentPanel(env_message) => {
                match self.environment_panel.update(env_message) {
                    environment::Action::AddEnvironment => {
                        let new_env = Environment::new(format!(
                            "Environment {}",
                            self.environments.len() + 1
                        ));
                        self.environments.push(new_env);
                        self.active_environment = Some(self.environments.len() - 1);

                        let environments = self.environments.clone();
                        Task::perform(
                            async move {
                                match storage::StorageManager::with_default_config() {
                                    Ok(storage_manager) => {
                                        match storage_manager
                                            .storage()
                                            .save_environments(&environments)
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
                    }
                    environment::Action::DeleteEnvironment(index) => {
                        if index < self.environments.len() && self.environments.len() > 1 {
                            self.environments.remove(index);
                            if let Some(active) = self.active_environment {
                                if active == index {
                                    self.active_environment = Some(0);
                                } else if active > index {
                                    self.active_environment = Some(active - 1);
                                }
                            }

                            let environments = self.environments.clone();
                            Task::perform(
                                async move {
                                    match storage::StorageManager::with_default_config() {
                                        Ok(storage_manager) => {
                                            match storage_manager
                                                .storage()
                                                .save_environments(&environments)
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
                    environment::Action::EnvironmentNameChanged(env_index, name) => {
                        if let Some(env) = self.environments.get_mut(env_index) {
                            env.name = name;

                            let environments = self.environments.clone();
                            Task::perform(
                                async move {
                                    match storage::StorageManager::with_default_config() {
                                        Ok(storage_manager) => {
                                            match storage_manager
                                                .storage()
                                                .save_environments(&environments)
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
                    environment::Action::EnvironmentDescriptionChanged(env_index, description) => {
                        if let Some(env) = self.environments.get_mut(env_index) {
                            env.description = if description.is_empty() {
                                None
                            } else {
                                Some(description)
                            };

                            let environments = self.environments.clone();
                            Task::perform(
                                async move {
                                    match storage::StorageManager::with_default_config() {
                                        Ok(storage_manager) => {
                                            match storage_manager
                                                .storage()
                                                .save_environments(&environments)
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
                    environment::Action::VariableKeyChanged(env_index, old_key, new_key) => {
                        if let Some(env) = self.environments.get_mut(env_index) {
                            if let Some(value) = env.variables.remove(&old_key) {
                                env.variables.insert(new_key, value);
                            }

                            let environments = self.environments.clone();
                            Task::perform(
                                async move {
                                    match storage::StorageManager::with_default_config() {
                                        Ok(storage_manager) => {
                                            match storage_manager
                                                .storage()
                                                .save_environments(&environments)
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
                    environment::Action::VariableValueChanged(env_index, key, value) => {
                        if let Some(env) = self.environments.get_mut(env_index) {
                            if let Some(var) = env.variables.get_mut(&key) {
                                var.value = value;
                            }

                            let environments = self.environments.clone();
                            Task::perform(
                                async move {
                                    match storage::StorageManager::with_default_config() {
                                        Ok(storage_manager) => {
                                            match storage_manager
                                                .storage()
                                                .save_environments(&environments)
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
                    environment::Action::AddVariable(env_index) => {
                        if let Some(env) = self.environments.get_mut(env_index) {
                            let var_count = env.variables.len();
                            env.add_variable(format!("variable_{}", var_count + 1), String::new());

                            let environments = self.environments.clone();
                            Task::perform(
                                async move {
                                    match storage::StorageManager::with_default_config() {
                                        Ok(storage_manager) => {
                                            match storage_manager
                                                .storage()
                                                .save_environments(&environments)
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
                    environment::Action::RemoveVariable(env_index, key) => {
                        if let Some(env) = self.environments.get_mut(env_index) {
                            env.variables.remove(&key);

                            let environments = self.environments.clone();
                            Task::perform(
                                async move {
                                    match storage::StorageManager::with_default_config() {
                                        Ok(storage_manager) => {
                                            match storage_manager
                                                .storage()
                                                .save_environments(&environments)
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
                    environment::Action::ToggleVariable(env_index, key) => {
                        if let Some(env) = self.environments.get_mut(env_index) {
                            if let Some(var) = env.variables.get_mut(&key) {
                                var.enabled = !var.enabled;
                            }

                            let environments = self.environments.clone();
                            Task::perform(
                                async move {
                                    match storage::StorageManager::with_default_config() {
                                        Ok(storage_manager) => {
                                            match storage_manager
                                                .storage()
                                                .save_environments(&environments)
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
                    environment::Action::ClosePopup => {
                        self.show_environment_popup = false;
                        Task::none()
                    }
                    environment::Action::EnvironmentSelected(index) => {
                        self.active_environment = Some(index);
                        Task::none()
                    }
                    environment::Action::None => Task::none(),
                }
            }
            Message::KeyPressed(key) => match key {
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
                _ => Task::none(),
            },
            Message::TimerTick => {
                if let Some(start_time) = self.request_start_time {
                    self.current_elapsed_time = start_time.elapsed().as_millis() as u64;
                }

                if self.is_loading {
                    self.response_panel.update_spinner();
                }

                Task::none()
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
                                self.request_panel
                                    .reset_undo_histories(&self.current_request.url, &self.current_request.body);

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

    fn perform_search(&mut self, next: bool, focus_id: Option<iced::widget::Id>) -> Task<Message> {
        let query = &self.request_panel.search_query;
        if query.is_empty() {
            if let Some(id) = focus_id {
                return operation::focus(id)
                    .map(|_: ()| Message::RequestPanel(request::Message::DoNothing));
            }
            return Task::none();
        }

        let content = &mut self.request_body_content;
        let text = content.text();

        let current_pos = self
            .request_panel
            .search_selection
            .map(|(_, end)| end)
            .unwrap_or(content.cursor().position);

        let current_idx = Self::position_to_byte_index(content, current_pos);

        let search_start_idx = if next {
            // Start search from current cursor position + char length to find the "next" match
            if let Some(c) = text.get(current_idx..).and_then(|s| s.chars().next()) {
                current_idx + c.len_utf8()
            } else {
                current_idx
            }
        } else {
            // For previous, search from beginning up to current cursor position
            current_idx
        };

        let search_range = if next {
            search_start_idx..text.len()
        } else {
            0..search_start_idx
        };

        let mut found_match = false;
        let mut match_start_byte = 0;
        let mut match_end_byte = 0;

        if next {
            // Search forward
            if let Some(start) = text[search_range].find(query) {
                match_start_byte = search_start_idx + start;
                match_end_byte = match_start_byte + query.len();
                found_match = true;
            } else {
                // Wrap around: search from beginning
                if let Some(start) = text[0..search_start_idx].find(query) {
                    match_start_byte = start;
                    match_end_byte = match_start_byte + query.len();
                    found_match = true;
                }
            }
        } else {
            // Search backward
            if let Some(start) = text[search_range.clone()].rfind(query) {
                match_start_byte = search_range.start + start;
                match_end_byte = match_start_byte + query.len();

                if match_end_byte == current_idx {
                    // If we found the match ending exactly at our cursor,
                    // we want to find the one before it.
                    if let Some(prev_start) = text[0..match_start_byte].rfind(query) {
                        match_start_byte = prev_start;
                        match_end_byte = match_start_byte + query.len();
                        found_match = true;
                    } else {
                        // If no previous match, try wrapping around to the end
                        if let Some(last_start) = text[search_start_idx..text.len()].rfind(query) {
                            match_start_byte = search_start_idx + last_start;
                            match_end_byte = match_start_byte + query.len();
                            found_match = true;
                        }
                    }
                } else {
                    found_match = true;
                }
            } else {
                // Wrap around: search from end
                if let Some(start) = text[search_start_idx..text.len()].rfind(query) {
                    match_start_byte = search_start_idx + start;
                    match_end_byte = match_start_byte + query.len();
                    found_match = true;
                }
            }
        }

        if found_match {
            let start_pos = Self::byte_index_to_position(content, match_start_byte);
            let end_pos = Self::byte_index_to_position(content, match_end_byte);

            // Do NOT modify the user's cursor or selection!
            // The highlight will be driven independently by `SearchFound` -> `search_selection`.

            // Calculate scroll position more precisely to center the match
            // The overlay highlight and scroll operation is now driven precisely by
            // `EditorView` detecting the active match change and dispatching a `ScrollTo` action.

            let message_task =
                Task::perform(async move { (start_pos, end_pos) }, |(start, end)| {
                    Message::RequestPanel(request::Message::SearchFound(start, end))
                });

            if let Some(id) = focus_id {
                Task::batch(vec![
                    message_task,
                    operation::focus(id)
                        .map(|_: ()| Message::RequestPanel(request::Message::DoNothing)),
                ])
            } else {
                message_task
            }
        } else {
            if let Some(id) = focus_id {
                operation::focus(id)
                    .map(|_: ()| Message::RequestPanel(request::Message::SearchNotFound))
            } else {
                Task::perform(async {}, |_| {
                    Message::RequestPanel(request::Message::SearchNotFound)
                })
            }
        }
    }

    // TODO: dedup the same logic with perform_request_search
    fn perform_response_search(
        &mut self,
        next: bool,
        focus_id: Option<iced::widget::Id>,
    ) -> Task<Message> {
        let query = &self.response_panel.search_query;
        if query.is_empty() {
            if let Some(id) = focus_id {
                return operation::focus(id)
                    .map(|_: ()| Message::ResponsePanel(response::Message::DoNothing));
            }
            return Task::none();
        }

        let content = &mut self.response_body_content;
        let text = content.text();

        let current_pos = self
            .response_panel
            .search_selection
            .map(|(_, end)| end)
            .unwrap_or(content.cursor().position);

        let current_idx = Self::position_to_byte_index(content, current_pos);

        let search_start_idx = if next {
            if let Some(c) = text.get(current_idx..).and_then(|s| s.chars().next()) {
                current_idx + c.len_utf8()
            } else {
                current_idx
            }
        } else {
            current_idx
        };

        let search_range = if next {
            search_start_idx..text.len()
        } else {
            0..search_start_idx
        };

        let mut found_match = false;
        let mut match_start_byte = 0;
        let mut match_end_byte = 0;

        if next {
            if let Some(start) = text[search_range].find(query) {
                match_start_byte = search_start_idx + start;
                match_end_byte = match_start_byte + query.len();
                found_match = true;
            } else {
                if let Some(start) = text[0..search_start_idx].find(query) {
                    match_start_byte = start;
                    match_end_byte = match_start_byte + query.len();
                    found_match = true;
                }
            }
        } else {
            if let Some(start) = text[search_range.clone()].rfind(query) {
                match_start_byte = search_range.start + start;
                match_end_byte = match_start_byte + query.len();

                if match_end_byte == current_idx {
                    if let Some(prev_start) = text[0..match_start_byte].rfind(query) {
                        match_start_byte = prev_start;
                        match_end_byte = match_start_byte + query.len();
                        found_match = true;
                    } else {
                        if let Some(last_start) = text[search_start_idx..text.len()].rfind(query) {
                            match_start_byte = search_start_idx + last_start;
                            match_end_byte = match_start_byte + query.len();
                            found_match = true;
                        }
                    }
                } else {
                    found_match = true;
                }
            } else {
                if let Some(start) = text[search_start_idx..text.len()].rfind(query) {
                    match_start_byte = search_start_idx + start;
                    match_end_byte = match_start_byte + query.len();
                    found_match = true;
                }
            }
        }

        if found_match {
            let start_pos = Self::byte_index_to_position(content, match_start_byte);
            let end_pos = Self::byte_index_to_position(content, match_end_byte);

            // Do NOT modify the user's cursor or selection!
            // The highlight will be driven independently by `SearchFound` -> `search_selection`.
            // The scroll operation is driven directly by `EditorView` detecting the active match change.

            let message_task =
                Task::perform(async move { (start_pos, end_pos) }, |(start, end)| {
                    Message::ResponsePanel(response::Message::SearchFound(start, end))
                });

            if let Some(id) = focus_id {
                Task::batch(vec![
                    message_task,
                    operation::focus(id)
                        .map(|_: ()| Message::ResponsePanel(response::Message::DoNothing)),
                ])
            } else {
                message_task
            }
        } else {
            if let Some(id) = focus_id {
                operation::focus(id)
                    .map(|_: ()| Message::ResponsePanel(response::Message::SearchNotFound))
            } else {
                Task::perform(async {}, |_| {
                    Message::ResponsePanel(response::Message::SearchNotFound)
                })
            }
        }
    }

    fn position_to_byte_index(
        content: &text_editor::Content,
        position: text_editor::Position,
    ) -> usize {
        let mut offset = 0;
        for (i, line) in content.lines().enumerate() {
            if i == position.line {
                let line_len_chars = line.text.chars().count();
                let target_col = position.column.min(line_len_chars);

                // Find byte offset of target_col
                let byte_offset = line
                    .text
                    .chars()
                    .take(target_col)
                    .map(|c| c.len_utf8())
                    .sum::<usize>();
                return offset + byte_offset;
            }
            offset += line.text.len() + line.ending.as_str().len();
        }
        offset
    }

    fn byte_index_to_position(
        content: &text_editor::Content,
        index: usize,
    ) -> text_editor::Position {
        let mut current_offset = 0;

        for (line_index, line) in content.lines().enumerate() {
            let line_bytes = line.text.len();
            let ending_bytes = line.ending.as_str().len();
            let total_bytes = line_bytes + ending_bytes;

            if index < current_offset + total_bytes {
                let offset_in_line = index - current_offset;
                let effective_offset = offset_in_line.min(line_bytes);
                let column = line.text[..effective_offset].chars().count();

                return text_editor::Position {
                    line: line_index,
                    column,
                };
            }
            current_offset += total_bytes;
        }

        let line_count = content.line_count();
        if line_count > 0 {
            let last_line_idx = line_count - 1;
            let last_line = content.line(last_line_idx).unwrap();
            text_editor::Position {
                line: last_line_idx,
                column: last_line.text.chars().count(),
            }
        } else {
            text_editor::Position { line: 0, column: 0 }
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
                            container(
                                self.environment_panel
                                    .view(&self.environments, self.active_environment)
                                    .map(Message::EnvironmentPanel)
                            )
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
        const MAX_JSON_FORMAT_SIZE: usize = 1000 * 1024; // 1MB

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
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, .. }) => {
                    // Forward all key presses to the KeyPressed handler
                    Some(Message::KeyPressed(key.clone()))
                }
                iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(modifiers)) => {
                    Some(Message::ModifiersChanged(modifiers))
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
