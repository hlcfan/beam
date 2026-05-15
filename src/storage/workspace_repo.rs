use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::Utc;
use ulid::Ulid;

use crate::error::{BeamError, Result};
use crate::models::{
    AuthConfig, BodyConfig, CollectionFile, EnvironmentFile, EnvironmentMeta, EnvironmentScope,
    EnvironmentVariable, FolderFile, FolderMeta, HeaderField, HttpMethod, LocalStateFile,
    QueryParamField, RequestDefinition, RequestFile, RequestMeta, ScriptConfig, WorkspaceFile,
};
use crate::schema::{SCHEMA_VERSION_V1, SchemaKind, validate_schema_version};
use crate::storage::io_backend::StorageIoBackend;
use crate::storage::{
    BootstrapReport, CreateEnvironmentInput, CreateFolderInput, CreateRequestInput,
    DeleteRequestInput, DuplicateRequestInput, MoveFolderInput, MoveRequestInput,
    RenameRequestInput, ReorderCollectionInput, WorkspaceStorage,
};
use crate::paths::COLLECTION_MANIFEST_FILE_NAME;
use crate::workspace_tree::{
    CollectionManifestFile, ManifestNode, Node, NodeKind,
    RootOrderFile, SharedStore, apply_child_move, assert_name_unique, collection_dir_path,
    collection_manifest_from_store, ensure_parent_kind, find_unique_name, folder_dir_name,
    folder_dir_path, node_by_id, node_by_kind, request_file_name, request_file_path,
    root_collection_id_of, root_order_file, scope_key,
};

pub struct WorkspaceRepository<B: StorageIoBackend> {
    backend: B,
    pub store: SharedStore,
}

impl<B: StorageIoBackend> WorkspaceRepository<B> {
    pub fn new(backend: B) -> Result<Self> {
        let store = load_full_shared_store(&backend)?;
        Ok(Self { backend, store })
    }

    pub fn initialize(&self) -> Result<BootstrapReport> {
        create_required_dirs(&self.backend)?;
        let mut report = BootstrapReport::default();

        if !self.backend.paths().workspace_file.exists() {
            self.backend.write_toml_file(
                &self.backend.paths().workspace_file,
                &WorkspaceFile::default(),
            )?;
            report.created_workspace_file = true;
        }

        if !self.backend.paths().local_state_file.exists() {
            self.backend.write_toml_file(
                &self.backend.paths().local_state_file,
                &LocalStateFile::default(),
            )?;
            report.created_local_state_file = true;
        }

        Ok(report)
    }

    pub fn bootstrap_sample_workspace_if_needed(&mut self) -> Result<()> {
        if !self.store.root_ids.is_empty() {
            return Ok(());
        }
        let local_state: LocalStateFile = self
            .backend
            .read_toml_file(&self.backend.paths().local_state_file)?;
        validate_schema_version(SchemaKind::LocalState, local_state.schema_version)?;
        if local_state.local_state.last_opened_request_id.is_some() {
            return Ok(());
        }

        let now = Utc::now();
        let collection_id = Ulid::new();
        let request_id = Ulid::new();
        self.store.root_ids.push(collection_id);
        self.store.nodes.insert(
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
        self.store.nodes.insert(
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
        self.store.requests.insert(
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
        self.store.rebuild_name_index();
        persist_shared_tree(&self.backend, &self.store)?;

        let mut local_state = local_state;
        local_state.local_state.last_opened_request_id = Some(request_id);
        local_state.local_state.updated_at = now;
        self.backend
            .write_toml_file(&self.backend.paths().local_state_file, &local_state)
    }

    pub fn load_request(&self, request_id: Ulid) -> Result<RequestFile> {
        self.store
            .requests
            .get(&request_id)
            .cloned()
            .ok_or_else(|| BeamError::NotFound {
                entity: "request",
                id: request_id.to_string(),
            })
    }

    pub fn create_request(&mut self, input: CreateRequestInput) -> Result<RequestFile> {
        let parent_id = input.parent.folder_id.unwrap_or(input.parent.collection_id);
        let name = input.name.trim();
        if name.is_empty() {
            return Err(BeamError::Validation {
                message: "Request name cannot be empty".to_string(),
            });
        }
        let unique_name = find_unique_name(&self.store.name_index, Some(parent_id), name, None);
        let request_file = default_request_file(&unique_name, input.method, input.url);
        let request_id = request_file.meta.request_id;
        let collection_id =
            root_collection_id_of(&self.store, parent_id).ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_request_parent",
                id: parent_id.to_string(),
            })?;
        let now = request_file.meta.created_at;

        if let Some(parent_node) = self.store.nodes.get_mut(&parent_id) {
            parent_node.children.push(request_id);
        }
        self.store.nodes.insert(
            request_id,
            Node {
                id: request_id,
                name: request_file.meta.name.clone(),
                kind: NodeKind::Request,
                description: request_file.meta.description.clone(),
                created_at: Some(now),
                updated_at: Some(request_file.meta.updated_at),
                parent_id: Some(parent_id),
                children: Vec::new(),
            },
        );
        self.store.name_index.insert(
            scope_key(Some(parent_id), &request_file.meta.name),
            request_id,
        );
        self.store.requests.insert(request_id, request_file.clone());

        let request_path = request_file_path(self.backend.paths(), &self.store, request_id)?;
        if let Some(parent) = request_path.parent() {
            self.backend.create_dir_all(parent)?;
        }
        self.backend.write_toml_file(&request_path, &request_file)?;

        let collection_dir = collection_dir_path(self.backend.paths(), &self.store, collection_id)?;
        self.backend.create_dir_all(&collection_dir)?;
        let manifest_path = collection_dir.join(COLLECTION_MANIFEST_FILE_NAME);
        let manifest = collection_manifest_from_store(&self.store, collection_id)?;
        self.backend.write_toml_file(&manifest_path, &manifest)?;

        Ok(request_file)
    }

    pub fn create_request_after(
        &mut self,
        input: CreateRequestInput,
        source_request_id: Ulid,
    ) -> Result<RequestFile> {
        let parent_id = input.parent.folder_id.unwrap_or(input.parent.collection_id);
        let name = input.name.trim();
        if name.is_empty() {
            return Err(BeamError::Validation {
                message: "Request name cannot be empty".to_string(),
            });
        }
        let unique_name = find_unique_name(&self.store.name_index, Some(parent_id), name, None);
        let request_file = default_request_file(&unique_name, input.method, input.url);
        let request_id = request_file.meta.request_id;
        let collection_id =
            root_collection_id_of(&self.store, parent_id).ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_request_parent",
                id: parent_id.to_string(),
            })?;
        let now = request_file.meta.created_at;

        if let Some(parent_node) = self.store.nodes.get_mut(&parent_id) {
            if let Some(index) = parent_node
                .children
                .iter()
                .position(|&id| id == source_request_id)
            {
                parent_node.children.insert(index + 1, request_id);
            } else {
                parent_node.children.push(request_id);
            }
        }
        self.store.nodes.insert(
            request_id,
            Node {
                id: request_id,
                name: request_file.meta.name.clone(),
                kind: NodeKind::Request,
                description: request_file.meta.description.clone(),
                created_at: Some(now),
                updated_at: Some(request_file.meta.updated_at),
                parent_id: Some(parent_id),
                children: Vec::new(),
            },
        );
        self.store.name_index.insert(
            scope_key(Some(parent_id), &request_file.meta.name),
            request_id,
        );
        self.store.requests.insert(request_id, request_file.clone());

