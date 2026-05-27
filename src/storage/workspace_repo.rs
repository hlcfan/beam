use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::Utc;
use ulid::Ulid;

use crate::error::{BeamError, Result};
use crate::models::{
    AuthConfig, BodyConfig, EnvironmentFile, EnvironmentMeta, EnvironmentScope,
    EnvironmentVariable, FolderFile, FolderMeta, HeaderField, HttpMethod, ItemType, LocalStateFile,
    ManifestItemRef, QueryParamField, RequestDefinition, RequestFile, RequestMeta, ScriptConfig,
    WorkspaceFile,
};
use crate::paths::FOLDER_MANIFEST_FILE_NAME;
use crate::schema::{SCHEMA_VERSION_V1, SchemaKind, validate_schema_version};
use crate::storage::io_backend::StorageIoBackend;
use crate::storage::{
    BootstrapReport, CreateEnvironmentInput, CreateFolderInput, CreateRequestInput,
    DeleteRequestInput, DuplicateRequestInput, MoveFolderInput, MoveRequestInput,
    RenameRequestInput, WorkspaceStorage,
};
use crate::workspace_tree::{
    Node, NodeKind, SharedStore, assert_name_unique, ensure_parent_kind, find_unique_name,
    folder_dir_name, folder_dir_path, node_by_kind, request_file_name, request_file_path,
    scope_key,
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

    pub fn initialize(&mut self) -> Result<BootstrapReport> {
        create_required_dirs(&self.backend)?;
        let mut report = BootstrapReport::default();
        let mut created_local_state = None;

        if !self.backend.paths().workspace_file.exists() {
            self.backend.write_toml_file(
                &self.backend.paths().workspace_file,
                &WorkspaceFile::default(),
            )?;
            report.created_workspace_file = true;
        }

        if !self.backend.paths().local_state_file.exists() {
            let local_state = LocalStateFile::default();
            self.backend
                .write_toml_file(&self.backend.paths().local_state_file, &local_state)?;
            created_local_state = Some(local_state);
            report.created_local_state_file = true;
        }

        if report.created_workspace_file
            && report.created_local_state_file
            && self.store.environments.is_empty()
        {
            let default_environment = self.create_environment(CreateEnvironmentInput {
                name: "Default".to_string(),
            })?;
            let mut local_state = created_local_state.unwrap_or_default();
            local_state.local_state.active_global_environment_id =
                Some(default_environment.environment.environment_id);
            local_state.local_state.updated_at = Utc::now();
            self.backend
                .write_toml_file(&self.backend.paths().local_state_file, &local_state)?;
            report.created_default_environment = true;
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
        let request_id = Ulid::new();
        self.store.root_ids.push(request_id);
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
                parent_id: None,
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
        let parent_id = input.parent.folder_id;
        let name = input.name.trim();
        if name.is_empty() {
            return Err(BeamError::Validation {
                message: "Request name cannot be empty".to_string(),
            });
        }
        let unique_name = find_unique_name(&self.store.name_index, parent_id, name, None);
        let request_file = default_request_file(&unique_name, input.method, input.url);
        let request_id = request_file.meta.request_id;
        let now = request_file.meta.created_at;

        match parent_id {
            Some(pid) => {
                if let Some(parent_node) = self.store.nodes.get_mut(&pid) {
                    parent_node.children.push(request_id);
                }
            }
            None => {
                self.store.root_ids.push(request_id);
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
                parent_id,
                children: Vec::new(),
            },
        );
        self.store
            .name_index
            .insert(scope_key(parent_id, &request_file.meta.name), request_id);
        self.store.requests.insert(request_id, request_file.clone());

        let request_path = request_file_path(self.backend.paths(), &self.store, request_id)?;
        if let Some(parent) = request_path.parent() {
            self.backend.create_dir_all(parent)?;
        }
        self.backend.write_toml_file(&request_path, &request_file)?;

        write_parent_manifest(&self.backend, &self.store, parent_id)?;

        Ok(request_file)
    }

    pub fn create_request_after(
        &mut self,
        input: CreateRequestInput,
        source_request_id: Ulid,
    ) -> Result<RequestFile> {
        let parent_id = input.parent.folder_id;
        let name = input.name.trim();
        if name.is_empty() {
            return Err(BeamError::Validation {
                message: "Request name cannot be empty".to_string(),
            });
        }
        let unique_name = find_unique_name(&self.store.name_index, parent_id, name, None);
        let request_file = default_request_file(&unique_name, input.method, input.url);
        let request_id = request_file.meta.request_id;
        let now = request_file.meta.created_at;

        match parent_id {
            Some(pid) => {
                if let Some(parent_node) = self.store.nodes.get_mut(&pid) {
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
            }
            None => {
                if let Some(index) = self
                    .store
                    .root_ids
                    .iter()
                    .position(|&id| id == source_request_id)
                {
                    self.store.root_ids.insert(index + 1, request_id);
                } else {
                    self.store.root_ids.push(request_id);
                }
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
                parent_id,
                children: Vec::new(),
            },
        );
        self.store
            .name_index
            .insert(scope_key(parent_id, &request_file.meta.name), request_id);
        self.store.requests.insert(request_id, request_file.clone());

        let request_path = request_file_path(self.backend.paths(), &self.store, request_id)?;
        if let Some(parent) = request_path.parent() {
            self.backend.create_dir_all(parent)?;
        }
        self.backend.write_toml_file(&request_path, &request_file)?;

        write_parent_manifest(&self.backend, &self.store, parent_id)?;

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
        let parent_id = node.parent_id;
        let unique_name = find_unique_name(
            &self.store.name_index,
            parent_id,
            next_name,
            Some(input.request_id),
        );

        let existing_path = request_file_path(self.backend.paths(), &self.store, input.request_id)?;

        self.store
            .name_index
            .remove(&scope_key(parent_id, &node.name));
        if let Some(request_node) = self.store.nodes.get_mut(&input.request_id) {
            request_node.name = unique_name.clone();
            request_node.updated_at = Some(Utc::now());
        }
        self.store
            .name_index
            .insert(scope_key(parent_id, &unique_name), input.request_id);

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

        write_parent_manifest(&self.backend, &self.store, parent_id)?;

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
        let parent_id = input.parent.folder_id;
        let unique_name = find_unique_name(&self.store.name_index, parent_id, name, None);

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

        match parent_id {
            Some(pid) => {
                if let Some(parent_node) = self.store.nodes.get_mut(&pid) {
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
            }
            None => {
                if let Some(index) = self
                    .store
                    .root_ids
                    .iter()
                    .position(|&id| id == input.request_id)
                {
                    self.store.root_ids.insert(index + 1, duplicated_id);
                } else {
                    self.store.root_ids.push(duplicated_id);
                }
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
                parent_id,
                children: Vec::new(),
            },
        );
        self.store
            .name_index
            .insert(scope_key(parent_id, &duplicated.meta.name), duplicated_id);
        self.store
            .requests
            .insert(duplicated_id, duplicated.clone());

        let request_path = request_file_path(self.backend.paths(), &self.store, duplicated_id)?;
        if let Some(parent) = request_path.parent() {
            self.backend.create_dir_all(parent)?;
        }
        self.backend.write_toml_file(&request_path, &duplicated)?;

        write_parent_manifest(&self.backend, &self.store, parent_id)?;

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
        let parent_id = node.parent_id;

        let request_path = request_file_path(self.backend.paths(), &self.store, input.request_id)?;

        self.store
            .name_index
            .remove(&scope_key(parent_id, &node.name));
        self.store.nodes.remove(&input.request_id);
        self.store.requests.remove(&input.request_id);
        match parent_id {
            Some(pid) => {
                if let Some(parent_node) = self.store.nodes.get_mut(&pid) {
                    parent_node
                        .children
                        .retain(|child_id| *child_id != input.request_id);
                }
            }
            None => {
                self.store.root_ids.retain(|id| *id != input.request_id);
            }
        }

        write_parent_manifest(&self.backend, &self.store, parent_id)?;

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
        let source_parent_id = request_node.parent_id;
        let destination_parent_id = input.new_parent.folder_id;

        if let Some(dest_id) = destination_parent_id {
            ensure_parent_kind(&self.store, dest_id)?;
        }
        assert_name_unique(
            &self.store.name_index,
            destination_parent_id,
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
                .remove(&scope_key(source_parent_id, &request_node.name));
        }

        // Remove from source parent
        match source_parent_id {
            Some(src_id) => {
                if let Some(src_parent) = self.store.nodes.get_mut(&src_id) {
                    src_parent.children.retain(|id| *id != input.request_id);
                }
            }
            None => {
                self.store.root_ids.retain(|id| *id != input.request_id);
            }
        }

        // Insert into destination parent
        match destination_parent_id {
            Some(dest_id) => {
                if let Some(dest_parent) = self.store.nodes.get_mut(&dest_id) {
                    let index = input.insertion_index.min(dest_parent.children.len());
                    dest_parent.children.insert(index, input.request_id);
                }
            }
            None => {
                let index = input.insertion_index.min(self.store.root_ids.len());
                self.store.root_ids.insert(index, input.request_id);
            }
        }

        if let Some(node) = self.store.nodes.get_mut(&input.request_id) {
            node.parent_id = destination_parent_id;
        }
        if source_parent_id != destination_parent_id {
            self.store.name_index.insert(
                scope_key(destination_parent_id, &request_node.name),
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

        // Update manifests for source and destination
        write_parent_manifest(&self.backend, &self.store, source_parent_id)?;
        if source_parent_id != destination_parent_id {
            write_parent_manifest(&self.backend, &self.store, destination_parent_id)?;
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
        let source_parent_id = folder_node.parent_id;
        let destination_parent_id = input.new_parent.folder_id;

        if let Some(dest_id) = destination_parent_id {
            ensure_parent_kind(&self.store, dest_id)?;
            if dest_id == input.folder_id
                || path_has_ancestor(&self.store, dest_id, input.folder_id)
            {
                return Err(BeamError::Validation {
                    message: "Cannot move a folder into itself or its own descendant.".to_string(),
                });
            }
        }
        assert_name_unique(
            &self.store.name_index,
            destination_parent_id,
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
                .remove(&scope_key(source_parent_id, &folder_node.name));
        }

        // Remove from source parent
        match source_parent_id {
            Some(src_id) => {
                if let Some(src_parent) = self.store.nodes.get_mut(&src_id) {
                    src_parent.children.retain(|id| *id != input.folder_id);
                }
            }
            None => {
                self.store.root_ids.retain(|id| *id != input.folder_id);
            }
        }

        // Insert into destination parent
        match destination_parent_id {
            Some(dest_id) => {
                if let Some(dest_parent) = self.store.nodes.get_mut(&dest_id) {
                    let index = input.insertion_index.min(dest_parent.children.len());
                    dest_parent.children.insert(index, input.folder_id);
                }
            }
            None => {
                let index = input.insertion_index.min(self.store.root_ids.len());
                self.store.root_ids.insert(index, input.folder_id);
            }
        }

        if let Some(node) = self.store.nodes.get_mut(&input.folder_id) {
            node.parent_id = destination_parent_id;
        }
        if source_parent_id != destination_parent_id {
            self.store.name_index.insert(
                scope_key(destination_parent_id, &folder_node.name),
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

        // Update manifests for source and destination
        write_parent_manifest(&self.backend, &self.store, source_parent_id)?;
        if source_parent_id != destination_parent_id {
            write_parent_manifest(&self.backend, &self.store, destination_parent_id)?;
        }

        let folder_dir = folder_dir_path(self.backend.paths(), &self.store, input.folder_id)?;
        let manifest_path = folder_dir.join(FOLDER_MANIFEST_FILE_NAME);

        Ok(FolderFile {
            folder: FolderMeta {
                folder_id: input.folder_id,
                parent_folder_id: destination_parent_id.and_then(|pid| {
                    self.store
                        .nodes
                        .get(&pid)
                        .filter(|n| n.kind == NodeKind::Folder)
                        .map(|_| pid)
                }),
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
                scope: EnvironmentScope::Global,
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
        let parent_id = input.parent.folder_id;
        let unique_name = find_unique_name(&self.store.name_index, parent_id, name, None);

        let now = Utc::now();
        let folder_id = Ulid::new();

        match parent_id {
            Some(pid) => {
                if let Some(parent_node) = self.store.nodes.get_mut(&pid) {
                    parent_node.children.push(folder_id);
                }
            }
            None => {
                self.store.root_ids.push(folder_id);
            }
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
                parent_id,
                children: Vec::new(),
            },
        );
        self.store
            .name_index
            .insert(scope_key(parent_id, &unique_name), folder_id);

        let folder_dir = folder_dir_path(self.backend.paths(), &self.store, folder_id)?;
        self.backend.create_dir_all(&folder_dir)?;

        // Write folder's own manifest
        write_folder_manifest(&self.backend, &self.store, folder_id, &folder_dir)?;

        // Update parent manifest
        write_parent_manifest(&self.backend, &self.store, parent_id)?;

        let manifest_path = folder_dir.join(FOLDER_MANIFEST_FILE_NAME);
        Ok(FolderFile {
            folder: FolderMeta {
                folder_id,
                parent_folder_id: parent_id.and_then(|pid| {
                    self.store
                        .nodes
                        .get(&pid)
                        .filter(|n| n.kind == NodeKind::Folder)
                        .map(|_| pid)
                }),
                name: name.to_string(),
                description: None,
                created_at: now,
                updated_at: now,
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
            .map(|node| node.parent_id)
            .ok_or_else(|| BeamError::Validation {
                message: format!("folder node {folder_id} is missing from store"),
            })?;
        let unique_name = find_unique_name(
            &self.store.name_index,
            parent_id,
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
            .remove(&scope_key(parent_id, &old_name));
        if let Some(folder) = self.store.nodes.get_mut(&folder_id) {
            folder.name = unique_name.clone();
            folder.updated_at = Some(Utc::now());
        }
        self.store
            .name_index
            .insert(scope_key(parent_id, &unique_name), folder_id);

        let new_dir = folder_dir_path(self.backend.paths(), &self.store, folder_id)?;
        if new_dir != old_dir {
            self.backend.rename(&old_dir, &new_dir)?;
        }

        // Update folder's own manifest and parent manifest
        write_folder_manifest(&self.backend, &self.store, folder_id, &new_dir)?;
        write_parent_manifest(&self.backend, &self.store, parent_id)?;

        let manifest_path = new_dir.join(FOLDER_MANIFEST_FILE_NAME);
        Ok(FolderFile {
            folder: FolderMeta {
                folder_id,
                parent_folder_id: parent_id.and_then(|pid| {
                    self.store
                        .nodes
                        .get(&pid)
                        .filter(|n| n.kind == NodeKind::Folder)
                        .map(|_| pid)
                }),
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

    pub fn delete_folder(&mut self, folder_id: Ulid) -> Result<()> {
        let folder_dir = folder_dir_path(self.backend.paths(), &self.store, folder_id)?;
        let parent_id = self
            .store
            .nodes
            .get(&folder_id)
            .map(|node| node.parent_id)
            .ok_or_else(|| BeamError::Validation {
                message: format!("folder node {folder_id} is missing from store"),
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
                self.store
                    .name_index
                    .remove(&scope_key(node.parent_id, &node.name));
            }
            self.store.requests.remove(node_id);
        }

        match parent_id {
            Some(pid) => {
                if let Some(parent) = self.store.nodes.get_mut(&pid) {
                    parent.children.retain(|child_id| *child_id != folder_id);
                }
            }
            None => {
                self.store.root_ids.retain(|id| *id != folder_id);
            }
        }

        write_parent_manifest(&self.backend, &self.store, parent_id)?;

        self.backend.remove_dir_all(&folder_dir)?;
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

/// Write the manifest for a folder (folder.toml inside the folder dir).
pub fn write_folder_manifest<B: StorageIoBackend>(
    backend: &B,
    store: &SharedStore,
    folder_id: Ulid,
    folder_dir: &Path,
) -> Result<PathBuf> {
    backend.create_dir_all(folder_dir)?;

    let folder_node = node_by_kind(store, folder_id, NodeKind::Folder)?;
    let items: Vec<ManifestItemRef> = folder_node
        .children
        .iter()
        .enumerate()
        .filter_map(|(i, &child_id)| {
            store.nodes.get(&child_id).map(|child| ManifestItemRef {
                item_id: child_id,
                item_type: match child.kind {
                    NodeKind::Folder => ItemType::Folder,
                    NodeKind::Request => ItemType::Request,
                },
                name: child.name.clone(),
                order: (i as i32 + 1) * 10,
            })
        })
        .collect();

    let now = Utc::now();
    let folder_file = FolderFile {
        folder: FolderMeta {
            folder_id,
            parent_folder_id: folder_node.parent_id.and_then(|pid| {
                store
                    .nodes
                    .get(&pid)
                    .filter(|n| n.kind == NodeKind::Folder)
                    .map(|_| pid)
            }),
            name: folder_node.name.clone(),
            description: folder_node.description.clone(),
            created_at: folder_node.created_at.unwrap_or(now),
            updated_at: folder_node.updated_at.unwrap_or(now),
        },
        items,
        manifest_path: None,
    };

    let manifest_path = folder_dir.join(FOLDER_MANIFEST_FILE_NAME);
    backend.write_toml_file(&manifest_path, &folder_file)?;
    Ok(manifest_path)
}

/// Write the parent's manifest. For a folder parent this is folder.toml; for the workspace root
/// this rewrites beam.workspace.toml (items section).
fn write_parent_manifest<B: StorageIoBackend>(
    backend: &B,
    store: &SharedStore,
    parent_id: Option<Ulid>,
) -> Result<()> {
    match parent_id {
        Some(pid) => {
            let folder_dir = folder_dir_path(backend.paths(), store, pid)?;
            write_folder_manifest(backend, store, pid, &folder_dir)?;
        }
        None => {
            write_workspace_items(backend, store)?;
        }
    }
    Ok(())
}

/// Rewrite the items list in beam.workspace.toml to reflect current root ordering.
pub fn write_workspace_items<B: StorageIoBackend>(backend: &B, store: &SharedStore) -> Result<()> {
    let workspace_file_path = &backend.paths().workspace_file;
    let mut workspace_file: WorkspaceFile = if workspace_file_path.exists() {
        backend.read_toml_file(workspace_file_path)?
    } else {
        WorkspaceFile::default()
    };

    workspace_file.items = store
        .root_ids
        .iter()
        .enumerate()
        .filter_map(|(i, &id)| {
            store.nodes.get(&id).map(|node| ManifestItemRef {
                item_id: id,
                item_type: match node.kind {
                    NodeKind::Folder => ItemType::Folder,
                    NodeKind::Request => ItemType::Request,
                },
                name: node.name.clone(),
                order: (i as i32 + 1) * 10,
            })
        })
        .collect();

    backend.write_toml_file(workspace_file_path, &workspace_file)?;
    Ok(())
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

/// Persist all root items: folder manifests, request files, and workspace.toml items list.
pub fn persist_shared_tree<B: StorageIoBackend>(backend: &B, store: &SharedStore) -> Result<()> {
    for &root_id in &store.root_ids {
        if let Some(node) = store.nodes.get(&root_id) {
            match node.kind {
                NodeKind::Folder => {
                    let folder_dir = folder_dir_path(backend.paths(), store, root_id)?;
                    persist_folder_subtree(backend, store, root_id, &folder_dir)?;
                }
                NodeKind::Request => {
                    write_request_payload(backend, store, root_id)?;
                }
            }
        }
    }
    write_workspace_items(backend, store)?;
    Ok(())
}

fn persist_folder_subtree<B: StorageIoBackend>(
    backend: &B,
    store: &SharedStore,
    folder_id: Ulid,
    folder_dir: &Path,
) -> Result<()> {
    backend.create_dir_all(folder_dir)?;
    write_folder_manifest(backend, store, folder_id, folder_dir)?;

    let children = store
        .nodes
        .get(&folder_id)
        .map(|n| n.children.clone())
        .unwrap_or_default();

    for child_id in children {
        if let Some(child_node) = store.nodes.get(&child_id) {
            match child_node.kind {
                NodeKind::Folder => {
                    let child_dir = folder_dir.join(folder_dir_name(&child_node.name));
                    persist_folder_subtree(backend, store, child_id, &child_dir)?;
                }
                NodeKind::Request => {
                    write_request_payload(backend, store, child_id)?;
                }
            }
        }
    }
    Ok(())
}

pub fn create_required_dirs<B: StorageIoBackend>(backend: &B) -> Result<()> {
    for dir in [
        backend.paths().root.as_path(),
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

/// Load the full shared store from disk using the new V2 layout.
/// Reads beam.workspace.toml for root item ordering, then loads each item recursively.
fn load_full_shared_store<B: StorageIoBackend>(backend: &B) -> Result<SharedStore> {
    let mut store = SharedStore::default();

    if !backend.paths().workspace_file.exists() {
        return Ok(store);
    }

    let workspace_file: WorkspaceFile = backend.read_toml_file(&backend.paths().workspace_file)?;

    for item_ref in &workspace_file.items {
        match item_ref.item_type {
            ItemType::Folder => {
                let folder_dir = backend.paths().root.join(folder_dir_name(&item_ref.name));
                if folder_dir.exists() {
                    load_folder_into_store(
                        backend,
                        &mut store,
                        item_ref.item_id,
                        None,
                        &folder_dir,
                    )?;
                    store.root_ids.push(item_ref.item_id);
                }
            }
            ItemType::Request => {
                let request_path = backend.paths().root.join(request_file_name(&item_ref.name));
                if request_path.exists() {
                    if let Ok(request_file) = backend.read_toml_file::<RequestFile>(&request_path) {
                        if request_file.meta.request_id == item_ref.item_id {
                            store.nodes.insert(
                                item_ref.item_id,
                                Node {
                                    id: item_ref.item_id,
                                    name: request_file.meta.name.clone(),
                                    kind: NodeKind::Request,
                                    description: request_file.meta.description.clone(),
                                    created_at: Some(request_file.meta.created_at),
                                    updated_at: Some(request_file.meta.updated_at),
                                    parent_id: None,
                                    children: Vec::new(),
                                },
                            );
                            store.requests.insert(item_ref.item_id, request_file);
                            store.root_ids.push(item_ref.item_id);
                        }
                    }
                }
            }
        }
    }

    // Load environment files from the environments directory.
    let env_dir = &backend.paths().environments_dir;
    if env_dir.exists() {
        if let Ok(()) = walk_files_recursive(backend, env_dir, |path| {
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

/// Recursively load a folder and all its children into the store.
fn load_folder_into_store<B: StorageIoBackend>(
    backend: &B,
    store: &mut SharedStore,
    folder_id: Ulid,
    parent_id: Option<Ulid>,
    folder_dir: &Path,
) -> Result<()> {
    let manifest_path = folder_dir.join(FOLDER_MANIFEST_FILE_NAME);
    if !manifest_path.exists() {
        return Ok(());
    }

    let folder_file: FolderFile = backend.read_toml_file(&manifest_path)?;
    let mut child_ids = Vec::new();

    for item_ref in &folder_file.items {
        if store.nodes.contains_key(&item_ref.item_id) {
            child_ids.push(item_ref.item_id);
            continue;
        }

        match item_ref.item_type {
            ItemType::Folder => {
                let subfolder_dir = folder_dir.join(folder_dir_name(&item_ref.name));
                if subfolder_dir.exists() {
                    load_folder_into_store(
                        backend,
                        store,
                        item_ref.item_id,
                        Some(folder_id),
                        &subfolder_dir,
                    )?;
                    child_ids.push(item_ref.item_id);
                }
            }
            ItemType::Request => {
                let request_path = folder_dir.join(request_file_name(&item_ref.name));
                if request_path.exists() {
                    if let Ok(request_file) = backend.read_toml_file::<RequestFile>(&request_path) {
                        if request_file.meta.request_id == item_ref.item_id {
                            store.nodes.insert(
                                item_ref.item_id,
                                Node {
                                    id: item_ref.item_id,
                                    name: request_file.meta.name.clone(),
                                    kind: NodeKind::Request,
                                    description: request_file.meta.description.clone(),
                                    created_at: Some(request_file.meta.created_at),
                                    updated_at: Some(request_file.meta.updated_at),
                                    parent_id: Some(folder_id),
                                    children: Vec::new(),
                                },
                            );
                            store.requests.insert(item_ref.item_id, request_file);
                            child_ids.push(item_ref.item_id);
                        }
                    }
                }
            }
        }
    }

    store.nodes.insert(
        folder_id,
        Node {
            id: folder_id,
            name: folder_file.folder.name.clone(),
            kind: NodeKind::Folder,
            description: folder_file.folder.description.clone(),
            created_at: Some(folder_file.folder.created_at),
            updated_at: Some(folder_file.folder.updated_at),
            parent_id,
            children: child_ids,
        },
    );

    Ok(())
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

fn walk_files_recursive<B: StorageIoBackend, F>(
    backend: &B,
    root: &Path,
    mut visitor: F,
) -> Result<()>
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
        let mut storage =
            WorkspaceRepository::new(backend.clone()).expect("load workspace into memory");

        let report = storage.initialize().expect("initialize");
        assert!(report.created_workspace_file);
        assert!(report.created_local_state_file);
        assert!(report.created_default_environment);
        assert!(backend.paths.workspace_file.exists());
        assert!(backend.paths.local_state_file.exists());
        assert!(
            backend
                .paths
                .environments_dir
                .join("default.env.toml")
                .exists()
        );
        let local_state = storage.load_local_state().expect("load local state");
        assert!(
            local_state
                .local_state
                .active_global_environment_id
                .is_some()
        );
    }

    #[test]
    fn initialize_does_not_validate_existing_workspace_or_local_state_files() {
        let dir = tempdir().expect("tempdir");
        let backend = FileSystemStorage::new(BeamPaths::from_root(dir.path().to_path_buf()));
        let mut storage =
            WorkspaceRepository::new(backend.clone()).expect("load workspace into memory");

        let _report = storage.initialize().expect("initialize");
        std::fs::write(&backend.paths.workspace_file, "not = valid = toml")
            .expect("write workspace");
        std::fs::write(&backend.paths.local_state_file, "not = valid = toml")
            .expect("write local state");

        let report = storage.initialize().expect("initialize");

        assert!(!report.created_workspace_file);
        assert!(!report.created_local_state_file);
        assert!(!report.created_default_environment);
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
        // Sample request should exist at workspace root (no collection wrapper)
        let request_id = local_state.local_state.last_opened_request_id.unwrap();
        let request_path = backend.paths.root.join(request_file_name("Sample Request"));
        assert!(
            request_path.exists(),
            "sample request file should exist at workspace root"
        );
        // workspace.toml should list the sample request
        let workspace: WorkspaceFile = backend
            .read_toml_file(&backend.paths().workspace_file)
            .expect("load workspace");
        assert_eq!(workspace.items.len(), 1);
        assert_eq!(workspace.items[0].item_id, request_id);
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
    fn persist_shared_tree_writes_workspace_items_and_requests() {
        use crate::models::{
            AuthConfig, BodyConfig, HttpMethod, RequestDefinition, RequestFile, RequestMeta,
            ScriptConfig,
        };
        use crate::workspace_tree::{Node, NodeKind, request_file_name};
        use chrono::Utc;

        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let backend = FileSystemStorage::new(paths.clone());
        let folder_id = Ulid::new();
        let request_id = Ulid::new();
        let now = Utc::now();

        let request_file = RequestFile {
            meta: RequestMeta {
                request_id,
                name: "Get Users".to_string(),
                description: None,
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
                    folder_id,
                    Node {
                        id: folder_id,
                        name: "Users".to_string(),
                        kind: NodeKind::Folder,
                        description: None,
                        created_at: Some(now),
                        updated_at: Some(now),
                        parent_id: None,
                        children: vec![request_id],
                    },
                ),
                (
                    request_id,
                    Node {
                        id: request_id,
                        name: "Get Users".to_string(),
                        kind: NodeKind::Request,
                        description: None,
                        created_at: Some(now),
                        updated_at: Some(now),
                        parent_id: Some(folder_id),
                        children: Vec::new(),
                    },
                ),
            ]),
            requests: HashMap::from([(request_id, request_file)]),
            root_ids: vec![folder_id],
            name_index: HashMap::new(),
            environments: HashMap::new(),
        };

        // Need workspace file to exist for write_workspace_items to read from
        backend.create_dir_all(&paths.root).unwrap();
        backend
            .write_toml_file(&paths.workspace_file, &WorkspaceFile::default())
            .unwrap();

        persist_shared_tree(&backend, &store).expect("persist shared tree");

        // Verify folder.toml exists inside folder dir
        let folder_dir = paths.root.join("users");
        let folder_manifest = folder_dir.join(FOLDER_MANIFEST_FILE_NAME);
        assert!(folder_manifest.exists(), "folder.toml should exist");
        let folder_file: FolderFile = backend
            .read_toml_file(&folder_manifest)
            .expect("read folder.toml");
        assert_eq!(folder_file.items.len(), 1);
        assert_eq!(folder_file.items[0].item_id, request_id);

        // Verify request file exists inside folder dir
        let request_path = folder_dir.join(request_file_name("Get Users"));
        assert!(request_path.exists(), "request file should exist");

        // Verify workspace.toml has the folder item
        let workspace: WorkspaceFile = backend
            .read_toml_file(&paths.workspace_file)
            .expect("read workspace");
        assert_eq!(workspace.items.len(), 1);
        assert_eq!(workspace.items[0].item_id, folder_id);
        assert!(matches!(workspace.items[0].item_type, ItemType::Folder));
    }

    #[test]
    fn load_full_shared_store_reads_workspace_file_and_folder_manifests() {
        use crate::models::{
            AuthConfig, BodyConfig, HttpMethod, ItemType, ManifestItemRef, RequestDefinition,
            RequestFile, RequestMeta, ScriptConfig,
        };
        use chrono::Utc;

        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let backend = FileSystemStorage::new(paths.clone());

        backend.create_dir_all(&paths.root).unwrap();
        backend.create_dir_all(&paths.environments_dir).unwrap();

        let folder_id = Ulid::new();
        let request_id = Ulid::new();
        let now = Utc::now();

        // Create workspace file with root items
        let workspace_file = WorkspaceFile {
            schema_version: SCHEMA_VERSION_V1,
            workspace: crate::models::WorkspaceMeta {
                workspace_id: Ulid::new(),
                name: "Test".to_string(),
                description: None,
                created_at: now,
                updated_at: now,
            },
            items: vec![ManifestItemRef {
                item_id: folder_id,
                item_type: ItemType::Folder,
                name: "Users".to_string(),
                order: 10,
            }],
        };
        backend
            .write_toml_file(&paths.workspace_file, &workspace_file)
            .unwrap();

        // Create folder directory and folder.toml
        let folder_dir = paths.root.join("users");
        backend.create_dir_all(&folder_dir).unwrap();

        let folder_file = FolderFile {
            folder: FolderMeta {
                folder_id,
                parent_folder_id: None,
                name: "Users".to_string(),
                description: None,
                created_at: now,
                updated_at: now,
            },
            items: vec![ManifestItemRef {
                item_id: request_id,
                item_type: ItemType::Request,
                name: "Get User".to_string(),
                order: 10,
            }],
            manifest_path: None,
        };
        backend
            .write_toml_file(&folder_dir.join(FOLDER_MANIFEST_FILE_NAME), &folder_file)
            .unwrap();

        // Create request file
        let request_file = RequestFile {
            meta: RequestMeta {
                request_id,
                name: "Get User".to_string(),
                description: None,
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
        backend
            .write_toml_file(
                &folder_dir.join(request_file_name("Get User")),
                &request_file,
            )
            .unwrap();

        let storage = WorkspaceRepository::new(backend).expect("load workspace into memory");
        assert_eq!(storage.store.root_ids, vec![folder_id]);
        assert_eq!(storage.store.nodes.len(), 2);
        assert_eq!(storage.store.requests.len(), 1);
        assert_eq!(
            storage.store.nodes.get(&folder_id).unwrap().children,
            vec![request_id]
        );
        assert_eq!(
            storage.store.nodes.get(&request_id).unwrap().parent_id,
            Some(folder_id)
        );
    }
}
