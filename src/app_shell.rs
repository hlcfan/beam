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
    AppFontSize, AuthConfig, BodyConfig, EnvironmentFile, EnvironmentMeta, EnvironmentVariable,
    FolderFile, HeaderField, HttpMethod, ItemType, LocalStateFile, QueryParamField, RequestFile,
    WorkspaceEntry, WorkspaceFile, WorkspacesRegistryFile,
};
use crate::paths::{BeamPaths, FOLDER_MANIFEST_FILE_NAME};
#[cfg(test)]
use crate::storage::RequestParentRef;
use crate::storage::fs_backend::FileSystemStorage;
use crate::storage::io_backend::StorageIoBackend;
use crate::storage::registry_repo::RegistryRepository;
use crate::storage::workspace_repo::WorkspaceRepository;
use crate::storage::{
    CreateEnvironmentInput, CreateFolderInput, CreateRequestInput, DeleteRequestInput,
    DuplicateRequestInput, MoveRequestInput, RenameRequestInput, WorkspaceStorage,
};
use crate::workspace_tree::{
    Node, NodeKind, SharedStore, folder_dir_name, request_file_name, scope_key,
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
pub struct WorkspaceTreeState {
    nodes: HashMap<Ulid, TreeNode>,
    roots: Vec<Ulid>,
    expanded: BTreeSet<Ulid>,
    selected_request_id: Option<Ulid>,
}

impl WorkspaceTreeState {
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

    pub fn roots(&self) -> &[Ulid] {
        &self.roots
    }

    pub fn move_node_to_root(&mut self, node_id: Ulid, insertion_index: usize) -> bool {
        let Some(node) = self.nodes.get(&node_id) else {
            return false;
        };
        let old_parent_id = node.parent_id;
        if let Some(old_parent_id) = old_parent_id {
            if let Some(old_parent) = self.nodes.get_mut(&old_parent_id) {
                old_parent.children.retain(|id| *id != node_id);
            }
        } else {
            self.roots.retain(|id| *id != node_id);
        }
        self.roots.retain(|id| *id != node_id);
        let index = insertion_index.min(self.roots.len());
        self.roots.insert(index, node_id);
        if let Some(node) = self.nodes.get_mut(&node_id) {
            node.parent_id = None;
        }
        true
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

    pub fn move_request_node(
        &mut self,
        request_id: Ulid,
        new_parent_id: Ulid,
        insertion_index: usize,
    ) -> bool {
        let Some(node) = self.nodes.get(&request_id) else {
            return false;
        };
        if node.kind != TreeNodeKind::Request {
            return false;
        }
        let old_parent_id = node.parent_id;

        if let Some(old_parent_id) = old_parent_id {
            if let Some(old_parent) = self.nodes.get_mut(&old_parent_id) {
                old_parent.children.retain(|id| *id != request_id);
            }
        } else {
            self.roots.retain(|id| *id != request_id);
        }

        if let Some(new_parent) = self.nodes.get_mut(&new_parent_id) {
            let index = insertion_index.min(new_parent.children.len());
            new_parent.children.insert(index, request_id);
        }

        if let Some(node) = self.nodes.get_mut(&request_id) {
            node.parent_id = Some(new_parent_id);
        }

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceState {
    pub workspace_id: Option<Ulid>,
    pub workspace_name: String,
    pub all_workspaces: Vec<WorkspaceEntry>,
    pub request_panel_title: String,
    pub response_panel_title: String,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            workspace_id: None,
            workspace_name: String::new(),
            all_workspaces: Vec::new(),
            request_panel_title: "Request".to_string(),
            response_panel_title: "Response".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocalEnvironmentSelectionState {
    pub active_global_environment_id: Option<Ulid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocalThemeState {
    pub theme_name: Option<String>,
    pub font_size: AppFontSize,
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
    pub workspace_tree: WorkspaceTreeState,
    pub shared_store: SharedStore,
    pub request_pane_data: HashMap<Ulid, RequestPaneData>,
    pub environments: Vec<EnvironmentMeta>,
    pub environment_selection: LocalEnvironmentSelectionState,
    pub theme: LocalThemeState,
    pub workspace: WorkspaceState,
    pub sync_lifecycle: SyncLifecycleState,
}

impl Default for AppShellState {
    fn default() -> Self {
        Self {
            layout: AppShellLayout::default(),
            modal_stack: ModalStack::default(),
            workspace_tree: WorkspaceTreeState::default(),
            shared_store: SharedStore::default(),
            request_pane_data: HashMap::new(),
            environments: Vec::new(),
            environment_selection: LocalEnvironmentSelectionState::default(),
            theme: LocalThemeState::default(),
            workspace: WorkspaceState::default(),
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

    pub fn effective_environment_id_for_selected_request(&self) -> Option<Ulid> {
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

    pub fn move_request_in_shared_store(
        &mut self,
        request_id: Ulid,
        new_parent_id: Ulid,
        insertion_index: usize,
    ) {
        let previous_parent_id = self
            .shared_store
            .nodes
            .get(&request_id)
            .and_then(|node| node.parent_id);

        if let Some(previous_parent_id) = previous_parent_id
            && let Some(previous_parent) = self.shared_store.nodes.get_mut(&previous_parent_id)
        {
            previous_parent
                .children
                .retain(|child_id| *child_id != request_id);
        }

        if let Some(new_parent) = self.shared_store.nodes.get_mut(&new_parent_id) {
            new_parent
                .children
                .retain(|child_id| *child_id != request_id);
            let index = insertion_index.min(new_parent.children.len());
            new_parent.children.insert(index, request_id);
        }

        if let Some(node) = self.shared_store.nodes.get_mut(&request_id) {
            node.parent_id = Some(new_parent_id);
        }

        let _ = self.shared_store.rebuild_name_index();
    }

    pub fn insert_request_at_root(
        &mut self,
        after_request_id: Option<Ulid>,
        request: &RequestFile,
    ) {
        let request_id = request.meta.request_id;
        self.shared_store
            .requests
            .insert(request_id, request.clone());
        self.shared_store.nodes.insert(
            request_id,
            Node {
                id: request_id,
                name: request.meta.name.clone(),
                kind: NodeKind::Request,
                description: request.meta.description.clone(),
                created_at: Some(request.meta.created_at),
                updated_at: Some(request.meta.updated_at),
                parent_id: None,
                children: Vec::new(),
            },
        );
        self.shared_store.root_ids.retain(|id| *id != request_id);
        if let Some(after_id) = after_request_id
            && let Some(index) = self
                .shared_store
                .root_ids
                .iter()
                .position(|&id| id == after_id)
        {
            self.shared_store.root_ids.insert(index + 1, request_id);
        } else {
            self.shared_store.root_ids.push(request_id);
        }
        let _ = self.shared_store.rebuild_name_index();

        self.workspace_tree.nodes.insert(
            request_id,
            TreeNode {
                id: request_id,
                name: request.meta.name.clone(),
                kind: TreeNodeKind::Request,
                request_method: Some(request.request.method),
                request_url: Some(request.request.url.clone()),
                manifest_path: request.file_path.clone(),
                parent_id: None,
                children: Vec::new(),
            },
        );
        self.workspace_tree.roots.retain(|id| *id != request_id);
        if let Some(after_id) = after_request_id
            && let Some(index) = self
                .workspace_tree
                .roots
                .iter()
                .position(|&id| id == after_id)
        {
            self.workspace_tree.roots.insert(index + 1, request_id);
        } else {
            self.workspace_tree.roots.push(request_id);
        }
    }

    pub fn apply_folder_move(
        &mut self,
        folder_id: Ulid,
        old_parent_id: Option<Ulid>,
        new_parent_id: Option<Ulid>,
        insertion_index: usize,
        folder_name: String,
    ) {
        // shared_store: remove from old parent
        match old_parent_id {
            Some(old_pid) => {
                if let Some(old_parent) = self.shared_store.nodes.get_mut(&old_pid) {
                    old_parent.children.retain(|id| *id != folder_id);
                }
            }
            None => self.shared_store.root_ids.retain(|id| *id != folder_id),
        }
        // shared_store: insert into new parent
        match new_parent_id {
            Some(new_pid) => {
                if let Some(new_parent) = self.shared_store.nodes.get_mut(&new_pid) {
                    let index = insertion_index.min(new_parent.children.len());
                    new_parent.children.insert(index, folder_id);
                }
            }
            None => {
                let index = insertion_index.min(self.shared_store.root_ids.len());
                self.shared_store.root_ids.insert(index, folder_id);
            }
        }
        if let Some(node) = self.shared_store.nodes.get_mut(&folder_id) {
            node.parent_id = new_parent_id;
        }
        // Only the moved folder's scope key changes; descendants retain folder_id as their scope.
        self.shared_store
            .name_index
            .remove(&scope_key(old_parent_id, &folder_name));
        self.shared_store
            .name_index
            .insert(scope_key(new_parent_id, &folder_name), folder_id);

        // workspace_tree: remove from old parent
        match old_parent_id {
            Some(old_pid) => {
                if let Some(old_parent) = self.workspace_tree.nodes.get_mut(&old_pid) {
                    old_parent.children.retain(|id| *id != folder_id);
                }
            }
            None => self.workspace_tree.roots.retain(|id| *id != folder_id),
        }
        // workspace_tree: insert into new parent
        match new_parent_id {
            Some(new_pid) => {
                if let Some(new_parent) = self.workspace_tree.nodes.get_mut(&new_pid) {
                    let index = insertion_index.min(new_parent.children.len());
                    new_parent.children.insert(index, folder_id);
                }
            }
            None => {
                let index = insertion_index.min(self.workspace_tree.roots.len());
                self.workspace_tree.roots.insert(index, folder_id);
            }
        }
        if let Some(node) = self.workspace_tree.nodes.get_mut(&folder_id) {
            node.parent_id = new_parent_id;
        }
    }

    pub fn replace_moved_folder_subtree_paths(
        &mut self,
        folder_id: Ulid,
        old_root: &Path,
        new_root: &Path,
    ) {
        self.workspace_tree
            .replace_subtree_path_prefix(folder_id, old_root, new_root);
        for request_id in self.workspace_tree.subtree_request_ids(folder_id) {
            let Some(request_file) = self.shared_store.requests.get_mut(&request_id) else {
                continue;
            };
            let Some(existing_path) = request_file.file_path.as_ref() else {
                continue;
            };
            let Ok(relative) = existing_path.strip_prefix(old_root) else {
                continue;
            };
            request_file.file_path = Some(new_root.join(relative));
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
                let _ = self.workspace_tree.upsert_request_node(
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
                let _ = self.workspace_tree.remove_request(*request_id);
            }
            AppEvent::RequestMoved {
                request,
                new_parent_id,
                insertion_index,
                ..
            } => {
                self.shared_store
                    .requests
                    .insert(request.meta.request_id, request.clone());
                if let Some(node) = self.shared_store.nodes.get_mut(&request.meta.request_id) {
                    node.name = request.meta.name.clone();
                }
                match new_parent_id {
                    Some(parent_id) => {
                        self.move_request_in_shared_store(
                            request.meta.request_id,
                            *parent_id,
                            *insertion_index,
                        );
                    }
                    None => {
                        let previous_parent_id = self
                            .shared_store
                            .nodes
                            .get(&request.meta.request_id)
                            .and_then(|node| node.parent_id);
                        if let Some(prev_id) = previous_parent_id {
                            if let Some(prev_parent) = self.shared_store.nodes.get_mut(&prev_id) {
                                prev_parent
                                    .children
                                    .retain(|id| *id != request.meta.request_id);
                            }
                        } else {
                            self.shared_store
                                .root_ids
                                .retain(|id| *id != request.meta.request_id);
                        }
                        self.shared_store
                            .root_ids
                            .retain(|id| *id != request.meta.request_id);
                        let index = (*insertion_index).min(self.shared_store.root_ids.len());
                        self.shared_store
                            .root_ids
                            .insert(index, request.meta.request_id);
                        if let Some(node) =
                            self.shared_store.nodes.get_mut(&request.meta.request_id)
                        {
                            node.parent_id = None;
                        }
                        let _ = self.shared_store.rebuild_name_index();
                    }
                }
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
                match new_parent_id {
                    Some(parent_id) => {
                        let _ = self.workspace_tree.move_request_node(
                            request.meta.request_id,
                            *parent_id,
                            *insertion_index,
                        );
                    }
                    None => {
                        let _ = self
                            .workspace_tree
                            .move_node_to_root(request.meta.request_id, *insertion_index);
                    }
                }
            }
            AppEvent::FolderUpserted {
                folder,
                manifest_path,
                ..
            } => {
                let folder_id = folder.folder_id;
                let parent_id = folder.parent_folder_id;
                if let Some(node) = self.shared_store.nodes.get_mut(&folder_id) {
                    node.name = folder.name.clone();
                    node.description = folder.description.clone();
                    node.updated_at = Some(folder.updated_at);
                } else {
                    self.shared_store.nodes.insert(
                        folder_id,
                        Node {
                            id: folder_id,
                            name: folder.name.clone(),
                            kind: NodeKind::Folder,
                            description: folder.description.clone(),
                            created_at: Some(folder.created_at),
                            updated_at: Some(folder.updated_at),
                            parent_id,
                            children: Vec::new(),
                        },
                    );
                    if let Some(pid) = parent_id {
                        if let Some(parent) = self.shared_store.nodes.get_mut(&pid) {
                            if !parent.children.contains(&folder_id) {
                                parent.children.push(folder_id);
                            }
                        }
                    } else {
                        if !self.shared_store.root_ids.contains(&folder_id) {
                            self.shared_store.root_ids.push(folder_id);
                        }
                    }
                }
                let _ = self.shared_store.rebuild_name_index();
                self.workspace_tree.nodes.insert(
                    folder_id,
                    TreeNode {
                        id: folder_id,
                        name: folder.name.clone(),
                        kind: TreeNodeKind::Folder,
                        request_method: None,
                        request_url: None,
                        manifest_path: manifest_path.clone(),
                        parent_id,
                        children: self
                            .workspace_tree
                            .node(folder_id)
                            .map(|n| n.children.clone())
                            .unwrap_or_default(),
                    },
                );
                if let Some(pid) = parent_id {
                    if let Some(parent) = self.workspace_tree.nodes.get_mut(&pid) {
                        if !parent.children.contains(&folder_id) {
                            parent.children.push(folder_id);
                        }
                    }
                } else {
                    if !self.workspace_tree.roots.contains(&folder_id) {
                        self.workspace_tree.roots.push(folder_id);
                    }
                }
            }
            AppEvent::FolderDeleted { folder_id, .. } => {
                let removed_request_ids = self.workspace_tree.remove_subtree(*folder_id);
                for request_id in removed_request_ids {
                    self.shared_store.requests.remove(&request_id);
                    self.request_pane_data.remove(&request_id);
                }
                let mut nodes_to_remove = Vec::new();
                if let Some(node) = self.shared_store.nodes.get(folder_id) {
                    nodes_to_remove.push(*folder_id);
                    let mut stack = node.children.clone();
                    while let Some(node_id) = stack.pop() {
                        if let Some(n) = self.shared_store.nodes.get(&node_id) {
                            nodes_to_remove.push(node_id);
                            stack.extend(n.children.clone());
                        }
                    }
                }
                for node_id in &nodes_to_remove {
                    if let Some(node) = self.shared_store.nodes.remove(node_id) {
                        self.shared_store
                            .name_index
                            .remove(&scope_key(node.parent_id, &node.name));
                    }
                }
            }
            AppEvent::WorkspaceSwitched {
                workspace_id,
                workspace_name,
                all_workspaces,
                workspace_tree,
                shared_store,
                request_pane_data,
                environments,
                environment_selection,
                ..
            } => {
                self.workspace.workspace_id = Some(*workspace_id);
                self.workspace.workspace_name = workspace_name.clone();
                self.workspace.all_workspaces = all_workspaces.clone();
                self.workspace_tree = workspace_tree.clone();
                self.shared_store = shared_store.clone();
                self.request_pane_data = request_pane_data.clone();
                self.environments = environments.clone();
                self.environment_selection = environment_selection.clone();
                // Keep the current theme; reset sync lifecycle.
                self.sync_lifecycle = SyncLifecycleState::default();
            }
            AppEvent::WorkspaceDeleted {
                workspace_id,
                all_workspaces,
                new_active_workspace_id,
                workspace_name: _,
                new_active_workspace_name,
                workspace_tree,
                shared_store,
                request_pane_data,
                environments,
                environment_selection,
                ..
            } => {
                self.workspace.all_workspaces = all_workspaces.clone();
                if self.workspace.workspace_id == Some(*workspace_id) {
                    self.workspace.workspace_id = *new_active_workspace_id;
                    self.workspace.workspace_name = new_active_workspace_name.clone();
                    self.workspace_tree = workspace_tree.clone();
                    self.shared_store = shared_store.clone();
                    self.request_pane_data = request_pane_data.clone();
                    self.environments = environments.clone();
                    self.environment_selection = environment_selection.clone();
                    self.sync_lifecycle = SyncLifecycleState::default();
                }
            }
            AppEvent::WorkspaceRenamed {
                workspace,
                all_workspaces,
                ..
            } => {
                self.workspace.all_workspaces = all_workspaces.clone();
                if self.workspace.workspace_id == Some(workspace.workspace_id) {
                    self.workspace.workspace_name = workspace.name.clone();
                }
            }
        }
    }
}

fn sort_environments(environments: &mut [EnvironmentMeta]) {
    environments.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
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
    MoveRequest,
    CreateFolder,
    RenameFolder,
    DeleteFolder,
    SwitchWorkspace,
    CreateWorkspace,
    DeleteWorkspace,
    RenameWorkspace,
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
            AppOperation::MoveRequest => "move_request",
            AppOperation::CreateFolder => "create_folder",
            AppOperation::RenameFolder => "rename_folder",
            AppOperation::DeleteFolder => "delete_folder",
            AppOperation::SwitchWorkspace => "switch_workspace",
            AppOperation::CreateWorkspace => "create_workspace",
            AppOperation::DeleteWorkspace => "delete_workspace",
            AppOperation::RenameWorkspace => "rename_workspace",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppCommand {
    CreateEnvironment {
        name: String,
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
    MoveRequest {
        input: MoveRequestInput,
        command_id: String,
    },
    CreateFolder {
        input: CreateFolderInput,
        command_id: String,
    },
    RenameFolder {
        folder_id: Ulid,
        new_name: String,
        command_id: String,
    },
    DeleteFolder {
        folder_id: Ulid,
        command_id: String,
    },
    SwitchWorkspace {
        workspace_id: Ulid,
        command_id: String,
    },
    CreateWorkspace {
        name: String,
        command_id: String,
    },
    DeleteWorkspace {
        workspace_id: Ulid,
        command_id: String,
    },
    RenameWorkspace {
        workspace_id: Ulid,
        new_name: String,
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
            | AppCommand::DeleteRequest { command_id, .. }
            | AppCommand::MoveRequest { command_id, .. }
            | AppCommand::CreateFolder { command_id, .. }
            | AppCommand::RenameFolder { command_id, .. }
            | AppCommand::DeleteFolder { command_id, .. }
            | AppCommand::SwitchWorkspace { command_id, .. }
            | AppCommand::CreateWorkspace { command_id, .. }
            | AppCommand::DeleteWorkspace { command_id, .. }
            | AppCommand::RenameWorkspace { command_id, .. } => command_id,
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
            AppCommand::MoveRequest { .. } => AppOperation::MoveRequest,
            AppCommand::CreateFolder { .. } => AppOperation::CreateFolder,
            AppCommand::RenameFolder { .. } => AppOperation::RenameFolder,
            AppCommand::DeleteFolder { .. } => AppOperation::DeleteFolder,
            AppCommand::SwitchWorkspace { .. } => AppOperation::SwitchWorkspace,
            AppCommand::CreateWorkspace { .. } => AppOperation::CreateWorkspace,
            AppCommand::DeleteWorkspace { .. } => AppOperation::DeleteWorkspace,
            AppCommand::RenameWorkspace { .. } => AppOperation::RenameWorkspace,
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
    RequestMoved {
        request: RequestFile,
        new_parent_id: Option<Ulid>,
        insertion_index: usize,
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
    FolderUpserted {
        folder: crate::models::FolderMeta,
        manifest_path: Option<PathBuf>,
        command_id: String,
    },
    FolderDeleted {
        folder_id: Ulid,
        command_id: String,
    },
    WorkspaceSwitched {
        workspace_id: Ulid,
        workspace_name: String,
        all_workspaces: Vec<WorkspaceEntry>,
        workspace_tree: WorkspaceTreeState,
        shared_store: SharedStore,
        request_pane_data: HashMap<Ulid, RequestPaneData>,
        environments: Vec<EnvironmentMeta>,
        environment_selection: LocalEnvironmentSelectionState,
        command_id: String,
    },
    WorkspaceDeleted {
        workspace_id: Ulid,
        workspace_name: String,
        all_workspaces: Vec<WorkspaceEntry>,
        new_active_workspace_id: Option<Ulid>,
        new_active_workspace_name: String,
        workspace_tree: WorkspaceTreeState,
        shared_store: SharedStore,
        request_pane_data: HashMap<Ulid, RequestPaneData>,
        environments: Vec<EnvironmentMeta>,
        environment_selection: LocalEnvironmentSelectionState,
        command_id: String,
    },
    WorkspaceRenamed {
        workspace: WorkspaceEntry,
        all_workspaces: Vec<WorkspaceEntry>,
        command_id: String,
    },
}

pub struct DataSyncRuntime {
    pub command_tx: SyncSender<AppCommand>,
    pub event_rx: Receiver<AppEvent>,
}

pub fn next_command_id() -> String {
    Ulid::new().to_string()
}

pub fn start_data_sync_worker(
    storage: WorkspaceRepository<FileSystemStorage>,
    registry: WorkspacesRegistryFile,
    registry_repo: RegistryRepository,
) -> DataSyncRuntime {
    let (command_tx, command_rx) = mpsc::sync_channel::<AppCommand>(APP_COMMAND_QUEUE_CAPACITY);
    let (event_tx, event_rx) = mpsc::channel::<AppEvent>();

    thread::Builder::new()
        .name("beam-data-sync-worker".to_string())
        .spawn(move || {
            data_sync_worker_loop(storage, registry, registry_repo, command_rx, event_tx)
        })
        .expect("failed to start data sync worker thread");

    DataSyncRuntime {
        command_tx,
        event_rx,
    }
}

fn data_sync_worker_loop(
    mut storage: WorkspaceRepository<FileSystemStorage>,
    mut registry: WorkspacesRegistryFile,
    registry_repo: RegistryRepository,
    command_rx: Receiver<AppCommand>,
    event_tx: mpsc::Sender<AppEvent>,
) {
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

            let is_workspace_command = matches!(
                command,
                AppCommand::SwitchWorkspace { .. }
                    | AppCommand::CreateWorkspace { .. }
                    | AppCommand::DeleteWorkspace { .. }
                    | AppCommand::RenameWorkspace { .. }
            );

            let _ = event_tx.send(AppEvent::SyncStarted {
                command_id: command_id.clone(),
                operation,
            });

            if is_workspace_command {
                match handle_workspace_command(&mut storage, &mut registry, &registry_repo, command)
                {
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
                continue;
            }

            if let Err(error) = validate_command_payload(&command) {
                log_sync_failure(&command_id, operation, &error);
                let _ = event_tx.send(AppEvent::SyncFailed {
                    command_id,
                    operation,
                    error,
                });
                continue;
            }

            match handle_command(&mut storage, command) {
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
        AppCommand::CreateFolder { input, .. } => {
            if input.name.trim().is_empty() {
                return Err("Folder name cannot be empty.".to_string());
            }
        }
        AppCommand::RenameFolder { new_name, .. } => {
            if new_name.trim().is_empty() {
                return Err("Folder name cannot be empty.".to_string());
            }
        }
        AppCommand::CreateWorkspace { name, .. } => {
            if name.trim().is_empty() {
                return Err("Workspace name cannot be empty.".to_string());
            }
        }
        AppCommand::RenameWorkspace { new_name, .. } => {
            if new_name.trim().is_empty() {
                return Err("Workspace name cannot be empty.".to_string());
            }
        }
        AppCommand::DeleteEnvironment { .. }
        | AppCommand::DeleteRequest { .. }
        | AppCommand::MoveRequest { .. }
        | AppCommand::DeleteFolder { .. }
        | AppCommand::SwitchWorkspace { .. }
        | AppCommand::DeleteWorkspace { .. } => {}
    }
    Ok(())
}

fn log_sync_failure(command_id: &str, operation: AppOperation, error: &str) {
    log::error!(
        "sync_failure command_id={command_id} operation={} error={}",
        operation.as_str(),
        error
    );
}

fn handle_command<B: StorageIoBackend>(
    storage: &mut WorkspaceRepository<B>,
    command: AppCommand,
) -> std::result::Result<Vec<AppEvent>, String> {
    match command {
        AppCommand::CreateEnvironment { name, command_id } => {
            let created = storage
                .create_environment(CreateEnvironmentInput { name })
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
        AppCommand::MoveRequest { input, command_id } => {
            let new_parent_id = input.new_parent.folder_id;
            let insertion_index = input.insertion_index;
            let moved = storage
                .move_request(input)
                .map_err(|error| error.to_string())?;
            Ok(vec![AppEvent::RequestMoved {
                request: moved,
                new_parent_id,
                insertion_index,
                command_id,
            }])
        }
        AppCommand::CreateFolder { input, command_id } => {
            let created = storage
                .create_folder(input)
                .map_err(|error| error.to_string())?;
            Ok(vec![AppEvent::FolderUpserted {
                folder: created.folder,
                manifest_path: created.manifest_path,
                command_id,
            }])
        }
        AppCommand::RenameFolder {
            folder_id,
            new_name,
            command_id,
        } => {
            let updated = storage
                .rename_folder(folder_id, &new_name)
                .map_err(|error| error.to_string())?;
            Ok(vec![AppEvent::FolderUpserted {
                folder: updated.folder,
                manifest_path: updated.manifest_path,
                command_id,
            }])
        }
        AppCommand::DeleteFolder {
            folder_id,
            command_id,
        } => {
            storage
                .delete_folder(folder_id)
                .map_err(|error| error.to_string())?;
            Ok(vec![AppEvent::FolderDeleted {
                folder_id,
                command_id,
            }])
        }
        // Workspace commands are handled by handle_workspace_command, never reach here.
        AppCommand::SwitchWorkspace { command_id, .. }
        | AppCommand::CreateWorkspace { command_id, .. }
        | AppCommand::DeleteWorkspace { command_id, .. }
        | AppCommand::RenameWorkspace { command_id, .. } => Err(format!(
            "workspace command {command_id} reached handle_command unexpectedly"
        )),
    }
}

fn handle_workspace_command(
    storage: &mut WorkspaceRepository<FileSystemStorage>,
    registry: &mut WorkspacesRegistryFile,
    registry_repo: &RegistryRepository,
    command: AppCommand,
) -> std::result::Result<Vec<AppEvent>, String> {
    match command {
        AppCommand::SwitchWorkspace {
            workspace_id,
            command_id,
        } => {
            // Update registry to mark new workspace as active.
            registry_repo
                .set_active_workspace(registry, workspace_id)
                .map_err(|e| e.to_string())?;

            // Load the new workspace.
            let entry = registry
                .registry
                .workspaces
                .iter()
                .find(|e| e.workspace_id == workspace_id)
                .cloned()
                .ok_or_else(|| format!("workspace {workspace_id} not found in registry"))?;

            let new_paths = registry_repo.workspace_paths(&entry);
            let new_backend = FileSystemStorage::new(new_paths.clone());
            let mut new_repo = WorkspaceRepository::new(new_backend).map_err(|e| e.to_string())?;

            // Initialize workspace files if this is a brand-new workspace.
            new_repo.initialize().map_err(|e| e.to_string())?;

            let local_state = new_repo.load_local_state().unwrap_or_default();

            let (workspace_tree, shared_store, request_pane_data, _warnings) =
                load_workspace_tree(&new_paths, &local_state);
            let mut env_warnings = Vec::new();
            let env_metas = load_environments(&new_paths, &mut env_warnings);

            *storage = new_repo;

            let all_workspaces = registry.registry.workspaces.clone();
            Ok(vec![AppEvent::WorkspaceSwitched {
                workspace_id,
                workspace_name: entry.name,
                all_workspaces,
                workspace_tree,
                shared_store,
                request_pane_data,
                environments: env_metas,
                environment_selection: LocalEnvironmentSelectionState {
                    active_global_environment_id: local_state
                        .local_state
                        .active_global_environment_id,
                },
                command_id,
            }])
        }
        AppCommand::CreateWorkspace { name, command_id } => {
            let entry = registry_repo
                .create_workspace(registry, &name)
                .map_err(|e| e.to_string())?;
            registry_repo
                .set_active_workspace(registry, entry.workspace_id)
                .map_err(|e| e.to_string())?;

            // Bootstrap the new workspace files.
            let ws_paths = registry_repo.workspace_paths(&entry);
            let ws_backend = FileSystemStorage::new(ws_paths.clone());
            let mut ws_repo = WorkspaceRepository::new(ws_backend).map_err(|e| e.to_string())?;
            ws_repo.initialize().map_err(|e| e.to_string())?;

            let local_state = ws_repo.load_local_state().unwrap_or_default();
            let (workspace_tree, shared_store, request_pane_data, _warnings) =
                load_workspace_tree(&ws_paths, &local_state);
            let mut env_warnings = Vec::new();
            let environments = load_environments(&ws_paths, &mut env_warnings);

            *storage = ws_repo;

            let all_workspaces = registry.registry.workspaces.clone();
            Ok(vec![AppEvent::WorkspaceSwitched {
                workspace_id: entry.workspace_id,
                workspace_name: entry.name,
                all_workspaces,
                workspace_tree,
                shared_store,
                request_pane_data,
                environments,
                environment_selection: LocalEnvironmentSelectionState {
                    active_global_environment_id: local_state
                        .local_state
                        .active_global_environment_id,
                },
                command_id,
            }])
        }
        AppCommand::DeleteWorkspace {
            workspace_id,
            command_id,
        } => {
            let workspace_name = registry
                .registry
                .workspaces
                .iter()
                .find(|e| e.workspace_id == workspace_id)
                .map(|entry| entry.name.clone())
                .ok_or_else(|| format!("workspace {workspace_id} not found in registry"))?;
            let deleted_active_workspace =
                registry.registry.active_workspace_id == Some(workspace_id);
            registry_repo
                .delete_workspace(registry, workspace_id)
                .map_err(|e| e.to_string())?;
            let new_active_workspace_id = registry.registry.active_workspace_id;
            let all_workspaces = registry.registry.workspaces.clone();
            let mut new_active_workspace_name = String::new();
            let mut workspace_tree = WorkspaceTreeState::default();
            let mut shared_store = SharedStore::default();
            let mut request_pane_data = HashMap::new();
            let mut environments = Vec::new();
            let mut environment_selection = LocalEnvironmentSelectionState::default();

            if deleted_active_workspace && let Some(active_workspace_id) = new_active_workspace_id {
                let entry = registry
                    .registry
                    .workspaces
                    .iter()
                    .find(|e| e.workspace_id == active_workspace_id)
                    .cloned()
                    .ok_or_else(|| {
                        format!("workspace {active_workspace_id} not found in registry")
                    })?;

                let new_paths = registry_repo.workspace_paths(&entry);
                let new_backend = FileSystemStorage::new(new_paths.clone());
                let mut new_repo =
                    WorkspaceRepository::new(new_backend).map_err(|e| e.to_string())?;

                new_repo.initialize().map_err(|e| e.to_string())?;

                let local_state = new_repo.load_local_state().unwrap_or_default();
                (workspace_tree, shared_store, request_pane_data, _) =
                    load_workspace_tree(&new_paths, &local_state);
                let mut env_warnings = Vec::new();
                environments = load_environments(&new_paths, &mut env_warnings);
                environment_selection = LocalEnvironmentSelectionState {
                    active_global_environment_id: local_state
                        .local_state
                        .active_global_environment_id,
                };
                new_active_workspace_name = entry.name;
                *storage = new_repo;
            }

            Ok(vec![AppEvent::WorkspaceDeleted {
                workspace_id,
                workspace_name,
                all_workspaces,
                new_active_workspace_id,
                new_active_workspace_name,
                workspace_tree,
                shared_store,
                request_pane_data,
                environments,
                environment_selection,
                command_id,
            }])
        }
        AppCommand::RenameWorkspace {
            workspace_id,
            new_name,
            command_id,
        } => {
            let entry = registry_repo
                .rename_workspace(registry, workspace_id, &new_name)
                .map_err(|e| e.to_string())?;
            let all_workspaces = registry.registry.workspaces.clone();
            Ok(vec![AppEvent::WorkspaceRenamed {
                workspace: entry,
                all_workspaces,
                command_id,
            }])
        }
        _ => unreachable!("non-workspace command routed to handle_workspace_command"),
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

pub fn startup_preload<S>(
    storage: &S,
    paths: &BeamPaths,
    workspace_entry: Option<&WorkspaceEntry>,
    all_workspaces: Vec<WorkspaceEntry>,
) -> StartupLoad
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

    let app_settings = match storage.load_app_settings() {
        Ok(settings) => settings,
        Err(error) => {
            return StartupLoad::Fatal {
                message: StartupMessage {
                    severity: StartupMessageSeverity::Fatal,
                    text: format!("Failed to load app settings: {error}"),
                },
            };
        }
    };

    let (workspace_tree, shared_store, request_pane_data, mut warnings) =
        load_workspace_tree(paths, &local_state);
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
    if workspace_tree.selected_request_id().is_none()
        && local_state.local_state.last_opened_request_id.is_some()
    {
        messages.push(StartupMessage {
            severity: StartupMessageSeverity::Warning,
            text: "Last opened request no longer exists and could not be restored.".to_string(),
        });
    }

    StartupLoad::Ready {
        state: AppShellState {
            workspace_tree,
            shared_store,
            request_pane_data,
            environments,
            environment_selection: LocalEnvironmentSelectionState {
                active_global_environment_id: local_state.local_state.active_global_environment_id,
            },
            theme: LocalThemeState {
                theme_name: app_settings.app_settings.theme_name.clone(),
                font_size: app_settings.app_settings.font_size,
            },
            workspace: WorkspaceState {
                workspace_id: workspace_entry.map(|e| e.workspace_id),
                workspace_name: workspace_entry.map(|e| e.name.clone()).unwrap_or_default(),
                all_workspaces,
                ..WorkspaceState::default()
            },
            ..AppShellState::default()
        },
        messages,
    }
}

fn load_workspace_tree(
    paths: &BeamPaths,
    local_state: &LocalStateFile,
) -> (
    WorkspaceTreeState,
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
            tree.select_request(request_id);
        }
    }

    (tree, shared_store, request_pane_data, warnings)
}

fn load_environments(paths: &BeamPaths, warnings: &mut Vec<String>) -> Vec<EnvironmentMeta> {
    let mut environments = Vec::new();
    let mut seen = HashSet::new();
    let entries = match fs::read_dir(&paths.environments_dir) {
        Ok(e) => e,
        Err(_) => return environments,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
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

    sort_environments(&mut environments);
    environments
}

fn parse_environment_file(path: &Path) -> Result<EnvironmentFile> {
    let file: EnvironmentFile = parse_toml(path)?;
    Ok(file.with_file_path(path))
}

fn load_shared_tree(paths: &BeamPaths, warnings: &mut Vec<String>) -> LoadedSharedTree {
    let mut loaded = LoadedSharedTree::default();

    if !paths.workspace_file.exists() {
        return loaded;
    }

    let workspace_file: WorkspaceFile = match parse_toml(&paths.workspace_file) {
        Ok(f) => f,
        Err(e) => {
            warnings.push(format!("Failed to parse workspace file: {e}"));
            return loaded;
        }
    };

    for item_ref in &workspace_file.items {
        match item_ref.item_type {
            ItemType::Folder => {
                let folder_dir = paths.root.join(folder_dir_name(&item_ref.name));
                if folder_dir.exists() {
                    load_folder_from_manifest(
                        &mut loaded,
                        &folder_dir,
                        item_ref.item_id,
                        None,
                        warnings,
                    );
                    if loaded.shared_store.nodes.contains_key(&item_ref.item_id) {
                        if !loaded.shared_store.root_ids.contains(&item_ref.item_id) {
                            loaded.shared_store.root_ids.push(item_ref.item_id);
                        }
                    }
                }
            }
            ItemType::Request => {
                let request_path = paths.root.join(request_file_name(&item_ref.name));
                load_root_request(
                    &mut loaded,
                    &request_path,
                    item_ref.item_id,
                    &item_ref.name,
                    warnings,
                );
            }
        }
    }

    loaded
}

fn load_folder_from_manifest(
    loaded: &mut LoadedSharedTree,
    folder_dir: &Path,
    folder_id: Ulid,
    parent_id: Option<Ulid>,
    warnings: &mut Vec<String>,
) {
    let manifest_path = folder_dir.join(FOLDER_MANIFEST_FILE_NAME);
    if !manifest_path.exists() {
        return;
    }
    let folder_file: FolderFile = match parse_toml(&manifest_path) {
        Ok(f) => f,
        Err(e) => {
            warnings.push(format!(
                "Failed to parse folder manifest {}: {e}",
                manifest_path.display()
            ));
            return;
        }
    };

    if !insert_loaded_node(
        loaded,
        Node {
            id: folder_id,
            name: folder_file.folder.name.clone(),
            kind: NodeKind::Folder,
            description: folder_file.folder.description.clone(),
            created_at: Some(folder_file.folder.created_at),
            updated_at: Some(folder_file.folder.updated_at),
            parent_id,
            children: Vec::new(),
        },
        Some(&manifest_path),
        &manifest_path,
        warnings,
    ) {
        return;
    }

    let mut child_ids = Vec::new();
    for item_ref in &folder_file.items {
        match item_ref.item_type {
            ItemType::Folder => {
                let subfolder_dir = folder_dir.join(folder_dir_name(&item_ref.name));
                if subfolder_dir.exists() {
                    load_folder_from_manifest(
                        loaded,
                        &subfolder_dir,
                        item_ref.item_id,
                        Some(folder_id),
                        warnings,
                    );
                    if loaded.shared_store.nodes.contains_key(&item_ref.item_id) {
                        child_ids.push(item_ref.item_id);
                    }
                }
            }
            ItemType::Request => {
                let request_path = folder_dir.join(request_file_name(&item_ref.name));
                if load_folder_request(
                    loaded,
                    &request_path,
                    item_ref.item_id,
                    &item_ref.name,
                    folder_id,
                    warnings,
                ) {
                    child_ids.push(item_ref.item_id);
                }
            }
        }
    }

    if let Some(folder_node) = loaded.shared_store.nodes.get_mut(&folder_id) {
        folder_node.children = child_ids;
    }
}

fn load_root_request(
    loaded: &mut LoadedSharedTree,
    request_path: &Path,
    expected_id: Ulid,
    manifest_name: &str,
    warnings: &mut Vec<String>,
) {
    if !request_path.exists() {
        return;
    }
    let (mut request_file, pane_data) = match parse_request_tree_meta(request_path) {
        Ok(parsed) => parsed,
        Err(e) => {
            warnings.push(format!(
                "Failed to load request file {}: {e}",
                request_path.display()
            ));
            return;
        }
    };
    if request_file.meta.request_id != expected_id {
        warnings.push(format!(
            "Request file {} declared id {} but workspace lists {}. Skipped.",
            request_path.display(),
            request_file.meta.request_id,
            expected_id
        ));
        return;
    }
    if request_file.meta.name != manifest_name {
        warnings.push(format!(
            "Request file {} name {:?} differed from manifest name {:?}. Using manifest name in memory.",
            request_path.display(),
            request_file.meta.name,
            manifest_name
        ));
        request_file.meta.name = manifest_name.to_string();
    }
    if !insert_loaded_node(
        loaded,
        Node {
            id: expected_id,
            name: request_file.meta.name.clone(),
            kind: NodeKind::Request,
            description: request_file.meta.description.clone(),
            created_at: Some(request_file.meta.created_at),
            updated_at: Some(request_file.meta.updated_at),
            parent_id: None,
            children: Vec::new(),
        },
        None,
        request_path,
        warnings,
    ) {
        return;
    }
    loaded
        .shared_store
        .requests
        .insert(expected_id, request_file);
    loaded.request_pane_data.insert(expected_id, pane_data);
    loaded.shared_store.root_ids.push(expected_id);
}

fn load_folder_request(
    loaded: &mut LoadedSharedTree,
    request_path: &Path,
    expected_id: Ulid,
    manifest_name: &str,
    parent_id: Ulid,
    warnings: &mut Vec<String>,
) -> bool {
    if !request_path.exists() {
        warnings.push(format!(
            "Failed to load request file {}: file not found.",
            request_path.display()
        ));
        return false;
    }
    let (mut request_file, pane_data) = match parse_request_tree_meta(request_path) {
        Ok(parsed) => parsed,
        Err(e) => {
            warnings.push(format!(
                "Failed to load request file {}: {e}",
                request_path.display()
            ));
            return false;
        }
    };
    if request_file.meta.request_id != expected_id {
        warnings.push(format!(
            "Request file {} declared id {} but folder manifest lists {}. Skipped.",
            request_path.display(),
            request_file.meta.request_id,
            expected_id
        ));
        return false;
    }
    if request_file.meta.name != manifest_name {
        warnings.push(format!(
            "Request file {} name {:?} differed from manifest name {:?}. Using manifest name in memory.",
            request_path.display(),
            request_file.meta.name,
            manifest_name
        ));
        request_file.meta.name = manifest_name.to_string();
    }
    if !insert_loaded_node(
        loaded,
        Node {
            id: expected_id,
            name: request_file.meta.name.clone(),
            kind: NodeKind::Request,
            description: request_file.meta.description.clone(),
            created_at: Some(request_file.meta.created_at),
            updated_at: Some(request_file.meta.updated_at),
            parent_id: Some(parent_id),
            children: Vec::new(),
        },
        None,
        request_path,
        warnings,
    ) {
        return false;
    }
    loaded
        .shared_store
        .requests
        .insert(expected_id, request_file);
    loaded.request_pane_data.insert(expected_id, pane_data);
    true
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
) -> WorkspaceTreeState {
    let nodes = shared_store
        .nodes
        .iter()
        .map(|(node_id, node)| {
            let (request_method, request_url, manifest_path) = match node.kind {
                NodeKind::Folder => (None, None, manifest_paths.get(node_id).cloned()),
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

    WorkspaceTreeState {
        nodes,
        roots: shared_store.root_ids.clone(),
        expanded: BTreeSet::new(),
        selected_request_id: None,
    }
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
    use std::path::Path;
    use std::sync::mpsc::Receiver;
    use std::time::Duration;

    use tempfile::tempdir;

    use super::*;
    use crate::models::{
        AppSettingsFile, EnvironmentMeta, EnvironmentScope, FolderMeta, LocalState,
        ManifestItemRef, RequestDefinition, RequestMeta, ScriptConfig, TreeState, WorkspaceMeta,
        WorkspacesRegistryFile,
    };
    use crate::paths::DataRootPaths;
    use crate::schema::SCHEMA_VERSION_V1;
    use crate::storage::MoveFolderInput;
    use crate::storage::fs_backend::FileSystemStorage;
    use crate::storage::registry_repo::RegistryRepository;
    use crate::storage::workspace_repo::WorkspaceRepository;
    use chrono::Utc;

    /// Create a minimal registry + repo for tests that need start_data_sync_worker.
    fn test_registry_for_dir(dir: &Path) -> (WorkspacesRegistryFile, RegistryRepository) {
        let data_root = DataRootPaths::new(
            dir.join("beam"),
            dir.join("beam_local"),
            dir.join("beam_logs"),
        );
        let repo = RegistryRepository::new(data_root);
        let registry = WorkspacesRegistryFile {
            schema_version: crate::schema::SCHEMA_VERSION_V1,
            registry: crate::models::WorkspacesRegistry {
                active_workspace_id: None,
                workspaces: vec![],
            },
        };
        (registry, repo)
    }

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
        let folder_id = Ulid::new();
        let request_id = Ulid::new();
        let missing_id = Ulid::new();
        let mut tree = WorkspaceTreeState::default();
        tree.nodes.insert(
            folder_id,
            TreeNode {
                id: folder_id,
                name: "Folder".to_string(),
                kind: TreeNodeKind::Folder,
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
                parent_id: Some(folder_id),
                children: Vec::new(),
            },
        );

        tree.set_expanded([folder_id, request_id, missing_id]);

        assert!(tree.is_expanded(folder_id));
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

    fn write_workspace_file(paths: &BeamPaths, items: Vec<ManifestItemRef>) {
        let workspace_file = WorkspaceFile {
            schema_version: SCHEMA_VERSION_V1,
            workspace: WorkspaceMeta {
                workspace_id: Ulid::new(),
                name: "Test".to_string(),
                description: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            items,
        };
        let encoded = toml::to_string_pretty(&workspace_file).expect("encode workspace");
        fs::create_dir_all(paths.workspace_file.parent().unwrap()).expect("create workspace dir");
        fs::write(&paths.workspace_file, encoded).expect("write workspace");
    }

    fn write_folder_manifest(
        folder_dir: &Path,
        folder_id: Ulid,
        name: &str,
        items: Vec<ManifestItemRef>,
    ) -> PathBuf {
        fs::create_dir_all(folder_dir).expect("create folder dir");
        let now = Utc::now();
        let folder_file = FolderFile {
            folder: FolderMeta {
                folder_id,
                parent_folder_id: None,
                name: name.to_string(),
                description: None,
                created_at: now,
                updated_at: now,
            },
            items,
            manifest_path: None,
        };
        let manifest_path = folder_dir.join(FOLDER_MANIFEST_FILE_NAME);
        let encoded = toml::to_string_pretty(&folder_file).expect("encode folder manifest");
        fs::write(&manifest_path, encoded).expect("write folder manifest");
        manifest_path
    }

    fn write_request_payload(
        parent_dir: &Path,
        request_id: Ulid,
        name: &str,
        method: HttpMethod,
        url: &str,
    ) -> PathBuf {
        let request_dir = parent_dir.to_path_buf();
        fs::create_dir_all(&request_dir).expect("create request dir");
        let request_path = request_dir.join(request_file_name(name));
        let request_file =
            sample_request_file(request_id, name, method, url).with_file_path(&request_path);
        let encoded = toml::to_string_pretty(&request_file).expect("encode request file");
        fs::write(&request_path, encoded).expect("write request file");
        request_path
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
        let folder_id = Ulid::new();
        let first_request_id = Ulid::new();
        let second_request_id = Ulid::new();
        let mut state = AppShellState::default();
        let now = Utc::now();
        state.shared_store.nodes.insert(
            folder_id,
            Node {
                id: folder_id,
                name: "Folder".to_string(),
                kind: NodeKind::Folder,
                description: None,
                created_at: Some(now),
                updated_at: Some(now),
                parent_id: None,
                children: vec![first_request_id],
            },
        );
        state.shared_store.root_ids.push(folder_id);
        state.shared_store.nodes.insert(
            first_request_id,
            Node {
                id: first_request_id,
                name: "First Request".to_string(),
                kind: NodeKind::Request,
                description: None,
                created_at: Some(now),
                updated_at: Some(now),
                parent_id: Some(folder_id),
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
        state.insert_request_into_shared_store(folder_id, Some(first_request_id), &second_request);

        assert_eq!(
            state
                .shared_store
                .nodes
                .get(&folder_id)
                .expect("folder exists")
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
            Some(folder_id)
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
        let other_env_id = Ulid::new();
        let now = Utc::now();
        let global_environment = EnvironmentMeta {
            environment_id: global_env_id,
            scope: EnvironmentScope::Global,
            name: "Global".to_string(),
            file_name: "global.env.toml".to_string(),
            description: None,
            created_at: now,
            updated_at: now,
        };
        let other_environment = EnvironmentMeta {
            environment_id: other_env_id,
            scope: EnvironmentScope::Global,
            name: "Other".to_string(),
            file_name: "other.env.toml".to_string(),
            description: None,
            created_at: now,
            updated_at: now,
        };

        state.apply_event(&AppEvent::EnvironmentUpserted {
            environment: other_environment.clone(),
            command_id: Ulid::new().to_string(),
        });
        state.apply_event(&AppEvent::EnvironmentUpserted {
            environment: global_environment.clone(),
            command_id: Ulid::new().to_string(),
        });

        assert_eq!(state.environments.len(), 2);
        state.environment_selection.active_global_environment_id = Some(global_env_id);

        state.apply_event(&AppEvent::EnvironmentDeleted {
            environment_id: global_env_id,
            command_id: Ulid::new().to_string(),
        });
        assert_eq!(state.environments.len(), 1);
        assert_eq!(
            state.environment_selection.active_global_environment_id,
            None
        );
    }

    #[test]
    fn effective_environment_returns_global_selection() {
        let mut state = AppShellState::default();
        let request_id = Ulid::new();
        let global_env_id = Ulid::new();
        let now = Utc::now();
        state.workspace_tree.nodes.insert(
            request_id,
            TreeNode {
                id: request_id,
                name: "Request".to_string(),
                kind: TreeNodeKind::Request,
                request_method: Some(HttpMethod::Get),
                request_url: Some("https://example.com".to_string()),
                manifest_path: None,
                parent_id: None,
                children: Vec::new(),
            },
        );
        state.workspace_tree.set_selected_request(Some(request_id));
        state.environments = vec![EnvironmentMeta {
            environment_id: global_env_id,
            scope: EnvironmentScope::Global,
            name: "Global".to_string(),
            file_name: "global.env.toml".to_string(),
            description: None,
            created_at: now,
            updated_at: now,
        }];

        assert_eq!(state.effective_environment_id_for_selected_request(), None);

        state.environment_selection.active_global_environment_id = Some(global_env_id);
        assert_eq!(
            state.effective_environment_id_for_selected_request(),
            Some(global_env_id)
        );

        state.environments.clear();
        assert_eq!(state.effective_environment_id_for_selected_request(), None);
    }

    #[test]
    fn environment_commands_propagate_to_shell_via_worker_events() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let backend = FileSystemStorage::new(paths);
        let mut storage = WorkspaceRepository::new(backend).expect("load workspace into memory");
        storage.initialize().expect("init storage");
        let (registry, registry_repo) = test_registry_for_dir(dir.path());
        let runtime = start_data_sync_worker(storage, registry, registry_repo);
        let mut state = AppShellState::default();

        let create_command_id = next_command_id();
        runtime
            .command_tx
            .send(AppCommand::CreateEnvironment {
                name: "Phase3 Global".to_string(),
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
        let backend = FileSystemStorage::new(paths.clone());

        let mut storage = WorkspaceRepository::new(backend).expect("load workspace into memory");
        storage.initialize().expect("init storage");
        let (registry, registry_repo) = test_registry_for_dir(dir.path());
        let runtime = start_data_sync_worker(storage, registry, registry_repo);
        let mut state = AppShellState::default();
        let parent = RequestParentRef { folder_id: None };

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
        let folder_id = Ulid::new();
        let request_id = Ulid::new();

        state.workspace_tree.roots.push(folder_id);
        state.workspace_tree.nodes.insert(
            folder_id,
            TreeNode {
                id: folder_id,
                name: "Folder".to_string(),
                kind: TreeNodeKind::Folder,
                request_method: None,
                request_url: None,
                manifest_path: None,
                parent_id: None,
                children: vec![request_id],
            },
        );
        state.workspace_tree.nodes.insert(
            request_id,
            TreeNode {
                id: request_id,
                name: "Old Name".to_string(),
                kind: TreeNodeKind::Request,
                request_method: Some(HttpMethod::Get),
                request_url: Some("https://example.com/old".to_string()),
                manifest_path: None,
                parent_id: Some(folder_id),
                children: Vec::new(),
            },
        );
        state.workspace_tree.set_selected_request(Some(request_id));

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

        let node = state.workspace_tree.node(request_id).expect("request node");
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
        assert!(state.workspace_tree.node(request_id).is_none());
        assert!(state.request_pane_data.get(&request_id).is_none());
        assert_eq!(state.workspace_tree.selected_request_id(), None);
    }

    #[test]
    fn app_shell_reducer_replaces_active_workspace_state_after_delete() {
        let deleted_workspace_id = Ulid::new();
        let next_workspace_id = Ulid::new();
        let deleted_request_id = Ulid::new();
        let next_request_id = Ulid::new();
        let now = Utc::now();

        let mut state = AppShellState::default();
        state.workspace.workspace_id = Some(deleted_workspace_id);
        state.workspace.workspace_name = "Deleted".to_string();
        state.workspace.all_workspaces = vec![
            WorkspaceEntry {
                workspace_id: deleted_workspace_id,
                name: "Deleted".to_string(),
                path: "deleted".to_string(),
                created_at: now,
            },
            WorkspaceEntry {
                workspace_id: next_workspace_id,
                name: "Next".to_string(),
                path: "next".to_string(),
                created_at: now,
            },
        ];
        state.workspace_tree.roots.push(deleted_request_id);
        state.workspace_tree.nodes.insert(
            deleted_request_id,
            TreeNode {
                id: deleted_request_id,
                name: "Deleted Request".to_string(),
                kind: TreeNodeKind::Request,
                request_method: Some(HttpMethod::Get),
                request_url: Some("https://deleted.example.com".to_string()),
                manifest_path: None,
                parent_id: None,
                children: Vec::new(),
            },
        );
        state
            .workspace_tree
            .set_selected_request(Some(deleted_request_id));
        state.request_pane_data.insert(
            deleted_request_id,
            RequestPaneData {
                method: HttpMethod::Get,
                url: "https://deleted.example.com".to_string(),
                headers: Vec::new(),
                query_params: Vec::new(),
                auth: AuthConfig::None,
                body: BodyConfig::None,
                post_script: None,
            },
        );

        let mut next_workspace_tree = WorkspaceTreeState::default();
        next_workspace_tree.roots.push(next_request_id);
        next_workspace_tree.nodes.insert(
            next_request_id,
            TreeNode {
                id: next_request_id,
                name: "Next Request".to_string(),
                kind: TreeNodeKind::Request,
                request_method: Some(HttpMethod::Post),
                request_url: Some("https://next.example.com".to_string()),
                manifest_path: None,
                parent_id: None,
                children: Vec::new(),
            },
        );
        next_workspace_tree.set_selected_request(Some(next_request_id));

        let mut next_shared_store = SharedStore::default();
        next_shared_store.nodes.insert(
            next_request_id,
            Node {
                id: next_request_id,
                name: "Next Request".to_string(),
                kind: NodeKind::Request,
                description: None,
                created_at: Some(now),
                updated_at: Some(now),
                parent_id: None,
                children: Vec::new(),
            },
        );
        next_shared_store.root_ids.push(next_request_id);
        next_shared_store.requests.insert(
            next_request_id,
            sample_request_file(
                next_request_id,
                "Next Request",
                HttpMethod::Post,
                "https://next.example.com",
            ),
        );

        let next_request_pane_data = HashMap::from([(
            next_request_id,
            RequestPaneData {
                method: HttpMethod::Post,
                url: "https://next.example.com".to_string(),
                headers: Vec::new(),
                query_params: Vec::new(),
                auth: AuthConfig::None,
                body: BodyConfig::None,
                post_script: None,
            },
        )]);
        let next_environments = vec![EnvironmentMeta {
            environment_id: Ulid::new(),
            scope: EnvironmentScope::Global,
            name: "Global".to_string(),
            file_name: "global.env.toml".to_string(),
            description: None,
            created_at: now,
            updated_at: now,
        }];

        state.apply_event(&AppEvent::WorkspaceDeleted {
            workspace_id: deleted_workspace_id,
            workspace_name: "Deleted".to_string(),
            all_workspaces: vec![WorkspaceEntry {
                workspace_id: next_workspace_id,
                name: "Next".to_string(),
                path: "next".to_string(),
                created_at: now,
            }],
            new_active_workspace_id: Some(next_workspace_id),
            new_active_workspace_name: "Next".to_string(),
            workspace_tree: next_workspace_tree.clone(),
            shared_store: next_shared_store,
            request_pane_data: next_request_pane_data.clone(),
            environments: next_environments.clone(),
            environment_selection: LocalEnvironmentSelectionState {
                active_global_environment_id: next_environments
                    .first()
                    .map(|environment| environment.environment_id),
            },
            command_id: Ulid::new().to_string(),
        });

        assert_eq!(state.workspace.workspace_id, Some(next_workspace_id));
        assert_eq!(state.workspace.workspace_name, "Next");
        assert_eq!(
            state.workspace_tree.selected_request_id(),
            Some(next_request_id)
        );
        assert!(state.workspace_tree.node(deleted_request_id).is_none());
        assert_eq!(state.request_pane_data, next_request_pane_data);
        assert_eq!(state.environments, next_environments);
    }

    #[test]
    fn create_workspace_switches_to_new_workspace() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam").join("placeholder"));
        let backend = FileSystemStorage::new(paths);
        let mut storage = WorkspaceRepository::new(backend).expect("load workspace into memory");
        storage.initialize().expect("init storage");
        let (registry, registry_repo) = test_registry_for_dir(dir.path());
        let runtime = start_data_sync_worker(storage, registry, registry_repo);
        let mut state = AppShellState::default();

        let command_id = next_command_id();
        runtime
            .command_tx
            .send(AppCommand::CreateWorkspace {
                name: "Created Workspace".to_string(),
                command_id: command_id.clone(),
            })
            .expect("queue create workspace");

        let events = apply_events_for_command(&mut state, &runtime.event_rx, &command_id);
        let created_workspace_id = events
            .iter()
            .find_map(|event| match event {
                AppEvent::WorkspaceSwitched {
                    workspace_id,
                    workspace_name,
                    command_id: event_command_id,
                    ..
                } if event_command_id == &command_id && workspace_name == "Created Workspace" => {
                    Some(*workspace_id)
                }
                _ => None,
            })
            .expect("create should emit WorkspaceSwitched");

        assert_eq!(state.workspace.workspace_id, Some(created_workspace_id));
        assert_eq!(state.workspace.workspace_name, "Created Workspace");
        assert!(state.workspace.all_workspaces.iter().any(|workspace| {
            workspace.workspace_id == created_workspace_id && workspace.name == "Created Workspace"
        }));

        let registry_file = dir.path().join("beam").join("workspaces.toml");
        let persisted_registry: WorkspacesRegistryFile =
            toml::from_str(&fs::read_to_string(&registry_file).expect("read registry"))
                .expect("decode registry");
        assert_eq!(
            persisted_registry.registry.active_workspace_id,
            Some(created_workspace_id)
        );
    }

    #[test]
    fn worker_emits_sync_failed_for_invalid_payload_without_completion() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let backend = FileSystemStorage::new(paths);
        let mut storage = WorkspaceRepository::new(backend).expect("load workspace into memory");
        storage.initialize().expect("init storage");
        let (registry, registry_repo) = test_registry_for_dir(dir.path());
        let runtime = start_data_sync_worker(storage, registry, registry_repo);
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
        let folder_id = Ulid::new();
        let request_id = Ulid::new();
        state.workspace_tree.roots.push(folder_id);
        state.workspace_tree.nodes.insert(
            folder_id,
            TreeNode {
                id: folder_id,
                name: "Folder".to_string(),
                kind: TreeNodeKind::Folder,
                request_method: None,
                request_url: None,
                manifest_path: None,
                parent_id: None,
                children: vec![request_id],
            },
        );
        state.workspace_tree.nodes.insert(
            request_id,
            TreeNode {
                id: request_id,
                name: "Before Optimistic".to_string(),
                kind: TreeNodeKind::Request,
                request_method: Some(HttpMethod::Get),
                request_url: Some("https://example.com/before".to_string()),
                manifest_path: None,
                parent_id: Some(folder_id),
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
            .workspace_tree
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
    fn startup_restores_last_request_and_expands_ancestors() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let backend = FileSystemStorage::new(paths.clone());
        let storage = WorkspaceRepository::new(backend).expect("load workspace into memory");
        storage
            .save_workspace(&WorkspaceFile::default())
            .expect("save workspace");
        storage
            .save_local_state(&LocalStateFile::default())
            .expect("save local state");

        let folder_id = Ulid::new();
        let request_id = Ulid::new();
        let folder_dir = paths.root.join(folder_dir_name("Sample"));
        let nested_dir = folder_dir.join(folder_dir_name("Nested"));
        // Write workspace file referencing the top-level folder
        write_workspace_file(
            &paths,
            vec![ManifestItemRef {
                item_id: folder_id,
                item_type: ItemType::Folder,
                name: "Sample".to_string(),
                order: 0,
            }],
        );
        // Write nested folder manifest for the top-level folder
        let nested_folder_id = Ulid::new();
        write_folder_manifest(
            &folder_dir,
            folder_id,
            "Sample",
            vec![ManifestItemRef {
                item_id: nested_folder_id,
                item_type: ItemType::Folder,
                name: "Nested".to_string(),
                order: 0,
            }],
        );
        // Write nested subfolder manifest
        write_folder_manifest(
            &nested_dir,
            nested_folder_id,
            "Nested",
            vec![ManifestItemRef {
                item_id: request_id,
                item_type: ItemType::Request,
                name: "Get Data".to_string(),
                order: 0,
            }],
        );
        write_request_payload(
            &nested_dir,
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
                updated_at: Utc::now(),
            },
            tree_state: TreeState::default(),
        };
        storage
            .save_local_state(&local_state)
            .expect("save local state");

        let load = startup_preload(&storage, &paths, None, vec![]);
        let StartupLoad::Ready { state, .. } = load else {
            panic!("startup should be ready");
        };

        assert_eq!(state.workspace_tree.selected_request_id(), Some(request_id));
        assert!(state.workspace_tree.expanded().contains(&nested_folder_id));
        assert!(state.workspace_tree.expanded().contains(&folder_id));
    }

    #[test]
    fn startup_emits_warning_for_missing_last_request() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let backend = FileSystemStorage::new(paths.clone());
        let storage = WorkspaceRepository::new(backend).expect("load workspace into memory");
        storage
            .save_workspace(&WorkspaceFile::default())
            .expect("save workspace");
        storage
            .save_local_state(&LocalStateFile::default())
            .expect("save local state");

        storage
            .save_workspace(&WorkspaceFile::default())
            .expect("save workspace");

        let local_state = LocalStateFile {
            schema_version: SCHEMA_VERSION_V1,
            local_state: LocalState {
                active_global_environment_id: None,
                last_opened_request_id: Some(Ulid::new()),
                updated_at: Utc::now(),
            },
            tree_state: TreeState::default(),
        };
        storage
            .save_local_state(&local_state)
            .expect("save local state");

        let load = startup_preload(&storage, &paths, None, vec![]);
        let StartupLoad::Ready { state, messages } = load else {
            panic!("startup should be ready");
        };
        assert_eq!(state.workspace_tree.selected_request_id(), None);
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
        let backend = FileSystemStorage::new(paths.clone());
        let storage = WorkspaceRepository::new(backend).expect("load workspace into memory");
        storage
            .save_workspace(&WorkspaceFile::default())
            .expect("save workspace");
        storage
            .save_local_state(&LocalStateFile::default())
            .expect("save local state");

        fs::write(paths.local_state_file.clone(), "not = valid = toml")
            .expect("corrupt local state");

        let load = startup_preload(&storage, &paths, None, vec![]);
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
        let backend = FileSystemStorage::new(paths.clone());
        let storage = WorkspaceRepository::new(backend).expect("load workspace into memory");
        storage
            .save_workspace(&WorkspaceFile::default())
            .expect("save workspace");
        storage
            .save_local_state(&LocalStateFile::default())
            .expect("save local state");

        let folder_id = Ulid::new();
        let request_id = Ulid::new();
        let folder_dir = paths.root.join(folder_dir_name("Sample Collection"));

        let workspace_toml = format!(
            r#"
schema_version = 1

[workspace]
workspace_id = "{}"
name = "Beam Workspace"
created_at = "2026-05-01T03:42:36.157016+00:00"
updated_at = "2026-05-01T03:42:36.157016+00:00"

[[items]]
item_id = "{folder_id}"
item_type = "folder"
name = "Sample Collection"
order = 0
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
updated_at = "2026-05-01T03:42:36.157016+00:00"

[tree_state]
expanded_item_ids = ["{folder_id}"]
"#,
            env_id = env_id
        );
        fs::write(&paths.local_state_file, local_state_toml).expect("write local state");
        storage
            .save_app_settings(&AppSettingsFile {
                schema_version: SCHEMA_VERSION_V1,
                app_settings: crate::models::AppSettings {
                    theme_name: Some("One Dark".to_string()),
                    font_size: AppFontSize::Large,
                    updated_at: Utc::now(),
                },
            })
            .expect("save app settings");

        let folder_manifest_path = write_folder_manifest(
            &folder_dir,
            folder_id,
            "Sample Collection",
            vec![ManifestItemRef {
                item_id: request_id,
                item_type: ItemType::Request,
                name: "Manifest Name".to_string(),
                order: 0,
            }],
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
url = "https://httpbingo.org/get"

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
        let request_path = folder_dir.join(request_file_name("Manifest Name"));
        fs::write(&request_path, request_toml).expect("write request");

        let load = startup_preload(&storage, &paths, None, vec![]);
        let StartupLoad::Ready { state, .. } = load else {
            panic!("startup should be ready");
        };

        assert_eq!(state.workspace_tree.selected_request_id(), Some(request_id));
        let request_node = state
            .workspace_tree
            .node(request_id)
            .expect("request node should exist");
        let folder_node = state
            .workspace_tree
            .node(folder_id)
            .expect("folder node should exist");
        assert_eq!(
            folder_node.manifest_path.as_deref(),
            Some(folder_manifest_path.as_path())
        );
        assert_eq!(request_node.name, "Manifest Name");
        assert_eq!(request_node.request_method, Some(HttpMethod::Get));
        assert_eq!(
            request_node.request_url.as_deref(),
            Some("https://httpbingo.org/get")
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
        assert_eq!(state.theme.font_size, AppFontSize::Large);
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
        let backend = FileSystemStorage::new(paths.clone());
        let storage = WorkspaceRepository::new(backend).expect("load workspace into memory");
        storage
            .save_workspace(&WorkspaceFile::default())
            .expect("save workspace");
        storage
            .save_local_state(&LocalStateFile::default())
            .expect("save local state");

        let top_folder_id = Ulid::new();
        let nested_folder_id = Ulid::new();
        let top_folder_dir = paths.root.join(folder_dir_name("Sample"));
        let nested_folder_dir = top_folder_dir.join(folder_dir_name("Nested"));

        write_workspace_file(
            &paths,
            vec![ManifestItemRef {
                item_id: top_folder_id,
                item_type: ItemType::Folder,
                name: "Sample".to_string(),
                order: 0,
            }],
        );
        write_folder_manifest(
            &top_folder_dir,
            top_folder_id,
            "Sample",
            vec![ManifestItemRef {
                item_id: nested_folder_id,
                item_type: ItemType::Folder,
                name: "Nested".to_string(),
                order: 0,
            }],
        );
        let nested_manifest_path =
            write_folder_manifest(&nested_folder_dir, nested_folder_id, "Nested", vec![]);

        let load = startup_preload(&storage, &paths, None, vec![]);
        let StartupLoad::Ready { state, .. } = load else {
            panic!("startup should be ready");
        };

        let folder_node = state
            .workspace_tree
            .node(nested_folder_id)
            .expect("folder node should exist");
        assert_eq!(
            folder_node.manifest_path.as_deref(),
            Some(nested_manifest_path.as_path())
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
        let backend = FileSystemStorage::new(paths.clone());
        let storage = WorkspaceRepository::new(backend).expect("load workspace into memory");
        storage
            .save_workspace(&WorkspaceFile::default())
            .expect("save workspace");
        storage
            .save_local_state(&LocalStateFile::default())
            .expect("save local state");

        let top_folder_id = Ulid::new();
        let nested_folder_id = Ulid::new();
        let top_dir = paths.root.join(folder_dir_name("Sample"));
        let nested_dir = top_dir.join(folder_dir_name("Nested"));

        write_workspace_file(
            &paths,
            vec![ManifestItemRef {
                item_id: top_folder_id,
                item_type: ItemType::Folder,
                name: "Sample".to_string(),
                order: 0,
            }],
        );
        write_folder_manifest(
            &top_dir,
            top_folder_id,
            "Sample",
            vec![ManifestItemRef {
                item_id: nested_folder_id,
                item_type: ItemType::Folder,
                name: "Nested".to_string(),
                order: 0,
            }],
        );
        write_folder_manifest(&nested_dir, nested_folder_id, "Nested", vec![]);

        let local_state = LocalStateFile {
            schema_version: SCHEMA_VERSION_V1,
            local_state: LocalState {
                active_global_environment_id: None,
                last_opened_request_id: None,
                updated_at: Utc::now(),
            },
            tree_state: TreeState {
                expanded_item_ids: vec![top_folder_id, nested_folder_id],
            },
        };
        storage
            .save_local_state(&local_state)
            .expect("save local state");

        let load = startup_preload(&storage, &paths, None, vec![]);
        let StartupLoad::Ready { state, .. } = load else {
            panic!("startup should be ready");
        };

        assert!(state.workspace_tree.expanded().contains(&top_folder_id));
        assert!(state.workspace_tree.expanded().contains(&nested_folder_id));
    }

    #[test]
    fn startup_restores_expanded_ids_from_current_local_state_shape() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let backend = FileSystemStorage::new(paths.clone());
        let storage = WorkspaceRepository::new(backend).expect("load workspace into memory");
        storage
            .save_workspace(&WorkspaceFile::default())
            .expect("save workspace");
        storage
            .save_local_state(&LocalStateFile::default())
            .expect("save local state");

        let folder_id = Ulid::new();
        let folder_dir = paths.root.join(folder_dir_name("Sample"));
        write_workspace_file(
            &paths,
            vec![ManifestItemRef {
                item_id: folder_id,
                item_type: ItemType::Folder,
                name: "Sample".to_string(),
                order: 0,
            }],
        );
        write_folder_manifest(&folder_dir, folder_id, "Sample", vec![]);

        let local_state_toml = format!(
            r#"
schema_version = 1

[local_state]
updated_at = "2026-01-01T00:00:00Z"

[tree_state]
expanded_item_ids = ["{folder_id}"]
"#
        );
        fs::write(&paths.local_state_file, local_state_toml).expect("write local state");

        let load = startup_preload(&storage, &paths, None, vec![]);
        let StartupLoad::Ready { state, .. } = load else {
            panic!("startup should be ready");
        };

        assert!(state.workspace_tree.expanded().contains(&folder_id));
    }

    #[test]
    fn startup_applies_workspace_order_to_loaded_folders() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let backend = FileSystemStorage::new(paths.clone());
        let storage = WorkspaceRepository::new(backend).expect("load workspace into memory");
        storage
            .save_workspace(&WorkspaceFile::default())
            .expect("save workspace");
        storage
            .save_local_state(&LocalStateFile::default())
            .expect("save local state");

        let first_id = Ulid::new();
        let second_id = Ulid::new();
        // Workspace lists second before first (explicit order)
        write_workspace_file(
            &paths,
            vec![
                ManifestItemRef {
                    item_id: second_id,
                    item_type: ItemType::Folder,
                    name: "Second".to_string(),
                    order: 0,
                },
                ManifestItemRef {
                    item_id: first_id,
                    item_type: ItemType::Folder,
                    name: "First".to_string(),
                    order: 1,
                },
            ],
        );
        write_folder_manifest(
            &paths.root.join(folder_dir_name("Second")),
            second_id,
            "Second",
            vec![],
        );
        write_folder_manifest(
            &paths.root.join(folder_dir_name("First")),
            first_id,
            "First",
            vec![],
        );

        let load = startup_preload(&storage, &paths, None, vec![]);
        let StartupLoad::Ready { state, .. } = load else {
            panic!("startup should be ready");
        };

        assert_eq!(state.shared_store.root_ids, vec![second_id, first_id]);
        assert_eq!(state.workspace_tree.visible_rows()[0].id, second_id);
        assert_eq!(state.workspace_tree.visible_rows()[1].id, first_id);
    }

    #[test]
    fn startup_skips_invalid_request_toml_with_warning() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let backend = FileSystemStorage::new(paths.clone());
        let storage = WorkspaceRepository::new(backend).expect("load workspace into memory");
        storage
            .save_workspace(&WorkspaceFile::default())
            .expect("save workspace");
        storage
            .save_local_state(&LocalStateFile::default())
            .expect("save local state");

        let folder_id = Ulid::new();
        let request_id = Ulid::new();
        let folder_dir = paths.root.join(folder_dir_name("Sample"));
        write_workspace_file(
            &paths,
            vec![ManifestItemRef {
                item_id: folder_id,
                item_type: ItemType::Folder,
                name: "Sample".to_string(),
                order: 0,
            }],
        );
        write_folder_manifest(
            &folder_dir,
            folder_id,
            "Sample",
            vec![ManifestItemRef {
                item_id: request_id,
                item_type: ItemType::Request,
                name: "Broken Request".to_string(),
                order: 0,
            }],
        );
        let request_path = folder_dir.join(request_file_name("Broken Request"));
        fs::write(&request_path, "not = valid = toml").expect("write invalid request file");

        let load = startup_preload(&storage, &paths, None, vec![]);
        let StartupLoad::Ready { state, messages } = load else {
            panic!("startup should be ready");
        };

        assert!(state.shared_store.nodes.contains_key(&folder_id));
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
        let backend = FileSystemStorage::new(paths.clone());
        let storage = WorkspaceRepository::new(backend).expect("load workspace into memory");
        storage
            .save_workspace(&WorkspaceFile::default())
            .expect("save workspace");
        storage
            .save_local_state(&LocalStateFile::default())
            .expect("save local state");

        let folder_id = Ulid::new();
        let good_request_id = Ulid::new();
        let missing_request_id = Ulid::new();
        let mismatched_request_id = Ulid::new();
        let folder_dir = paths.root.join(folder_dir_name("Sample"));
        write_workspace_file(
            &paths,
            vec![ManifestItemRef {
                item_id: folder_id,
                item_type: ItemType::Folder,
                name: "Sample".to_string(),
                order: 0,
            }],
        );
        write_folder_manifest(
            &folder_dir,
            folder_id,
            "Sample",
            vec![
                ManifestItemRef {
                    item_id: good_request_id,
                    item_type: ItemType::Request,
                    name: "Good Request".to_string(),
                    order: 0,
                },
                ManifestItemRef {
                    item_id: missing_request_id,
                    item_type: ItemType::Request,
                    name: "Missing Request".to_string(),
                    order: 1,
                },
                ManifestItemRef {
                    item_id: mismatched_request_id,
                    item_type: ItemType::Request,
                    name: "Wrong Id Request".to_string(),
                    order: 2,
                },
            ],
        );
        write_request_payload(
            &folder_dir,
            good_request_id,
            "Good Request",
            HttpMethod::Get,
            "https://example.com/good",
        );
        write_request_payload(
            &folder_dir,
            Ulid::new(),
            "Wrong Id Request",
            HttpMethod::Post,
            "https://example.com/wrong",
        );

        let load = startup_preload(&storage, &paths, None, vec![]);
        let StartupLoad::Ready { state, messages } = load else {
            panic!("startup should be ready");
        };

        assert!(state.shared_store.nodes.contains_key(&folder_id));
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
            message.text.contains("folder manifest lists")
                && message.text.contains("wrong-id-request.request.toml")
        }));
    }

    #[test]
    fn startup_skips_duplicate_node_ids_and_keeps_remaining_children() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let backend = FileSystemStorage::new(paths.clone());
        let storage = WorkspaceRepository::new(backend).expect("load workspace into memory");
        storage
            .save_workspace(&WorkspaceFile::default())
            .expect("save workspace");
        storage
            .save_local_state(&LocalStateFile::default())
            .expect("save local state");

        let folder_id = Ulid::new();
        let duplicate_id = Ulid::new();
        let folder_dir = paths.root.join(folder_dir_name("Sample"));
        write_workspace_file(
            &paths,
            vec![ManifestItemRef {
                item_id: folder_id,
                item_type: ItemType::Folder,
                name: "Sample".to_string(),
                order: 0,
            }],
        );
        write_folder_manifest(
            &folder_dir,
            folder_id,
            "Sample",
            vec![
                ManifestItemRef {
                    item_id: duplicate_id,
                    item_type: ItemType::Request,
                    name: "First".to_string(),
                    order: 0,
                },
                ManifestItemRef {
                    item_id: duplicate_id,
                    item_type: ItemType::Request,
                    name: "Second".to_string(),
                    order: 1,
                },
            ],
        );
        write_request_payload(
            &folder_dir,
            duplicate_id,
            "First",
            HttpMethod::Get,
            "https://example.com/first",
        );
        write_request_payload(
            &folder_dir,
            duplicate_id,
            "Second",
            HttpMethod::Post,
            "https://example.com/second",
        );

        let load = startup_preload(&storage, &paths, None, vec![]);
        let StartupLoad::Ready { state, messages } = load else {
            panic!("startup should be ready");
        };

        let folder_node = state
            .shared_store
            .nodes
            .get(&folder_id)
            .expect("folder should exist");
        assert_eq!(folder_node.children, vec![duplicate_id]);
        assert_eq!(state.shared_store.requests.len(), 1);
        assert!(messages.iter().any(|message| {
            message.text.contains("Duplicate node_id")
                && message.text.contains(&duplicate_id.to_string())
        }));
    }

    #[test]
    fn startup_skips_invalid_folder_manifest_and_loads_remaining_folders() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let backend = FileSystemStorage::new(paths.clone());
        let storage = WorkspaceRepository::new(backend).expect("load workspace into memory");
        storage
            .save_workspace(&WorkspaceFile::default())
            .expect("save workspace");
        storage
            .save_local_state(&LocalStateFile::default())
            .expect("save local state");

        let valid_folder_id = Ulid::new();
        let broken_folder_id = Ulid::new();
        let valid_dir = paths.root.join(folder_dir_name("Valid"));
        let broken_dir = paths.root.join(folder_dir_name("Broken"));

        write_workspace_file(
            &paths,
            vec![
                ManifestItemRef {
                    item_id: valid_folder_id,
                    item_type: ItemType::Folder,
                    name: "Valid".to_string(),
                    order: 0,
                },
                ManifestItemRef {
                    item_id: broken_folder_id,
                    item_type: ItemType::Folder,
                    name: "Broken".to_string(),
                    order: 1,
                },
            ],
        );
        write_folder_manifest(&valid_dir, valid_folder_id, "Valid", vec![]);
        // Write broken manifest for broken folder
        fs::create_dir_all(&broken_dir).expect("create broken folder dir");
        fs::write(
            broken_dir.join(FOLDER_MANIFEST_FILE_NAME),
            "not = valid = toml",
        )
        .expect("write broken manifest");

        let load = startup_preload(&storage, &paths, None, vec![]);
        let StartupLoad::Ready { state, messages } = load else {
            panic!("startup should be ready");
        };

        assert_eq!(state.shared_store.root_ids, vec![valid_folder_id]);
        assert_eq!(state.workspace_tree.visible_rows().len(), 1);
        assert_eq!(state.workspace_tree.visible_rows()[0].id, valid_folder_id);
        assert!(messages.iter().any(|message| {
            message.text.contains("Failed to parse folder manifest")
                && message.text.contains("folder.toml")
        }));
    }

    #[test]
    fn startup_load_after_repo_folder_move_keeps_single_root_folder_and_subtree() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let backend = FileSystemStorage::new(paths.clone());
        let mut storage =
            WorkspaceRepository::new(backend.clone()).expect("load workspace into memory");
        storage.initialize().expect("initialize workspace");

        let folder_a = storage
            .create_folder(CreateFolderInput {
                parent: crate::storage::FolderParentRef { folder_id: None },
                known_parent_manifest_path: None,
                name: "Folder A".to_string(),
            })
            .expect("create folder A");
        let folder_b = storage
            .create_folder(CreateFolderInput {
                parent: crate::storage::FolderParentRef {
                    folder_id: Some(folder_a.folder.folder_id),
                },
                known_parent_manifest_path: None,
                name: "Folder B".to_string(),
            })
            .expect("create folder B");
        let request_c = storage
            .create_request(CreateRequestInput {
                parent: crate::storage::RequestParentRef {
                    folder_id: Some(folder_b.folder.folder_id),
                },
                known_parent_manifest_path: None,
                name: "Request C".to_string(),
                method: HttpMethod::Get,
                url: "https://example.com".to_string(),
            })
            .expect("create request C");

        storage
            .move_folder(MoveFolderInput {
                folder_id: folder_b.folder.folder_id,
                new_parent: crate::storage::FolderParentRef { folder_id: None },
                insertion_index: 1,
                known_folder_manifest_path: None,
                known_target_manifest_path: None,
            })
            .expect("move folder B to root");

        let load = startup_preload(&storage, &paths, None, vec![]);
        let StartupLoad::Ready { state, messages } = load else {
            panic!("startup should be ready");
        };

        let folder_a_node = state
            .shared_store
            .nodes
            .get(&folder_a.folder.folder_id)
            .expect("folder A loaded");
        let folder_b_node = state
            .shared_store
            .nodes
            .get(&folder_b.folder.folder_id)
            .expect("folder B loaded");
        let request_c_node = state
            .shared_store
            .nodes
            .get(&request_c.meta.request_id)
            .expect("request C loaded");

        assert_eq!(
            state.shared_store.root_ids,
            vec![folder_a.folder.folder_id, folder_b.folder.folder_id]
        );
        assert_eq!(
            state.workspace_tree.roots,
            vec![folder_a.folder.folder_id, folder_b.folder.folder_id]
        );
        assert!(
            folder_a_node.children.is_empty(),
            "folder A should not retain folder B"
        );
        assert_eq!(
            folder_b_node.parent_id, None,
            "folder B should be root after reload"
        );
        assert_eq!(folder_b_node.children, vec![request_c.meta.request_id]);
        assert_eq!(request_c_node.parent_id, Some(folder_b.folder.folder_id));
        assert!(
            messages
                .iter()
                .all(|message| !message.text.contains("Duplicate node_id")),
            "startup should not report duplicate node ids after a clean folder move"
        );
    }

    #[test]
    fn move_request_node_removes_from_roots_when_moving_into_folder() {
        let folder_id = Ulid::new();
        let request_id = Ulid::new();
        let mut tree = WorkspaceTreeState::default();
        tree.nodes.insert(
            folder_id,
            TreeNode {
                id: folder_id,
                name: "Folder".to_string(),
                kind: TreeNodeKind::Folder,
                request_method: None,
                request_url: None,
                manifest_path: None,
                parent_id: None,
                children: Vec::new(),
            },
        );
        tree.nodes.insert(
            request_id,
            TreeNode {
                id: request_id,
                name: "Root Request".to_string(),
                kind: TreeNodeKind::Request,
                request_method: Some(HttpMethod::Get),
                request_url: Some("https://example.com".to_string()),
                manifest_path: None,
                parent_id: None,
                children: Vec::new(),
            },
        );
        tree.roots = vec![folder_id, request_id];
        tree.set_expanded([folder_id]);

        tree.move_request_node(request_id, folder_id, 0);

        assert!(
            !tree.roots.contains(&request_id),
            "request should be removed from roots"
        );
        let folder_node = tree.nodes.get(&folder_id).unwrap();
        assert!(
            folder_node.children.contains(&request_id),
            "request should be in folder children"
        );
        let rows = tree.visible_rows();
        let ids: Vec<Ulid> = rows.iter().map(|r| r.id).collect();
        assert_eq!(
            ids.iter().filter(|&&id| id == request_id).count(),
            1,
            "request should appear exactly once"
        );
    }
}