        let request_path = request_file_path(self.backend.paths(), &self.store, request_id)?;
        if let Some(parent) = request_path.parent() {
            self.backend.create_dir_all(parent)?;
        }
        self.backend.write_toml_file(&request_path, &request_file)?;

        let collection_dir = collection_dir_path(self.backend.paths(), &self.store, collection_id)?;
        self.backend.create_dir_all(&collection_dir)?;
        let manifest_path = collection_dir.join(COLLECTION_MANIFEST_FILE_NAME);
        let manifest = collection_manifest_from_store(&self.store, collection_id)?;
        self.backend.write_toml_file(&manifest_path, &manifest)?;

        Ok(request_file)
    }

    pub fn rename_request(&mut self, input: RenameRequestInput) -> Result<RequestFile> {
        let next_name = input.new_name.trim();
        if next_name.is_empty() {
            return Err(BeamError::Validation {
                message: "Request name cannot be empty".to_string(),
            });
        }

        let node = self
            .store
            .nodes
            .get(&input.request_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "request",
                id: input.request_id.to_string(),
            })?
            .clone();
        let parent_id = node.parent_id.ok_or_else(|| BeamError::Validation {
            message: format!("request node {} is missing parent_id", input.request_id),
        })?;
        let unique_name = find_unique_name(
            &self.store.name_index,
            Some(parent_id),
            next_name,
            Some(input.request_id),
        );

        let collection_id =
            root_collection_id_of(&self.store, input.request_id).ok_or_else(|| {
                BeamError::NotFound {
                    entity: "collection_for_request",
                    id: input.request_id.to_string(),
                }
            })?;

        let existing_path = request_file_path(self.backend.paths(), &self.store, input.request_id)?;

        self.store
            .name_index
            .remove(&scope_key(Some(parent_id), &node.name));
        if let Some(request_node) = self.store.nodes.get_mut(&input.request_id) {
            request_node.name = unique_name.clone();
            request_node.updated_at = Some(Utc::now());
        }
        self.store
            .name_index
            .insert(scope_key(Some(parent_id), &unique_name), input.request_id);

        let mut request_file = self
            .store
            .requests
            .get(&input.request_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "request_file",
                id: input.request_id.to_string(),
            })?
            .clone();
        request_file.meta.name = unique_name.clone();
        request_file.meta.updated_at = Utc::now();
        self.store
            .requests
            .insert(input.request_id, request_file.clone());

        let new_path = request_file_path(self.backend.paths(), &self.store, input.request_id)?;
        self.backend.write_toml_file(&new_path, &request_file)?;

        let collection_dir = collection_dir_path(self.backend.paths(), &self.store, collection_id)?;
        self.backend.create_dir_all(&collection_dir)?;
        let manifest_path = collection_dir.join(COLLECTION_MANIFEST_FILE_NAME);
        let manifest = collection_manifest_from_store(&self.store, collection_id)?;
        self.backend.write_toml_file(&manifest_path, &manifest)?;

        if new_path != existing_path {
            let _ = self.backend.remove_file(&existing_path);
        }

        Ok(request_file)
    }

    pub fn duplicate_request(&mut self, input: DuplicateRequestInput) -> Result<RequestFile> {
        let name = input.duplicate_name.trim();
        if name.is_empty() {
            return Err(BeamError::Validation {
                message: "Duplicate request name cannot be empty".to_string(),
            });
        }
        let parent_id = input.parent.folder_id.unwrap_or(input.parent.collection_id);
        let unique_name = find_unique_name(&self.store.name_index, Some(parent_id), name, None);

        let source = self
            .store
            .requests
            .get(&input.request_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "request",
                id: input.request_id.to_string(),
            })?
            .clone();

        let now = Utc::now();
        let mut duplicated = source.clone();
        duplicated.meta.request_id = Ulid::new();
        duplicated.meta.name = unique_name;
        duplicated.meta.created_at = now;
        duplicated.meta.updated_at = now;
        let duplicated_id = duplicated.meta.request_id;

        let collection_id =
            root_collection_id_of(&self.store, parent_id).ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_request_parent",
                id: parent_id.to_string(),
            })?;

        if let Some(parent_node) = self.store.nodes.get_mut(&parent_id) {
            if let Some(index) = parent_node
                .children
                .iter()
                .position(|&id| id == input.request_id)
            {
                parent_node.children.insert(index + 1, duplicated_id);
            } else {
                parent_node.children.push(duplicated_id);
            }
        }
        self.store.nodes.insert(
            duplicated_id,
            Node {
                id: duplicated_id,
                name: duplicated.meta.name.clone(),
                kind: NodeKind::Request,
                description: duplicated.meta.description.clone(),
                created_at: Some(duplicated.meta.created_at),
                updated_at: Some(duplicated.meta.updated_at),
                parent_id: Some(parent_id),
                children: Vec::new(),
            },
        );
        self.store.name_index.insert(
            scope_key(Some(parent_id), &duplicated.meta.name),
            duplicated_id,
        );
        self.store
            .requests
            .insert(duplicated_id, duplicated.clone());

        let request_path = request_file_path(self.backend.paths(), &self.store, duplicated_id)?;
        if let Some(parent) = request_path.parent() {
            self.backend.create_dir_all(parent)?;
        }
        self.backend.write_toml_file(&request_path, &duplicated)?;

        let collection_dir = collection_dir_path(self.backend.paths(), &self.store, collection_id)?;
        self.backend.create_dir_all(&collection_dir)?;
        let manifest_path = collection_dir.join(COLLECTION_MANIFEST_FILE_NAME);
        let manifest = collection_manifest_from_store(&self.store, collection_id)?;
        self.backend.write_toml_file(&manifest_path, &manifest)?;

        Ok(duplicated)
    }

    pub fn delete_request(&mut self, input: DeleteRequestInput) -> Result<()> {
        let node = self
            .store
            .nodes
            .get(&input.request_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "request",
                id: input.request_id.to_string(),
            })?
            .clone();
        let parent_id = node.parent_id.ok_or_else(|| BeamError::Validation {
            message: format!("request node {} is missing parent_id", input.request_id),
        })?;

        let collection_id =
            root_collection_id_of(&self.store, parent_id).ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_request_parent",
                id: parent_id.to_string(),
            })?;

        let request_path = request_file_path(self.backend.paths(), &self.store, input.request_id)?;

        self.store
            .name_index
            .remove(&scope_key(Some(parent_id), &node.name));
        self.store.nodes.remove(&input.request_id);
        self.store.requests.remove(&input.request_id);
        if let Some(parent_node) = self.store.nodes.get_mut(&parent_id) {
            parent_node
                .children
                .retain(|child_id| *child_id != input.request_id);
        }

        let collection_dir = collection_dir_path(self.backend.paths(), &self.store, collection_id)?;
        let manifest_path = collection_dir.join(COLLECTION_MANIFEST_FILE_NAME);
        let manifest = collection_manifest_from_store(&self.store, collection_id)?;
        self.backend.write_toml_file(&manifest_path, &manifest)?;

        if request_path.exists() {
            let _ = self.backend.remove_file(&request_path);
        }

        Ok(())
    }

    pub fn move_request(&mut self, input: MoveRequestInput) -> Result<RequestFile> {
        let request_node = self
            .store
            .nodes
            .get(&input.request_id)
            .cloned()
            .ok_or_else(|| BeamError::NotFound {
                entity: "request",
                id: input.request_id.to_string(),
            })?;
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

        ensure_parent_kind(&self.store, destination_parent_id)?;
        assert_name_unique(
            &self.store.name_index,
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

        let old_request_path =
            request_file_path(self.backend.paths(), &self.store, input.request_id)?;

        if source_parent_id != destination_parent_id {
            self.store
                .name_index
                .remove(&scope_key(Some(source_parent_id), &request_node.name));
        }

        apply_child_move(
            &mut self.store,
            input.request_id,
            source_parent_id,
            destination_parent_id,
            input.insertion_index,
        )?;
        if let Some(node) = self.store.nodes.get_mut(&input.request_id) {
            node.parent_id = Some(destination_parent_id);
        }
        if source_parent_id != destination_parent_id {
            self.store.name_index.insert(
                scope_key(Some(destination_parent_id), &request_node.name),
                input.request_id,
            );
        }

        let new_request_path =
            request_file_path(self.backend.paths(), &self.store, input.request_id)?;
        if new_request_path != old_request_path {
            if let Some(parent_dir) = new_request_path.parent() {
                self.backend.create_dir_all(parent_dir)?;
            }
            self.backend.rename(&old_request_path, &new_request_path)?;
        }

        let source_collection_id = root_collection_id_of(&self.store, source_parent_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_request_parent",
                id: source_parent_id.to_string(),
            })?;
        let destination_collection_id = root_collection_id_of(&self.store, destination_parent_id)
            .ok_or_else(|| BeamError::NotFound {
            entity: "collection_for_request_parent",
            id: destination_parent_id.to_string(),
        })?;

        for collection_id in [source_collection_id, destination_collection_id] {
            let collection_dir =
                collection_dir_path(self.backend.paths(), &self.store, collection_id)?;
            self.backend.create_dir_all(&collection_dir)?;
            let manifest_path = collection_dir.join(COLLECTION_MANIFEST_FILE_NAME);
            let manifest = collection_manifest_from_store(&self.store, collection_id)?;
            self.backend.write_toml_file(&manifest_path, &manifest)?;
        }

        let mut request_file = self
            .store
            .requests
            .get(&input.request_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "request_file",
                id: input.request_id.to_string(),
            })?
            .clone();
        request_file.file_path = Some(new_request_path.clone());
        self.store
            .requests
            .insert(input.request_id, request_file.clone());

        Ok(request_file)
    }

    pub fn move_folder(&mut self, input: MoveFolderInput) -> Result<FolderFile> {
        let folder_node = self
            .store
            .nodes
            .get(&input.folder_id)
            .cloned()
            .ok_or_else(|| BeamError::NotFound {
                entity: "folder",
                id: input.folder_id.to_string(),
            })?;
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
        ensure_parent_kind(&self.store, destination_parent_id)?;
        if destination_parent_id == input.folder_id
            || path_has_ancestor(&self.store, destination_parent_id, input.folder_id)
        {
            return Err(BeamError::Validation {
                message: "Cannot move a folder into itself or its own descendant.".to_string(),
            });
        }
        assert_name_unique(
            &self.store.name_index,
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

        let old_folder_dir = folder_dir_path(self.backend.paths(), &self.store, input.folder_id)?;

        if source_parent_id != destination_parent_id {
            self.store
                .name_index
                .remove(&scope_key(Some(source_parent_id), &folder_node.name));
        }
        apply_child_move(
            &mut self.store,
            input.folder_id,
            source_parent_id,
            destination_parent_id,
            input.insertion_index,
        )?;
        if let Some(node) = self.store.nodes.get_mut(&input.folder_id) {
            node.parent_id = Some(destination_parent_id);
        }
        if source_parent_id != destination_parent_id {
            self.store.name_index.insert(
                scope_key(Some(destination_parent_id), &folder_node.name),
                input.folder_id,
            );
        }

        let new_folder_dir = folder_dir_path(self.backend.paths(), &self.store, input.folder_id)?;
        if new_folder_dir != old_folder_dir {
            if let Some(parent_dir) = new_folder_dir.parent() {
                self.backend.create_dir_all(parent_dir)?;
            }
            self.backend.rename(&old_folder_dir, &new_folder_dir)?;
        }

        let source_collection_id = root_collection_id_of(&self.store, source_parent_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_folder_parent",
                id: source_parent_id.to_string(),
            })?;
        let destination_collection_id = root_collection_id_of(&self.store, destination_parent_id)
            .ok_or_else(|| BeamError::NotFound {
            entity: "collection_for_folder_parent",
            id: destination_parent_id.to_string(),
        })?;

        for collection_id in [source_collection_id, destination_collection_id] {
            let collection_dir =
                collection_dir_path(self.backend.paths(), &self.store, collection_id)?;
            self.backend.create_dir_all(&collection_dir)?;
            let manifest_path = collection_dir.join(COLLECTION_MANIFEST_FILE_NAME);
            let manifest = collection_manifest_from_store(&self.store, collection_id)?;
            self.backend.write_toml_file(&manifest_path, &manifest)?;
        }

        let manifest_path =
            collection_dir_path(self.backend.paths(), &self.store, destination_collection_id)?
                .join(COLLECTION_MANIFEST_FILE_NAME);

        Ok(FolderFile {
            folder: FolderMeta {
                folder_id: input.folder_id,
                collection_id: destination_collection_id,
                parent_folder_id: if self.store.nodes.get(&destination_parent_id).map(|n| n.kind)
                    == Some(NodeKind::Folder)
                {
                    Some(destination_parent_id)
                } else {
                    None
                },
                name: folder_node.name,
                description: folder_node.description,
                created_at: folder_node.created_at.unwrap_or_else(Utc::now),
                updated_at: folder_node.updated_at.unwrap_or_else(Utc::now),
            },
            items: Vec::new(),
            manifest_path: Some(manifest_path),
        })
    }

    pub fn save_request(&mut self, request_file: &RequestFile) -> Result<()> {
        let request_id = request_file.meta.request_id;
        self.store.requests.insert(request_id, request_file.clone());
        let request_path = request_file_path(self.backend.paths(), &self.store, request_id)?;
        self.backend.write_toml_file(&request_path, request_file)
    }

    pub fn create_environment(&mut self, input: CreateEnvironmentInput) -> Result<EnvironmentFile> {
        let name = input.name.trim();
        if name.is_empty() {
            return Err(BeamError::Validation {
                message: "Environment name cannot be empty".to_string(),
            });
        }

        let collection_id = match input.scope {
            EnvironmentScope::Global => None,
            EnvironmentScope::Collection => input.collection_id,
        };
        if matches!(input.scope, EnvironmentScope::Collection) && collection_id.is_none() {
            return Err(BeamError::Validation {
                message: "Collection environment requires collection_id".to_string(),
            });
        }

        self.backend
            .create_dir_all(&self.backend.paths().environments_dir)?;
        let file_path = environment_file_path_for_name(
            &self.backend,
            &self.backend.paths().environments_dir,
            name,
            None,
        )?;
        let file_name = file_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let environment_file = EnvironmentFile {
            schema_version: SCHEMA_VERSION_V1,
            environment: EnvironmentMeta {
                environment_id: Ulid::new(),
                collection_id,
                scope: input.scope,
                name: name.to_string(),
                file_name,
                description: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            variables: Vec::new(),
            file_path: None,
        }
        .with_file_path(&file_path);
        self.backend
            .write_toml_file(&file_path, &environment_file)?;
        self.store.environments.insert(
            environment_file.environment.environment_id,
            environment_file.clone(),
        );
        Ok(environment_file)
    }

    pub fn rename_environment(
        &mut self,
        environment_id: Ulid,
        new_name: &str,
    ) -> Result<EnvironmentFile> {
        let next_name = new_name.trim();
        if next_name.is_empty() {
            return Err(BeamError::Validation {
                message: "Environment name cannot be empty".to_string(),
            });
        }

        let mut environment_file = self
            .store
            .environments
            .get(&environment_id)
            .cloned()
            .ok_or_else(|| BeamError::NotFound {
                entity: "environment",
                id: environment_id.to_string(),
            })?;

        let existing_path =
            environment_file
                .file_path
                .clone()
                .ok_or_else(|| BeamError::NotFound {
                    entity: "environment_file_path",
                    id: environment_id.to_string(),
                })?;

        environment_file.environment.name = next_name.to_string();
        environment_file.environment.updated_at = Utc::now();
        if environment_file.environment.file_name.trim().is_empty() {
            environment_file.environment.file_name = existing_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();
        }

        let parent_dir = existing_path.parent().ok_or_else(|| BeamError::NotFound {
            entity: "environment_parent_dir",
            id: existing_path.to_string_lossy().to_string(),
        })?;
        let next_path = environment_file_path_for_name(
            &self.backend,
            parent_dir,
            &environment_file.environment.name,
            Some(&existing_path),
        )?;
        let next_file_name = next_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        if next_path != existing_path {
            self.backend.rename(&existing_path, &next_path)?;
        }
        environment_file.environment.file_name = next_file_name;
        environment_file = environment_file.with_file_path(&next_path);
        self.backend
            .write_toml_file(&next_path, &environment_file)?;
        self.store
            .environments
            .insert(environment_id, environment_file.clone());
        Ok(environment_file)
    }

    pub fn update_environment_variables(
        &mut self,
        environment_id: Ulid,
        variables: Vec<EnvironmentVariable>,
    ) -> Result<EnvironmentFile> {
        let mut environment_file = self
            .store
            .environments
            .get(&environment_id)
            .cloned()
            .ok_or_else(|| BeamError::NotFound {
                entity: "environment",
                id: environment_id.to_string(),
            })?;

        let existing_path =
            environment_file
                .file_path
                .clone()
                .ok_or_else(|| BeamError::NotFound {
                    entity: "environment_file_path",
                    id: environment_id.to_string(),
                })?;

        environment_file.environment.updated_at = Utc::now();
        environment_file.variables = variables;
        if environment_file.environment.file_name.trim().is_empty() {
            environment_file.environment.file_name = existing_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();
        }
        self.backend
            .write_toml_file(&existing_path, &environment_file)?;
        self.store
            .environments
            .insert(environment_id, environment_file.clone());
        Ok(environment_file)
    }

    pub fn delete_environment(&mut self, environment_id: Ulid) -> Result<()> {
        let environment_file =
            self.store
                .environments
                .remove(&environment_id)
                .ok_or_else(|| BeamError::NotFound {
                    entity: "environment",
                    id: environment_id.to_string(),
                })?;

        if let Some(path) = environment_file.file_path {
            if path.exists() {
                self.backend.remove_file(&path)?;
            }
        }
        Ok(())
    }

    pub fn create_folder(&mut self, input: CreateFolderInput) -> Result<FolderFile> {
        let name = input.name.trim();
        if name.is_empty() {
            return Err(BeamError::Validation {
                message: "Folder name cannot be empty".to_string(),
            });
        }
        let parent_id = input
            .parent
            .parent_folder_id
            .unwrap_or(input.parent.collection_id);
        let unique_name = find_unique_name(&self.store.name_index, Some(parent_id), name, None);

        let now = Utc::now();
        let folder_id = Ulid::new();
        let collection_id =
            root_collection_id_of(&self.store, parent_id).ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_folder_parent",
                id: parent_id.to_string(),
            })?;

        if let Some(parent_node) = self.store.nodes.get_mut(&parent_id) {
            parent_node.children.push(folder_id);
        }
        self.store.nodes.insert(
            folder_id,
            Node {
                id: folder_id,
                name: unique_name.clone(),
                kind: NodeKind::Folder,
                description: None,
                created_at: Some(now),
                updated_at: Some(now),
                parent_id: Some(parent_id),
                children: Vec::new(),
            },
        );
        self.store
            .name_index
            .insert(scope_key(Some(parent_id), &unique_name), folder_id);

        let folder_dir = folder_dir_path(self.backend.paths(), &self.store, folder_id)?;
        self.backend.create_dir_all(&folder_dir)?;

        let collection_dir = collection_dir_path(self.backend.paths(), &self.store, collection_id)?;
        self.backend.create_dir_all(&collection_dir)?;
        let manifest_path = collection_dir.join(COLLECTION_MANIFEST_FILE_NAME);
        let manifest = collection_manifest_from_store(&self.store, collection_id)?;
        self.backend.write_toml_file(&manifest_path, &manifest)?;

        Ok(FolderFile {
            folder: FolderMeta {
                folder_id,
                collection_id,
                parent_folder_id: input.parent.parent_folder_id,
                name: name.to_string(),
                description: None,
                created_at: now,
                updated_at: now,
            },
            items: Vec::new(),
            manifest_path: Some(manifest_path),
        })
    }

    pub fn rename_collection(
        &mut self,
        collection_id: Ulid,
        new_name: &str,
    ) -> Result<CollectionFile> {
        let next_name = new_name.trim();
        if next_name.is_empty() {
            return Err(BeamError::Validation {
                message: "Collection name cannot be empty".to_string(),
            });
        }
        let sibling_names: Vec<String> = self
            .store
            .root_ids
            .iter()
            .filter(|&&id| id != collection_id)
            .filter_map(|id| self.store.nodes.get(id))
            .map(|node| node.name.clone())
            .collect();
        if sibling_names
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(next_name))
        {
            return Err(BeamError::Validation {
                message: format!("A collection named '{next_name}' already exists"),
            });
        }

        let old_dir = collection_dir_path(self.backend.paths(), &self.store, collection_id)?;
        let old_name = self
            .store
            .nodes
            .get(&collection_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "collection",
                id: collection_id.to_string(),
            })?
            .name
            .clone();

        self.store.name_index.remove(&scope_key(None, &old_name));
        if let Some(collection) = self.store.nodes.get_mut(&collection_id) {
            collection.name = next_name.to_string();
            collection.updated_at = Some(Utc::now());
        }
        self.store
            .name_index
            .insert(scope_key(None, next_name), collection_id);

        let new_dir = collection_dir_path(self.backend.paths(), &self.store, collection_id)?;
        if new_dir != old_dir {
            self.backend.rename(&old_dir, &new_dir)?;
        }

        let manifest_path = new_dir.join(COLLECTION_MANIFEST_FILE_NAME);
        let manifest = collection_manifest_from_store(&self.store, collection_id)?;
        self.backend.write_toml_file(&manifest_path, &manifest)?;

        Ok(CollectionFile {
            collection: crate::models::CollectionMeta {
                collection_id,
                name: next_name.to_string(),
                description: self
                    .store
                    .nodes
                    .get(&collection_id)
                    .and_then(|n| n.description.clone()),
                created_at: self
                    .store
                    .nodes
                    .get(&collection_id)
                    .and_then(|n| n.created_at)
                    .unwrap_or_else(Utc::now),
                updated_at: self
                    .store
                    .nodes
                    .get(&collection_id)
                    .and_then(|n| n.updated_at)
                    .unwrap_or_else(Utc::now),
            },
            items: Vec::new(),
            manifest_path: Some(manifest_path),
        })
    }

    pub fn rename_folder(&mut self, folder_id: Ulid, new_name: &str) -> Result<FolderFile> {
        let next_name = new_name.trim();
        if next_name.is_empty() {
            return Err(BeamError::Validation {
                message: "Folder name cannot be empty".to_string(),
            });
        }
        let parent_id = self
            .store
            .nodes
            .get(&folder_id)
            .and_then(|node| node.parent_id)
            .ok_or_else(|| BeamError::Validation {
                message: format!("folder node {folder_id} is missing parent_id"),
            })?;
        let unique_name = find_unique_name(
            &self.store.name_index,
            Some(parent_id),
            next_name,
            Some(folder_id),
        );

        let old_dir = folder_dir_path(self.backend.paths(), &self.store, folder_id)?;
        let old_name = self
            .store
            .nodes
            .get(&folder_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "folder",
                id: folder_id.to_string(),
            })?
            .name
            .clone();

        self.store
            .name_index
            .remove(&scope_key(Some(parent_id), &old_name));
        if let Some(folder) = self.store.nodes.get_mut(&folder_id) {
            folder.name = unique_name.clone();
            folder.updated_at = Some(Utc::now());
        }
        self.store
            .name_index
            .insert(scope_key(Some(parent_id), &unique_name), folder_id);

        let new_dir = folder_dir_path(self.backend.paths(), &self.store, folder_id)?;
        if new_dir != old_dir {
            self.backend.rename(&old_dir, &new_dir)?;
        }

        let collection_id =
            root_collection_id_of(&self.store, folder_id).ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_folder",
                id: folder_id.to_string(),
            })?;
        let collection_dir = collection_dir_path(self.backend.paths(), &self.store, collection_id)?;
        let manifest_path = collection_dir.join(COLLECTION_MANIFEST_FILE_NAME);
        let manifest = collection_manifest_from_store(&self.store, collection_id)?;
        self.backend.write_toml_file(&manifest_path, &manifest)?;

        Ok(FolderFile {
            folder: FolderMeta {
                folder_id,
                collection_id,
                parent_folder_id: if self.store.nodes.get(&parent_id).map(|n| n.kind)
                    == Some(NodeKind::Folder)
                {
                    Some(parent_id)
                } else {
                    None
                },
                name: next_name.to_string(),
                description: self
                    .store
                    .nodes
                    .get(&folder_id)
                    .and_then(|n| n.description.clone()),
                created_at: self
                    .store
                    .nodes
                    .get(&folder_id)
                    .and_then(|n| n.created_at)
                    .unwrap_or_else(Utc::now),
                updated_at: self
                    .store
                    .nodes
                    .get(&folder_id)
                    .and_then(|n| n.updated_at)
                    .unwrap_or_else(Utc::now),
            },
            items: Vec::new(),
            manifest_path: Some(manifest_path),
        })
    }

    pub fn delete_collection(&mut self, collection_id: Ulid) -> Result<()> {
        let collection_dir = collection_dir_path(self.backend.paths(), &self.store, collection_id)?;

        let mut nodes_to_remove = Vec::new();
        if let Some(collection) = self.store.nodes.get(&collection_id) {
            nodes_to_remove.push(collection_id);
            let mut stack = collection.children.clone();
            while let Some(node_id) = stack.pop() {
                if let Some(node) = self.store.nodes.get(&node_id) {
                    nodes_to_remove.push(node_id);
                    stack.extend(node.children.clone());
                }
            }
        }

        for node_id in &nodes_to_remove {
            if let Some(node) = self.store.nodes.remove(node_id) {
                self.store.name_index.retain(|_, id| *id != node.id);
            }
            self.store.requests.remove(node_id);
        }
        self.store.root_ids.retain(|id| *id != collection_id);

        self.backend.remove_dir_all(&collection_dir)?;
        Ok(())
    }

    pub fn delete_folder(&mut self, folder_id: Ulid) -> Result<()> {
        let folder_dir = folder_dir_path(self.backend.paths(), &self.store, folder_id)?;
        let parent_id = self
            .store
            .nodes
            .get(&folder_id)
            .and_then(|node| node.parent_id)
            .ok_or_else(|| BeamError::Validation {
                message: format!("folder node {folder_id} is missing parent_id"),
            })?;

        let mut nodes_to_remove = Vec::new();
        nodes_to_remove.push(folder_id);
        let mut stack = self
            .store
            .nodes
            .get(&folder_id)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        while let Some(node_id) = stack.pop() {
            if let Some(node) = self.store.nodes.get(&node_id) {
                nodes_to_remove.push(node_id);
                stack.extend(node.children.clone());
            }
        }

        for node_id in &nodes_to_remove {
            if let Some(node) = self.store.nodes.remove(node_id) {
                self.store.name_index.retain(|_, id| *id != node.id);
            }
            self.store.requests.remove(node_id);
        }

        if let Some(parent) = self.store.nodes.get_mut(&parent_id) {
            parent.children.retain(|child_id| *child_id != folder_id);
        }

        let collection_id =
            root_collection_id_of(&self.store, parent_id).ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_folder",
                id: folder_id.to_string(),
            })?;
        let collection_dir = collection_dir_path(self.backend.paths(), &self.store, collection_id)?;
        let manifest_path = collection_dir.join(COLLECTION_MANIFEST_FILE_NAME);
        let manifest = collection_manifest_from_store(&self.store, collection_id)?;
        self.backend.write_toml_file(&manifest_path, &manifest)?;

        self.backend.remove_dir_all(&folder_dir)?;
        Ok(())
    }

    pub fn reorder_collection(&mut self, input: ReorderCollectionInput) -> Result<()> {
        let Some(current_index) = self
            .store
            .root_ids
            .iter()
            .position(|id| *id == input.collection_id)
        else {
            return Err(BeamError::NotFound {
                entity: "collection",
                id: input.collection_id.to_string(),
            });
        };
        let root_id = self.store.root_ids.remove(current_index);
        let adjusted_index = if current_index < input.insertion_index {
            input.insertion_index.saturating_sub(1)
        } else {
            input.insertion_index
        };
        let index = adjusted_index.min(self.store.root_ids.len());
        self.store.root_ids.insert(index, root_id);

        let root_order = RootOrderFile {
            schema_version: SCHEMA_VERSION_V1,
            root_ids: self.store.root_ids.clone(),
        };
        self.backend
            .create_dir_all(&self.backend.paths().collections_dir)?;
        self.backend.write_toml_file(
            &self.backend.paths().collections_root_order_file,
            &root_order,
        )?;

        Ok(())
    }

    pub fn load_workspace(&self) -> Result<WorkspaceFile> {
        self.backend
            .read_toml_file(&self.backend.paths().workspace_file)
    }

    pub fn save_workspace(&self, workspace_file: &WorkspaceFile) -> Result<()> {
        self.backend
            .write_toml_file(&self.backend.paths().workspace_file, workspace_file)
    }

    pub fn load_local_state(&self) -> Result<LocalStateFile> {
        self.backend
            .read_toml_file(&self.backend.paths().local_state_file)
    }

    pub fn save_local_state(&self, local_state_file: &LocalStateFile) -> Result<()> {
        self.backend
            .write_toml_file(&self.backend.paths().local_state_file, local_state_file)
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
}

