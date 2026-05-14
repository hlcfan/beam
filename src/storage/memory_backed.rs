use std::collections::HashSet;
use std::path::Path;

use chrono::Utc;
use ulid::Ulid;

use crate::error::{BeamError, Result};
use crate::models::{
    CollectionFile, EnvironmentFile, EnvironmentMeta, EnvironmentScope, EnvironmentVariable,
    FolderFile, FolderMeta, HeaderField, HttpMethod, QueryParamField, RequestDefinition,
    RequestFile, RequestMeta, ScriptConfig,
};
use crate::schema::SCHEMA_VERSION_V1;
use crate::storage::{
    CreateEnvironmentInput, CreateFolderInput, CreateRequestInput, DeleteRequestInput,
    DuplicateRequestInput, RenameRequestInput, ReorderCollectionInput,
};
use crate::storage::io_backend::StorageIoBackend;
use crate::tree_store::{
    COLLECTION_MANIFEST_FILE_NAME, CollectionManifestFile, ManifestNode, Node, NodeKind,
    RootOrderFile, SharedStore, assert_name_unique, collection_dir_path,
    collection_manifest_from_store, folder_dir_name, folder_dir_path, request_file_name,
    request_file_path, root_collection_id_of, scope_key,
};

pub struct MemoryBackedStorage<B: StorageIoBackend> {
    backend: B,
    pub store: SharedStore,
}

