use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use ulid::Ulid;

use crate::error::{BeamError, Result};
use crate::models::{
    AuthConfig, BodyConfig, FolderFile, FolderMeta, LocalStateFile, RequestDefinition, RequestFile,
    RequestMeta, ScriptConfig, WorkspaceFile,
};
use crate::paths::BeamPaths;
use crate::schema::{SchemaKind, validate_schema_version};
use crate::storage::{
    BootstrapReport, FolderParentRef, KnownParentManifestPath, MoveFolderInput, MoveRequestInput,
    RequestParentRef, WorkspaceStorage,
};
use crate::tree_store::{
    COLLECTION_MANIFEST_FILE_NAME, Node, NodeKind, SharedStore, assert_name_unique,
    folder_dir_path, persist_shared_tree, request_file_path, root_collection_id_of,
    shared_store_from_collection_manifest_path, write_collection_manifest,
};

#[derive(Debug, Clone)]
pub struct TomlWorkspaceStorage {
    pub paths: BeamPaths,
}

impl TomlWorkspaceStorage {
    pub fn new(paths: BeamPaths) -> Self {
        Self { paths }
    }

    pub fn bootstrap_sample_workspace_if_needed(&self) -> Result<()> {
        if self.collections_dir_has_entries()? {
            return Ok(());
        }
        let local_state = self.load_local_state()?;
        if local_state.local_state.last_opened_request_id.is_some() {
            return Ok(());
        }

        let now = Utc::now();
        let collection_id = Ulid::new();
        let request_id = Ulid::new();
        let mut store = SharedStore::default();
        store.root_ids.push(collection_id);
        store.nodes.insert(
            collection_id,
            Node {
                id: collection_id,
                name: "Sample Collection".to_string(),
                kind: NodeKind::Collection,
                description: Some("Try Beam with a sample GET request.".to_string()),
                created_at: Some(now),
                updated_at: Some(now),
                parent_id: None,
                children: vec![request_id],
            },
        );
        store.nodes.insert(
            request_id,
            Node {
                id: request_id,
                name: "Sample Request".to_string(),
                kind: NodeKind::Request,
                description: Some(
                    "Calls httpbin.org/get so you can send a request right away.".to_string(),
                ),
                created_at: Some(now),
                updated_at: Some(now),
                parent_id: Some(collection_id),
                children: Vec::new(),
            },
        );
        store.requests.insert(
            request_id,
            RequestFile {
                meta: RequestMeta {
                    request_id,
                    name: "Sample Request".to_string(),
                    description: Some(
                        "Calls httpbin.org/get so you can send a request right away.".to_string(),
                    ),
                    created_at: now,
                    updated_at: now,
                },
                request: RequestDefinition {
                    method: crate::models::HttpMethod::Get,
                    url: "https://httpbin.org/get".to_string(),
                    headers: Vec::new(),
                    query_params: Vec::new(),
                },
                auth: AuthConfig::None,
                body: BodyConfig::None,
                scripts: ScriptConfig::default(),
                file_path: None,
            },
        );
        store.rebuild_name_index();
        persist_shared_tree(&self.paths, &store)?;

        let mut local_state = local_state;
        local_state.local_state.last_opened_request_id = Some(request_id);
        local_state.local_state.updated_at = now;
        self.save_local_state(&local_state)
    }

    pub fn persist_theme_state(&self, theme_name: &str) -> Result<()> {
        let mut local_state = match self.load_local_state() {
            Ok(state) => state,
            Err(_) => LocalStateFile::default(),
        };

        let changed = local_state.local_state.theme_name.as_deref() != Some(theme_name);
        if !changed {
            return Ok(());
        }

        local_state.local_state.theme_name = Some(theme_name.to_string());
        local_state.local_state.updated_at = Utc::now();
        self.save_local_state(&local_state)
    }