impl<B: StorageIoBackend> WorkspaceStorage for WorkspaceRepository<B> {
    fn load_workspace(&self) -> Result<WorkspaceFile> {
        WorkspaceRepository::load_workspace(self)
    }

    fn save_workspace(&self, workspace_file: &WorkspaceFile) -> Result<()> {
        WorkspaceRepository::save_workspace(self, workspace_file)
    }

    fn load_local_state(&self) -> Result<LocalStateFile> {
        WorkspaceRepository::load_local_state(self)
    }

    fn save_local_state(&self, local_state_file: &LocalStateFile) -> Result<()> {
        WorkspaceRepository::save_local_state(self, local_state_file)
    }
}

pub fn write_collection_manifest<B: StorageIoBackend>(
    backend: &B,
    store: &SharedStore,
    collection_id: ulid::Ulid,
) -> Result<PathBuf> {
    let collection_dir = collection_dir_path(backend.paths(), store, collection_id)?;
    backend.create_dir_all(&collection_dir)?;

    let manifest_path = collection_dir.join(COLLECTION_MANIFEST_FILE_NAME);
    let manifest = collection_manifest_from_store(store, collection_id)?;
    backend.write_toml_file(&manifest_path, &manifest)?;
    Ok(manifest_path)
}

pub fn write_request_payload<B: StorageIoBackend>(
    backend: &B,
    store: &SharedStore,
    request_id: ulid::Ulid,
) -> Result<PathBuf> {
    let request_path = request_file_path(backend.paths(), store, request_id)?;
    let request_dir = request_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| BeamError::Validation {
            message: format!(
                "request path {} has no parent directory",
                request_path.display()
            ),
        })?;
    backend.create_dir_all(&request_dir)?;

    let mut request_file =
        store
            .requests
            .get(&request_id)
            .cloned()
            .ok_or_else(|| BeamError::NotFound {
                entity: "request_file",
                id: request_id.to_string(),
            })?;
    let request_node = node_by_kind(store, request_id, NodeKind::Request)?;
    if request_file.meta.request_id != request_id {
        return Err(BeamError::Validation {
            message: format!(
                "request payload {} declared request_id {}",
                request_id, request_file.meta.request_id
            ),
        });
    }

    request_file.meta.name = request_node.name.clone();
    request_file.file_path = Some(request_path.clone());
    backend.write_toml_file(&request_path, &request_file)?;
    Ok(request_path)
}

