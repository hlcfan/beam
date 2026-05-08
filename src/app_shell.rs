use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use ulid::Ulid;

use crate::error::{BeamError, Result};
use crate::models::{
    AuthConfig, BodyConfig, CollectionFile, CollectionItemRef, EnvironmentFile, EnvironmentMeta,
    EnvironmentScope, FolderFile, HeaderField, HttpMethod, ItemType, LocalStateFile,
    QueryParamField, RequestFile,
};
use crate::paths::BeamPaths;
use crate::storage::WorkspaceStorage;

const MIN_SPLIT_RATIO: f32 = 0.1;
const MAX_SPLIT_RATIO: f32 = 0.9;

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
    pub workspace: WorkspacePlaceholderState,
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
            workspace: WorkspacePlaceholderState {
                request_panel_title: "Request".to_string(),
                response_panel_title: "Response".to_string(),
            },
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
    Ok(LocalStateFile {
        schema_version: local_state_file.local_state.schema_version,
        local_state: crate::models::LocalState {
            active_global_environment_id: local_state_file.local_state.active_global_environment_id,
            last_opened_request_id: local_state_file.local_state.last_opened_request_id,
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
    })
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
            for ancestor in tree.ancestors(request_id) {
                tree.expanded.insert(ancestor);
            }
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

            environments.push(file.environment);
        }
    }

    environments.sort_by(|a, b| {
        let scope_rank = |scope: EnvironmentScope| match scope {
            EnvironmentScope::Global => 0_u8,
            EnvironmentScope::Collection => 1_u8,
        };
        scope_rank(a.scope)
            .cmp(&scope_rank(b.scope))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| {
                a.environment_id
                    .to_string()
                    .cmp(&b.environment_id.to_string())
            })
    });
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

    use tempfile::tempdir;

    use super::*;
    use crate::models::{LocalState, TreeState, WorkspaceFile};
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
        assert!(state.collections.expanded().contains(&folder_id));
        assert!(state.collections.expanded().contains(&collection_id));
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