    fn create_required_dirs(&self) -> Result<()> {
        for dir in [
            self.paths.root.as_path(),
            self.paths.collections_dir.as_path(),
            self.paths.environments_dir.as_path(),
            self.paths.local_dir.as_path(),
            self.paths.local_dir.join("history").as_path(),
            self.paths.local_dir.join("history/by-request").as_path(),
            self.paths.local_dir.join("history/responses").as_path(),
            self.paths.local_dir.join("script_results").as_path(),
        ] {
            fs::create_dir_all(dir).map_err(|source| BeamError::Io {
                path: dir.to_path_buf(),
                source,
            })?;
        }
        Ok(())
    }

    fn collections_dir_has_entries(&self) -> Result<bool> {
        let mut entries =
            fs::read_dir(&self.paths.collections_dir).map_err(|source| BeamError::Io {
                path: self.paths.collections_dir.clone(),
                source,
            })?;

        if let Some(entry) = entries.next() {
            entry.map_err(|source| BeamError::Io {
                path: self.paths.collections_dir.clone(),
                source,
            })?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn write_toml_file<T: serde::Serialize>(&self, path: &Path, value: &T) -> Result<()> {
        let encoded = toml::to_string_pretty(value)?;
        atomic_write(path, encoded.as_bytes())
    }

    fn read_toml_string(&self, path: &Path) -> Result<String> {
        fs::read_to_string(path).map_err(|source| BeamError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    fn parse_toml_str<T: for<'de> serde::Deserialize<'de>>(
        &self,
        path: &Path,
        content: &str,
    ) -> Result<T> {
        toml::from_str(content).map_err(|source| BeamError::TomlDecode {
            path: path.to_path_buf(),
            source,
        })
    }

    fn read_toml_file<T: for<'de> serde::Deserialize<'de>>(&self, path: &Path) -> Result<T> {
        let content = self.read_toml_string(path)?;
        self.parse_toml_str(path, &content)
    }

    fn ensure_dir(&self, path: &Path) -> Result<()> {
        fs::create_dir_all(path).map_err(|source| BeamError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    fn move_path(&self, from: &Path, to: &Path) -> Result<()> {
        fs::rename(from, to).map_err(|source| BeamError::Io {
            path: from.to_path_buf(),
            source,
        })
    }

    fn delete_file(&self, path: &Path) -> Result<()> {
        fs::remove_file(path).map_err(|source| BeamError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    fn delete_dir_all(&self, path: &Path) -> Result<()> {
        fs::remove_dir_all(path).map_err(|source| BeamError::Io {
            path: path.to_path_buf(),
            source,
        })
    }



    fn hydrate_request_file_path(&self, file: RequestFile, path: &Path) -> RequestFile {
        file.with_file_path(path)
    }


    fn walk_files_recursive<F>(&self, root: &Path, mut visitor: F) -> Result<()>
    where
        F: FnMut(&Path),
    {
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let entries = fs::read_dir(&dir).map_err(|source| BeamError::Io {
                path: dir.clone(),
                source,
            })?;
            for entry in entries {
                let entry = entry.map_err(|source| BeamError::Io {
                    path: dir.clone(),
                    source,
                })?;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
                    visitor(&path);
                }
            }
        }
        Ok(())
    }






    fn resolve_request_file_path(
        &self,
        request_id: Ulid,
        _known_request_path: Option<&Path>,
    ) -> Result<PathBuf> {
        self.find_request_file_by_id(request_id)
    }
















    fn find_request_file_by_id(&self, request_id: Ulid) -> Result<PathBuf> {
        let manifest_path = self.find_collection_manifest_path_containing_node(request_id)?;
        let store = self.load_store_from_manifest_path(&manifest_path)?;
        request_file_path(&self.paths, &store, request_id)
    }

    fn read_request_file(&self, path: &Path) -> Result<RequestFile> {
        let file = self.read_toml_file(path)?;
        Ok(self.hydrate_request_file_path(file, path))
    }




    fn collection_manifest_path_from_known_parent(
        &self,
        known_parent_manifest_path: Option<&KnownParentManifestPath>,
    ) -> Option<PathBuf> {
        known_parent_manifest_path.map(|known| match known {
            KnownParentManifestPath::Collection(path) | KnownParentManifestPath::Folder(path) => {
                path.clone()
            }
        })
    }

    fn find_collection_manifest_path_by_collection_id(
        &self,
        collection_id: Ulid,
    ) -> Result<PathBuf> {
        let mut found: Option<PathBuf> = None;
        self.walk_files_recursive(&self.paths.collections_dir, |path| {
            if path.file_name().and_then(|name| name.to_str())
                != Some(COLLECTION_MANIFEST_FILE_NAME)
            {
                return;
            }
            let Ok(store) = shared_store_from_collection_manifest_path(path) else {
                return;
            };
            if store.root_ids.first().copied() == Some(collection_id) {
                found = Some(path.to_path_buf());
            }
        })?;
        found.ok_or_else(|| BeamError::NotFound {
            entity: "collection_manifest",
            id: collection_id.to_string(),
        })
    }


    fn find_collection_manifest_path_containing_node(&self, node_id: Ulid) -> Result<PathBuf> {
        let mut found: Option<PathBuf> = None;
        self.walk_files_recursive(&self.paths.collections_dir, |path| {
            if path.file_name().and_then(|name| name.to_str())
                != Some(COLLECTION_MANIFEST_FILE_NAME)
            {
                return;
            }
            let Ok(store) = shared_store_from_collection_manifest_path(path) else {
                return;
            };
            if store.nodes.contains_key(&node_id) {
                found = Some(path.to_path_buf());
            }
        })?;
        found.ok_or_else(|| BeamError::NotFound {
            entity: "collection_manifest_for_node",
            id: node_id.to_string(),
        })
    }

    fn resolve_collection_manifest_path_for_request_parent(
        &self,
        parent: RequestParentRef,
        known_parent_manifest_path: Option<&KnownParentManifestPath>,
    ) -> Result<PathBuf> {
        if let Some(path) =
            self.collection_manifest_path_from_known_parent(known_parent_manifest_path)
        {
            return Ok(path);
        }
        if let Some(folder_id) = parent.folder_id {
            return self.find_collection_manifest_path_containing_node(folder_id);
        }
        self.find_collection_manifest_path_by_collection_id(parent.collection_id)
    }

    fn resolve_collection_manifest_path_for_folder_parent(
        &self,
        parent: FolderParentRef,
        known_parent_manifest_path: Option<&KnownParentManifestPath>,
    ) -> Result<PathBuf> {
        if let Some(path) =
            self.collection_manifest_path_from_known_parent(known_parent_manifest_path)
        {
            return Ok(path);
        }
        if let Some(parent_folder_id) = parent.parent_folder_id {
            return self.find_collection_manifest_path_containing_node(parent_folder_id);
        }
        self.find_collection_manifest_path_by_collection_id(parent.collection_id)
    }

    fn resolve_collection_manifest_path_for_request(
        &self,
        request_id: Ulid,
        _known_request_path: Option<&Path>,
        known_parent_manifest_path: Option<&KnownParentManifestPath>,
    ) -> Result<PathBuf> {
        if let Some(path) =
            self.collection_manifest_path_from_known_parent(known_parent_manifest_path)
        {
            return Ok(path);
        }
        self.find_collection_manifest_path_containing_node(request_id)
    }

    fn resolve_collection_manifest_path_for_folder(
        &self,
        folder_id: Ulid,
        known_manifest_path: Option<&Path>,
    ) -> Result<PathBuf> {
        if let Some(path) = known_manifest_path {
            return Ok(path.to_path_buf());
        }
        self.find_collection_manifest_path_containing_node(folder_id)
    }


    fn load_store_from_manifest_path(&self, manifest_path: &Path) -> Result<SharedStore> {
        shared_store_from_collection_manifest_path(manifest_path)
    }




    fn insert_child_at(parent: &mut Node, insertion_index: usize, child_id: Ulid) {
        let index = insertion_index.min(parent.children.len());
        parent.children.insert(index, child_id);
    }

    fn remove_child(parent: &mut Node, child_id: Ulid) -> Option<usize> {
        let index = parent.children.iter().position(|id| *id == child_id)?;
        parent.children.remove(index);
        Some(index)
    }

    fn ensure_parent_kind(store: &SharedStore, parent_id: Ulid) -> Result<NodeKind> {
        let parent = store
            .nodes
            .get(&parent_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "parent_node",
                id: parent_id.to_string(),
            })?;
        match parent.kind {
            NodeKind::Collection | NodeKind::Folder => Ok(parent.kind),
            NodeKind::Request => Err(BeamError::Validation {
                message: format!("request node {parent_id} cannot accept child nodes"),
            }),
        }
    }

    fn merge_shared_stores(
        &self,
        mut left: SharedStore,
        right: SharedStore,
    ) -> Result<SharedStore> {
        for (node_id, node) in right.nodes {
            if left.nodes.insert(node_id, node).is_some() {
                return Err(BeamError::Validation {
                    message: format!("duplicate node {node_id} found while merging stores"),
                });
            }
        }
        for (request_id, request) in right.requests {
            if left.requests.insert(request_id, request).is_some() {
                return Err(BeamError::Validation {
                    message: format!("duplicate request {request_id} found while merging stores"),
                });
            }
        }
        left.root_ids.extend(right.root_ids);
        let _ = left.rebuild_name_index();
        Ok(left)
    }

    fn write_collection_manifests(
        &self,
        store: &SharedStore,
        collection_ids: &[Ulid],
    ) -> Result<()> {
        let mut written = HashSet::new();
        for collection_id in collection_ids.iter().copied() {
            if written.insert(collection_id) {
                write_collection_manifest(&self.paths, store, collection_id)?;
            }
        }
        Ok(())
    }

    fn apply_child_move(
        store: &mut SharedStore,
        child_id: Ulid,
        source_parent_id: Ulid,
        destination_parent_id: Ulid,
        insertion_index: usize,
    ) -> Result<()> {
        let removed_index = {
            let source_parent =
                store
                    .nodes
                    .get_mut(&source_parent_id)
                    .ok_or_else(|| BeamError::NotFound {
                        entity: "source_parent",
                        id: source_parent_id.to_string(),
                    })?;
            Self::remove_child(source_parent, child_id).ok_or_else(|| BeamError::NotFound {
                entity: "child_in_source_parent",
                id: child_id.to_string(),
            })?
        };

        let adjusted_index =
            if source_parent_id == destination_parent_id && removed_index < insertion_index {
                insertion_index.saturating_sub(1)
            } else {
                insertion_index
            };

        let destination_parent =
            store
                .nodes
                .get_mut(&destination_parent_id)
                .ok_or_else(|| BeamError::NotFound {
                    entity: "destination_parent",
                    id: destination_parent_id.to_string(),
                })?;
        Self::insert_child_at(destination_parent, adjusted_index, child_id);
        Ok(())
    }




    fn folder_file_from_store(
        &self,
        store: &SharedStore,
        folder_id: Ulid,
        manifest_path: &Path,
    ) -> Result<FolderFile> {
        let folder = store
            .nodes
            .get(&folder_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "folder",
                id: folder_id.to_string(),
            })?;
        let collection_id =
            root_collection_id_of(store, folder_id).ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_folder",
                id: folder_id.to_string(),
            })?;
        Ok(FolderFile {
            folder: FolderMeta {
                folder_id,
                collection_id,
                parent_folder_id: folder.parent_id.filter(|parent_id| {
                    store
                        .nodes
                        .get(parent_id)
                        .is_some_and(|parent| parent.kind == NodeKind::Folder)
                }),
                name: folder.name.clone(),
                description: folder.description.clone(),
                created_at: folder.created_at.unwrap_or_else(Utc::now),
                updated_at: folder.updated_at.unwrap_or_else(Utc::now),
            },
            items: Vec::new(),
            manifest_path: Some(manifest_path.to_path_buf()),
        })
    }



    fn path_has_ancestor(store: &SharedStore, start_id: Ulid, ancestor_id: Ulid) -> bool {
        let mut cursor = Some(start_id);
        let mut seen = HashSet::new();
        while let Some(node_id) = cursor {
            if node_id == ancestor_id {
                return true;
            }
            if !seen.insert(node_id) {
                return false;
            }
            cursor = store.nodes.get(&node_id).and_then(|node| node.parent_id);
        }
        false
    }




}

impl crate::storage::io_backend::StorageIoBackend for TomlWorkspaceStorage {
    fn paths(&self) -> &BeamPaths {
        &self.paths
    }

    fn read_toml_file<T: serde::de::DeserializeOwned>(&self, path: &Path) -> Result<T> {
        TomlWorkspaceStorage::read_toml_file(self, path)
    }

    fn write_toml_file<T: serde::Serialize>(&self, path: &Path, value: &T) -> Result<()> {
        TomlWorkspaceStorage::write_toml_file(self, path, value)
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        self.ensure_dir(path)
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        self.move_path(from, to)
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        self.delete_file(path)
    }

    fn remove_dir_all(&self, path: &Path) -> Result<()> {
        self.delete_dir_all(path)
    }
}

impl WorkspaceStorage for TomlWorkspaceStorage {
    fn initialize(&self) -> Result<BootstrapReport> {
        self.create_required_dirs()?;

        let mut report = BootstrapReport::default();

        if !self.paths.workspace_file.exists() {
            self.save_workspace(&WorkspaceFile::default())?;
            report.created_workspace_file = true;
        }

        if !self.paths.local_state_file.exists() {
            self.save_local_state(&LocalStateFile::default())?;
            report.created_local_state_file = true;
        }

        Ok(report)
    }

    fn load_workspace(&self) -> Result<WorkspaceFile> {
        let content = self.read_toml_string(&self.paths.workspace_file)?;
        let workspace: WorkspaceFile = self.parse_toml_str(&self.paths.workspace_file, &content)?;
        validate_schema_version(SchemaKind::Workspace, workspace.schema_version)?;
        Ok(workspace)
    }

    fn save_workspace(&self, workspace_file: &WorkspaceFile) -> Result<()> {
        validate_schema_version(SchemaKind::Workspace, workspace_file.schema_version)?;
        self.write_toml_file(&self.paths.workspace_file, workspace_file)
    }

    fn load_local_state(&self) -> Result<LocalStateFile> {
        let content = self.read_toml_string(&self.paths.local_state_file)?;
        let local_state: LocalStateFile =
            self.parse_toml_str(&self.paths.local_state_file, &content)?;
        validate_schema_version(SchemaKind::LocalState, local_state.schema_version)?;
        Ok(local_state)
    }

    fn save_local_state(&self, local_state_file: &LocalStateFile) -> Result<()> {
        validate_schema_version(SchemaKind::LocalState, local_state_file.schema_version)?;
        self.write_toml_file(&self.paths.local_state_file, local_state_file)
    }
















    fn move_request(&self, input: MoveRequestInput) -> Result<RequestFile> {
        let source_manifest_path = self.resolve_collection_manifest_path_for_request(
            input.request_id,
            input.known_request_path.as_deref(),
            None,
        )?;
        let target_manifest_path = self.resolve_collection_manifest_path_for_request_parent(
            input.new_parent,
            input.known_target_manifest_path.as_ref(),
        )?;
        let old_request_path =
            self.resolve_request_file_path(input.request_id, input.known_request_path.as_deref())?;
        let request_file = self.read_request_file(&old_request_path)?;

        let mut store = if source_manifest_path == target_manifest_path {
            self.load_store_from_manifest_path(&source_manifest_path)?
        } else {
            let source_store = self.load_store_from_manifest_path(&source_manifest_path)?;
            let target_store = self.load_store_from_manifest_path(&target_manifest_path)?;
            self.merge_shared_stores(source_store, target_store)?
        };

        let request_node = store
            .nodes
            .get(&input.request_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "request",
                id: input.request_id.to_string(),
            })?
            .clone();
        if request_node.kind != NodeKind::Request {
            return Err(BeamError::Validation {
                message: format!("node {} is not a request", input.request_id),
            });
        }
        let source_parent_id = request_node
            .parent_id
            .ok_or_else(|| BeamError::Validation {
                message: format!("request node {} is missing parent_id", input.request_id),
            })?;
        let destination_parent_id = input
            .new_parent
            .folder_id
            .unwrap_or(input.new_parent.collection_id);
        Self::ensure_parent_kind(&store, destination_parent_id)?;
        assert_name_unique(
            &store.name_index,
            Some(destination_parent_id),
            &request_node.name,
            Some(input.request_id),
        )
        .map_err(|_| BeamError::Validation {
            message: format!(
                "A request named '{}' already exists in this scope",
                request_node.name
            ),
        })?;

        if source_parent_id != destination_parent_id {
            store.name_index.remove(&crate::tree_store::scope_key(
                Some(source_parent_id),
                &request_node.name,
            ));
        }
        Self::apply_child_move(
            &mut store,
            input.request_id,
            source_parent_id,
            destination_parent_id,
            input.insertion_index,
        )?;
        if let Some(node) = store.nodes.get_mut(&input.request_id) {
            node.parent_id = Some(destination_parent_id);
        }
        if source_parent_id != destination_parent_id {
            store.name_index.insert(
                crate::tree_store::scope_key(Some(destination_parent_id), &request_node.name),
                input.request_id,
            );
        }

        let new_request_path = request_file_path(&self.paths, &store, input.request_id)?;
        if new_request_path != old_request_path {
            if let Some(parent_dir) = new_request_path.parent() {
                fs::create_dir_all(parent_dir).map_err(|source| BeamError::Io {
                    path: parent_dir.to_path_buf(),
                    source,
                })?;
            }
            fs::rename(&old_request_path, &new_request_path).map_err(|source| BeamError::Io {
                path: old_request_path.clone(),
                source,
            })?;
        }

        let source_collection_id =
            root_collection_id_of(&store, source_parent_id).ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_request_parent",
                id: source_parent_id.to_string(),
            })?;
        let destination_collection_id = root_collection_id_of(&store, destination_parent_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_request_parent",
                id: destination_parent_id.to_string(),
            })?;
        self.write_collection_manifests(
            &store,
            &[source_collection_id, destination_collection_id],
        )?;

        Ok(request_file.with_file_path(new_request_path))
    }

    fn move_folder(&self, input: MoveFolderInput) -> Result<FolderFile> {
        let source_manifest_path = self.resolve_collection_manifest_path_for_folder(
            input.folder_id,
            input.known_folder_manifest_path.as_deref(),
        )?;
        let target_manifest_path = self.resolve_collection_manifest_path_for_folder_parent(
            input.new_parent,
            input.known_target_manifest_path.as_ref(),
        )?;

        let mut store = if source_manifest_path == target_manifest_path {
            self.load_store_from_manifest_path(&source_manifest_path)?
        } else {
            let source_store = self.load_store_from_manifest_path(&source_manifest_path)?;
            let target_store = self.load_store_from_manifest_path(&target_manifest_path)?;
            self.merge_shared_stores(source_store, target_store)?
        };

        let folder_node = store
            .nodes
            .get(&input.folder_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "folder",
                id: input.folder_id.to_string(),
            })?
            .clone();
        if folder_node.kind != NodeKind::Folder {
            return Err(BeamError::Validation {
                message: format!("node {} is not a folder", input.folder_id),
            });
        }
        let source_parent_id = folder_node.parent_id.ok_or_else(|| BeamError::Validation {
            message: format!("folder node {} is missing parent_id", input.folder_id),
        })?;
        let destination_parent_id = input
            .new_parent
            .parent_folder_id
            .unwrap_or(input.new_parent.collection_id);
        Self::ensure_parent_kind(&store, destination_parent_id)?;
        if destination_parent_id == input.folder_id
            || Self::path_has_ancestor(&store, destination_parent_id, input.folder_id)
        {
            return Err(BeamError::Validation {
                message: "Cannot move a folder into itself or its own descendant.".to_string(),
            });
        }
        assert_name_unique(
            &store.name_index,
            Some(destination_parent_id),
            &folder_node.name,
            Some(input.folder_id),
        )
        .map_err(|_| BeamError::Validation {
            message: format!(
                "A folder named '{}' already exists in this scope",
                folder_node.name
            ),
        })?;

        let old_folder_dir = folder_dir_path(&self.paths, &store, input.folder_id)?;
        if source_parent_id != destination_parent_id {
            store.name_index.remove(&crate::tree_store::scope_key(
                Some(source_parent_id),
                &folder_node.name,
            ));
        }
        Self::apply_child_move(
            &mut store,
            input.folder_id,
            source_parent_id,
            destination_parent_id,
            input.insertion_index,
        )?;
        if let Some(node) = store.nodes.get_mut(&input.folder_id) {
            node.parent_id = Some(destination_parent_id);
        }
        if source_parent_id != destination_parent_id {
            store.name_index.insert(
                crate::tree_store::scope_key(Some(destination_parent_id), &folder_node.name),
                input.folder_id,
            );
        }

        let new_folder_dir = folder_dir_path(&self.paths, &store, input.folder_id)?;
        if new_folder_dir != old_folder_dir {
            if let Some(parent_dir) = new_folder_dir.parent() {
                fs::create_dir_all(parent_dir).map_err(|source| BeamError::Io {
                    path: parent_dir.to_path_buf(),
                    source,
                })?;
            }
            fs::rename(&old_folder_dir, &new_folder_dir).map_err(|source| BeamError::Io {
                path: old_folder_dir.clone(),
                source,
            })?;
        }

        let source_collection_id =
            root_collection_id_of(&store, source_parent_id).ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_folder_parent",
                id: source_parent_id.to_string(),
            })?;
        let destination_collection_id = root_collection_id_of(&store, destination_parent_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_folder_parent",
                id: destination_parent_id.to_string(),
            })?;
        self.write_collection_manifests(
            &store,
            &[source_collection_id, destination_collection_id],
        )?;

        let manifest_path = if source_collection_id == destination_collection_id {
            source_manifest_path
        } else {
            target_manifest_path
        };
        self.folder_file_from_store(&store, input.folder_id, &manifest_path)
    }


}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp_path = temp_path(path);

    fs::write(&tmp_path, bytes).map_err(|source| BeamError::Io {
        path: tmp_path.clone(),
        source,
    })?;

    fs::rename(&tmp_path, path).map_err(|source| BeamError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(())
}