pub fn write_root_order<B: StorageIoBackend>(backend: &B, store: &SharedStore) -> Result<PathBuf> {
    backend.create_dir_all(&backend.paths().collections_dir)?;
    backend.write_toml_file(
        &backend.paths().collections_root_order_file,
        &root_order_file(store),
    )?;
    Ok(backend.paths().collections_root_order_file.clone())
}

pub fn persist_collection_subtree<B: StorageIoBackend>(
    backend: &B,
    store: &SharedStore,
    collection_id: ulid::Ulid,
) -> Result<PathBuf> {
    let manifest_path = write_collection_manifest(backend, store, collection_id)?;
    write_request_payloads_in_subtree(backend, store, collection_id)?;
    Ok(manifest_path)
}

pub fn persist_shared_tree<B: StorageIoBackend>(backend: &B, store: &SharedStore) -> Result<()> {
    for collection_id in store.root_ids.iter().copied() {
        persist_collection_subtree(backend, store, collection_id)?;
    }
    write_root_order(backend, store)?;
    Ok(())
}

fn write_request_payloads_in_subtree<B: StorageIoBackend>(
    backend: &B,
    store: &SharedStore,
    node_id: ulid::Ulid,
) -> Result<()> {
    let node = node_by_id(store, node_id)?;
    match node.kind {
        NodeKind::Collection | NodeKind::Folder => {
            for child_id in node.children.iter().copied() {
                write_request_payloads_in_subtree(backend, store, child_id)?;
            }
        }
        NodeKind::Request => {
            write_request_payload(backend, store, node_id)?;
        }
    }
    Ok(())
}

