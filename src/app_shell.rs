use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::Instant;

use ulid::Ulid;

use crate::error::{BeamError, Result};
use crate::models::{
    AuthConfig, BodyConfig, EnvironmentFile, EnvironmentMeta, EnvironmentScope,
    EnvironmentVariable, HeaderField, HttpMethod, LocalStateFile, QueryParamField, RequestFile,
};
use crate::paths::BeamPaths;
#[cfg(test)]
use crate::storage::RequestParentRef;
use crate::storage::{
    CreateEnvironmentInput, CreateRequestInput, DeleteRequestInput, DuplicateRequestInput,
    RenameRequestInput, WorkspaceStorage,
};
use crate::tree_store::{
    COLLECTION_MANIFEST_FILE_NAME, CollectionManifestFile, ManifestNode, Node, NodeKind,
    RootOrderFile, SharedStore, folder_dir_name, request_file_name, scope_key,
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
    pub manifest_path: Option<PathBuf>,
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
        self.expanded = ids
            .into_iter()
            .filter(|id| {
                self.nodes
                    .get(id)
                    .is_some_and(|node| node.kind != TreeNodeKind::Request)
            })
            .collect();
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

    pub fn rename_node_with_manifest_path(
        &mut self,
        id: Ulid,
        new_name: String,
        manifest_path: Option<PathBuf>,
    ) -> bool {
        let Some(node) = self.nodes.get_mut(&id) else {
            return false;
        };
        node.name = new_name;
        node.manifest_path = manifest_path;
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
        manifest_path: Option<PathBuf>,
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
                manifest_path,
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
        manifest_path: Option<PathBuf>,
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
                manifest_path,
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
        manifest_path: Option<PathBuf>,
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
            node.manifest_path = manifest_path;
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
                manifest_path,
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

    fn collect_subtree_ids(&self, root_id: Ulid, out: &mut Vec<Ulid>) {
        let Some(node) = self.nodes.get(&root_id) else {
            return;
        };
        out.push(root_id);
        for child_id in &node.children {
            self.collect_subtree_ids(*child_id, out);
        }
    }

    pub fn subtree_request_ids(&self, root_id: Ulid) -> Vec<Ulid> {
        let mut ids = Vec::new();
        self.collect_subtree_ids(root_id, &mut ids);
        ids.into_iter()
            .filter(|id| {
                self.nodes
                    .get(id)
                    .is_some_and(|node| node.kind == TreeNodeKind::Request)
            })
            .collect()
    }

    pub fn replace_subtree_path_prefix(&mut self, root_id: Ulid, old_root: &Path, new_root: &Path) {
        let mut ids = Vec::new();
        self.collect_subtree_ids(root_id, &mut ids);
        for id in ids {
            let Some(node) = self.nodes.get_mut(&id) else {
                continue;
            };
            let Some(existing_path) = node.manifest_path.as_ref() else {
                continue;
            };
            let Ok(relative) = existing_path.strip_prefix(old_root) else {
                continue;
            };
            node.manifest_path = Some(new_root.join(relative));
        }
    }

    pub fn remove_subtree(&mut self, root_id: Ulid) -> Vec<Ulid> {
        let Some(root_node) = self.nodes.get(&root_id).cloned() else {
            return Vec::new();
        };
        let mut ids = Vec::new();
        self.collect_subtree_ids(root_id, &mut ids);
        let request_ids: Vec<Ulid> = ids
            .iter()
            .copied()
            .filter(|id| {
                self.nodes
                    .get(id)
                    .is_some_and(|node| node.kind == TreeNodeKind::Request)
            })
            .collect();

        if let Some(parent_id) = root_node.parent_id {
            if let Some(parent) = self.nodes.get_mut(&parent_id) {
                parent.children.retain(|id| *id != root_id);
            }
        } else {
            self.roots.retain(|id| *id != root_id);
        }

        for id in &ids {
            self.nodes.remove(id);
            self.expanded.remove(id);
        }
        if self
            .selected_request_id
            .is_some_and(|selected_id| request_ids.contains(&selected_id))
        {
            self.selected_request_id = None;
        }
        request_ids
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
    pub shared_store: SharedStore,
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
            shared_store: SharedStore::default(),
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

    pub fn collection_ancestor_for_node(&self, mut node_id: Ulid) -> Option<Ulid> {
        loop {
            let node = self.collections.node(node_id)?;
            if node.kind == TreeNodeKind::Collection {
                return Some(node.id);
            }
            node_id = node.parent_id?;
        }
    }

    pub fn active_collection_id_for_selected_request(&self) -> Option<Ulid> {
        self.collections
            .selected_request_id()
            .and_then(|request_id| self.collection_ancestor_for_node(request_id))
    }

    pub fn effective_environment_id_for_selected_request(&self) -> Option<Ulid> {
        let collection_environment_id = self
            .active_collection_id_for_selected_request()
            .and_then(|collection_id| {
                self.environment_selection
                    .active_collection_environment_ids
                    .get(&collection_id)
                    .copied()
            })
            .filter(|environment_id| {
                self.environments
                    .iter()
                    .any(|environment| environment.environment_id == *environment_id)
            });
        if collection_environment_id.is_some() {
            return collection_environment_id;
        }

        self.environment_selection
            .active_global_environment_id
            .filter(|environment_id| {
                self.environments
                    .iter()
                    .any(|environment| environment.environment_id == *environment_id)
            })
    }

    pub fn insert_request_into_shared_store(
        &mut self,
        parent_id: Ulid,
        after_request_id: Option<Ulid>,
        request: &RequestFile,
    ) {
        self.shared_store
            .requests
            .insert(request.meta.request_id, request.clone());

        let request_id = request.meta.request_id;
        let previous_parent_id = self
            .shared_store
            .nodes
            .get(&request_id)
            .and_then(|node| node.parent_id);
        if let Some(node) = self.shared_store.nodes.get_mut(&request_id) {
            node.name = request.meta.name.clone();
            node.description = request.meta.description.clone();
            node.created_at = Some(request.meta.created_at);
            node.updated_at = Some(request.meta.updated_at);
            node.parent_id = Some(parent_id);
            node.children.clear();
        } else {
            self.shared_store.nodes.insert(
                request_id,
                Node {
                    id: request_id,
                    name: request.meta.name.clone(),
                    kind: NodeKind::Request,
                    description: request.meta.description.clone(),
                    created_at: Some(request.meta.created_at),
                    updated_at: Some(request.meta.updated_at),
                    parent_id: Some(parent_id),
                    children: Vec::new(),
                },
            );
        }

        if let Some(previous_parent_id) =
            previous_parent_id.filter(|existing_parent_id| *existing_parent_id != parent_id)
            && let Some(previous_parent) = self.shared_store.nodes.get_mut(&previous_parent_id)
        {
            previous_parent
                .children
                .retain(|child_id| *child_id != request_id);
        }

        if let Some(parent) = self.shared_store.nodes.get_mut(&parent_id) {
            parent.children.retain(|child_id| *child_id != request_id);
            if let Some(after_request_id) = after_request_id
                && let Some(index) = parent
                    .children
                    .iter()
                    .position(|child_id| *child_id == after_request_id)
            {
                parent.children.insert(index + 1, request_id);
            } else {
                parent.children.push(request_id);
            }
        }

        let _ = self.shared_store.rebuild_name_index();
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
                self.shared_store
                    .requests
                    .insert(request.meta.request_id, request.clone());
                if let Some(node) = self.shared_store.nodes.get_mut(&request.meta.request_id) {
                    node.name = request.meta.name.clone();
                }
                let _ = self.shared_store.rebuild_name_index();
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
                    request.file_path.clone(),
                );
            }
            AppEvent::RequestDeleted { request_id, .. } => {
                self.shared_store.requests.remove(request_id);
                if let Some(removed_node) = self.shared_store.nodes.remove(request_id) {
                    if let Some(parent_id) = removed_node.parent_id {
                        if let Some(parent) = self.shared_store.nodes.get_mut(&parent_id) {
                            parent.children.retain(|child_id| child_id != request_id);
                        }
                    } else {
                        self.shared_store
                            .root_ids
                            .retain(|root_id| root_id != request_id);
                    }
                    self.shared_store
                        .name_index
                        .retain(|_, indexed_id| indexed_id != request_id);
                }
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
        input: DuplicateRequestInput,
        command_id: String,
    },
    RenameRequest {
        input: RenameRequestInput,
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
        input: DeleteRequestInput,
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
        AppCommand::DuplicateRequest { input, .. } => {
            if input.duplicate_name.trim().is_empty() {
                return Err("Request name cannot be empty.".to_string());
            }
        }
        AppCommand::RenameRequest { input, .. } => {
            if input.new_name.trim().is_empty() {
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
        AppCommand::DuplicateRequest { input, command_id } => {
            let duplicated = storage
                .duplicate_request(input)
                .map_err(|error| error.to_string())?;
            Ok(vec![AppEvent::RequestUpserted {
                request: duplicated,
                command_id,
            }])
        }
        AppCommand::RenameRequest { input, command_id } => {
            let renamed = storage
                .rename_request(input)
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
        AppCommand::DeleteRequest { input, command_id } => {
            storage
                .delete_request(input.clone())
                .map_err(|error| error.to_string())?;
            Ok(vec![AppEvent::RequestDeleted {
                request_id: input.request_id,
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

#[derive(Debug, Default)]
struct LoadedSharedTree {
    shared_store: SharedStore,
    manifest_paths: HashMap<Ulid, PathBuf>,
    request_pane_data: HashMap<Ulid, RequestPaneData>,
}

pub fn startup_preload<S>(storage: &S, paths: &BeamPaths) -> StartupLoad
where
    S: WorkspaceStorage,
{
    if let Err(error) = storage.load_workspace() {
        return StartupLoad::Fatal {
            message: StartupMessage {
                severity: StartupMessageSeverity::Fatal,
                text: format!("Failed to load workspace metadata: {error}"),
            },
        };
    }

    let local_state = match storage.load_local_state() {
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

    let (collections, shared_store, request_pane_data, mut warnings) =
        load_collection_tree(paths, &local_state);
    // TOCHECK: environments can be load with collection tree in parallel
    let environments = load_environments(paths, &mut warnings);
    let mut messages: Vec<StartupMessage> = warnings
        .drain(..)
        .map(|text| StartupMessage {
            severity: StartupMessageSeverity::Warning,
            text,
        })
        .collect();

    // TOCHECK: if no last open request id found, shall default to first request?
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
            shared_store,
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

fn load_collection_tree(
    paths: &BeamPaths,
    local_state: &LocalStateFile,
) -> (
    CollectionsTreeState,
    SharedStore,
    HashMap<Ulid, RequestPaneData>,
    Vec<String>,
) {
    let mut warnings = Vec::new();
    let loaded_tree = load_shared_tree(paths, &mut warnings);
    let request_pane_data = loaded_tree.request_pane_data.clone();
    let mut tree =
        build_tree_from_shared_store(&loaded_tree.shared_store, &loaded_tree.manifest_paths);
    let shared_store = loaded_tree.shared_store;
    tree.set_expanded(local_state.tree_state.expanded_item_ids.iter().copied());

    if let Some(request_id) = local_state.local_state.last_opened_request_id {
        if tree.request_exists(request_id) {
            tree.set_selected_request(Some(request_id));
        }
    }

    (tree, shared_store, request_pane_data, warnings)
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
    let file: EnvironmentFile = parse_toml(path)?;
    Ok(file.with_file_path(path))
}

fn load_shared_tree(paths: &BeamPaths, warnings: &mut Vec<String>) -> LoadedSharedTree {
    let mut loaded = LoadedSharedTree::default();
    let root_order = load_root_order(paths, warnings);
    let mut collection_dirs = list_collection_dirs(paths);
    collection_dirs.sort_by(|left, right| left.file_name().cmp(&right.file_name()));

    for collection_dir in collection_dirs {
        let manifest_path = collection_dir.join(COLLECTION_MANIFEST_FILE_NAME);
        if !manifest_path.exists() {
            continue;
        }

        match parse_collection_tree_manifest(&manifest_path) {
            Ok(manifest) => load_collection_from_manifest(
                &mut loaded,
                &collection_dir,
                &manifest_path,
                manifest,
                warnings,
            ),
            Err(error) => warnings.push(format!(
                "Failed to load collection manifest {}: {error}",
                manifest_path.display()
            )),
        }
    }

    apply_root_order(&mut loaded.shared_store.root_ids, root_order, warnings);
    loaded
}

fn list_collection_dirs(paths: &BeamPaths) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(&paths.collections_dir) else {
        return Vec::new();
    };

    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect()
}

fn load_root_order(paths: &BeamPaths, warnings: &mut Vec<String>) -> Option<RootOrderFile> {
    if !paths.collections_root_order_file.exists() {
        return None;
    }

    match parse_root_order_file(&paths.collections_root_order_file) {
        Ok(root_order) => Some(root_order),
        Err(error) => {
            warnings.push(format!(
                "Failed to load root order file {}: {error}",
                paths.collections_root_order_file.display()
            ));
            None
        }
    }
}

fn apply_root_order(
    root_ids: &mut Vec<Ulid>,
    root_order: Option<RootOrderFile>,
    warnings: &mut Vec<String>,
) {
    let Some(root_order) = root_order else {
        return;
    };

    let available_ids: HashSet<Ulid> = root_ids.iter().copied().collect();
    let mut ordered_ids = Vec::with_capacity(root_ids.len());
    let mut seen = HashSet::new();

    for root_id in root_order.root_ids {
        if !available_ids.contains(&root_id) {
            warnings.push(format!(
                "Root order references collection {} that was not loaded. Skipped missing root entry.",
                root_id
            ));
            continue;
        }
        if seen.insert(root_id) {
            ordered_ids.push(root_id);
        }
    }

    for root_id in root_ids.iter().copied() {
        if seen.insert(root_id) {
            ordered_ids.push(root_id);
        }
    }

    *root_ids = ordered_ids;
}

fn load_collection_from_manifest(
    loaded: &mut LoadedSharedTree,
    collection_dir: &Path,
    manifest_path: &Path,
    manifest: CollectionManifestFile,
    warnings: &mut Vec<String>,
) {
    if manifest.kind != NodeKind::Collection {
        warnings.push(format!(
            "Collection manifest {} declared kind {:?} instead of collection. Skipped collection.",
            manifest_path.display(),
            manifest.kind
        ));
        return;
    }

    let collection_id = manifest.id;
    let collection_name = manifest.name;
    if !insert_loaded_node(
        loaded,
        Node {
            id: collection_id,
            name: collection_name,
            kind: NodeKind::Collection,
            description: manifest.description,
            created_at: manifest.created_at,
            updated_at: manifest.updated_at,
            parent_id: None,
            children: Vec::new(),
        },
        Some(manifest_path),
        manifest_path,
        warnings,
    ) {
        return;
    }

    loaded.shared_store.root_ids.push(collection_id);
    let child_ids = load_manifest_children(
        loaded,
        &manifest.children,
        collection_id,
        collection_dir,
        manifest_path,
        warnings,
    );
    if let Some(collection_node) = loaded.shared_store.nodes.get_mut(&collection_id) {
        collection_node.children = child_ids;
    }
}

fn load_manifest_children(
    loaded: &mut LoadedSharedTree,
    children: &[ManifestNode],
    parent_id: Ulid,
    parent_dir: &Path,
    manifest_path: &Path,
    warnings: &mut Vec<String>,
) -> Vec<Ulid> {
    let mut child_ids = Vec::new();
    for child in children {
        if let Some(child_id) = load_manifest_node(
            loaded,
            child,
            Some(parent_id),
            parent_dir,
            manifest_path,
            warnings,
        ) {
            child_ids.push(child_id);
        }
    }
    child_ids
}

fn load_manifest_node(
    loaded: &mut LoadedSharedTree,
    manifest_node: &ManifestNode,
    parent_id: Option<Ulid>,
    parent_dir: &Path,
    manifest_path: &Path,
    warnings: &mut Vec<String>,
) -> Option<Ulid> {
    match manifest_node.kind {
        NodeKind::Collection => {
            warnings.push(format!(
                "Nested collection node {} found in {}. Skipped invalid child node.",
                manifest_node.id,
                manifest_path.display()
            ));
            None
        }
        NodeKind::Folder => {
            let folder_id = manifest_node.id;
            if !insert_loaded_node(
                loaded,
                Node {
                    id: folder_id,
                    name: manifest_node.name.clone(),
                    kind: NodeKind::Folder,
                    description: manifest_node.description.clone(),
                    created_at: manifest_node.created_at,
                    updated_at: manifest_node.updated_at,
                    parent_id,
                    children: Vec::new(),
                },
                Some(manifest_path),
                manifest_path,
                warnings,
            ) {
                return None;
            }

            let folder_dir = parent_dir.join(folder_dir_name(&manifest_node.name));
            let child_ids = load_manifest_children(
                loaded,
                &manifest_node.children,
                folder_id,
                &folder_dir,
                manifest_path,
                warnings,
            );
            if let Some(folder_node) = loaded.shared_store.nodes.get_mut(&folder_id) {
                folder_node.children = child_ids;
            }
            Some(folder_id)
        }
        NodeKind::Request => {
            if !manifest_node.children.is_empty() {
                warnings.push(format!(
                    "Request node {} in {} unexpectedly contains children. Ignored request subtree.",
                    manifest_node.id,
                    manifest_path.display()
                ));
            }

            let request_path = parent_dir.join(request_file_name(&manifest_node.name));
            let (mut request_file, pane_data) = match parse_request_tree_meta(&request_path) {
                Ok(parsed) => parsed,
                Err(error) => {
                    warnings.push(format!(
                        "Failed to load request file {} for node {}: {error}",
                        request_path.display(),
                        manifest_node.id
                    ));
                    return None;
                }
            };

            if request_file.meta.request_id != manifest_node.id {
                warnings.push(format!(
                    "Request file {} declared request_id {} but manifest node expects {}. Skipped request.",
                    request_path.display(),
                    request_file.meta.request_id,
                    manifest_node.id
                ));
                return None;
            }

            if request_file.meta.name != manifest_node.name {
                warnings.push(format!(
                    "Request file {} name {:?} differed from manifest name {:?}. Using manifest name in memory.",
                    request_path.display(),
                    request_file.meta.name,
                    manifest_node.name
                ));
                request_file.meta.name = manifest_node.name.clone();
            }

            if !insert_loaded_node(
                loaded,
                Node {
                    id: manifest_node.id,
                    name: manifest_node.name.clone(),
                    kind: NodeKind::Request,
                    description: manifest_node.description.clone(),
                    created_at: manifest_node.created_at,
                    updated_at: manifest_node.updated_at,
                    parent_id,
                    children: Vec::new(),
                },
                None,
                manifest_path,
                warnings,
            ) {
                return None;
            }

            loaded
                .shared_store
                .requests
                .insert(manifest_node.id, request_file);
            loaded.request_pane_data.insert(manifest_node.id, pane_data);
            Some(manifest_node.id)
        }
    }
}

fn insert_loaded_node(
    loaded: &mut LoadedSharedTree,
    node: Node,
    manifest_path: Option<&Path>,
    source_path: &Path,
    warnings: &mut Vec<String>,
) -> bool {
    if loaded.shared_store.nodes.contains_key(&node.id) {
        warnings.push(format!(
            "Duplicate node_id {} found while loading {}. Skipped duplicate node.",
            node.id,
            source_path.display()
        ));
        return false;
    }

    let name_key = scope_key(node.parent_id, &node.name);
    if let Some(existing_id) = loaded.shared_store.name_index.get(&name_key).copied() {
        warnings.push(format!(
            "Duplicate scoped name key `{name_key}` for nodes {} and {} while loading {}. Skipped duplicate node.",
            existing_id,
            node.id,
            source_path.display()
        ));
        return false;
    }

    if let Some(path) = manifest_path {
        loaded.manifest_paths.insert(node.id, path.to_path_buf());
    }
    loaded.shared_store.name_index.insert(name_key, node.id);
    loaded.shared_store.nodes.insert(node.id, node);
    true
}

fn build_tree_from_shared_store(
    shared_store: &SharedStore,
    manifest_paths: &HashMap<Ulid, PathBuf>,
) -> CollectionsTreeState {
    let nodes = shared_store
        .nodes
        .iter()
        .map(|(node_id, node)| {
            let (request_method, request_url, manifest_path) = match node.kind {
                NodeKind::Collection | NodeKind::Folder => {
                    (None, None, manifest_paths.get(node_id).cloned())
                }
                NodeKind::Request => {
                    let request_file = shared_store.requests.get(node_id);
                    (
                        request_file.map(|request| request.request.method),
                        request_file.map(|request| request.request.url.clone()),
                        request_file.and_then(|request| request.file_path.clone()),
                    )
                }
            };
            let kind = match node.kind {
                NodeKind::Collection => TreeNodeKind::Collection,
                NodeKind::Folder => TreeNodeKind::Folder,
                NodeKind::Request => TreeNodeKind::Request,
            };
            (
                *node_id,
                TreeNode {
                    id: node.id,
                    name: node.name.clone(),
                    kind,
                    request_method,
                    request_url,
                    manifest_path,
                    parent_id: node.parent_id,
                    children: node.children.clone(),
                },
            )
        })
        .collect();

    CollectionsTreeState {
        nodes,
        roots: shared_store.root_ids.clone(),
        expanded: BTreeSet::new(),
        selected_request_id: None,
    }
}

fn parse_collection_tree_manifest(path: &Path) -> Result<CollectionManifestFile> {
    parse_toml(path)
}

fn parse_root_order_file(path: &Path) -> Result<RootOrderFile> {
    parse_toml(path)
}

fn parse_request_tree_meta(path: &Path) -> Result<(RequestFile, RequestPaneData)> {
    let request_file: RequestFile = parse_toml(path)?;
    let request_file = request_file.with_file_path(path);
    let pane_data = RequestPaneData {
        method: request_file.request.method,
        url: request_file.request.url.clone(),
        headers: request_file.request.headers.clone(),
        query_params: request_file.request.query_params.clone(),
        auth: request_file.auth.clone(),
        body: request_file.body.clone(),
        post_script: request_file.scripts.post_response.clone(),
    };
    Ok((request_file, pane_data))
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

    #[test]
    fn tree_expansion_state_ignores_missing_and_request_ids() {
        let collection_id = Ulid::new();
        let request_id = Ulid::new();
        let missing_id = Ulid::new();
        let mut tree = CollectionsTreeState::default();
        tree.nodes.insert(
            collection_id,
            TreeNode {
                id: collection_id,
                name: "Collection".to_string(),
                kind: TreeNodeKind::Collection,
                request_method: None,
                request_url: None,
                manifest_path: None,
                parent_id: None,
                children: vec![request_id],
            },
        );
        tree.nodes.insert(
            request_id,
            TreeNode {
                id: request_id,
                name: "Request".to_string(),
                kind: TreeNodeKind::Request,
                request_method: Some(HttpMethod::Get),
                request_url: Some("https://example.com".to_string()),
                manifest_path: None,
                parent_id: Some(collection_id),
                children: Vec::new(),
            },
        );

        tree.set_expanded([collection_id, request_id, missing_id]);

        assert!(tree.is_expanded(collection_id));
        assert!(!tree.is_expanded(request_id));
        assert!(!tree.is_expanded(missing_id));
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
            file_path: None,
        }
    }

    fn write_collection_manifest(
        collection_dir: &Path,
        manifest: &CollectionManifestFile,
    ) -> PathBuf {
        fs::create_dir_all(collection_dir).expect("create collection dir");
        let manifest_path = collection_dir.join(COLLECTION_MANIFEST_FILE_NAME);
        let encoded = toml::to_string_pretty(manifest).expect("encode collection manifest");
        fs::write(&manifest_path, encoded).expect("write collection manifest");
        manifest_path
    }

    fn write_request_payload(
        parent_dir: &Path,
        request_id: Ulid,
        name: &str,
        method: HttpMethod,
        url: &str,
    ) -> PathBuf {
        fs::create_dir_all(parent_dir).expect("create parent dir");
        let request_path = parent_dir.join(request_file_name(name));
        let request_file =
            sample_request_file(request_id, name, method, url).with_file_path(&request_path);
        let encoded = toml::to_string_pretty(&request_file).expect("encode request file");
        fs::write(&request_path, encoded).expect("write request file");
        request_path
    }

    fn write_root_order(paths: &BeamPaths, root_ids: Vec<Ulid>) {
        let root_order = RootOrderFile {
            schema_version: SCHEMA_VERSION_V1,
            root_ids,
        };
        let encoded = toml::to_string_pretty(&root_order).expect("encode root order");
        fs::write(&paths.collections_root_order_file, encoded).expect("write root order");
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
    fn insert_request_into_shared_store_inserts_node_and_preserves_order() {
        let collection_id = Ulid::new();
        let first_request_id = Ulid::new();
        let second_request_id = Ulid::new();
        let mut state = AppShellState::default();
        let now = Utc::now();
        state.shared_store.nodes.insert(
            collection_id,
            Node {
                id: collection_id,
                name: "Collection".to_string(),
                kind: NodeKind::Collection,
                description: None,
                created_at: Some(now),
                updated_at: Some(now),
                parent_id: None,
                children: vec![first_request_id],
            },
        );
        state.shared_store.root_ids.push(collection_id);
        state.shared_store.nodes.insert(
            first_request_id,
            Node {
                id: first_request_id,
                name: "First Request".to_string(),
                kind: NodeKind::Request,
                description: None,
                created_at: Some(now),
                updated_at: Some(now),
                parent_id: Some(collection_id),
                children: Vec::new(),
            },
        );
        let first_request = sample_request_file(
            first_request_id,
            "First Request",
            HttpMethod::Get,
            "https://one",
        );
        state
            .shared_store
            .requests
            .insert(first_request_id, first_request);

        let second_request = sample_request_file(
            second_request_id,
            "Second Request",
            HttpMethod::Post,
            "https://two",
        );
        state.insert_request_into_shared_store(
            collection_id,
            Some(first_request_id),
            &second_request,
        );

        assert_eq!(
            state
                .shared_store
                .nodes
                .get(&collection_id)
                .expect("collection exists")
                .children,
            vec![first_request_id, second_request_id]
        );
        assert_eq!(
            state
                .shared_store
                .nodes
                .get(&second_request_id)
                .expect("request node exists")
                .parent_id,
            Some(collection_id)
        );
        assert_eq!(
            state.shared_store.requests.get(&second_request_id),
            Some(&second_request)
        );
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
            input: RenameRequestInput {
                request_id: Ulid::new(),
                new_name: "   ".to_string(),
                known_request_path: None,
                known_parent_manifest_path: None,
            },
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
    fn effective_environment_prefers_collection_selection_with_global_fallback() {
        let mut state = AppShellState::default();
        let collection_id = Ulid::new();
        let folder_id = Ulid::new();
        let request_id = Ulid::new();
        let global_env_id = Ulid::new();
        let collection_env_id = Ulid::new();
        let now = Utc::now();
        state.collections.roots.push(collection_id);
        state.collections.nodes.insert(
            collection_id,
            TreeNode {
                id: collection_id,
                name: "Collection".to_string(),
                kind: TreeNodeKind::Collection,
                request_method: None,
                request_url: None,
                manifest_path: None,
                parent_id: None,
                children: vec![folder_id],
            },
        );
        state.collections.nodes.insert(
            folder_id,
            TreeNode {
                id: folder_id,
                name: "Folder".to_string(),
                kind: TreeNodeKind::Folder,
                request_method: None,
                request_url: None,
                manifest_path: None,
                parent_id: Some(collection_id),
                children: vec![request_id],
            },
        );
        state.collections.nodes.insert(
            request_id,
            TreeNode {
                id: request_id,
                name: "Request".to_string(),
                kind: TreeNodeKind::Request,
                request_method: Some(HttpMethod::Get),
                request_url: Some("https://example.com".to_string()),
                manifest_path: None,
                parent_id: Some(folder_id),
                children: Vec::new(),
            },
        );
        state.collections.set_selected_request(Some(request_id));
        state.environments = vec![
            EnvironmentMeta {
                environment_id: global_env_id,
                collection_id: None,
                scope: EnvironmentScope::Global,
                name: "Global".to_string(),
                file_name: "global.env.toml".to_string(),
                description: None,
                created_at: now,
                updated_at: now,
            },
            EnvironmentMeta {
                environment_id: collection_env_id,
                collection_id: Some(collection_id),
                scope: EnvironmentScope::Collection,
                name: "Collection".to_string(),
                file_name: "collection.env.toml".to_string(),
                description: None,
                created_at: now,
                updated_at: now,
            },
        ];
        state.environment_selection.active_global_environment_id = Some(global_env_id);

        assert_eq!(
            state.effective_environment_id_for_selected_request(),
            Some(global_env_id)
        );

        state
            .environment_selection
            .active_collection_environment_ids
            .insert(collection_id, collection_env_id);
        assert_eq!(
            state.effective_environment_id_for_selected_request(),
            Some(collection_env_id)
        );

        state
            .environments
            .retain(|env| env.environment_id != collection_env_id);
        assert_eq!(
            state.effective_environment_id_for_selected_request(),
            Some(global_env_id)
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
        write_collection_manifest(
            &collection_dir,
            &CollectionManifestFile {
                schema_version: SCHEMA_VERSION_V1,
                id: collection_id,
                name: "Phase4 Collection".to_string(),
                kind: NodeKind::Collection,
                description: None,
                created_at: None,
                updated_at: None,
                children: Vec::new(),
            },
        );

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
                    known_parent_manifest_path: None,
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
                input: DuplicateRequestInput {
                    request_id: created.meta.request_id,
                    duplicate_name: "Phase4 Request Copy".to_string(),
                    parent,
                    known_request_path: created.file_path.clone(),
                    known_parent_manifest_path: None,
                },
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
                input: RenameRequestInput {
                    request_id: saved_request.meta.request_id,
                    new_name: "Phase4 Request Saved".to_string(),
                    known_request_path: saved_request.file_path.clone(),
                    known_parent_manifest_path: None,
                },
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
                input: DeleteRequestInput {
                    request_id: saved_request.meta.request_id,
                    known_request_path: saved_request.file_path.clone(),
                    known_parent_manifest_path: None,
                },
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
                manifest_path: None,
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
                manifest_path: None,
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
                input: RenameRequestInput {
                    request_id: Ulid::new(),
                    new_name: "   ".to_string(),
                    known_request_path: None,
                    known_parent_manifest_path: None,
                },
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
                manifest_path: None,
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
                manifest_path: None,
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
        fs::create_dir_all(collection_dir.join(folder_dir_name("Nested")))
            .expect("create nested dir");
        write_collection_manifest(
            &collection_dir,
            &CollectionManifestFile {
                schema_version: SCHEMA_VERSION_V1,
                id: collection_id,
                name: "Sample".to_string(),
                kind: NodeKind::Collection,
                description: None,
                created_at: None,
                updated_at: None,
                children: vec![ManifestNode {
                    id: folder_id,
                    name: "Nested".to_string(),
                    kind: NodeKind::Folder,
                    description: None,
                    created_at: None,
                    updated_at: None,
                    children: vec![ManifestNode {
                        id: request_id,
                        name: "Get Data".to_string(),
                        kind: NodeKind::Request,
                        description: None,
                        created_at: None,
                        updated_at: None,
                        children: Vec::new(),
                    }],
                }],
            },
        );
        write_request_payload(
            &collection_dir.join(folder_dir_name("Nested")),
            request_id,
            "Get Data",
            HttpMethod::Get,
            "https://example.com/data",
        );

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
        fs::create_dir_all(&collection_dir).expect("create collection dir");

        let workspace_toml = format!(
            r#"
schema_version = 1

[workspace]
workspace_id = "{}"
name = "Beam Workspace"
created_at = "2026-05-01T03:42:36.157016+00:00"
updated_at = "2026-05-01T03:42:36.157016+00:00"
"#,
            Ulid::new()
        );
        fs::write(&paths.workspace_file, workspace_toml).expect("write workspace");

        let env_id = Ulid::new();
        let local_state_toml = format!(
            r#"
schema_version = 1

[local_state]
last_opened_request_id = "{request_id}"
active_global_environment_id = "{env_id}"
theme_name = "One Dark"
updated_at = "2026-05-01T03:42:36.157016+00:00"

[collection_environment_selection]
"{collection_id}" = "{env_id}"

[tree_state]
expanded_item_ids = ["{collection_id}"]
"#,
            env_id = env_id
        );
        fs::write(&paths.local_state_file, local_state_toml).expect("write local state");

        let collection_manifest_path = write_collection_manifest(
            &collection_dir,
            &CollectionManifestFile {
                schema_version: SCHEMA_VERSION_V1,
                id: collection_id,
                name: "Sample Collection".to_string(),
                kind: NodeKind::Collection,
                description: None,
                created_at: None,
                updated_at: None,
                children: vec![ManifestNode {
                    id: request_id,
                    name: "Manifest Name".to_string(),
                    kind: NodeKind::Request,
                    description: None,
                    created_at: None,
                    updated_at: None,
                    children: Vec::new(),
                }],
            },
        );

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
        let request_path = collection_dir.join(request_file_name("Manifest Name"));
        fs::write(&request_path, request_toml).expect("write request");

        let load = startup_preload(&storage, &paths);
        let StartupLoad::Ready { state, .. } = load else {
            panic!("startup should be ready");
        };

        assert_eq!(state.collections.selected_request_id(), Some(request_id));
        let request_node = state
            .collections
            .node(request_id)
            .expect("request node should exist");
        let collection_node = state
            .collections
            .node(collection_id)
            .expect("collection node should exist");
        assert_eq!(
            collection_node.manifest_path.as_deref(),
            Some(collection_manifest_path.as_path())
        );
        assert_eq!(request_node.name, "Manifest Name");
        assert_eq!(request_node.request_method, Some(HttpMethod::Get));
        assert_eq!(
            request_node.request_url.as_deref(),
            Some("https://httpbin.org/get")
        );
        assert_eq!(
            request_node.manifest_path.as_deref(),
            Some(request_path.as_path())
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
        assert_eq!(
            state
                .environment_selection
                .active_collection_environment_ids
                .get(&collection_id),
            Some(&env_id)
        );
        assert_eq!(
            state
                .shared_store
                .requests
                .get(&request_id)
                .map(|request| request.meta.name.as_str()),
            Some("Manifest Name")
        );
    }

    #[test]
    fn startup_hydrates_folder_node_manifest_path() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let storage = TomlWorkspaceStorage::new(paths.clone());
        storage.initialize().expect("init");

        let collection_id = Ulid::new();
        let folder_id = Ulid::new();
        let collection_dir = paths.collections_dir.join("sample");
        let collection_manifest_path = write_collection_manifest(
            &collection_dir,
            &CollectionManifestFile {
                schema_version: SCHEMA_VERSION_V1,
                id: collection_id,
                name: "Sample".to_string(),
                kind: NodeKind::Collection,
                description: None,
                created_at: None,
                updated_at: None,
                children: vec![ManifestNode {
                    id: folder_id,
                    name: "Nested".to_string(),
                    kind: NodeKind::Folder,
                    description: None,
                    created_at: None,
                    updated_at: None,
                    children: Vec::new(),
                }],
            },
        );

        let load = startup_preload(&storage, &paths);
        let StartupLoad::Ready { state, .. } = load else {
            panic!("startup should be ready");
        };

        let folder_node = state
            .collections
            .node(folder_id)
            .expect("folder node should exist");
        assert_eq!(
            folder_node.manifest_path.as_deref(),
            Some(collection_manifest_path.as_path())
        );
    }

    #[test]
    fn parse_environment_file_hydrates_runtime_path() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("global.env.toml");
        fs::write(
            &file_path,
            format!(
                r#"
schema_version = 1

[environment]
environment_id = "{}"
scope = "global"
name = "Global"
file_name = "global.env.toml"
description = ""
created_at = "2026-01-01T00:00:00Z"
updated_at = "2026-01-01T00:00:00Z"
"#,
                Ulid::new()
            ),
        )
        .expect("write environment");

        let parsed = parse_environment_file(&file_path).expect("parse environment file");
        assert_eq!(parsed.file_path.as_deref(), Some(file_path.as_path()));
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
        write_collection_manifest(
            &collection_dir,
            &CollectionManifestFile {
                schema_version: SCHEMA_VERSION_V1,
                id: collection_id,
                name: "Sample".to_string(),
                kind: NodeKind::Collection,
                description: None,
                created_at: None,
                updated_at: None,
                children: vec![ManifestNode {
                    id: folder_id,
                    name: "Nested".to_string(),
                    kind: NodeKind::Folder,
                    description: None,
                    created_at: None,
                    updated_at: None,
                    children: Vec::new(),
                }],
            },
        );

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
    fn startup_restores_expanded_ids_from_current_local_state_shape() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let storage = TomlWorkspaceStorage::new(paths.clone());
        storage.initialize().expect("init");

        let collection_id = Ulid::new();
        let collection_dir = paths.collections_dir.join("sample");
        write_collection_manifest(
            &collection_dir,
            &CollectionManifestFile {
                schema_version: SCHEMA_VERSION_V1,
                id: collection_id,
                name: "Sample".to_string(),
                kind: NodeKind::Collection,
                description: None,
                created_at: None,
                updated_at: None,
                children: Vec::new(),
            },
        );

        let local_state_toml = format!(
            r#"
schema_version = 1

[local_state]
updated_at = "2026-01-01T00:00:00Z"

[tree_state]
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

    #[test]
    fn startup_applies_root_order_to_loaded_collections() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let storage = TomlWorkspaceStorage::new(paths.clone());
        storage.initialize().expect("init");

        let first_id = Ulid::new();
        let second_id = Ulid::new();
        write_collection_manifest(
            &paths.collections_dir.join("b-second"),
            &CollectionManifestFile {
                schema_version: SCHEMA_VERSION_V1,
                id: second_id,
                name: "Second".to_string(),
                kind: NodeKind::Collection,
                description: None,
                created_at: None,
                updated_at: None,
                children: Vec::new(),
            },
        );
        write_collection_manifest(
            &paths.collections_dir.join("a-first"),
            &CollectionManifestFile {
                schema_version: SCHEMA_VERSION_V1,
                id: first_id,
                name: "First".to_string(),
                kind: NodeKind::Collection,
                description: None,
                created_at: None,
                updated_at: None,
                children: Vec::new(),
            },
        );
        write_root_order(&paths, vec![second_id, first_id]);

        let load = startup_preload(&storage, &paths);
        let StartupLoad::Ready { state, .. } = load else {
            panic!("startup should be ready");
        };

        assert_eq!(state.shared_store.root_ids, vec![second_id, first_id]);
        assert_eq!(state.collections.visible_rows()[0].id, second_id);
        assert_eq!(state.collections.visible_rows()[1].id, first_id);
    }

    #[test]
    fn startup_skips_invalid_request_toml_with_warning() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let storage = TomlWorkspaceStorage::new(paths.clone());
        storage.initialize().expect("init");

        let collection_id = Ulid::new();
        let request_id = Ulid::new();
        let collection_dir = paths.collections_dir.join("sample");
        write_collection_manifest(
            &collection_dir,
            &CollectionManifestFile {
                schema_version: SCHEMA_VERSION_V1,
                id: collection_id,
                name: "Sample".to_string(),
                kind: NodeKind::Collection,
                description: None,
                created_at: None,
                updated_at: None,
                children: vec![ManifestNode {
                    id: request_id,
                    name: "Broken Request".to_string(),
                    kind: NodeKind::Request,
                    description: None,
                    created_at: None,
                    updated_at: None,
                    children: Vec::new(),
                }],
            },
        );
        let request_path = collection_dir.join(request_file_name("Broken Request"));
        fs::write(&request_path, "not = valid = toml").expect("write invalid request file");

        let load = startup_preload(&storage, &paths);
        let StartupLoad::Ready { state, messages } = load else {
            panic!("startup should be ready");
        };

        assert!(state.shared_store.nodes.contains_key(&collection_id));
        assert!(!state.shared_store.nodes.contains_key(&request_id));
        assert!(state.shared_store.requests.is_empty());
        assert!(messages.iter().any(|message| {
            message.text.contains("Failed to load request file")
                && message.text.contains("broken-request.request.toml")
        }));
    }

    #[test]
    fn startup_skips_missing_and_mismatched_requests_with_warnings() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let storage = TomlWorkspaceStorage::new(paths.clone());
        storage.initialize().expect("init");

        let collection_id = Ulid::new();
        let good_request_id = Ulid::new();
        let missing_request_id = Ulid::new();
        let mismatched_request_id = Ulid::new();
        let collection_dir = paths.collections_dir.join("sample");
        write_collection_manifest(
            &collection_dir,
            &CollectionManifestFile {
                schema_version: SCHEMA_VERSION_V1,
                id: collection_id,
                name: "Sample".to_string(),
                kind: NodeKind::Collection,
                description: None,
                created_at: None,
                updated_at: None,
                children: vec![
                    ManifestNode {
                        id: good_request_id,
                        name: "Good Request".to_string(),
                        kind: NodeKind::Request,
                        description: None,
                        created_at: None,
                        updated_at: None,
                        children: Vec::new(),
                    },
                    ManifestNode {
                        id: missing_request_id,
                        name: "Missing Request".to_string(),
                        kind: NodeKind::Request,
                        description: None,
                        created_at: None,
                        updated_at: None,
                        children: Vec::new(),
                    },
                    ManifestNode {
                        id: mismatched_request_id,
                        name: "Wrong Id Request".to_string(),
                        kind: NodeKind::Request,
                        description: None,
                        created_at: None,
                        updated_at: None,
                        children: Vec::new(),
                    },
                ],
            },
        );
        write_request_payload(
            &collection_dir,
            good_request_id,
            "Good Request",
            HttpMethod::Get,
            "https://example.com/good",
        );
        write_request_payload(
            &collection_dir,
            Ulid::new(),
            "Wrong Id Request",
            HttpMethod::Post,
            "https://example.com/wrong",
        );

        let load = startup_preload(&storage, &paths);
        let StartupLoad::Ready { state, messages } = load else {
            panic!("startup should be ready");
        };

        assert!(state.shared_store.nodes.contains_key(&collection_id));
        assert!(state.shared_store.nodes.contains_key(&good_request_id));
        assert!(!state.shared_store.nodes.contains_key(&missing_request_id));
        assert!(
            !state
                .shared_store
                .nodes
                .contains_key(&mismatched_request_id)
        );
        assert_eq!(state.shared_store.requests.len(), 1);
        assert!(messages.iter().any(|message| {
            message.text.contains("Failed to load request file")
                && message.text.contains("missing-request.request.toml")
        }));
        assert!(messages.iter().any(|message| {
            message.text.contains("manifest node expects")
                && message.text.contains("wrong-id-request.request.toml")
        }));
    }

    #[test]
    fn startup_skips_duplicate_node_ids_and_keeps_remaining_children() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let storage = TomlWorkspaceStorage::new(paths.clone());
        storage.initialize().expect("init");

        let collection_id = Ulid::new();
        let duplicate_id = Ulid::new();
        let collection_dir = paths.collections_dir.join("sample");
        write_collection_manifest(
            &collection_dir,
            &CollectionManifestFile {
                schema_version: SCHEMA_VERSION_V1,
                id: collection_id,
                name: "Sample".to_string(),
                kind: NodeKind::Collection,
                description: None,
                created_at: None,
                updated_at: None,
                children: vec![
                    ManifestNode {
                        id: duplicate_id,
                        name: "First".to_string(),
                        kind: NodeKind::Request,
                        description: None,
                        created_at: None,
                        updated_at: None,
                        children: Vec::new(),
                    },
                    ManifestNode {
                        id: duplicate_id,
                        name: "Second".to_string(),
                        kind: NodeKind::Request,
                        description: None,
                        created_at: None,
                        updated_at: None,
                        children: Vec::new(),
                    },
                ],
            },
        );
        write_request_payload(
            &collection_dir,
            duplicate_id,
            "First",
            HttpMethod::Get,
            "https://example.com/first",
        );
        write_request_payload(
            &collection_dir,
            duplicate_id,
            "Second",
            HttpMethod::Post,
            "https://example.com/second",
        );

        let load = startup_preload(&storage, &paths);
        let StartupLoad::Ready { state, messages } = load else {
            panic!("startup should be ready");
        };

        let collection_node = state
            .shared_store
            .nodes
            .get(&collection_id)
            .expect("collection should exist");
        assert_eq!(collection_node.children, vec![duplicate_id]);
        assert_eq!(state.shared_store.requests.len(), 1);
        assert!(messages.iter().any(|message| {
            message.text.contains("Duplicate node_id")
                && message.text.contains(&duplicate_id.to_string())
        }));
    }

    #[test]
    fn startup_skips_invalid_collection_manifest_and_loads_remaining_collections() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let storage = TomlWorkspaceStorage::new(paths.clone());
        storage.initialize().expect("init");

        let valid_collection_id = Ulid::new();
        write_collection_manifest(
            &paths.collections_dir.join("valid"),
            &CollectionManifestFile {
                schema_version: SCHEMA_VERSION_V1,
                id: valid_collection_id,
                name: "Valid".to_string(),
                kind: NodeKind::Collection,
                description: None,
                created_at: None,
                updated_at: None,
                children: Vec::new(),
            },
        );
        let broken_manifest_path = paths
            .collections_dir
            .join("broken")
            .join(COLLECTION_MANIFEST_FILE_NAME);
        fs::create_dir_all(
            broken_manifest_path
                .parent()
                .expect("broken manifest parent directory"),
        )
        .expect("create broken collection dir");
        fs::write(&broken_manifest_path, "not = valid = toml").expect("write broken manifest");

        let load = startup_preload(&storage, &paths);
        let StartupLoad::Ready { state, messages } = load else {
            panic!("startup should be ready");
        };

        assert_eq!(state.shared_store.root_ids, vec![valid_collection_id]);
        assert_eq!(state.collections.visible_rows().len(), 1);
        assert_eq!(state.collections.visible_rows()[0].id, valid_collection_id);
        assert!(messages.iter().any(|message| {
            message.text.contains("Failed to load collection manifest")
                && message.text.contains("broken/.manifest.toml")
        }));
    }
}