fn temp_path(path: &Path) -> PathBuf {
    let mut file_name = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "tmp".to_string());
    file_name.push_str(".tmp");
    path.with_file_name(file_name)
}


#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::models::{
        AuthConfig, BodyConfig, HttpMethod, RequestDefinition, RequestFile, RequestMeta,
        ScriptConfig,
    };
    use crate::tree_store::{
        COLLECTION_MANIFEST_FILE_NAME, CollectionManifestFile, NodeKind,
        shared_store_from_collection_manifest_path,
    };
    use chrono::Utc;
    use ulid::Ulid;

    #[test]
    fn bootstrap_creates_default_workspace_and_local_state_files() {
        let dir = tempdir().expect("tempdir");
        let storage = TomlWorkspaceStorage::new(BeamPaths::from_root(dir.path().to_path_buf()));

        let report = storage.initialize().expect("initialize");
        assert!(report.created_workspace_file);
        assert!(report.created_local_state_file);
        assert!(storage.paths.workspace_file.exists());
        assert!(storage.paths.local_state_file.exists());
    }


    #[test]
    fn bootstrap_sample_workspace_if_needed_seeds_existing_empty_workspace() {
        let dir = tempdir().expect("tempdir");
        let storage = TomlWorkspaceStorage::new(BeamPaths::from_root(dir.path().to_path_buf()));

        storage.initialize().expect("initialize");
        storage
            .bootstrap_sample_workspace_if_needed()
            .expect("bootstrap sample workspace");

        let local_state = storage.load_local_state().expect("load local state");
        assert!(local_state.local_state.last_opened_request_id.is_some());
        assert!(
            storage
                .paths
                .collections_dir
                .join("sample-collection")
                .exists()
        );
    }

    #[test]
    fn workspace_roundtrip_preserves_data() {
        let dir = tempdir().expect("tempdir");
        let storage = TomlWorkspaceStorage::new(BeamPaths::from_root(dir.path().to_path_buf()));
        storage.initialize().expect("initialize");

        let workspace = WorkspaceFile::default();
        storage.save_workspace(&workspace).expect("save workspace");
        let loaded = storage.load_workspace().expect("load workspace");

        assert_eq!(workspace, loaded);
    }

    #[test]
    fn load_local_state_ignores_nested_expanded_and_selection_fields() {
        let dir = tempdir().expect("tempdir");
        let storage = TomlWorkspaceStorage::new(BeamPaths::from_root(dir.path().to_path_buf()));
        storage.initialize().expect("initialize");

        let collection_id = Ulid::new();
        let environment_id = Ulid::new();
        let expanded_id = Ulid::new();
        let local_state_toml = format!(
            r#"
schema_version = 1

[local_state]
updated_at = "2026-01-01T00:00:00Z"
expanded_item_ids = ["{expanded_id}"]

[[local_state.collection_environment_selections]]
collection_id = "{collection_id}"
environment_id = "{environment_id}"
"#
        );
        fs::write(&storage.paths.local_state_file, local_state_toml).expect("write local state");

        let loaded = storage.load_local_state().expect("load local state");
        assert!(loaded.tree_state.expanded_item_ids.is_empty());
        assert!(loaded.collection_environment_selection.is_empty());
    }

    #[test]
    fn persist_theme_state_updates_theme_fields() {
        let dir = tempdir().expect("tempdir");
        let storage = TomlWorkspaceStorage::new(BeamPaths::from_root(dir.path().to_path_buf()));
        storage.initialize().expect("initialize");

        storage
            .persist_theme_state("One Dark")
            .expect("persist theme state");
        let loaded = storage.load_local_state().expect("load local state");

        assert_eq!(loaded.local_state.theme_name.as_deref(), Some("One Dark"));
    }

    #[test]
    fn initialize_does_not_validate_existing_workspace_or_local_state_files() {
        let dir = tempdir().expect("tempdir");
        let storage = TomlWorkspaceStorage::new(BeamPaths::from_root(dir.path().to_path_buf()));
        storage.create_required_dirs().expect("create dirs");

        fs::write(&storage.paths.workspace_file, "not = valid = toml").expect("write workspace");
        fs::write(&storage.paths.local_state_file, "not = valid = toml")
            .expect("write local state");

        let report = storage.initialize().expect("initialize");

        assert!(!report.created_workspace_file);
        assert!(!report.created_local_state_file);
    }

    #[test]
    fn request_toml_uses_explicit_auth_type_and_body_mode() {
        let request = RequestFile {
            meta: RequestMeta {
                request_id: Ulid::new(),
                name: "Get User".to_string(),
                description: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            request: RequestDefinition {
                method: HttpMethod::Get,
                url: "https://api.example.com/users/1".to_string(),
                headers: Vec::new(),
                query_params: Vec::new(),
            },
            auth: AuthConfig::ApiKey {
                key: Some("X-API-Key".to_string()),
                value: Some("secret".to_string()),
                location: crate::models::ApiKeyLocation::Header,
            },
            body: BodyConfig::Json {
                text: "{\"name\":\"Alice\"}".to_string(),
            },
            scripts: ScriptConfig::default(),
            file_path: None,
        };

        let encoded = toml::to_string_pretty(&request).expect("encode request");
        assert!(encoded.contains("[auth]"));
        assert!(encoded.contains("type = \"api_key\""));
        assert!(encoded.contains("[body]"));
        assert!(encoded.contains("mode = \"json\""));
        assert!(!encoded.contains("last_response"));
        assert!(!encoded.contains("collection_index"));
        assert!(!encoded.contains("request_index"));
    }




















}