pub fn create_required_dirs<B: StorageIoBackend>(backend: &B) -> Result<()> {
    for dir in [
        backend.paths().root.as_path(),
        backend.paths().collections_dir.as_path(),
        backend.paths().environments_dir.as_path(),
        backend.paths().local_dir.as_path(),
        backend.paths().local_dir.join("history").as_path(),
        backend
            .paths()
            .local_dir
            .join("history/by-request")
            .as_path(),
        backend
            .paths()
            .local_dir
            .join("history/responses")
            .as_path(),
        backend.paths().local_dir.join("script_results").as_path(),
    ] {
        backend.create_dir_all(dir)?;
    }
    Ok(())
}

fn default_request_file(name: &str, method: HttpMethod, url: String) -> RequestFile {
    let now = Utc::now();
    RequestFile {
        meta: RequestMeta {
            request_id: Ulid::new(),
            name: name.to_string(),
            description: None,
            created_at: now,
            updated_at: now,
        },
        request: RequestDefinition {
            method,
            url,
            headers: vec![
                HeaderField {
                    // TODO: is it better to put these strings in a constant?
                    name: "Content-Type".to_string(),
                    value: "application/json".to_string(),
                    enabled: true,
                    description: None,
                    secret: false,
                },
                HeaderField {
                    name: "User-Agent".to_string(),
                    value: "BeamApp/1.0".to_string(),
                    enabled: true,
                    description: None,
                    secret: false,
                },
            ],
            query_params: vec![QueryParamField {
                name: String::new(),
                value: String::new(),
                enabled: true,
                description: None,
            }],
        },
        auth: crate::models::AuthConfig::None,
        body: crate::models::BodyConfig::None,
        scripts: ScriptConfig::default(),
        file_path: None,
    }
}

