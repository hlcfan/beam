use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::Instant;

use ulid::Ulid;

use crate::error::{BeamError, Result};
use crate::models::{
    AuthConfig, BodyConfig, CollectionFile, CollectionItemRef, EnvironmentFile, EnvironmentMeta,
    EnvironmentScope, EnvironmentVariable, FolderFile, HeaderField, HttpMethod, ItemType,
    LocalStateFile, QueryParamField, RequestFile,
};
use crate::paths::BeamPaths;
use crate::storage::{
    CreateEnvironmentInput, CreateRequestInput, RequestParentRef, WorkspaceStorage,
};

const MIN_SPLIT_RATIO: f32 = 0.1;
const MAX_SPLIT_RATIO: f32 = 0.9;
const APP_COMMAND_QUEUE_CAPACITY: usize = 256;
const WORKER_BATCH_DRAIN_LIMIT: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneSplit {
    ratio: f32,
    min_ratio: f32,
    max_ratio: f32,
}

impl PaneSplit {
    pub fn new(ratio: f32) -> Self {
        Self {
            ratio: ratio.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO),
            min_ratio: MIN_SPLIT_RATIO,
            max_ratio: MAX_SPLIT_RATIO,
        }
    }

    pub fn ratio(&self) -> f32 {
        self.ratio
    }

    pub fn set_ratio(&mut self, ratio: f32) {
        self.ratio = ratio.clamp(self.min_ratio, self.max_ratio);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AppShellLayout {
    pub collections_workspace: PaneSplit,
    pub request_response: PaneSplit,
}

impl Default for AppShellLayout {
    fn default() -> Self {
        Self {
            collections_workspace: PaneSplit::new(0.25),
            request_response: PaneSplit::new(0.5),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalKind {
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppShortcut {
    CommandComma,
    Escape,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModalStack {
    stack: Vec<ModalKind>,
}

impl ModalStack {
    pub fn open(&mut self, modal: ModalKind) {
        if self.stack.last() != Some(&modal) {
            self.stack.push(modal);
        }
    }

    pub fn close_top(&mut self) -> Option<ModalKind> {
        self.stack.pop()
    }

    pub fn is_open(&self, modal: ModalKind) -> bool {
        self.stack.contains(&modal)
    }

    pub fn top(&self) -> Option<ModalKind> {
        self.stack.last().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeNodeKind {
    Collection,
    Folder,
    Request,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    pub id: Ulid,
    pub name: String,
    pub kind: TreeNodeKind,
    pub request_method: Option<HttpMethod>,
    pub request_url: Option<String>,
    pub parent_id: Option<Ulid>,
    pub children: Vec<Ulid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestPaneData {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<HeaderField>,
    pub query_params: Vec<QueryParamField>,
    pub auth: AuthConfig,
    pub body: BodyConfig,
    pub post_script: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeRow {
    pub id: Ulid,
    pub kind: TreeNodeKind,
    pub depth: usize,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CollectionsTreeState {
    nodes: HashMap<Ulid, TreeNode>,
    roots: Vec<Ulid>,
    expanded: BTreeSet<Ulid>,
    selected_request_id: Option<Ulid>,
}

impl CollectionsTreeState {
    pub fn selected_request_id(&self) -> Option<Ulid> {
        self.selected_request_id
    }

    pub fn set_selected_request(&mut self, request_id: Option<Ulid>) {
        self.selected_request_id = request_id;
    }

    pub fn set_expanded<I>(&mut self, ids: I)
    where
        I: IntoIterator<Item = Ulid>,
    {
        self.expanded = ids.into_iter().collect();
    }

    pub fn expanded(&self) -> &BTreeSet<Ulid> {
        &self.expanded
    }

    pub fn is_expanded(&self, id: Ulid) -> bool {
        self.expanded.contains(&id)
    }

    pub fn toggle_expanded(&mut self, id: Ulid) -> bool {
        let Some(node) = self.nodes.get(&id) else {
            return false;
        };
        if node.kind == TreeNodeKind::Request {
            return false;
        }

        if self.expanded.contains(&id) {
            self.expanded.remove(&id);
            false
        } else {
            self.expanded.insert(id);
            true
        }
    }

    pub fn select_request(&mut self, request_id: Ulid) {
        if !self.request_exists(request_id) {
            return;
        }
        self.selected_request_id = Some(request_id);
        for ancestor in self.ancestors(request_id) {
            self.expanded.insert(ancestor);
        }
    }

    pub fn node(&self, id: Ulid) -> Option<&TreeNode> {
        self.nodes.get(&id)
    }

    pub fn rename_node(&mut self, id: Ulid, new_name: String) -> bool {
        let Some(node) = self.nodes.get_mut(&id) else {
            return false;
        };
        node.name = new_name;
        true
    }

    pub fn visible_rows(&self) -> Vec<TreeRow> {
        let mut rows = Vec::new();
        for root in &self.roots {
            self.push_rows(*root, 0, &mut rows);
        }
        rows
    }

    fn push_rows(&self, id: Ulid, depth: usize, rows: &mut Vec<TreeRow>) {
        let Some(node) = self.nodes.get(&id) else {
            return;
        };
        rows.push(TreeRow {
            id: node.id,
            kind: node.kind,
            depth,
            selected: node.kind == TreeNodeKind::Request
                && Some(node.id) == self.selected_request_id,
        });

        if node.kind != TreeNodeKind::Request && self.expanded.contains(&id) {
            for child_id in &node.children {
                self.push_rows(*child_id, depth + 1, rows);
            }
        }
    }

    fn request_exists(&self, request_id: Ulid) -> bool {
        self.nodes
            .get(&request_id)
            .is_some_and(|node| node.kind == TreeNodeKind::Request)
    }

    fn ancestors(&self, id: Ulid) -> Vec<Ulid> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut cursor = self.node(id).and_then(|node| node.parent_id);
        while let Some(parent_id) = cursor {
            if !seen.insert(parent_id) {
                break;
            }
            out.push(parent_id);
            cursor = self.node(parent_id).and_then(|node| node.parent_id);
        }
        out
    }

    pub fn insert_request_child(
        &mut self,
        parent_id: Ulid,
        request_id: Ulid,
        name: String,
        method: HttpMethod,
        url: String,
    ) {
        if let Some(parent) = self.nodes.get_mut(&parent_id) {
            parent.children.push(request_id);
        }
        self.nodes.insert(
            request_id,
            TreeNode {
                id: request_id,
                name,
                kind: TreeNodeKind::Request,
                request_method: Some(method),
                request_url: Some(url),
                parent_id: Some(parent_id),
                children: Vec::new(),
            },
        );
    }

    pub fn insert_request_child_after(
        &mut self,
        parent_id: Ulid,
        after_request_id: Ulid,
        request_id: Ulid,
        name: String,
        method: HttpMethod,
        url: String,
    ) {
        if let Some(parent) = self.nodes.get_mut(&parent_id) {
            if let Some(index) = parent
                .children
                .iter()
                .position(|&id| id == after_request_id)
            {
                parent.children.insert(index + 1, request_id);
            } else {
                parent.children.push(request_id);
            }
        }
        self.nodes.insert(
            request_id,
            TreeNode {
                id: request_id,
                name,
                kind: TreeNodeKind::Request,
                request_method: Some(method),
                request_url: Some(url),
                parent_id: Some(parent_id),
                children: Vec::new(),
            },
        );
    }

    pub fn upsert_request_node(
        &mut self,
        request_id: Ulid,
        parent_id: Option<Ulid>,
        name: String,
        method: HttpMethod,
        url: String,
    ) -> bool {
        if self.nodes.contains_key(&request_id) {
            let (kind, current_parent) = {
                let node = self.nodes.get(&request_id).expect("request node exists");
                (node.kind, node.parent_id)
            };
            if kind != TreeNodeKind::Request {
                return false;
            }

            let next_parent = parent_id.filter(|next| Some(*next) != current_parent);
            if next_parent.is_some()
                && let Some(current_parent_id) = current_parent
                && let Some(existing_parent) = self.nodes.get_mut(&current_parent_id)
            {
                existing_parent.children.retain(|id| *id != request_id);
            }
            if let Some(next_parent_id) = next_parent
                && let Some(new_parent) = self.nodes.get_mut(&next_parent_id)
                && !new_parent.children.contains(&request_id)
            {
                new_parent.children.push(request_id);
            }

            let node = self
                .nodes
                .get_mut(&request_id)
                .expect("request node exists for update");
            if let Some(next_parent_id) = next_parent {
                node.parent_id = Some(next_parent_id);
            }
            node.name = name;
            node.request_method = Some(method);
            node.request_url = Some(url);
            return true;
        }

        let Some(parent_id) = parent_id else {
            return false;
        };
        let Some(parent) = self.nodes.get_mut(&parent_id) else {
            return false;
        };
        if !parent.children.contains(&request_id) {
            parent.children.push(request_id);
        }
        self.nodes.insert(
            request_id,
            TreeNode {
                id: request_id,
                name,
                kind: TreeNodeKind::Request,
                request_method: Some(method),
                request_url: Some(url),
                parent_id: Some(parent_id),
                children: Vec::new(),
            },
        );
        true
    }

    pub fn remove_request(&mut self, request_id: Ulid) -> bool {
        let Some(node) = self.nodes.get(&request_id) else {
            return false;
        };
        if node.kind != TreeNodeKind::Request {
            return false;
        }
        let parent_id = node.parent_id;
        self.nodes.remove(&request_id);
        if let Some(parent) = parent_id.and_then(|id| self.nodes.get_mut(&id)) {
            parent.children.retain(|id| *id != request_id);
        }
        if self.selected_request_id == Some(request_id) {
            self.selected_request_id = None;
        }
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspacePlaceholderState {
    pub request_panel_title: String,
    pub response_panel_title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocalEnvironmentSelectionState {
    pub active_global_environment_id: Option<Ulid>,
    pub active_collection_environment_ids: HashMap<Ulid, Ulid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocalThemeState {
    pub theme_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncLifecycleState {
    pub inflight_count: usize,
    pub last_error: Option<String>,
    pub last_operation: Option<String>,
    pub last_success_at: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupMessageSeverity {
    Warning,
    Fatal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupMessage {
    pub severity: StartupMessageSeverity,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppShellState {
    pub layout: AppShellLayout,
    pub modal_stack: ModalStack,
    pub collections: CollectionsTreeState,
    pub request_pane_data: HashMap<Ulid, RequestPaneData>,
    pub environments: Vec<EnvironmentMeta>,
    pub environment_selection: LocalEnvironmentSelectionState,
    pub theme: LocalThemeState,
    pub workspace: WorkspacePlaceholderState,
    pub sync_lifecycle: SyncLifecycleState,
}

impl Default for AppShellState {
    fn default() -> Self {
        Self {
            layout: AppShellLayout::default(),
            modal_stack: ModalStack::default(),
            collections: CollectionsTreeState::default(),
            request_pane_data: HashMap::new(),
            environments: Vec::new(),
            environment_selection: LocalEnvironmentSelectionState::default(),
            theme: LocalThemeState::default(),
            workspace: WorkspacePlaceholderState {
                request_panel_title: "Request".to_string(),
                response_panel_title: "Response".to_string(),
            },
            sync_lifecycle: SyncLifecycleState::default(),
        }
    }
}

impl AppShellState {
    pub fn handle_shortcut(&mut self, shortcut: AppShortcut) {
        match shortcut {
            AppShortcut::CommandComma => {
                if self.modal_stack.is_open(ModalKind::Settings) {
                    self.modal_stack.close_top();
                } else {
                    self.modal_stack.open(ModalKind::Settings);
                }
            }
            AppShortcut::Escape => {
                self.modal_stack.close_top();
            }
        }
    }

    pub fn apply_event(&mut self, event: &AppEvent) {
        match event {
            AppEvent::SyncStarted { operation, .. } => {
                self.sync_lifecycle.inflight_count =
                    self.sync_lifecycle.inflight_count.saturating_add(1);
                self.sync_lifecycle.last_operation = Some(operation.as_str().to_string());
            }
            AppEvent::SyncCompleted { operation, .. } => {
                self.sync_lifecycle.inflight_count =
                    self.sync_lifecycle.inflight_count.saturating_sub(1);
                self.sync_lifecycle.last_error = None;
                self.sync_lifecycle.last_operation = Some(operation.as_str().to_string());
                self.sync_lifecycle.last_success_at = Some(Instant::now());
            }
            AppEvent::SyncFailed {
                operation, error, ..
            } => {
                self.sync_lifecycle.inflight_count =
                    self.sync_lifecycle.inflight_count.saturating_sub(1);
                self.sync_lifecycle.last_operation = Some(operation.as_str().to_string());
                self.sync_lifecycle.last_error = Some(error.clone());
            }
            AppEvent::EnvironmentUpserted { environment, .. } => {
                if let Some(existing) = self
                    .environments
                    .iter_mut()
                    .find(|entry| entry.environment_id == environment.environment_id)
                {
                    *existing = environment.clone();
                } else {
                    self.environments.push(environment.clone());
                }
                sort_environments(&mut self.environments);
            }
            AppEvent::EnvironmentDeleted { environment_id, .. } => {
                self.environments
                    .retain(|environment| environment.environment_id != *environment_id);
                if self.environment_selection.active_global_environment_id == Some(*environment_id)
                {
                    self.environment_selection.active_global_environment_id = None;
                }
                self.environment_selection
                    .active_collection_environment_ids
                    .retain(|_, selected_environment_id| {
                        *selected_environment_id != *environment_id
                    });
            }
            AppEvent::RequestUpserted { request, .. } => {
                self.request_pane_data.insert(
                    request.meta.request_id,
                    RequestPaneData {
                        method: request.request.method,
                        url: request.request.url.clone(),
                        headers: request.request.headers.clone(),
                        query_params: request.request.query_params.clone(),
                        auth: request.auth.clone(),
                        body: request.body.clone(),
                        post_script: request.scripts.post_response.clone(),
                    },
                );
                let _ = self.collections.upsert_request_node(
                    request.meta.request_id,
                    None,
                    request.meta.name.clone(),
                    request.request.method,
                    request.request.url.clone(),
                );
            }
            AppEvent::RequestDeleted { request_id, .. } => {
                self.request_pane_data.remove(request_id);
                let _ = self.collections.remove_request(*request_id);
            }
        }
    }
}

fn sort_environments(environments: &mut [EnvironmentMeta]) {
    let scope_rank = |scope: EnvironmentScope| match scope {
        EnvironmentScope::Global => 0_u8,
        EnvironmentScope::Collection => 1_u8,
    };
    environments.sort_by(|a, b| {
        scope_rank(a.scope)
            .cmp(&scope_rank(b.scope))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| {
                a.environment_id
                    .to_string()
                    .cmp(&b.environment_id.to_string())
            })
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppOperation {
    CreateEnvironment,
    RenameEnvironment,
    UpdateEnvironmentVariables,
    DeleteEnvironment,
    CreateRequest,
    CreateRequestAfter,
    DuplicateRequest,
    RenameRequest,
    UpdateRequest,
    SaveRequest,
    DeleteRequest,
}

impl AppOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            AppOperation::CreateEnvironment => "create_environment",
            AppOperation::RenameEnvironment => "rename_environment",
            AppOperation::UpdateEnvironmentVariables => "update_environment_variables",
            AppOperation::DeleteEnvironment => "delete_environment",
            AppOperation::CreateRequest => "create_request",
            AppOperation::CreateRequestAfter => "create_request_after",
            AppOperation::DuplicateRequest => "duplicate_request",
            AppOperation::RenameRequest => "rename_request",
            AppOperation::UpdateRequest => "update_request",
            AppOperation::SaveRequest => "save_request",
            AppOperation::DeleteRequest => "delete_request",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppCommand {
    CreateEnvironment {
        name: String,
        scope: EnvironmentScope,
        collection_id: Option<Ulid>,
        command_id: String,
    },
    RenameEnvironment {
        environment_id: Ulid,
        new_name: String,
        command_id: String,
    },
    UpdateEnvironmentVariables {
        environment_id: Ulid,
        variables: Vec<EnvironmentVariable>,
        command_id: String,
    },
    DeleteEnvironment {
        environment_id: Ulid,
        command_id: String,
    },
    CreateRequest {
        input: CreateRequestInput,
        command_id: String,
    },
    CreateRequestAfter {
        input: CreateRequestInput,
        source_request_id: Ulid,
        command_id: String,
    },
    DuplicateRequest {
        request_id: Ulid,
        duplicate_name: String,
        parent: RequestParentRef,
        command_id: String,
    },
    RenameRequest {
        request_id: Ulid,
        new_name: String,
        command_id: String,
    },
    UpdateRequest {
        request_file: RequestFile,
        command_id: String,
    },
    SaveRequest {
        request_file: RequestFile,
        command_id: String,
    },
    DeleteRequest {
        request_id: Ulid,
        command_id: String,
    },
}

impl AppCommand {
    pub fn command_id(&self) -> &str {
        match self {
            AppCommand::CreateEnvironment { command_id, .. }
            | AppCommand::RenameEnvironment { command_id, .. }
            | AppCommand::UpdateEnvironmentVariables { command_id, .. }
            | AppCommand::DeleteEnvironment { command_id, .. }
            | AppCommand::CreateRequest { command_id, .. }
            | AppCommand::CreateRequestAfter { command_id, .. }
            | AppCommand::DuplicateRequest { command_id, .. }
            | AppCommand::RenameRequest { command_id, .. }
            | AppCommand::UpdateRequest { command_id, .. }
            | AppCommand::SaveRequest { command_id, .. }
            | AppCommand::DeleteRequest { command_id, .. } => command_id,
        }
    }

    pub fn operation(&self) -> AppOperation {
        match self {
            AppCommand::CreateEnvironment { .. } => AppOperation::CreateEnvironment,
            AppCommand::RenameEnvironment { .. } => AppOperation::RenameEnvironment,
            AppCommand::UpdateEnvironmentVariables { .. } => {
                AppOperation::UpdateEnvironmentVariables
            }
            AppCommand::DeleteEnvironment { .. } => AppOperation::DeleteEnvironment,
            AppCommand::CreateRequest { .. } => AppOperation::CreateRequest,
            AppCommand::CreateRequestAfter { .. } => AppOperation::CreateRequestAfter,
            AppCommand::DuplicateRequest { .. } => AppOperation::DuplicateRequest,
            AppCommand::RenameRequest { .. } => AppOperation::RenameRequest,
            AppCommand::UpdateRequest { .. } => AppOperation::UpdateRequest,
            AppCommand::SaveRequest { .. } => AppOperation::SaveRequest,
            AppCommand::DeleteRequest { .. } => AppOperation::DeleteRequest,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppEvent {
    SyncStarted {
        command_id: String,
        operation: AppOperation,
    },
    EnvironmentUpserted {
        environment: EnvironmentMeta,
        command_id: String,
    },
    EnvironmentDeleted {
        environment_id: Ulid,
        command_id: String,
    },
    RequestUpserted {
        request: RequestFile,
        command_id: String,
    },
    RequestDeleted {
        request_id: Ulid,
        command_id: String,
    },
    SyncFailed {
        command_id: String,
        operation: AppOperation,
        error: String,
    },
    SyncCompleted {
        command_id: String,
        operation: AppOperation,
    },
}

pub struct DataSyncRuntime {
    pub command_tx: SyncSender<AppCommand>,
    pub event_rx: Receiver<AppEvent>,
}

pub fn next_command_id() -> String {
    Ulid::new().to_string()
}

pub fn start_data_sync_worker<S>(storage: S) -> DataSyncRuntime
where
    S: WorkspaceStorage + Send + 'static,
{
    let (command_tx, command_rx) = mpsc::sync_channel::<AppCommand>(APP_COMMAND_QUEUE_CAPACITY);
    let (event_tx, event_rx) = mpsc::channel::<AppEvent>();

    thread::Builder::new()
        .name("beam-data-sync-worker".to_string())
        .spawn(move || data_sync_worker_loop(storage, command_rx, event_tx))
        .expect("failed to start data sync worker thread");

    DataSyncRuntime {
        command_tx,
        event_rx,
    }
}

fn data_sync_worker_loop<S>(
    storage: S,
    command_rx: Receiver<AppCommand>,
    event_tx: mpsc::Sender<AppEvent>,
) where
    S: WorkspaceStorage,
{
    while let Ok(first_command) = command_rx.recv() {
        let mut command_batch = vec![first_command];
        while command_batch.len() < WORKER_BATCH_DRAIN_LIMIT {
            match command_rx.try_recv() {
                Ok(command) => command_batch.push(command),
                Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
            }
        }

        for command in coalesce_commands(command_batch) {
            let command_id = command.command_id().to_string();
            let operation = command.operation();
            let _ = event_tx.send(AppEvent::SyncStarted {
                command_id: command_id.clone(),
                operation,
            });

            if let Err(error) = validate_command_payload(&command) {
                log_sync_failure(&command_id, operation, &error);
                let _ = event_tx.send(AppEvent::SyncFailed {
                    command_id,
                    operation,
                    error,
                });
                continue;
            }

            match handle_command(&storage, command) {
                Ok(domain_events) => {
                    for event in domain_events {
                        let _ = event_tx.send(event);
                    }
                    let _ = event_tx.send(AppEvent::SyncCompleted {
                        command_id,
                        operation,
                    });
                }
                Err(error) => {
                    log_sync_failure(&command_id, operation, &error);
                    let _ = event_tx.send(AppEvent::SyncFailed {
                        command_id,
                        operation,
                        error,
                    });
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum CommandCoalesceKey {
    Request(Ulid),
    EnvironmentVariables(Ulid),
}

fn command_coalesce_key(command: &AppCommand) -> Option<CommandCoalesceKey> {
    match command {
        AppCommand::UpdateRequest { request_file, .. }
        | AppCommand::SaveRequest { request_file, .. } => {
            Some(CommandCoalesceKey::Request(request_file.meta.request_id))
        }
        AppCommand::UpdateEnvironmentVariables { environment_id, .. } => {
            Some(CommandCoalesceKey::EnvironmentVariables(*environment_id))
        }
        _ => None,
    }
}

/// Coalesces a drained worker batch using latest-wins semantics for high-frequency edits.
///
/// How it works:
/// - Build a key for coalescable commands (currently request saves/updates and environment variable updates).
/// - Record the last index seen for each key in the batch.
/// - Keep command order stable while dropping earlier superseded commands for the same key.
///
/// Result:
/// - Only the most recent mutation per entity/field key is executed in this batch.
/// - Non-coalescable commands (create/delete/rename, etc.) are always preserved.
fn coalesce_commands(commands: Vec<AppCommand>) -> Vec<AppCommand> {
    let mut latest_indices: HashMap<CommandCoalesceKey, usize> = HashMap::new();
    for (index, command) in commands.iter().enumerate() {
        if let Some(key) = command_coalesce_key(command) {
            latest_indices.insert(key, index);
        }
    }

    commands
        .into_iter()
        .enumerate()
        .filter_map(|(index, command)| match command_coalesce_key(&command) {
            Some(key) if latest_indices.get(&key).copied() == Some(index) => Some(command),
            Some(_) => None,
            None => Some(command),
        })
        .collect()
}

fn validate_command_payload(command: &AppCommand) -> std::result::Result<(), String> {
    match command {
        AppCommand::CreateEnvironment { name, .. } => {
            if name.trim().is_empty() {
                return Err("Environment name cannot be empty.".to_string());
            }
        }
        AppCommand::RenameEnvironment { new_name, .. } => {
            if new_name.trim().is_empty() {
                return Err("Environment name cannot be empty.".to_string());
            }
        }
        AppCommand::UpdateEnvironmentVariables { variables, .. } => {
            let mut seen = HashSet::new();
            for variable in variables {
                let normalized_name = variable.name.trim().to_lowercase();
                if normalized_name.is_empty() {
                    return Err("Environment variable name cannot be empty.".to_string());
                }
                if !seen.insert(normalized_name) {
                    return Err("Environment variable names must be unique.".to_string());
                }
            }
        }
        AppCommand::CreateRequest { input, .. } | AppCommand::CreateRequestAfter { input, .. } => {
            if input.name.trim().is_empty() {
                return Err("Request name cannot be empty.".to_string());
            }
        }
        AppCommand::DuplicateRequest { duplicate_name, .. } => {
            if duplicate_name.trim().is_empty() {
                return Err("Request name cannot be empty.".to_string());
            }
        }
        AppCommand::RenameRequest { new_name, .. } => {
            if new_name.trim().is_empty() {
                return Err("Request name cannot be empty.".to_string());
            }
        }
        AppCommand::UpdateRequest { request_file, .. }
        | AppCommand::SaveRequest { request_file, .. } => {
            if request_file.meta.name.trim().is_empty() {
                return Err("Request name cannot be empty.".to_string());
            }
        }
        AppCommand::DeleteEnvironment { .. } | AppCommand::DeleteRequest { .. } => {}
    }
    Ok(())
}

fn log_sync_failure(command_id: &str, operation: AppOperation, error: &str) {
    eprintln!(
        "sync_failure command_id={command_id} operation={} error={}",
        operation.as_str(),
        error
    );
}

fn handle_command<S>(storage: &S, command: AppCommand) -> std::result::Result<Vec<AppEvent>, String>
where
    S: WorkspaceStorage,
{
    match command {
        AppCommand::CreateEnvironment {
            name,
            scope,
            collection_id,
            command_id,
        } => {
            let created = storage
                .create_environment(CreateEnvironmentInput {
                    name,
                    scope,
                    collection_id,
                })
                .map_err(|error| error.to_string())?;
            Ok(vec![AppEvent::EnvironmentUpserted {
                environment: created.environment,
                command_id,
            }])
        }
        AppCommand::RenameEnvironment {
            environment_id,
            new_name,
            command_id,
        } => {
            let updated = storage
                .rename_environment(environment_id, &new_name)
                .map_err(|error| error.to_string())?;
            Ok(vec![AppEvent::EnvironmentUpserted {
                environment: updated.environment,
                command_id,
            }])
        }
        AppCommand::UpdateEnvironmentVariables {
            environment_id,
            variables,
            command_id,
        } => {
            let updated = storage
                .update_environment_variables(environment_id, variables)
                .map_err(|error| error.to_string())?;
            Ok(vec![AppEvent::EnvironmentUpserted {
                environment: updated.environment,
                command_id,
            }])
        }
        AppCommand::DeleteEnvironment {
            environment_id,
            command_id,
        } => {
            storage
                .delete_environment(environment_id)
                .map_err(|error| error.to_string())?;
            Ok(vec![AppEvent::EnvironmentDeleted {
                environment_id,
                command_id,
            }])
        }
        AppCommand::CreateRequest { input, command_id } => {
            let created = storage
                .create_request(input)
                .map_err(|error| error.to_string())?;
            Ok(vec![AppEvent::RequestUpserted {
                request: created,
                command_id,
            }])
        }
        AppCommand::CreateRequestAfter {
            input,
            source_request_id,
            command_id,
        } => {
            let created = storage
                .create_request_after(input, source_request_id)
                .map_err(|error| error.to_string())?;
            Ok(vec![AppEvent::RequestUpserted {
                request: created,
                command_id,
            }])
        }
        AppCommand::DuplicateRequest {
            request_id,
            duplicate_name,
            parent,
            command_id,
        } => {
            let duplicated = storage
                .duplicate_request(request_id, &duplicate_name, parent)
                .map_err(|error| error.to_string())?;
            Ok(vec![AppEvent::RequestUpserted {
                request: duplicated,
                command_id,
            }])
        }
        AppCommand::RenameRequest {
            request_id,
            new_name,
            command_id,
        } => {
            let renamed = storage
                .rename_request(request_id, &new_name)
                .map_err(|error| error.to_string())?;
            Ok(vec![AppEvent::RequestUpserted {
                request: renamed,
                command_id,
            }])
        }
        AppCommand::UpdateRequest {
            request_file,
            command_id,
        }
        | AppCommand::SaveRequest {
            request_file,
            command_id,
        } => {
            storage
                .save_request(&request_file)
                .map_err(|error| error.to_string())?;
            Ok(vec![AppEvent::RequestUpserted {
                request: request_file,
                command_id,
            }])
        }
        AppCommand::DeleteRequest {
            request_id,
            command_id,
        } => {
            storage
                .delete_request(request_id)
                .map_err(|error| error.to_string())?;
            Ok(vec![AppEvent::RequestDeleted {
                request_id,
                command_id,
            }])
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StartupLoad {
    Ready {
        state: AppShellState,
        messages: Vec<StartupMessage>,
    },
    Fatal {
        message: StartupMessage,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CollectionManifest {
    collection_id: Ulid,
    name: String,
    items: Vec<CollectionItemRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FolderManifest {
    folder_id: Ulid,
    collection_id: Ulid,
    name: String,
    parent_folder_id: Option<Ulid>,
    items: Vec<CollectionItemRef>,
}

pub fn startup_preload<S>(_storage: &S, paths: &BeamPaths) -> StartupLoad
where
    S: WorkspaceStorage,
{
    if let Err(error) = load_workspace_file(paths) {
        return StartupLoad::Fatal {
            message: StartupMessage {
                severity: StartupMessageSeverity::Fatal,
                text: format!("Failed to load workspace metadata: {error}"),
            },
        };
    }

    let local_state = match load_local_state_file(paths) {
        Ok(state) => state,
        Err(error) => {
            return StartupLoad::Fatal {
                message: StartupMessage {
                    severity: StartupMessageSeverity::Fatal,
                    text: format!("Failed to load local state: {error}"),
                },
            };
        }
    };

    let (collections, request_pane_data, mut warnings) = load_collection_tree(paths, &local_state);
    let environments = load_environments(paths, &mut warnings);
    let mut messages: Vec<StartupMessage> = warnings
        .drain(..)
        .map(|text| StartupMessage {
            severity: StartupMessageSeverity::Warning,
            text,
        })
        .collect();

    if collections.selected_request_id().is_none()
        && local_state.local_state.last_opened_request_id.is_some()
    {
        messages.push(StartupMessage {
            severity: StartupMessageSeverity::Warning,
            text: "Last opened request no longer exists and could not be restored.".to_string(),
        });
    }

    StartupLoad::Ready {
        state: AppShellState {
            collections,
            request_pane_data,
            environments,
            environment_selection: LocalEnvironmentSelectionState {
                active_global_environment_id: local_state.local_state.active_global_environment_id,
                active_collection_environment_ids: local_state
                    .collection_environment_selection
                    .clone()
                    .into_iter()
                    .collect(),
            },
            theme: LocalThemeState {
                theme_name: local_state.local_state.theme_name.clone(),
            },
            ..AppShellState::default()
        },
        messages,
    }
}

fn load_workspace_file(paths: &BeamPaths) -> Result<()> {
    // Accept both current in-code schema and real on-disk schema.
    if parse_toml::<crate::models::WorkspaceFile>(&paths.workspace_file).is_ok() {
        return Ok(());
    }
    let workspace_file: WorkspaceTomlFile = parse_toml(&paths.workspace_file)?;
    if workspace_file.workspace.schema_version == 0 {
        return Err(BeamError::Validation {
            message: "Invalid workspace schema_version 0".to_string(),
        });
    }
    Ok(())
}

fn load_local_state_file(paths: &BeamPaths) -> Result<LocalStateFile> {
    if let Ok(mut state) = parse_toml::<LocalStateFile>(&paths.local_state_file) {
        merge_nested_local_state_fields(&paths.local_state_file, &mut state);
        return Ok(state);
    }

    let local_state_file: LocalStateTomlFile = parse_toml(&paths.local_state_file)?;
    let state = LocalStateFile {
        schema_version: local_state_file.local_state.schema_version,
        local_state: crate::models::LocalState {
            active_global_environment_id: local_state_file.local_state.active_global_environment_id,
            last_opened_request_id: local_state_file.local_state.last_opened_request_id,
            theme_name: local_state_file.local_state.theme_name,
            updated_at: local_state_file.local_state.updated_at,
        },
        collection_environment_selection: local_state_file
            .local_state
            .collection_environment_selections
            .into_iter()
            .map(|entry| (entry.collection_id, entry.environment_id))
            .collect(),
        tree_state: crate::models::TreeState {
            expanded_item_ids: local_state_file.local_state.expanded_item_ids,
        },
    };
    Ok(state)
}

fn merge_nested_local_state_fields(path: &Path, state: &mut LocalStateFile) {
    let Ok(parsed_file) = parse_toml::<LocalStateNestedFile>(path) else {
        return;
    };

    if state.tree_state.expanded_item_ids.is_empty()
        && !parsed_file.local_state.expanded_item_ids.is_empty()
    {
        state.tree_state.expanded_item_ids = parsed_file.local_state.expanded_item_ids;
    }

    if state.collection_environment_selection.is_empty()
        && !parsed_file
            .local_state
            .collection_environment_selections
            .is_empty()
    {
        state.collection_environment_selection = parsed_file
            .local_state
            .collection_environment_selections
            .into_iter()
            .map(|entry| (entry.collection_id, entry.environment_id))
            .collect();
    }
}

fn load_collection_tree(
    paths: &BeamPaths,
    local_state: &LocalStateFile,
) -> (
    CollectionsTreeState,
    HashMap<Ulid, RequestPaneData>,
    Vec<String>,
) {
    let mut warnings = Vec::new();
    let (collections, folders, requests_by_id, request_pane_data) =
        load_navigation_manifest(paths, &mut warnings);
    let mut tree = build_tree(&collections, &folders, &requests_by_id, &mut warnings);
    tree.set_expanded(local_state.tree_state.expanded_item_ids.iter().copied());

    if let Some(request_id) = local_state.local_state.last_opened_request_id {
        if tree.request_exists(request_id) {
            tree.set_selected_request(Some(request_id));
        }
    }

    (tree, request_pane_data, warnings)
}

fn load_environments(paths: &BeamPaths, warnings: &mut Vec<String>) -> Vec<EnvironmentMeta> {
    let mut environments = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![
        paths.environments_dir.clone(),
        paths.collections_dir.clone(),
    ];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".env.toml"))
            {
                continue;
            }
            let Ok(file) = parse_environment_file(path.as_path()) else {
                warnings.push(format!(
                    "Failed to load environment file {}: unsupported format.",
                    path.display()
                ));
                continue;
            };

            if !seen.insert(file.environment.environment_id) {
                warnings.push(format!(
                    "Duplicate environment_id {} found in {}. Skipped duplicate.",
                    file.environment.environment_id,
                    path.display()
                ));
                continue;
            }
            let mut environment = file.environment;
            if let Some(file_name) = path.file_name().and_then(|name| name.to_str()) {
                environment.file_name = file_name.to_string();
            }
            environments.push(environment);
        }
    }

    sort_environments(&mut environments);
    environments
}

fn parse_environment_file(path: &Path) -> Result<EnvironmentFile> {
    if let Ok(current) = parse_toml::<EnvironmentFile>(path) {
        return Ok(current);
    }

    let legacy: EnvironmentTomlFile = parse_toml(path)?;
    Ok(EnvironmentFile {
        schema_version: legacy.environment.schema_version,
        environment: crate::models::EnvironmentMeta {
            environment_id: legacy.environment.environment_id,
            collection_id: legacy.environment.collection_id,
            scope: legacy.environment.scope,
            name: legacy.environment.name,
            file_name: String::new(),
            description: legacy.environment.description,
            created_at: legacy.environment.created_at,
            updated_at: legacy.environment.updated_at,
        },
        variables: legacy.variables,
    })
}

fn load_navigation_manifest(
    paths: &BeamPaths,
    warnings: &mut Vec<String>,
) -> (
    Vec<CollectionManifest>,
    Vec<FolderManifest>,
    HashMap<Ulid, RequestTreeMetaWithName>,
    HashMap<Ulid, RequestPaneData>,
) {
    let mut collections = Vec::new();
    let mut folders = Vec::new();
    let mut requests_by_id = HashMap::new();
    let mut request_pane_data = HashMap::new();
    let mut seen_collections = HashSet::new();
    let mut seen_folders = HashSet::new();
    let mut seen_requests = HashSet::new();

    let mut stack = vec![paths.collections_dir.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }

            match file_name(path.as_path()) {
                Some("collection.toml") => match parse_collection_manifest(path.as_path()) {
                    Ok(mut file) => {
                        sort_items(&mut file.items);
                        if seen_collections.insert(file.collection.collection_id) {
                            collections.push(CollectionManifest {
                                collection_id: file.collection.collection_id,
                                name: file.collection.name,
                                items: file.items,
                            });
                        } else {
                            warnings.push(format!(
                                "Duplicate collection_id {} found in {}. Skipped duplicate.",
                                file.collection.collection_id,
                                path.display()
                            ));
                        }
                    }
                    Err(error) => warnings.push(format!(
                        "Failed to load collection file {}: {error}",
                        path.display()
                    )),
                },
                Some("folder.toml") => match parse_folder_manifest(path.as_path()) {
                    Ok(mut file) => {
                        sort_items(&mut file.items);
                        if seen_folders.insert(file.folder.folder_id) {
                            folders.push(FolderManifest {
                                folder_id: file.folder.folder_id,
                                collection_id: file.folder.collection_id,
                                name: file.folder.name,
                                parent_folder_id: file.folder.parent_folder_id,
                                items: file.items,
                            });
                        } else {
                            warnings.push(format!(
                                "Duplicate folder_id {} found in {}. Skipped duplicate.",
                                file.folder.folder_id,
                                path.display()
                            ));
                        }
                    }
                    Err(error) => warnings.push(format!(
                        "Failed to load folder file {}: {error}",
                        path.display()
                    )),
                },
                Some(file) if file.ends_with(".request.toml") => {
                    match parse_request_tree_meta(path.as_path()) {
                        Ok((request_meta, pane_data)) => {
                            if seen_requests.insert(request_meta.request_id) {
                                requests_by_id
                                    .insert(request_meta.request_id, request_meta.clone());
                                request_pane_data.insert(request_meta.request_id, pane_data);
                            } else {
                                warnings.push(format!(
                                    "Duplicate request_id {} found in {}. Skipped duplicate.",
                                    request_meta.request_id,
                                    path.display()
                                ));
                            }
                        }
                        Err(error) => warnings.push(format!(
                            "Failed to load request file {}: {error}",
                            path.display()
                        )),
                    }
                }
                Some(_) if path.extension().and_then(|ext| ext.to_str()) == Some("toml") => {
                    match parse_request_tree_meta(path.as_path()) {
                        Ok((request_meta, pane_data)) => {
                            if seen_requests.insert(request_meta.request_id) {
                                requests_by_id
                                    .insert(request_meta.request_id, request_meta.clone());
                                request_pane_data.insert(request_meta.request_id, pane_data);
                            } else {
                                warnings.push(format!(
                                    "Duplicate request_id {} found in {}. Skipped duplicate.",
                                    request_meta.request_id,
                                    path.display()
                                ));
                            }
                        }
                        Err(_) => {}
                    }
                }
                _ => {}
            }
        }
    }

    (collections, folders, requests_by_id, request_pane_data)
}

fn build_tree(
    collections: &[CollectionManifest],
    folders: &[FolderManifest],
    requests_by_id: &HashMap<Ulid, RequestTreeMetaWithName>,
    warnings: &mut Vec<String>,
) -> CollectionsTreeState {
    let mut tree = CollectionsTreeState::default();

    for collection in collections {
        tree.roots.push(collection.collection_id);
        tree.nodes.insert(
            collection.collection_id,
            TreeNode {
                id: collection.collection_id,
                name: collection.name.clone(),
                kind: TreeNodeKind::Collection,
                request_method: None,
                request_url: None,
                parent_id: None,
                children: Vec::new(),
            },
        );
    }

    for folder in folders {
        tree.nodes.insert(
            folder.folder_id,
            TreeNode {
                id: folder.folder_id,
                name: folder.name.clone(),
                kind: TreeNodeKind::Folder,
                request_method: None,
                request_url: None,
                parent_id: folder.parent_folder_id.or(Some(folder.collection_id)),
                children: Vec::new(),
            },
        );
    }

    for collection in collections {
        for item in &collection.items {
            attach_item(
                &mut tree,
                collection.collection_id,
                Some(collection.collection_id),
                item,
                requests_by_id,
                warnings,
            );
        }
    }

    for folder in folders {
        for item in &folder.items {
            attach_item(
                &mut tree,
                folder.folder_id,
                Some(folder.folder_id),
                item,
                requests_by_id,
                warnings,
            );
        }
    }

    tree
}

fn attach_item(
    tree: &mut CollectionsTreeState,
    parent_id: Ulid,
    parent_ref: Option<Ulid>,
    item: &CollectionItemRef,
    requests_by_id: &HashMap<Ulid, RequestTreeMetaWithName>,
    warnings: &mut Vec<String>,
) {
    match item.item_type {
        ItemType::Folder => {
            if let Some(node) = tree.nodes.get_mut(&item.item_id) {
                node.parent_id = parent_ref;
            } else {
                warnings.push(format!(
                    "Folder {} referenced by {} was not found on disk.",
                    item.item_id, parent_id
                ));
                return;
            }
        }
        ItemType::Request => {
            let request_meta = requests_by_id.get(&item.item_id);
            let request_name = request_meta.map_or_else(|| item.name.clone(), |m| m.name.clone());
            let request_method = request_meta.map(|m| m.method);
            let request_url = request_meta.map(|m| m.url.clone());
            tree.nodes.entry(item.item_id).or_insert_with(|| TreeNode {
                id: item.item_id,
                name: request_name,
                kind: TreeNodeKind::Request,
                request_method,
                request_url,
                parent_id: parent_ref,
                children: Vec::new(),
            });
        }
    }

    if let Some(parent) = tree.nodes.get_mut(&parent_id) {
        parent.children.push(item.item_id);
    }
}

fn parse_collection_manifest(path: &Path) -> Result<CollectionFile> {
    parse_toml(path)
}

fn parse_folder_manifest(path: &Path) -> Result<FolderFile> {
    parse_toml(path)
}

fn sort_items(items: &mut [CollectionItemRef]) {
    items.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then_with(|| a.item_id.to_string().cmp(&b.item_id.to_string()))
    });
}

fn parse_request_tree_meta(path: &Path) -> Result<(RequestTreeMetaWithName, RequestPaneData)> {
    let request_file: RequestFile = parse_toml(path)?;
    let tree_meta = RequestTreeMetaWithName {
        request_id: request_file.meta.request_id,
        name: request_file.meta.name,
        method: request_file.request.method,
        url: request_file.request.url.clone(),
    };
    let pane_data = RequestPaneData {
        method: request_file.request.method,
        url: request_file.request.url,
        headers: request_file.request.headers,
        query_params: request_file.request.query_params,
        auth: request_file.auth,
        body: request_file.body,
        post_script: request_file.scripts.post_response,
    };
    Ok((tree_meta, pane_data))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RequestTreeMetaWithName {
    request_id: Ulid,
    name: String,
    method: HttpMethod,
    url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct EnvironmentTomlFile {
    environment: EnvironmentTomlMeta,
    #[serde(default)]
    variables: Vec<crate::models::EnvironmentVariable>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct EnvironmentTomlMeta {
    schema_version: u32,
    environment_id: Ulid,
    collection_id: Option<Ulid>,
    scope: EnvironmentScope,
    name: String,
    description: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct WorkspaceTomlFile {
    workspace: WorkspaceTomlMeta,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct WorkspaceTomlMeta {
    schema_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct LocalStateTomlFile {
    local_state: LocalStateToml,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct CollectionEnvironmentSelectionToml {
    collection_id: Ulid,
    environment_id: Ulid,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct LocalStateToml {
    schema_version: u32,
    active_global_environment_id: Option<Ulid>,
    last_opened_request_id: Option<Ulid>,
    #[serde(default)]
    theme_name: Option<String>,
    #[serde(default)]
    updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    expanded_item_ids: Vec<Ulid>,
    #[serde(default, rename = "collection_environment_selections")]
    collection_environment_selections: Vec<CollectionEnvironmentSelectionToml>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, Default)]
struct LocalStateNestedFile {
    #[serde(default)]
    local_state: LocalStateNestedState,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, Default)]
struct LocalStateNestedState {
    #[serde(default)]
    expanded_item_ids: Vec<Ulid>,
    #[serde(default, rename = "collection_environment_selections")]
    collection_environment_selections: Vec<CollectionEnvironmentSelectionToml>,
}

fn file_name(path: &Path) -> Option<&str> {
    path.file_name().and_then(|name| name.to_str())
}

fn parse_toml<T: for<'de> serde::Deserialize<'de>>(path: &Path) -> Result<T> {
    let content = fs::read_to_string(path).map_err(|source| BeamError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str(&content).map_err(|source| BeamError::TomlDecode {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::mpsc::Receiver;
    use std::time::Duration;

    use tempfile::tempdir;

    use super::*;
    use crate::models::{
        LocalState, RequestDefinition, RequestMeta, ScriptConfig, TreeState, WorkspaceFile,
    };
    use crate::schema::SCHEMA_VERSION_V1;
    use crate::storage::toml_backend::TomlWorkspaceStorage;
    use chrono::Utc;

    #[test]
    fn split_ratios_are_clamped() {
        let mut split = PaneSplit::new(0.25);
        split.set_ratio(0.95);
        assert!((split.ratio() - 0.9).abs() < f32::EPSILON);

        split.set_ratio(0.02);
        assert!((split.ratio() - 0.1).abs() < f32::EPSILON);
    }

    fn sample_request_file(
        request_id: Ulid,
        name: &str,
        method: HttpMethod,
        url: &str,
    ) -> RequestFile {
        let now = Utc::now();
        RequestFile {
            meta: RequestMeta {
                request_id,
                name: name.to_string(),
                description: None,
                created_at: now,
                updated_at: now,
            },
            request: RequestDefinition {
                method,
                url: url.to_string(),
                headers: Vec::new(),
                query_params: Vec::new(),
            },
            auth: AuthConfig::None,
            body: BodyConfig::None,
            scripts: ScriptConfig::default(),
        }
    }

    fn apply_events_for_command(
        state: &mut AppShellState,
        event_rx: &Receiver<AppEvent>,
        command_id: &str,
    ) -> Vec<AppEvent> {
        let mut events = Vec::new();
        loop {
            let event = event_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("expected worker event");
            let is_terminal = matches!(
                &event,
                AppEvent::SyncCompleted {
                    command_id: event_command_id,
                    ..
                } if event_command_id == command_id
            ) || matches!(
                &event,
                AppEvent::SyncFailed {
                    command_id: event_command_id,
                    ..
                } if event_command_id == command_id
            );
            state.apply_event(&event);
            events.push(event);
            if is_terminal {
                break;
            }
        }
        events
    }

    #[test]
    fn coalescing_keeps_latest_request_edit_commands() {
        let request_id = Ulid::new();
        let save_one = AppCommand::SaveRequest {
            request_file: sample_request_file(
                request_id,
                "First Save",
                HttpMethod::Get,
                "https://example.com/first",
            ),
            command_id: Ulid::new().to_string(),
        };
        let save_two = AppCommand::SaveRequest {
            request_file: sample_request_file(
                request_id,
                "Latest Save",
                HttpMethod::Post,
                "https://example.com/latest",
            ),
            command_id: Ulid::new().to_string(),
        };
        let create_environment = AppCommand::CreateEnvironment {
            name: "Global".to_string(),
            scope: EnvironmentScope::Global,
            collection_id: None,
            command_id: Ulid::new().to_string(),
        };
        let coalesced = coalesce_commands(vec![
            save_one.clone(),
            create_environment.clone(),
            save_two.clone(),
        ]);

        assert_eq!(coalesced.len(), 2);
        assert!(matches!(
            &coalesced[0],
            AppCommand::CreateEnvironment { name, .. } if name == "Global"
        ));
        assert!(matches!(
            &coalesced[1],
            AppCommand::SaveRequest { request_file, .. }
                if request_file.meta.request_id == request_id
                    && request_file.meta.name == "Latest Save"
                    && request_file.request.method == HttpMethod::Post
        ));
    }

    #[test]
    fn command_validation_rejects_empty_rename_payload() {
        let command = AppCommand::RenameRequest {
            request_id: Ulid::new(),
            new_name: "   ".to_string(),
            command_id: Ulid::new().to_string(),
        };
        let error = validate_command_payload(&command).expect_err("expected validation error");
        assert_eq!(error, "Request name cannot be empty.");
    }

    #[test]
    fn app_shell_reducer_tracks_sync_lifecycle() {
        let mut state = AppShellState::default();
        let command_id = Ulid::new().to_string();
        let operation = AppOperation::UpdateRequest;

        state.apply_event(&AppEvent::SyncStarted {
            command_id: command_id.clone(),
            operation,
        });
        assert_eq!(state.sync_lifecycle.inflight_count, 1);
        assert_eq!(
            state.sync_lifecycle.last_operation.as_deref(),
            Some(operation.as_str())
        );
        assert_eq!(state.sync_lifecycle.last_error, None);

        state.apply_event(&AppEvent::SyncFailed {
            command_id: command_id.clone(),
            operation,
            error: "disk write failed".to_string(),
        });
        assert_eq!(state.sync_lifecycle.inflight_count, 0);
        assert_eq!(
            state.sync_lifecycle.last_operation.as_deref(),
            Some(operation.as_str())
        );
        assert_eq!(
            state.sync_lifecycle.last_error.as_deref(),
            Some("disk write failed")
        );

        state.apply_event(&AppEvent::SyncStarted {
            command_id: command_id.clone(),
            operation,
        });
        state.apply_event(&AppEvent::SyncCompleted {
            command_id,
            operation,
        });
        assert_eq!(state.sync_lifecycle.inflight_count, 0);
        assert_eq!(state.sync_lifecycle.last_error, None);
        assert!(state.sync_lifecycle.last_success_at.is_some());
    }

    #[test]
    fn app_shell_reducer_applies_environment_upsert_and_delete() {
        let mut state = AppShellState::default();
        let global_env_id = Ulid::new();
        let collection_env_id = Ulid::new();
        let collection_id = Ulid::new();
        let now = Utc::now();
        let global_environment = EnvironmentMeta {
            environment_id: global_env_id,
            collection_id: None,
            scope: EnvironmentScope::Global,
            name: "Global".to_string(),
            file_name: "global.env.toml".to_string(),
            description: None,
            created_at: now,
            updated_at: now,
        };
        let collection_environment = EnvironmentMeta {
            environment_id: collection_env_id,
            collection_id: Some(collection_id),
            scope: EnvironmentScope::Collection,
            name: "Collection".to_string(),
            file_name: "collection.env.toml".to_string(),
            description: None,
            created_at: now,
            updated_at: now,
        };

        state.apply_event(&AppEvent::EnvironmentUpserted {
            environment: collection_environment.clone(),
            command_id: Ulid::new().to_string(),
        });
        state.apply_event(&AppEvent::EnvironmentUpserted {
            environment: global_environment.clone(),
            command_id: Ulid::new().to_string(),
        });

        assert_eq!(state.environments.len(), 2);
        assert_eq!(state.environments[0].environment_id, global_env_id);
        state.environment_selection.active_global_environment_id = Some(global_env_id);
        state
            .environment_selection
            .active_collection_environment_ids
            .insert(collection_id, global_env_id);

        state.apply_event(&AppEvent::EnvironmentDeleted {
            environment_id: global_env_id,
            command_id: Ulid::new().to_string(),
        });
        assert_eq!(state.environments.len(), 1);
        assert_eq!(
            state.environment_selection.active_global_environment_id,
            None
        );
        assert!(
            state
                .environment_selection
                .active_collection_environment_ids
                .is_empty()
        );
    }

    #[test]
    fn environment_commands_propagate_to_shell_via_worker_events() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let storage = TomlWorkspaceStorage::new(paths);
        storage.initialize().expect("init storage");
        let runtime = start_data_sync_worker(storage);
        let mut state = AppShellState::default();

        let create_command_id = next_command_id();
        runtime
            .command_tx
            .send(AppCommand::CreateEnvironment {
                name: "Phase3 Global".to_string(),
                scope: EnvironmentScope::Global,
                collection_id: None,
                command_id: create_command_id.clone(),
            })
            .expect("queue create environment");
        let create_events =
            apply_events_for_command(&mut state, &runtime.event_rx, &create_command_id);
        let created_environment = create_events
            .iter()
            .find_map(|event| match event {
                AppEvent::EnvironmentUpserted {
                    environment,
                    command_id,
                } if command_id == &create_command_id => Some(environment.clone()),
                _ => None,
            })
            .expect("create should emit EnvironmentUpserted");
        assert!(
            state
                .environments
                .iter()
                .any(|entry| entry.environment_id == created_environment.environment_id)
        );

        let rename_command_id = next_command_id();
        runtime
            .command_tx
            .send(AppCommand::RenameEnvironment {
                environment_id: created_environment.environment_id,
                new_name: "Phase3 Global Renamed".to_string(),
                command_id: rename_command_id.clone(),
            })
            .expect("queue rename environment");
        apply_events_for_command(&mut state, &runtime.event_rx, &rename_command_id);
        assert_eq!(
            state
                .environments
                .iter()
                .find(|entry| entry.environment_id == created_environment.environment_id)
                .map(|entry| entry.name.as_str()),
            Some("Phase3 Global Renamed")
        );

        let update_command_id = next_command_id();
        runtime
            .command_tx
            .send(AppCommand::UpdateEnvironmentVariables {
                environment_id: created_environment.environment_id,
                variables: vec![EnvironmentVariable {
                    name: "api_base".to_string(),
                    value: "https://api.example.com".to_string(),
                    enabled: true,
                    secret: false,
                    description: None,
                }],
                command_id: update_command_id.clone(),
            })
            .expect("queue update environment variables");
        let update_events =
            apply_events_for_command(&mut state, &runtime.event_rx, &update_command_id);
        assert!(update_events.iter().any(|event| matches!(
            event,
            AppEvent::EnvironmentUpserted {
                environment,
                command_id
            } if command_id == &update_command_id
                && environment.environment_id == created_environment.environment_id
        )));

        state.environment_selection.active_global_environment_id =
            Some(created_environment.environment_id);
        let delete_command_id = next_command_id();
        runtime
            .command_tx
            .send(AppCommand::DeleteEnvironment {
                environment_id: created_environment.environment_id,
                command_id: delete_command_id.clone(),
            })
            .expect("queue delete environment");
        apply_events_for_command(&mut state, &runtime.event_rx, &delete_command_id);
        assert!(
            state
                .environments
                .iter()
                .all(|entry| entry.environment_id != created_environment.environment_id)
        );
        assert_eq!(
            state.environment_selection.active_global_environment_id,
            None
        );
    }

    #[test]
    fn request_commands_emit_expected_worker_events() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let storage = TomlWorkspaceStorage::new(paths.clone());
        storage.initialize().expect("init storage");
        let collection_id = Ulid::new();
        let collection_dir = paths.collections_dir.join("sample");
        fs::create_dir_all(&collection_dir).expect("create collection dir");
        let collection_toml = format!(
            r#"
[collection]
collection_id = "{collection_id}"
name = "Phase4 Collection"
description = ""
created_at = "2026-01-01T00:00:00Z"
updated_at = "2026-01-01T00:00:00Z"
"#
        );
        fs::write(collection_dir.join("collection.toml"), collection_toml)
            .expect("write collection");

        let runtime = start_data_sync_worker(storage);
        let mut state = AppShellState::default();
        let parent = RequestParentRef {
            collection_id,
            folder_id: None,
        };

        let create_command_id = next_command_id();
        runtime
            .command_tx
            .send(AppCommand::CreateRequest {
                input: CreateRequestInput {
                    parent,
                    name: "Phase4 Request".to_string(),
                    method: HttpMethod::Get,
                    url: "https://example.com/phase4".to_string(),
                },
                command_id: create_command_id.clone(),
            })
            .expect("queue create request");
        let create_events =
            apply_events_for_command(&mut state, &runtime.event_rx, &create_command_id);
        let created = create_events
            .iter()
            .find_map(|event| match event {
                AppEvent::RequestUpserted {
                    request,
                    command_id,
                } if command_id == &create_command_id => Some(request.clone()),
                _ => None,
            })
            .expect("create should emit RequestUpserted");

        let duplicate_command_id = next_command_id();
        runtime
            .command_tx
            .send(AppCommand::DuplicateRequest {
                request_id: created.meta.request_id,
                duplicate_name: "Phase4 Request Copy".to_string(),
                parent,
                command_id: duplicate_command_id.clone(),
            })
            .expect("queue duplicate request");
        let duplicate_events =
            apply_events_for_command(&mut state, &runtime.event_rx, &duplicate_command_id);
        let duplicated = duplicate_events
            .iter()
            .find_map(|event| match event {
                AppEvent::RequestUpserted {
                    request,
                    command_id,
                } if command_id == &duplicate_command_id => Some(request.clone()),
                _ => None,
            })
            .expect("duplicate should emit RequestUpserted");
        assert_ne!(duplicated.meta.request_id, created.meta.request_id);

        let save_command_id = next_command_id();
        let mut saved_request = duplicated.clone();
        saved_request.request.method = HttpMethod::Post;
        saved_request.request.url = "https://example.com/phase4/saved".to_string();
        runtime
            .command_tx
            .send(AppCommand::SaveRequest {
                request_file: saved_request.clone(),
                command_id: save_command_id.clone(),
            })
            .expect("queue save request");
        let save_events = apply_events_for_command(&mut state, &runtime.event_rx, &save_command_id);
        assert!(save_events.iter().any(|event| matches!(
            event,
            AppEvent::RequestUpserted {
                request,
                command_id
            } if command_id == &save_command_id
                && request.meta.request_id == saved_request.meta.request_id
                && request.request.method == HttpMethod::Post
                && request.request.url == "https://example.com/phase4/saved"
        )));

        let rename_command_id = next_command_id();
        runtime
            .command_tx
            .send(AppCommand::RenameRequest {
                request_id: saved_request.meta.request_id,
                new_name: "Phase4 Request Saved".to_string(),
                command_id: rename_command_id.clone(),
            })
            .expect("queue rename request");
        let rename_events =
            apply_events_for_command(&mut state, &runtime.event_rx, &rename_command_id);
        assert!(rename_events.iter().any(|event| matches!(
            event,
            AppEvent::RequestUpserted {
                request,
                command_id
            } if command_id == &rename_command_id
                && request.meta.request_id == saved_request.meta.request_id
                && request.meta.name == "Phase4 Request Saved"
        )));

        let delete_command_id = next_command_id();
        runtime
            .command_tx
            .send(AppCommand::DeleteRequest {
                request_id: saved_request.meta.request_id,
                command_id: delete_command_id.clone(),
            })
            .expect("queue delete request");
        let delete_events =
            apply_events_for_command(&mut state, &runtime.event_rx, &delete_command_id);
        assert!(delete_events.iter().any(|event| matches!(
            event,
            AppEvent::RequestDeleted {
                request_id,
                command_id
            } if command_id == &delete_command_id && *request_id == saved_request.meta.request_id
        )));
    }

    #[test]
    fn app_shell_reducer_applies_request_upsert_and_delete() {
        let mut state = AppShellState::default();
        let collection_id = Ulid::new();
        let request_id = Ulid::new();

        state.collections.roots.push(collection_id);
        state.collections.nodes.insert(
            collection_id,
            TreeNode {
                id: collection_id,
                name: "Collection".to_string(),
                kind: TreeNodeKind::Collection,
                request_method: None,
                request_url: None,
                parent_id: None,
                children: vec![request_id],
            },
        );
        state.collections.nodes.insert(
            request_id,
            TreeNode {
                id: request_id,
                name: "Old Name".to_string(),
                kind: TreeNodeKind::Request,
                request_method: Some(HttpMethod::Get),
                request_url: Some("https://example.com/old".to_string()),
                parent_id: Some(collection_id),
                children: Vec::new(),
            },
        );
        state.collections.set_selected_request(Some(request_id));

        let updated = sample_request_file(
            request_id,
            "New Name",
            HttpMethod::Post,
            "https://example.com/new",
        );
        state.apply_event(&AppEvent::RequestUpserted {
            request: updated,
            command_id: Ulid::new().to_string(),
        });

        let node = state.collections.node(request_id).expect("request node");
        assert_eq!(node.name, "New Name");
        assert_eq!(node.request_method, Some(HttpMethod::Post));
        assert_eq!(node.request_url.as_deref(), Some("https://example.com/new"));
        let pane = state
            .request_pane_data
            .get(&request_id)
            .expect("request pane data");
        assert_eq!(pane.method, HttpMethod::Post);
        assert_eq!(pane.url, "https://example.com/new");

        state.apply_event(&AppEvent::RequestDeleted {
            request_id,
            command_id: Ulid::new().to_string(),
        });
        assert!(state.collections.node(request_id).is_none());
        assert!(state.request_pane_data.get(&request_id).is_none());
        assert_eq!(state.collections.selected_request_id(), None);
    }

    #[test]
    fn worker_emits_sync_failed_for_invalid_payload_without_completion() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let storage = TomlWorkspaceStorage::new(paths);
        storage.initialize().expect("init storage");
        let runtime = start_data_sync_worker(storage);
        let mut state = AppShellState::default();

        let command_id = next_command_id();
        runtime
            .command_tx
            .send(AppCommand::RenameRequest {
                request_id: Ulid::new(),
                new_name: "   ".to_string(),
                command_id: command_id.clone(),
            })
            .expect("queue invalid rename request");

        let events = apply_events_for_command(&mut state, &runtime.event_rx, &command_id);
        assert!(matches!(
            &events[0],
            AppEvent::SyncStarted {
                command_id: event_command_id,
                operation: AppOperation::RenameRequest,
            } if event_command_id == &command_id
        ));
        assert!(events.iter().any(|event| matches!(
            event,
            AppEvent::SyncFailed {
                command_id: event_command_id,
                operation: AppOperation::RenameRequest,
                error
            } if event_command_id == &command_id && error == "Request name cannot be empty."
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            AppEvent::SyncCompleted {
                command_id: event_command_id,
                ..
            } if event_command_id == &command_id
        )));
    }

    #[test]
    fn sync_failed_does_not_rollback_existing_request_state() {
        let mut state = AppShellState::default();
        let collection_id = Ulid::new();
        let request_id = Ulid::new();
        state.collections.roots.push(collection_id);
        state.collections.nodes.insert(
            collection_id,
            TreeNode {
                id: collection_id,
                name: "Collection".to_string(),
                kind: TreeNodeKind::Collection,
                request_method: None,
                request_url: None,
                parent_id: None,
                children: vec![request_id],
            },
        );
        state.collections.nodes.insert(
            request_id,
            TreeNode {
                id: request_id,
                name: "Before Optimistic".to_string(),
                kind: TreeNodeKind::Request,
                request_method: Some(HttpMethod::Get),
                request_url: Some("https://example.com/before".to_string()),
                parent_id: Some(collection_id),
                children: Vec::new(),
            },
        );

        let command_id = next_command_id();
        state.apply_event(&AppEvent::RequestUpserted {
            request: sample_request_file(
                request_id,
                "Optimistic Name",
                HttpMethod::Post,
                "https://example.com/optimistic",
            ),
            command_id: command_id.clone(),
        });
        state.apply_event(&AppEvent::SyncFailed {
            command_id,
            operation: AppOperation::RenameRequest,
            error: "disk write failed".to_string(),
        });

        let node = state
            .collections
            .node(request_id)
            .expect("request should remain");
        assert_eq!(node.name, "Optimistic Name");
        assert_eq!(node.request_method, Some(HttpMethod::Post));
        assert_eq!(
            node.request_url.as_deref(),
            Some("https://example.com/optimistic")
        );
        assert!(state.request_pane_data.contains_key(&request_id));
    }

    #[test]
    fn settings_modal_shortcuts_toggle_and_close() {
        let mut shell = AppShellState::default();
        assert!(shell.modal_stack.is_empty());

        shell.handle_shortcut(AppShortcut::CommandComma);
        assert_eq!(shell.modal_stack.top(), Some(ModalKind::Settings));

        shell.handle_shortcut(AppShortcut::Escape);
        assert!(shell.modal_stack.is_empty());
    }

    #[test]
    fn startup_restores_last_request_without_overriding_tree_expansion_state() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let storage = TomlWorkspaceStorage::new(paths.clone());
        storage.initialize().expect("init");

        let collection_id = Ulid::new();
        let folder_id = Ulid::new();
        let request_id = Ulid::new();
        let collection_dir = paths.collections_dir.join("sample");
        fs::create_dir_all(&collection_dir).expect("create collection dir");
        fs::create_dir_all(collection_dir.join("nested")).expect("create nested dir");

        let collection_toml = format!(
            r#"
[collection]
collection_id = "{collection_id}"
name = "Sample"
description = ""
created_at = "2026-01-01T00:00:00Z"
updated_at = "2026-01-01T00:00:00Z"

[[items]]
item_id = "{folder_id}"
item_type = "folder"
name = "Nested"
order = 0
"#
        );
        fs::write(collection_dir.join("collection.toml"), collection_toml)
            .expect("write collection");

        let folder_toml = format!(
            r#"
[folder]
folder_id = "{folder_id}"
collection_id = "{collection_id}"
name = "Nested"
description = ""
created_at = "2026-01-01T00:00:00Z"
updated_at = "2026-01-01T00:00:00Z"

[[items]]
item_id = "{request_id}"
item_type = "request"
name = "Get Data"
order = 0
"#
        );
        fs::write(collection_dir.join("nested/folder.toml"), folder_toml).expect("write folder");

        let local_state = LocalStateFile {
            schema_version: SCHEMA_VERSION_V1,
            local_state: LocalState {
                active_global_environment_id: None,
                last_opened_request_id: Some(request_id),
                theme_name: None,
                updated_at: Utc::now(),
            },
            collection_environment_selection: Default::default(),
            tree_state: TreeState::default(),
        };
        storage
            .save_local_state(&local_state)
            .expect("save local state");

        let load = startup_preload(&storage, &paths);
        let StartupLoad::Ready { state, .. } = load else {
            panic!("startup should be ready");
        };

        assert_eq!(state.collections.selected_request_id(), Some(request_id));
        assert!(!state.collections.expanded().contains(&folder_id));
        assert!(!state.collections.expanded().contains(&collection_id));
    }

    #[test]
    fn startup_emits_warning_for_missing_last_request() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let storage = TomlWorkspaceStorage::new(paths.clone());
        storage.initialize().expect("init");

        storage
            .save_workspace(&WorkspaceFile::default())
            .expect("save workspace");

        let local_state = LocalStateFile {
            schema_version: SCHEMA_VERSION_V1,
            local_state: LocalState {
                active_global_environment_id: None,
                last_opened_request_id: Some(Ulid::new()),
                theme_name: None,
                updated_at: Utc::now(),
            },
            collection_environment_selection: Default::default(),
            tree_state: TreeState::default(),
        };
        storage
            .save_local_state(&local_state)
            .expect("save local state");

        let load = startup_preload(&storage, &paths);
        let StartupLoad::Ready { state, messages } = load else {
            panic!("startup should be ready");
        };
        assert_eq!(state.collections.selected_request_id(), None);
        assert!(
            messages
                .iter()
                .any(|message| message.text.contains("Last opened request"))
        );
    }

    #[test]
    fn startup_returns_fatal_for_corrupt_local_state() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let storage = TomlWorkspaceStorage::new(paths.clone());
        storage.initialize().expect("init");

        fs::write(paths.local_state_file.clone(), "not = valid = toml")
            .expect("corrupt local state");

        let load = startup_preload(&storage, &paths);
        let StartupLoad::Fatal { message } = load else {
            panic!("startup should fail");
        };
        assert_eq!(message.severity, StartupMessageSeverity::Fatal);
        assert!(message.text.contains("Failed to load local state"));
    }

    #[test]
    fn startup_loads_workspace_local_state_and_request_metadata() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let storage = TomlWorkspaceStorage::new(paths.clone());
        storage.initialize().expect("init");

        let collection_id = Ulid::new();
        let request_id = Ulid::new();
        let collection_dir = paths.collections_dir.join("sample");
        fs::create_dir_all(collection_dir.join("requests")).expect("create requests dir");

        let workspace_toml = format!(
            r#"
[workspace]
workspace_id = "{}"
name = "Beam Workspace"
schema_version = 1
created_at = "2026-05-01T03:42:36.157016+00:00"
updated_at = "2026-05-01T03:42:36.157016+00:00"
"#,
            Ulid::new()
        );
        fs::write(&paths.workspace_file, workspace_toml).expect("write workspace");

        let local_state_toml = format!(
            r#"
[local_state]
schema_version = 2
last_opened_request_id = "{request_id}"
active_global_environment_id = "{env_id}"
theme_name = "One Dark"
expanded_item_ids = ["{collection_id}"]

[[local_state.collection_environment_selections]]
collection_id = "{collection_id}"
environment_id = "{env_id}"
"#,
            env_id = Ulid::new()
        );
        fs::write(&paths.local_state_file, local_state_toml).expect("write local state");

        let collection_toml = format!(
            r#"
[collection]
collection_id = "{collection_id}"
name = "Sample Collection"
created_at = "2026-05-05T12:21:17.791360+00:00"
updated_at = "2026-05-05T12:21:17.791367+00:00"

[[items]]
item_id = "{request_id}"
item_type = "request"
name = "Manifest Name"
order = 10
"#
        );
        fs::write(collection_dir.join("collection.toml"), collection_toml)
            .expect("write collection");

        let request_toml = format!(
            r#"
[meta]
request_id = "{request_id}"
name = "Request From File"
created_at = "2026-05-01T03:42:36.158281+00:00"
updated_at = "2026-05-01T03:42:36.158281+00:00"

[request]
method = "GET"
url = "https://httpbin.org/get"

[[request.query_params]]
name = "page"
value = "1"
enabled = true
description = ""

[[request.headers]]
name = "Accept"
value = "application/json"
enabled = true
description = ""
secret = false

[auth]
type = "bearer"
token = "abc123"

[body]
mode = "json"
text = "{{\"name\":\"beam\"}}"

[scripts]
post_response = "console.log(response.status)"
"#
        );
        fs::write(
            collection_dir.join(format!("requests/sample-request-{request_id}.toml")),
            request_toml,
        )
        .expect("write request");

        let load = startup_preload(&storage, &paths);
        let StartupLoad::Ready { state, .. } = load else {
            panic!("startup should be ready");
        };

        assert_eq!(state.collections.selected_request_id(), Some(request_id));
        let request_node = state
            .collections
            .node(request_id)
            .expect("request node should exist");
        assert_eq!(request_node.name, "Request From File");
        assert_eq!(request_node.request_method, Some(HttpMethod::Get));
        assert_eq!(
            request_node.request_url.as_deref(),
            Some("https://httpbin.org/get")
        );
        let pane_data = state
            .request_pane_data
            .get(&request_id)
            .expect("pane data should exist");
        assert_eq!(pane_data.method, HttpMethod::Get);
        assert_eq!(pane_data.query_params.len(), 1);
        assert_eq!(pane_data.query_params[0].name, "page");
        assert_eq!(pane_data.headers.len(), 1);
        assert_eq!(pane_data.headers[0].name, "Accept");
        assert!(matches!(pane_data.auth, AuthConfig::Bearer { .. }));
        assert!(matches!(pane_data.body, BodyConfig::Json { .. }));
        assert_eq!(
            pane_data.post_script.as_deref(),
            Some("console.log(response.status)")
        );
        assert_eq!(state.theme.theme_name.as_deref(), Some("One Dark"));
    }

    #[test]
    fn startup_uses_tree_state_expanded_ids() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let storage = TomlWorkspaceStorage::new(paths.clone());
        storage.initialize().expect("init");

        let collection_id = Ulid::new();
        let folder_id = Ulid::new();
        let collection_dir = paths.collections_dir.join("sample");
        fs::create_dir_all(collection_dir.join("nested")).expect("create nested dir");

        let collection_toml = format!(
            r#"
[collection]
collection_id = "{collection_id}"
name = "Sample"
description = ""
created_at = "2026-01-01T00:00:00Z"
updated_at = "2026-01-01T00:00:00Z"

[[items]]
item_id = "{folder_id}"
item_type = "folder"
name = "Nested"
order = 0
"#
        );
        fs::write(collection_dir.join("collection.toml"), collection_toml)
            .expect("write collection");

        let folder_toml = format!(
            r#"
[folder]
folder_id = "{folder_id}"
collection_id = "{collection_id}"
name = "Nested"
description = ""
created_at = "2026-01-01T00:00:00Z"
updated_at = "2026-01-01T00:00:00Z"
"#
        );
        fs::write(collection_dir.join("nested/folder.toml"), folder_toml).expect("write folder");

        let local_state = LocalStateFile {
            schema_version: SCHEMA_VERSION_V1,
            local_state: LocalState {
                active_global_environment_id: None,
                last_opened_request_id: None,
                theme_name: None,
                updated_at: Utc::now(),
            },
            collection_environment_selection: Default::default(),
            tree_state: TreeState {
                expanded_item_ids: vec![collection_id, folder_id],
            },
        };
        storage
            .save_local_state(&local_state)
            .expect("save local state");

        let load = startup_preload(&storage, &paths);
        let StartupLoad::Ready { state, .. } = load else {
            panic!("startup should be ready");
        };

        assert!(state.collections.expanded().contains(&collection_id));
        assert!(state.collections.expanded().contains(&folder_id));
    }

    #[test]
    fn startup_restores_nested_expanded_ids_when_current_shape_parses() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let storage = TomlWorkspaceStorage::new(paths.clone());
        storage.initialize().expect("init");

        let collection_id = Ulid::new();
        let collection_dir = paths.collections_dir.join("sample");
        fs::create_dir_all(&collection_dir).expect("create collection dir");

        let collection_toml = format!(
            r#"
[collection]
collection_id = "{collection_id}"
name = "Sample"
description = ""
created_at = "2026-01-01T00:00:00Z"
updated_at = "2026-01-01T00:00:00Z"
"#
        );
        fs::write(collection_dir.join("collection.toml"), collection_toml)
            .expect("write collection");

        let local_state_toml = format!(
            r#"
schema_version = 1

[local_state]
updated_at = "2026-01-01T00:00:00Z"
expanded_item_ids = ["{collection_id}"]
"#
        );
        fs::write(&paths.local_state_file, local_state_toml).expect("write local state");

        let load = startup_preload(&storage, &paths);
        let StartupLoad::Ready { state, .. } = load else {
            panic!("startup should be ready");
        };

        assert!(state.collections.expanded().contains(&collection_id));
    }
}