impl<B: StorageIoBackend> MemoryBackedStorage<B> {
    pub fn new(backend: B) -> Result<Self> {
        let store = load_full_shared_store(&backend)?;
        Ok(Self { backend, store })
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
        // TODO: if name exists, can we append an incremental suffix like " (1, 2, 3, etc.)?
        assert_name_unique(&self.store.name_index, Some(parent_id), name, None).map_err(|_| {
            BeamError::Validation {
                message: format!("A request named '{name}' already exists in this scope"),
            }
        })?;

        let request_file = default_request_file(name, input.method, input.url);
        let request_id = request_file.meta.request_id;
        let collection_id = root_collection_id_of(&self.store, parent_id).ok_or_else(|| {
            BeamError::NotFound {
                entity: "collection_for_request_parent",
                id: parent_id.to_string(),
            }
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
        assert_name_unique(&self.store.name_index, Some(parent_id), name, None).map_err(|_| {
            BeamError::Validation {
                message: format!("A request named '{name}' already exists in this scope"),
            }
        })?;

        let request_file = default_request_file(name, input.method, input.url);
        let request_id = request_file.meta.request_id;
        let collection_id = root_collection_id_of(&self.store, parent_id).ok_or_else(|| {
            BeamError::NotFound {
                entity: "collection_for_request_parent",
                id: parent_id.to_string(),
            }
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
        assert_name_unique(
            &self.store.name_index,
            Some(parent_id),
            next_name,
            Some(input.request_id),
        )
        .map_err(|_| BeamError::Validation {
            message: format!("A request named '{next_name}' already exists in this scope"),
        })?;

        let collection_id =
            root_collection_id_of(&self.store, input.request_id).ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_request",
                id: input.request_id.to_string(),
            })?;

        let existing_path =
            request_file_path(self.backend.paths(), &self.store, input.request_id)?;

        self.store
            .name_index
            .remove(&scope_key(Some(parent_id), &node.name));
        if let Some(request_node) = self.store.nodes.get_mut(&input.request_id) {
            request_node.name = next_name.to_string();
            request_node.updated_at = Some(Utc::now());
        }
        self.store.name_index.insert(
            scope_key(Some(parent_id), next_name),
            input.request_id,
        );

        let mut request_file = self
            .store
            .requests
            .get(&input.request_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "request_file",
                id: input.request_id.to_string(),
            })?
            .clone();
        request_file.meta.name = next_name.to_string();
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
        assert_name_unique(&self.store.name_index, Some(parent_id), name, None).map_err(|_| {
            BeamError::Validation {
                message: format!("A request named '{name}' already exists in this scope"),
            }
        })?;

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
        duplicated.meta.name = name.to_string();
        duplicated.meta.created_at = now;
        duplicated.meta.updated_at = now;
        let duplicated_id = duplicated.meta.request_id;

        let collection_id = root_collection_id_of(&self.store, parent_id).ok_or_else(|| {
            BeamError::NotFound {
                entity: "collection_for_request_parent",
                id: parent_id.to_string(),
            }
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
        self.store.requests.insert(duplicated_id, duplicated.clone());

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

        let request_path =
            request_file_path(self.backend.paths(), &self.store, input.request_id)?;

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

    pub fn save_request(&mut self, request_file: &RequestFile) -> Result<()> {
        let request_id = request_file.meta.request_id;
        self.store.requests.insert(request_id, request_file.clone());
        let request_path = request_file_path(self.backend.paths(), &self.store, request_id)?;
        self.backend.write_toml_file(&request_path, request_file)
    }

    pub fn create_environment(&self, input: CreateEnvironmentInput) -> Result<EnvironmentFile> {
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
        self.backend.write_toml_file(&file_path, &environment_file)?;
        Ok(environment_file)
    }

    pub fn rename_environment(
        &self,
        environment_id: Ulid,
        new_name: &str,
    ) -> Result<EnvironmentFile> {
        let next_name = new_name.trim();
        if next_name.is_empty() {
            return Err(BeamError::Validation {
                message: "Environment name cannot be empty".to_string(),
            });
        }

        let existing_path = find_environment_file_by_id(&self.backend, environment_id)?;
        let mut environment_file = read_environment_file(&self.backend, &existing_path)?;
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
        self.backend.write_toml_file(&next_path, &environment_file)?;
        Ok(environment_file)
    }

    pub fn update_environment_variables(
        &self,
        environment_id: Ulid,
        variables: Vec<EnvironmentVariable>,
    ) -> Result<EnvironmentFile> {
        let existing_path = find_environment_file_by_id(&self.backend, environment_id)?;
        let mut environment_file = read_environment_file(&self.backend, &existing_path)?;
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
        Ok(environment_file)
    }

    pub fn delete_environment(&self, environment_id: Ulid) -> Result<()> {
        let environment_path = find_environment_file_by_id(&self.backend, environment_id)?;
        self.backend.remove_file(&environment_path)
    }

    pub fn create_folder(&mut self, input: CreateFolderInput) -> Result<FolderFile> {
        let name = input.name.trim();
        if name.is_empty() {
            return Err(BeamError::Validation {
                message: "Folder name cannot be empty".to_string(),
            });
        }
        let parent_id = input.parent.parent_folder_id.unwrap_or(input.parent.collection_id);
        assert_name_unique(&self.store.name_index, Some(parent_id), name, None).map_err(|_| {
            BeamError::Validation {
                message: format!("A folder named '{name}' already exists in this scope"),
            }
        })?;

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
                name: name.to_string(),
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
            .insert(scope_key(Some(parent_id), name), folder_id);

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

    pub fn rename_collection(&mut self, collection_id: Ulid, new_name: &str) -> Result<CollectionFile> {
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

        self.store
            .name_index
            .remove(&scope_key(None, &old_name));
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
        assert_name_unique(
            &self.store.name_index,
            Some(parent_id),
            next_name,
            Some(folder_id),
        )
        .map_err(|_| BeamError::Validation {
            message: format!("A folder named '{next_name}' already exists in this scope"),
        })?;

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
            folder.name = next_name.to_string();
            folder.updated_at = Some(Utc::now());
        }
        self.store
            .name_index
            .insert(scope_key(Some(parent_id), next_name), folder_id);

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
                parent_folder_id: if self
                    .store
                    .nodes
                    .get(&parent_id)
                    .map(|n| n.kind)
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
                self.store
                    .name_index
                    .retain(|_, id| *id != node.id);
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
                self.store
                    .name_index
                    .retain(|_, id| *id != node.id);
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

    let entries = std::fs::read_dir(collections_dir).map_err(|source| BeamError::Io {
        path: collections_dir.clone(),
        source,
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| BeamError::Io {
            path: collections_dir.clone(),
            source,
        })?;
        let path = entry.path();
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
        let child_ids =
            load_manifest_children(backend, &mut store, &manifest.children, collection_id, collection_dir)?;
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
            node_children = load_manifest_children(backend, store, &child.children, node_id, &folder_dir)?;
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
        store.name_index.insert(scope_key(Some(parent_id), &child.name), node_id);
        child_ids.push(node_id);
    }
    Ok(child_ids)
}

fn read_environment_file<B: StorageIoBackend>(backend: &B, path: &Path) -> Result<EnvironmentFile> {
    let file: EnvironmentFile = backend.read_toml_file(path)?;
    Ok(file.with_file_path(path))
}

fn find_environment_file_by_id<B: StorageIoBackend>(
    backend: &B,
    environment_id: Ulid,
) -> Result<std::path::PathBuf> {
    let mut found: Option<std::path::PathBuf> = None;
    for root in [&backend.paths().environments_dir, &backend.paths().collections_dir] {
        if !root.exists() {
            continue;
        }
        walk_files_recursive(root, |path| {
            if !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".env.toml"))
            {
                return;
            }
            if let Ok(file) = backend.read_toml_file::<EnvironmentFile>(path) {
                if file.environment.environment_id == environment_id {
                    found = Some(path.to_path_buf());
                }
            }
        })?;
        if found.is_some() {
            break;
        }
    }

    found.ok_or_else(|| BeamError::NotFound {
        entity: "environment",
        id: environment_id.to_string(),
    })
}

fn environment_file_path_for_name<B: StorageIoBackend>(
    _backend: &B,
    dir: &Path,
    environment_name: &str,
    exclude_path: Option<&Path>,
) -> Result<std::path::PathBuf> {
    let preferred_stem = slugify(environment_name);
    let excluded = exclude_path.and_then(|path| path.file_name().map(|name| name.to_owned()));
    let mut used_names = HashSet::new();
    for entry in std::fs::read_dir(dir).map_err(|source| BeamError::Io {
        path: dir.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| BeamError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
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

fn walk_files_recursive<F>(root: &Path, mut visitor: F) -> Result<()>
where
    F: FnMut(&Path),
{
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|source| BeamError::Io {
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