fn load_full_shared_store<B: StorageIoBackend>(backend: &B) -> Result<SharedStore> {
    let mut store = SharedStore::default();
    let collections_dir = &backend.paths().collections_dir;

    if !collections_dir.exists() {
        return Ok(store);
    }

    let entries = backend.read_dir(collections_dir)?;

    for path in entries {
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join(COLLECTION_MANIFEST_FILE_NAME);
        if !manifest_path.exists() {
            continue;
        }

        let manifest: CollectionManifestFile = backend.read_toml_file(&manifest_path)?;
        if manifest.kind != NodeKind::Collection {
            continue;
        }

        let collection_id = manifest.id;
        if store.nodes.contains_key(&collection_id) {
            continue;
        }

        store.nodes.insert(
            collection_id,
            Node {
                id: collection_id,
                name: manifest.name.clone(),
                kind: NodeKind::Collection,
                description: manifest.description,
                created_at: manifest.created_at,
                updated_at: manifest.updated_at,
                parent_id: None,
                children: Vec::new(),
            },
        );
        store.root_ids.push(collection_id);

        let collection_dir = &path;
        let child_ids = load_manifest_children(
            backend,
            &mut store,
            &manifest.children,
            collection_id,
            collection_dir,
        )?;
        if let Some(collection) = store.nodes.get_mut(&collection_id) {
            collection.children = child_ids;
        }
    }

    if backend.paths().collections_root_order_file.exists() {
        if let Ok(root_order) =
            backend.read_toml_file::<RootOrderFile>(&backend.paths().collections_root_order_file)
        {
            let available_ids: HashSet<_> = store.root_ids.iter().copied().collect();
            let mut ordered_ids = Vec::with_capacity(store.root_ids.len());
            let mut seen = HashSet::new();
            for root_id in root_order.root_ids {
                if available_ids.contains(&root_id) && seen.insert(root_id) {
                    ordered_ids.push(root_id);
                }
            }
            for root_id in store.root_ids.iter().copied() {
                if seen.insert(root_id) {
                    ordered_ids.push(root_id);
                }
            }
            store.root_ids = ordered_ids;
        }
    }

    // Load environment files into memory for O(1) lookups.
    for root in [
        &backend.paths().environments_dir,
        &backend.paths().collections_dir,
    ] {
        if !root.exists() {
            continue;
        }
        if let Ok(()) = walk_files_recursive(backend, root, |path| {
            if !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".env.toml"))
            {
                return;
            }
            if let Ok(file) = backend.read_toml_file::<EnvironmentFile>(path) {
                store
                    .environments
                    .insert(file.environment.environment_id, file.with_file_path(path));
            }
        }) {}
    }

    store.rebuild_name_index();
    Ok(store)
}

fn load_manifest_children<B: StorageIoBackend>(
    backend: &B,
    store: &mut SharedStore,
    children: &[ManifestNode],
    parent_id: Ulid,
    parent_dir: &Path,
) -> Result<Vec<Ulid>> {
    let mut child_ids = Vec::new();
    for child in children {
        if child.kind == NodeKind::Collection {
            continue;
        }
        if store.nodes.contains_key(&child.id) {
            continue;
        }

        let node_id = child.id;
        let mut node_children = Vec::new();

        if child.kind == NodeKind::Request {
            let request_path = parent_dir.join(request_file_name(&child.name));
            if request_path.exists() {
                if let Ok(request_file) = backend.read_toml_file::<RequestFile>(&request_path) {
                    if request_file.meta.request_id == node_id {
                        store.requests.insert(node_id, request_file);
                    }
                }
            }
        } else if child.kind == NodeKind::Folder {
            let folder_dir = parent_dir.join(folder_dir_name(&child.name));
            node_children =
                load_manifest_children(backend, store, &child.children, node_id, &folder_dir)?;
        }

        store.nodes.insert(
            node_id,
            Node {
                id: node_id,
                name: child.name.clone(),
                kind: child.kind,
                description: child.description.clone(),
                created_at: child.created_at,
                updated_at: child.updated_at,
                parent_id: Some(parent_id),
                children: node_children,
            },
        );
        store
            .name_index
            .insert(scope_key(Some(parent_id), &child.name), node_id);
        child_ids.push(node_id);
    }
    Ok(child_ids)
}

fn environment_file_path_for_name<B: StorageIoBackend>(
    backend: &B,
    dir: &Path,
    environment_name: &str,
    exclude_path: Option<&Path>,
) -> Result<std::path::PathBuf> {
    let preferred_stem = slugify(environment_name);
    let excluded = exclude_path.and_then(|path| path.file_name().map(|name| name.to_owned()));
    let mut used_names = HashSet::new();
    for path in backend.read_dir(dir)? {
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name() else {
            continue;
        };
        if excluded
            .as_ref()
            .is_some_and(|excluded_name| excluded_name == file_name)
        {
            continue;
        }
        used_names.insert(file_name.to_string_lossy().to_string());
    }

    let mut suffix = 1_u32;
    loop {
        let file_name = if suffix == 1 {
            format!("{preferred_stem}.env.toml")
        } else {
            format!("{preferred_stem}-{suffix}.env.toml")
        };
        if !used_names.contains(&file_name) {
            return Ok(dir.join(file_name));
        }
        suffix += 1;
    }
}

fn walk_files_recursive<B: StorageIoBackend, F>(backend: &B, root: &Path, mut visitor: F) -> Result<()>
where
    F: FnMut(&Path),
{
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let paths = backend.read_dir(&dir)?;
        for path in paths {
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                visitor(&path);
            }
        }
    }
    Ok(())
}

fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_dash = false;
    for ch in input.chars() {
        let lowered = ch.to_ascii_lowercase();
        if lowered.is_ascii_alphanumeric() {
            out.push(lowered);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let normalized = out.trim_matches('-').to_string();
    if normalized.is_empty() {
        "request".to_string()
    } else {
        normalized
    }
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::paths::BeamPaths;
    use crate::storage::fs_backend::FileSystemStorage;
    use tempfile::tempdir;

    #[test]
    fn bootstrap_creates_default_workspace_and_local_state_files() {
        let dir = tempdir().expect("tempdir");
        let backend = FileSystemStorage::new(BeamPaths::from_root(dir.path().to_path_buf()));
        let storage =
            WorkspaceRepository::new(backend.clone()).expect("load workspace into memory");

        let report = storage.initialize().expect("initialize");
        assert!(report.created_workspace_file);
        assert!(report.created_local_state_file);
        assert!(backend.paths.workspace_file.exists());
        assert!(backend.paths.local_state_file.exists());
    }

    #[test]
    fn initialize_does_not_validate_existing_workspace_or_local_state_files() {
        let dir = tempdir().expect("tempdir");
        let backend = FileSystemStorage::new(BeamPaths::from_root(dir.path().to_path_buf()));
        let storage =
            WorkspaceRepository::new(backend.clone()).expect("load workspace into memory");

        let _report = storage.initialize().expect("initialize");
        std::fs::write(&backend.paths.workspace_file, "not = valid = toml")
            .expect("write workspace");
        std::fs::write(&backend.paths.local_state_file, "not = valid = toml")
            .expect("write local state");

        let report = storage.initialize().expect("initialize");

        assert!(!report.created_workspace_file);
        assert!(!report.created_local_state_file);
    }

    #[test]
    fn bootstrap_sample_workspace_if_needed_seeds_existing_empty_workspace() {
        let dir = tempdir().expect("tempdir");
        let backend = FileSystemStorage::new(BeamPaths::from_root(dir.path().to_path_buf()));

        let mut storage =
            WorkspaceRepository::new(backend.clone()).expect("load workspace into memory");
        storage.initialize().expect("initialize");
        storage
            .bootstrap_sample_workspace_if_needed()
            .expect("bootstrap sample workspace");

        let local_state: LocalStateFile = backend
            .read_toml_file(&backend.paths().local_state_file)
            .expect("load local state");
        assert!(local_state.local_state.last_opened_request_id.is_some());
        assert!(
            backend
                .paths
                .collections_dir
                .join("sample-collection")
                .exists()
        );
    }

    #[test]
    fn workspace_roundtrip_preserves_data() {
        let dir = tempdir().expect("tempdir");
        let backend = FileSystemStorage::new(BeamPaths::from_root(dir.path().to_path_buf()));
        let storage = WorkspaceRepository::new(backend).expect("load workspace into memory");

        let workspace = WorkspaceFile::default();
        storage.save_workspace(&workspace).expect("save workspace");
        let loaded = storage.load_workspace().expect("load workspace");

        assert_eq!(workspace, loaded);
    }

    #[test]
    fn load_local_state_ignores_nested_expanded_and_selection_fields() {
        let dir = tempdir().expect("tempdir");
        let backend = FileSystemStorage::new(BeamPaths::from_root(dir.path().to_path_buf()));
        let storage =
            WorkspaceRepository::new(backend.clone()).expect("load workspace into memory");

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
        std::fs::create_dir_all(backend.paths().local_state_file.parent().unwrap())
            .expect("create beam_local dir");
        std::fs::write(&backend.paths().local_state_file, local_state_toml)
            .expect("write local state");

        let loaded = storage.load_local_state().expect("load local state");
        assert!(loaded.tree_state.expanded_item_ids.is_empty());
        assert!(loaded.collection_environment_selection.is_empty());
    }

    #[test]
    fn persist_theme_state_updates_theme_fields() {
        let dir = tempdir().expect("tempdir");
        let backend = FileSystemStorage::new(BeamPaths::from_root(dir.path().to_path_buf()));
        let storage = WorkspaceRepository::new(backend).expect("load workspace into memory");
        storage
            .save_local_state(&LocalStateFile::default())
            .expect("save local state");

        storage
            .persist_theme_state("One Dark")
            .expect("persist theme state");
        let loaded = storage.load_local_state().expect("load local state");

        assert_eq!(loaded.local_state.theme_name.as_deref(), Some("One Dark"));
    }

    #[test]
    fn request_toml_uses_explicit_auth_type_and_body_mode() {
        use chrono::Utc;
        use ulid::Ulid;

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

    #[test]
    fn persist_shared_tree_writes_root_order_manifest_and_requests() {
        use crate::models::{AuthConfig, BodyConfig, HttpMethod, RequestDefinition, RequestFile, RequestMeta, ScriptConfig};
        use crate::paths::COLLECTION_MANIFEST_FILE_NAME;
        use crate::workspace_tree::{Node, NodeKind, request_file_name};
        use chrono::Utc;

        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let backend = FileSystemStorage::new(paths.clone());
        let first_collection_id = Ulid::new();
        let second_collection_id = Ulid::new();
        let folder_id = Ulid::new();
        let request_id = Ulid::new();
        let now = Utc::now();

        let request_file = RequestFile {
            meta: RequestMeta {
                request_id,
                name: "Outdated Name".to_string(),
                description: Some("Request description".to_string()),
                created_at: now,
                updated_at: now,
            },
            request: RequestDefinition {
                method: HttpMethod::Get,
                url: "https://example.com/users".to_string(),
                headers: Vec::new(),
                query_params: Vec::new(),
            },
            auth: AuthConfig::None,
            body: BodyConfig::None,
            scripts: ScriptConfig::default(),
            file_path: None,
        };
        let store = SharedStore {
            nodes: HashMap::from([
                (
                    first_collection_id,
                    Node {
                        id: first_collection_id,
                        name: "First".to_string(),
                        kind: NodeKind::Collection,
                        description: Some("First collection".to_string()),
                        created_at: Some(now),
                        updated_at: Some(now),
                        parent_id: None,
                        children: Vec::new(),
                    },
                ),
                (
                    second_collection_id,
                    Node {
                        id: second_collection_id,
                        name: "Second".to_string(),
                        kind: NodeKind::Collection,
                        description: Some("Second collection".to_string()),
                        created_at: Some(now),
                        updated_at: Some(now),
                        parent_id: None,
                        children: vec![folder_id],
                    },
                ),
                (
                    folder_id,
                    Node {
                        id: folder_id,
                        name: "Users".to_string(),
                        kind: NodeKind::Folder,
                        description: Some("Folder".to_string()),
                        created_at: Some(now),
                        updated_at: Some(now),
                        parent_id: Some(second_collection_id),
                        children: vec![request_id],
                    },
                ),
                (
                    request_id,
                    Node {
                        id: request_id,
                        name: "Get Users".to_string(),
                        kind: NodeKind::Request,
                        description: Some("Request node".to_string()),
                        created_at: Some(now),
                        updated_at: Some(now),
                        parent_id: Some(folder_id),
                        children: Vec::new(),
                    },
                ),
            ]),
            requests: HashMap::from([(request_id, request_file)]),
            root_ids: vec![second_collection_id, first_collection_id],
            name_index: HashMap::new(),
            environments: HashMap::new(),
        };

        persist_shared_tree(&backend, &store).expect("persist shared tree");

        let manifest_path = paths
            .collections_dir
            .join("second")
            .join(COLLECTION_MANIFEST_FILE_NAME);
        let encoded_manifest = std::fs::read_to_string(&manifest_path).expect("read manifest");
        let manifest: CollectionManifestFile =
            toml::from_str(&encoded_manifest).expect("decode manifest");
        assert_eq!(manifest.id, second_collection_id);
        assert_eq!(manifest.children[0].id, folder_id);
        assert_eq!(manifest.children[0].children[0].id, request_id);

        let request_path = paths
            .collections_dir
            .join("second")
            .join("users")
            .join(request_file_name("Get Users"));
        let encoded_request = std::fs::read_to_string(&request_path).expect("read request");
        let persisted_request: RequestFile =
            toml::from_str(&encoded_request).expect("decode request");
        assert_eq!(persisted_request.meta.name, "Get Users");

        let encoded_root_order =
            std::fs::read_to_string(&paths.collections_root_order_file).expect("read root order");
        let root_order: RootOrderFile =
            toml::from_str(&encoded_root_order).expect("decode root order");
        assert_eq!(
            root_order.root_ids,
            vec![second_collection_id, first_collection_id]
        );
    }

    #[test]
    fn load_full_shared_store_reads_manifest_and_request_files() {
        use crate::models::{AuthConfig, BodyConfig, HttpMethod, RequestDefinition, RequestFile, RequestMeta, ScriptConfig};
        use crate::workspace_tree::{CollectionManifestFile, ManifestNode, NodeKind};
        use chrono::Utc;

        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let backend = FileSystemStorage::new(paths.clone());
        backend.create_dir_all(&paths.collections_dir).unwrap();

        let collection_id = Ulid::new();
        let folder_id = Ulid::new();
        let request_id = Ulid::new();
        let manifest = CollectionManifestFile {
            schema_version: crate::schema::SCHEMA_VERSION_V1,
            id: collection_id,
            name: "API".to_string(),
            kind: NodeKind::Collection,
            description: None,
            created_at: None,
            updated_at: None,
            children: vec![ManifestNode {
                id: folder_id,
                name: "Users".to_string(),
                kind: NodeKind::Folder,
                description: None,
                created_at: None,
                updated_at: None,
                children: vec![ManifestNode {
                    id: request_id,
                    name: "Get User".to_string(),
                    kind: NodeKind::Request,
                    description: None,
                    created_at: None,
                    updated_at: None,
                    children: Vec::new(),
                }],
            }],
        };

        let collection_dir = paths.collections_dir.join("api");
        backend.create_dir_all(&collection_dir).unwrap();
        backend.write_toml_file(&collection_dir.join(COLLECTION_MANIFEST_FILE_NAME), &manifest).unwrap();

        let request_file = RequestFile {
            meta: RequestMeta {
                request_id,
                name: "Get User".to_string(),
                description: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            request: RequestDefinition {
                method: HttpMethod::Get,
                url: "https://example.com/users".to_string(),
                headers: Vec::new(),
                query_params: Vec::new(),
            },
            auth: AuthConfig::None,
            body: BodyConfig::None,
            scripts: ScriptConfig::default(),
            file_path: None,
        };
        let request_path = collection_dir.join("users").join("get-user.request.toml");
        backend.create_dir_all(request_path.parent().unwrap()).unwrap();
        backend.write_toml_file(&request_path, &request_file).unwrap();

        let storage = WorkspaceRepository::new(backend).expect("load workspace into memory");
        assert_eq!(storage.store.root_ids, vec![collection_id]);
        assert_eq!(storage.store.nodes.len(), 3);
        assert_eq!(storage.store.requests.len(), 1);
        assert_eq!(
            storage.store.nodes.get(&collection_id).unwrap().children,
            vec![folder_id]
        );
        assert_eq!(
            storage.store.nodes.get(&folder_id).unwrap().children,
            vec![request_id]
        );
    }
}
