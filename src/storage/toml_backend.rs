use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use ulid::Ulid;

use crate::error::{BeamError, Result};
use crate::models::{
    AuthConfig, BodyConfig, CollectionFile, CollectionItemRef, EnvironmentFile, EnvironmentMeta,
    EnvironmentScope, EnvironmentVariable, FolderFile, FolderMeta, HeaderField, ItemType,
    LocalStateFile, QueryParamField, RequestDefinition, RequestFile, RequestMeta, ScriptConfig,
    WorkspaceFile,
};
use crate::paths::BeamPaths;
use crate::schema::{SCHEMA_VERSION_V1, SchemaKind, validate_schema_version};
use crate::storage::{
    BootstrapReport, CreateEnvironmentInput, CreateFolderInput, CreateRequestInput,
    DeleteRequestInput, DuplicateRequestInput, FolderParentRef, KnownParentManifestPath,
    MoveFolderInput, MoveRequestInput, RenameRequestInput, ReorderCollectionInput,
    RequestParentRef, WorkspaceStorage,
};
use crate::tree_store::{
    COLLECTION_MANIFEST_FILE_NAME, Node, NodeKind, SharedStore, assert_name_unique,
    collection_dir_path, folder_dir_path, persist_shared_tree, request_file_path,
    root_collection_id_of, shared_store_from_collection_manifest_path, write_collection_manifest,
    write_request_payload, write_root_order,
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

    fn hydrate_collection_manifest_path(
        &self,
        file: CollectionFile,
        path: &Path,
    ) -> CollectionFile {
        file.with_manifest_path(path)
    }

    fn hydrate_folder_manifest_path(&self, file: FolderFile, path: &Path) -> FolderFile {
        file.with_manifest_path(path)
    }

    fn hydrate_request_file_path(&self, file: RequestFile, path: &Path) -> RequestFile {
        file.with_file_path(path)
    }

    fn hydrate_environment_file_path(&self, file: EnvironmentFile, path: &Path) -> EnvironmentFile {
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

    fn find_collection_dir_by_id(&self, collection_id: Ulid) -> Result<PathBuf> {
        let manifest_path = self.find_collection_manifest_path_by_collection_id(collection_id)?;
        self.manifest_parent_dir(&manifest_path)
    }

    fn find_folder_dir_by_id(&self, folder_id: Ulid) -> Result<PathBuf> {
        let manifest_path = self.find_collection_manifest_path_containing_node(folder_id)?;
        let store = self.load_store_from_manifest_path(&manifest_path)?;
        folder_dir_path(&self.paths, &store, folder_id)
    }

    fn manifest_parent_dir(&self, manifest_path: &Path) -> Result<PathBuf> {
        manifest_path
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| BeamError::NotFound {
                entity: "manifest_parent_dir",
                id: manifest_path.to_string_lossy().to_string(),
            })
    }

    fn resolve_collection_dir(
        &self,
        collection_id: Ulid,
        known_manifest_path: Option<&Path>,
    ) -> Result<PathBuf> {
        if let Some(path) = known_manifest_path {
            let store = self.load_store_from_manifest_path(path)?;
            if store.root_ids.first().copied() == Some(collection_id) {
                return self.manifest_parent_dir(path);
            }
        }
        self.find_collection_dir_by_id(collection_id)
    }

    fn resolve_folder_dir(
        &self,
        folder_id: Ulid,
        known_manifest_path: Option<&Path>,
    ) -> Result<PathBuf> {
        if let Some(path) = known_manifest_path {
            let store = self.load_store_from_manifest_path(path)?;
            if store.nodes.contains_key(&folder_id) {
                return folder_dir_path(&self.paths, &store, folder_id);
            }
        }
        self.find_folder_dir_by_id(folder_id)
    }

    fn resolve_request_file_path(
        &self,
        request_id: Ulid,
        _known_request_path: Option<&Path>,
    ) -> Result<PathBuf> {
        self.find_request_file_by_id(request_id)
    }

    fn request_dir_for_parent(
        &self,
        parent: RequestParentRef,
        known_parent_manifest_path: Option<&KnownParentManifestPath>,
    ) -> Result<PathBuf> {
        let manifest_path = self.resolve_collection_manifest_path_for_request_parent(
            parent,
            known_parent_manifest_path,
        )?;
        let store = self.load_store_from_manifest_path(&manifest_path)?;
        if let Some(folder_id) = parent.folder_id {
            return folder_dir_path(&self.paths, &store, folder_id);
        }
        self.manifest_parent_dir(&manifest_path)
    }

    fn folder_dir_for_parent(
        &self,
        parent: FolderParentRef,
        known_parent_manifest_path: Option<&KnownParentManifestPath>,
    ) -> Result<PathBuf> {
        let manifest_path = self.resolve_collection_manifest_path_for_folder_parent(
            parent,
            known_parent_manifest_path,
        )?;
        let store = self.load_store_from_manifest_path(&manifest_path)?;
        if let Some(parent_folder_id) = parent.parent_folder_id {
            return folder_dir_path(&self.paths, &store, parent_folder_id);
        }
        self.manifest_parent_dir(&manifest_path)
    }

    fn append_folder_to_parent_manifest(
        &self,
        parent: FolderParentRef,
        known_parent_manifest_path: Option<&KnownParentManifestPath>,
        folder_id: Ulid,
        name: &str,
    ) -> Result<()> {
        if let Some(known_parent_manifest_path) = known_parent_manifest_path {
            return match known_parent_manifest_path {
                KnownParentManifestPath::Folder(path) => {
                    let mut folder_file = self.read_folder_file(path)?;
                    let next_order = folder_file
                        .items
                        .iter()
                        .map(|item| item.order)
                        .max()
                        .unwrap_or(-1)
                        + 1;
                    folder_file.items.push(CollectionItemRef {
                        item_id: folder_id,
                        item_type: ItemType::Folder,
                        name: name.to_string(),
                        order: next_order,
                    });
                    self.write_toml_file(path, &folder_file)
                }
                KnownParentManifestPath::Collection(path) => {
                    let mut collection_file = self.read_collection_file(path)?;
                    let next_order = collection_file
                        .items
                        .iter()
                        .map(|item| item.order)
                        .max()
                        .unwrap_or(-1)
                        + 1;
                    collection_file.items.push(CollectionItemRef {
                        item_id: folder_id,
                        item_type: ItemType::Folder,
                        name: name.to_string(),
                        order: next_order,
                    });
                    self.write_toml_file(path, &collection_file)
                }
            };
        }
        if let Some(parent_folder_id) = parent.parent_folder_id {
            let folder_dir = self.find_folder_dir_by_id(parent_folder_id)?;
            let path = folder_dir.join("folder.toml");
            let mut folder_file = self.read_folder_file(&path)?;
            let next_order = folder_file
                .items
                .iter()
                .map(|item| item.order)
                .max()
                .unwrap_or(-1)
                + 1;
            folder_file.items.push(CollectionItemRef {
                item_id: folder_id,
                item_type: ItemType::Folder,
                name: name.to_string(),
                order: next_order,
            });
            return self.write_toml_file(&path, &folder_file);
        }

        let collection_dir = self.find_collection_dir_by_id(parent.collection_id)?;
        let path = collection_dir.join("collection.toml");
        let mut collection_file = self.read_collection_file(&path)?;
        let next_order = collection_file
            .items
            .iter()
            .map(|item| item.order)
            .max()
            .unwrap_or(-1)
            + 1;
        collection_file.items.push(CollectionItemRef {
            item_id: folder_id,
            item_type: ItemType::Folder,
            name: name.to_string(),
            order: next_order,
        });
        self.write_toml_file(&path, &collection_file)
    }

    fn append_request_to_parent_manifest(
        &self,
        parent: RequestParentRef,
        known_parent_manifest_path: Option<&KnownParentManifestPath>,
        request_id: Ulid,
        name: &str,
    ) -> Result<()> {
        if let Some(known_parent_manifest_path) = known_parent_manifest_path {
            return match known_parent_manifest_path {
                KnownParentManifestPath::Folder(path) => {
                    let mut folder_file = self.read_folder_file(path)?;
                    let next_order = folder_file
                        .items
                        .iter()
                        .map(|item| item.order)
                        .max()
                        .unwrap_or(-1)
                        + 1;
                    folder_file.items.push(CollectionItemRef {
                        item_id: request_id,
                        item_type: ItemType::Request,
                        name: name.to_string(),
                        order: next_order,
                    });
                    self.write_toml_file(path, &folder_file)
                }
                KnownParentManifestPath::Collection(path) => {
                    let mut collection_file = self.read_collection_file(path)?;
                    let next_order = collection_file
                        .items
                        .iter()
                        .map(|item| item.order)
                        .max()
                        .unwrap_or(-1)
                        + 1;
                    collection_file.items.push(CollectionItemRef {
                        item_id: request_id,
                        item_type: ItemType::Request,
                        name: name.to_string(),
                        order: next_order,
                    });
                    self.write_toml_file(path, &collection_file)
                }
            };
        }
        if let Some(folder_id) = parent.folder_id {
            let folder_dir = self.find_folder_dir_by_id(folder_id)?;
            let path = folder_dir.join("folder.toml");
            let mut folder_file = self.read_folder_file(&path)?;
            let next_order = folder_file
                .items
                .iter()
                .map(|item| item.order)
                .max()
                .unwrap_or(-1)
                + 1;
            folder_file.items.push(CollectionItemRef {
                item_id: request_id,
                item_type: ItemType::Request,
                name: name.to_string(),
                order: next_order,
            });
            self.write_toml_file(&path, &folder_file)?;
            return Ok(());
        }

        let collection_dir = self.find_collection_dir_by_id(parent.collection_id)?;
        let path = collection_dir.join("collection.toml");
        let mut collection_file = self.read_collection_file(&path)?;
        let next_order = collection_file
            .items
            .iter()
            .map(|item| item.order)
            .max()
            .unwrap_or(-1)
            + 1;
        collection_file.items.push(CollectionItemRef {
            item_id: request_id,
            item_type: ItemType::Request,
            name: name.to_string(),
            order: next_order,
        });
        self.write_toml_file(&path, &collection_file)
    }

    fn insert_request_to_parent_manifest_after(
        &self,
        parent: RequestParentRef,
        source_request_id: Ulid,
        request_id: Ulid,
        name: &str,
    ) -> Result<()> {
        if let Some(folder_id) = parent.folder_id {
            let folder_dir = self.find_folder_dir_by_id(folder_id)?;
            let path = folder_dir.join("folder.toml");
            let mut folder_file = self.read_folder_file(&path)?;
            let source_order = folder_file
                .items
                .iter()
                .find(|item| {
                    item.item_id == source_request_id && item.item_type == ItemType::Request
                })
                .map(|item| item.order);
            let next_order = folder_file
                .items
                .iter()
                .map(|item| item.order)
                .max()
                .unwrap_or(-1)
                + 1;
            let inserted_order = source_order.map_or(next_order, |order| order + 1);
            for item in &mut folder_file.items {
                if item.order >= inserted_order {
                    item.order += 1;
                }
            }
            folder_file.items.push(CollectionItemRef {
                item_id: request_id,
                item_type: ItemType::Request,
                name: name.to_string(),
                order: inserted_order,
            });
            self.write_toml_file(&path, &folder_file)?;
            return Ok(());
        }

        let collection_dir = self.find_collection_dir_by_id(parent.collection_id)?;
        let path = collection_dir.join("collection.toml");
        let mut collection_file = self.read_collection_file(&path)?;
        let source_order = collection_file
            .items
            .iter()
            .find(|item| item.item_id == source_request_id && item.item_type == ItemType::Request)
            .map(|item| item.order);
        let next_order = collection_file
            .items
            .iter()
            .map(|item| item.order)
            .max()
            .unwrap_or(-1)
            + 1;
        let inserted_order = source_order.map_or(next_order, |order| order + 1);
        for item in &mut collection_file.items {
            if item.order >= inserted_order {
                item.order += 1;
            }
        }
        collection_file.items.push(CollectionItemRef {
            item_id: request_id,
            item_type: ItemType::Request,
            name: name.to_string(),
            order: inserted_order,
        });
        self.write_toml_file(&path, &collection_file)
    }

    fn update_folder_name_in_parent_manifest(
        &self,
        parent: FolderParentRef,
        known_parent_manifest_path: Option<&KnownParentManifestPath>,
        folder_id: Ulid,
        new_name: &str,
    ) -> Result<()> {
        if let Some(known_parent_manifest_path) = known_parent_manifest_path {
            return match known_parent_manifest_path {
                KnownParentManifestPath::Folder(path) => {
                    let mut folder_file = self.read_folder_file(path)?;
                    for item in &mut folder_file.items {
                        if item.item_id == folder_id && item.item_type == ItemType::Folder {
                            item.name = new_name.to_string();
                        }
                    }
                    self.write_toml_file(path, &folder_file)
                }
                KnownParentManifestPath::Collection(path) => {
                    let mut collection_file = self.read_collection_file(path)?;
                    for item in &mut collection_file.items {
                        if item.item_id == folder_id && item.item_type == ItemType::Folder {
                            item.name = new_name.to_string();
                        }
                    }
                    self.write_toml_file(path, &collection_file)
                }
            };
        }
        if let Some(parent_folder_id) = parent.parent_folder_id {
            let parent_dir = self.find_folder_dir_by_id(parent_folder_id)?;
            let path = parent_dir.join("folder.toml");
            let mut folder_file = self.read_folder_file(&path)?;
            for item in &mut folder_file.items {
                if item.item_id == folder_id && item.item_type == ItemType::Folder {
                    item.name = new_name.to_string();
                }
            }
            return self.write_toml_file(&path, &folder_file);
        }

        let collection_dir = self.find_collection_dir_by_id(parent.collection_id)?;
        let path = collection_dir.join("collection.toml");
        let mut collection_file = self.read_collection_file(&path)?;
        for item in &mut collection_file.items {
            if item.item_id == folder_id && item.item_type == ItemType::Folder {
                item.name = new_name.to_string();
            }
        }
        self.write_toml_file(&path, &collection_file)
    }

    fn remove_request_from_parent_manifest(
        &self,
        parent: RequestParentRef,
        known_parent_manifest_path: Option<&KnownParentManifestPath>,
        request_id: Ulid,
    ) -> Result<()> {
        if let Some(known_parent_manifest_path) = known_parent_manifest_path {
            return match known_parent_manifest_path {
                KnownParentManifestPath::Folder(path) => {
                    let mut folder_file = self.read_folder_file(path)?;
                    folder_file.items.retain(|item| {
                        !(item.item_id == request_id && item.item_type == ItemType::Request)
                    });
                    self.write_toml_file(path, &folder_file)
                }
                KnownParentManifestPath::Collection(path) => {
                    let mut collection_file = self.read_collection_file(path)?;
                    collection_file.items.retain(|item| {
                        !(item.item_id == request_id && item.item_type == ItemType::Request)
                    });
                    self.write_toml_file(path, &collection_file)
                }
            };
        }
        if let Some(folder_id) = parent.folder_id {
            let folder_dir = self.find_folder_dir_by_id(folder_id)?;
            let path = folder_dir.join("folder.toml");
            let mut folder_file = self.read_folder_file(&path)?;
            folder_file.items.retain(|item| {
                !(item.item_id == request_id && item.item_type == ItemType::Request)
            });
            return self.write_toml_file(&path, &folder_file);
        }

        let collection_dir = self.find_collection_dir_by_id(parent.collection_id)?;
        let path = collection_dir.join("collection.toml");
        let mut collection_file = self.read_collection_file(&path)?;
        collection_file
            .items
            .retain(|item| !(item.item_id == request_id && item.item_type == ItemType::Request));
        self.write_toml_file(&path, &collection_file)
    }

    fn remove_folder_from_parent_manifest(
        &self,
        parent: FolderParentRef,
        known_parent_manifest_path: Option<&KnownParentManifestPath>,
        folder_id: Ulid,
    ) -> Result<()> {
        if let Some(known_parent_manifest_path) = known_parent_manifest_path {
            return match known_parent_manifest_path {
                KnownParentManifestPath::Folder(path) => {
                    let mut parent_file = self.read_folder_file(path)?;
                    parent_file.items.retain(|item| {
                        !(item.item_id == folder_id && item.item_type == ItemType::Folder)
                    });
                    self.write_toml_file(path, &parent_file)
                }
                KnownParentManifestPath::Collection(path) => {
                    let mut collection_file = self.read_collection_file(path)?;
                    collection_file.items.retain(|item| {
                        !(item.item_id == folder_id && item.item_type == ItemType::Folder)
                    });
                    self.write_toml_file(path, &collection_file)
                }
            };
        }
        if let Some(parent_folder_id) = parent.parent_folder_id {
            let parent_dir = self.find_folder_dir_by_id(parent_folder_id)?;
            let path = parent_dir.join("folder.toml");
            let mut parent_file = self.read_folder_file(&path)?;
            parent_file
                .items
                .retain(|item| !(item.item_id == folder_id && item.item_type == ItemType::Folder));
            return self.write_toml_file(&path, &parent_file);
        }

        let collection_dir = self.find_collection_dir_by_id(parent.collection_id)?;
        let path = collection_dir.join("collection.toml");
        let mut collection_file = self.read_collection_file(&path)?;
        collection_file
            .items
            .retain(|item| !(item.item_id == folder_id && item.item_type == ItemType::Folder));
        self.write_toml_file(&path, &collection_file)
    }

    fn sibling_collection_names(&self, skip_collection_id: Option<Ulid>) -> Result<Vec<String>> {
        let mut names = Vec::new();
        self.walk_files_recursive(&self.paths.collections_dir, |path| {
            if path.file_name().and_then(|name| name.to_str()) != Some("collection.toml") {
                return;
            }
            let Ok(file) = self.read_collection_file(path) else {
                return;
            };
            if skip_collection_id.is_some_and(|skip_id| file.collection.collection_id == skip_id) {
                return;
            }
            names.push(file.collection.name);
        })?;
        Ok(names)
    }

    fn sibling_folder_names(
        &self,
        parent: FolderParentRef,
        known_parent_manifest_path: Option<&KnownParentManifestPath>,
        skip_folder_id: Option<Ulid>,
    ) -> Result<Vec<String>> {
        if let Some(known_parent_manifest_path) = known_parent_manifest_path {
            return match known_parent_manifest_path {
                KnownParentManifestPath::Folder(path) => {
                    let parent_file = self.read_folder_file(path)?;
                    let names = parent_file
                        .items
                        .iter()
                        .filter(|item| {
                            item.item_type == ItemType::Folder
                                && skip_folder_id.is_none_or(|skip_id| item.item_id != skip_id)
                        })
                        .map(|item| item.name.clone())
                        .collect();
                    Ok(names)
                }
                KnownParentManifestPath::Collection(path) => {
                    let collection_file = self.read_collection_file(path)?;
                    let names = collection_file
                        .items
                        .iter()
                        .filter(|item| {
                            item.item_type == ItemType::Folder
                                && skip_folder_id.is_none_or(|skip_id| item.item_id != skip_id)
                        })
                        .map(|item| item.name.clone())
                        .collect();
                    Ok(names)
                }
            };
        }
        if let Some(parent_folder_id) = parent.parent_folder_id {
            let parent_dir = self.find_folder_dir_by_id(parent_folder_id)?;
            let path = parent_dir.join("folder.toml");
            let parent_file = self.read_folder_file(&path)?;
            let names = parent_file
                .items
                .iter()
                .filter(|item| {
                    item.item_type == ItemType::Folder
                        && skip_folder_id.is_none_or(|skip_id| item.item_id != skip_id)
                })
                .map(|item| item.name.clone())
                .collect();
            return Ok(names);
        }

        let collection_dir = self.find_collection_dir_by_id(parent.collection_id)?;
        let path = collection_dir.join("collection.toml");
        let collection_file = self.read_collection_file(&path)?;
        let names = collection_file
            .items
            .iter()
            .filter(|item| {
                item.item_type == ItemType::Folder
                    && skip_folder_id.is_none_or(|skip_id| item.item_id != skip_id)
            })
            .map(|item| item.name.clone())
            .collect();
        Ok(names)
    }

    fn write_request_new_path(
        &self,
        request_file: &RequestFile,
        parent: RequestParentRef,
        known_parent_manifest_path: Option<&KnownParentManifestPath>,
    ) -> Result<PathBuf> {
        let request_dir = self.request_dir_for_parent(parent, known_parent_manifest_path)?;
        self.write_request_new_path_with_dir(request_file, &request_dir)
    }

    fn write_request_new_path_with_dir(
        &self,
        request_file: &RequestFile,
        request_dir: &Path,
    ) -> Result<PathBuf> {
        fs::create_dir_all(request_dir).map_err(|source| BeamError::Io {
            path: request_dir.to_path_buf(),
            source,
        })?;
        let file_path =
            self.request_file_path_for_name(request_dir, &request_file.meta.name, None)?;
        self.write_toml_file(&file_path, request_file)?;
        Ok(file_path)
    }

    fn request_file_path_for_name(
        &self,
        request_dir: &Path,
        request_name: &str,
        exclude_path: Option<&Path>,
    ) -> Result<PathBuf> {
        let preferred_stem = slugify(request_name);
        let excluded = exclude_path.and_then(|path| path.file_name().map(|name| name.to_owned()));
        let mut used_names = HashSet::new();

        for entry in fs::read_dir(request_dir).map_err(|source| BeamError::Io {
            path: request_dir.to_path_buf(),
            source,
        })? {
            let entry = entry.map_err(|source| BeamError::Io {
                path: request_dir.to_path_buf(),
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

        let mut suffix = 1;
        loop {
            let file_name = if suffix == 1 {
                format!("{preferred_stem}.request.toml")
            } else {
                format!("{preferred_stem}-{suffix}.request.toml")
            };
            if !used_names.contains(&file_name) {
                return Ok(request_dir.join(file_name));
            }
            suffix += 1;
        }
    }

    fn environment_file_path_for_name(
        &self,
        dir: &Path,
        environment_name: &str,
        exclude_path: Option<&Path>,
    ) -> Result<PathBuf> {
        let preferred_stem = slugify(environment_name);
        let excluded = exclude_path.and_then(|path| path.file_name().map(|name| name.to_owned()));
        let mut used_names = HashSet::new();
        for entry in fs::read_dir(dir).map_err(|source| BeamError::Io {
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

    fn find_environment_file_by_id(&self, environment_id: Ulid) -> Result<PathBuf> {
        let mut found: Option<PathBuf> = None;
        for root in [&self.paths.environments_dir, &self.paths.collections_dir] {
            if !root.exists() {
                continue;
            }
            self.walk_files_recursive(root, |path| {
                if !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".env.toml"))
                {
                    return;
                }
                let Ok(file) = self.read_environment_file(path) else {
                    return;
                };
                if file.environment.environment_id == environment_id {
                    found = Some(path.to_path_buf());
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

    fn find_request_file_by_id(&self, request_id: Ulid) -> Result<PathBuf> {
        let manifest_path = self.find_collection_manifest_path_containing_node(request_id)?;
        let store = self.load_store_from_manifest_path(&manifest_path)?;
        request_file_path(&self.paths, &store, request_id)
    }

    fn read_request_file(&self, path: &Path) -> Result<RequestFile> {
        let file = self.read_toml_file(path)?;
        Ok(self.hydrate_request_file_path(file, path))
    }

    fn read_collection_file(&self, path: &Path) -> Result<CollectionFile> {
        let file = self.read_toml_file(path)?;
        Ok(self.hydrate_collection_manifest_path(file, path))
    }

    fn read_folder_file(&self, path: &Path) -> Result<FolderFile> {
        let file = self.read_toml_file(path)?;
        Ok(self.hydrate_folder_manifest_path(file, path))
    }

    fn read_environment_file(&self, path: &Path) -> Result<EnvironmentFile> {
        let file = self.read_toml_file(path)?;
        Ok(self.hydrate_environment_file_path(file, path))
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

    fn root_collection_names_from_manifests(
        &self,
        skip_collection_id: Option<Ulid>,
    ) -> Result<Vec<String>> {
        let mut names = Vec::new();
        self.walk_files_recursive(&self.paths.collections_dir, |path| {
            if path.file_name().and_then(|name| name.to_str())
                != Some(COLLECTION_MANIFEST_FILE_NAME)
            {
                return;
            }
            let Ok(store) = shared_store_from_collection_manifest_path(path) else {
                return;
            };
            let Some(collection_id) = store.root_ids.first().copied() else {
                return;
            };
            if skip_collection_id == Some(collection_id) {
                return;
            }
            if let Some(collection) = store.nodes.get(&collection_id) {
                names.push(collection.name.clone());
            }
        })?;
        Ok(names)
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

    fn resolve_collection_manifest_path_for_collection(
        &self,
        collection_id: Ulid,
        known_manifest_path: Option<&Path>,
    ) -> Result<PathBuf> {
        if let Some(path) = known_manifest_path {
            return Ok(path.to_path_buf());
        }
        self.find_collection_manifest_path_by_collection_id(collection_id)
    }

    fn load_store_from_manifest_path(&self, manifest_path: &Path) -> Result<SharedStore> {
        shared_store_from_collection_manifest_path(manifest_path)
    }

    fn default_request_file(
        &self,
        name: &str,
        method: crate::models::HttpMethod,
        url: String,
    ) -> RequestFile {
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
            auth: AuthConfig::None,
            body: BodyConfig::None,
            scripts: ScriptConfig::default(),
            file_path: None,
        }
    }

    fn insert_child(parent: &mut Node, child_id: Ulid) {
        parent.children.push(child_id);
    }

    fn insert_child_after(parent: &mut Node, after_child_id: Ulid, child_id: Ulid) {
        if let Some(index) = parent.children.iter().position(|id| *id == after_child_id) {
            parent.children.insert(index + 1, child_id);
        } else {
            parent.children.push(child_id);
        }
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

    fn reposition_root(
        root_ids: &mut Vec<Ulid>,
        collection_id: Ulid,
        insertion_index: usize,
    ) -> Result<()> {
        let Some(current_index) = root_ids.iter().position(|id| *id == collection_id) else {
            return Err(BeamError::NotFound {
                entity: "collection",
                id: collection_id.to_string(),
            });
        };
        let root_id = root_ids.remove(current_index);
        let adjusted_index = if current_index < insertion_index {
            insertion_index.saturating_sub(1)
        } else {
            insertion_index
        };
        let index = adjusted_index.min(root_ids.len());
        root_ids.insert(index, root_id);
        Ok(())
    }

    fn load_ordered_collection_ids(&self) -> Result<Vec<Ulid>> {
        let mut collection_ids = Vec::new();
        self.walk_files_recursive(&self.paths.collections_dir, |path| {
            if path.file_name().and_then(|name| name.to_str())
                != Some(COLLECTION_MANIFEST_FILE_NAME)
            {
                return;
            }
            let Ok(store) = shared_store_from_collection_manifest_path(path) else {
                return;
            };
            if let Some(collection_id) = store.root_ids.first().copied() {
                collection_ids.push(collection_id);
            }
        })?;

        let root_order = if self.paths.collections_root_order_file.exists() {
            self.read_toml_file::<crate::tree_store::RootOrderFile>(
                &self.paths.collections_root_order_file,
            )
            .ok()
        } else {
            None
        };

        let available_ids: HashSet<Ulid> = collection_ids.iter().copied().collect();
        let mut ordered_ids = Vec::with_capacity(collection_ids.len());
        let mut seen = HashSet::new();

        if let Some(root_order) = root_order {
            for root_id in root_order.root_ids {
                if available_ids.contains(&root_id) && seen.insert(root_id) {
                    ordered_ids.push(root_id);
                }
            }
        }

        for collection_id in collection_ids {
            if seen.insert(collection_id) {
                ordered_ids.push(collection_id);
            }
        }

        Ok(ordered_ids)
    }

    fn collection_file_from_store(
        &self,
        store: &SharedStore,
        manifest_path: &Path,
    ) -> Result<CollectionFile> {
        let collection_id = *store.root_ids.first().ok_or_else(|| BeamError::NotFound {
            entity: "collection",
            id: "root".to_string(),
        })?;
        let collection = store
            .nodes
            .get(&collection_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "collection",
                id: collection_id.to_string(),
            })?;
        Ok(CollectionFile {
            collection: crate::models::CollectionMeta {
                collection_id,
                name: collection.name.clone(),
                description: collection.description.clone(),
                created_at: collection.created_at.unwrap_or_else(Utc::now),
                updated_at: collection.updated_at.unwrap_or_else(Utc::now),
            },
            items: Vec::new(),
            manifest_path: Some(manifest_path.to_path_buf()),
        })
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

    fn find_request_parent(&self, request_id: Ulid) -> Result<RequestParentRef> {
        let mut found: Option<RequestParentRef> = None;
        self.walk_files_recursive(&self.paths.collections_dir, |path| {
            if path.file_name().and_then(|n| n.to_str()) == Some("collection.toml") {
                let Ok(file) = self.read_collection_file(path) else {
                    return;
                };
                if file
                    .items
                    .iter()
                    .any(|item| item.item_id == request_id && item.item_type == ItemType::Request)
                {
                    found = Some(RequestParentRef {
                        collection_id: file.collection.collection_id,
                        folder_id: None,
                    });
                }
            } else if path.file_name().and_then(|n| n.to_str()) == Some("folder.toml") {
                let Ok(file) = self.read_folder_file(path) else {
                    return;
                };
                if file
                    .items
                    .iter()
                    .any(|item| item.item_id == request_id && item.item_type == ItemType::Request)
                {
                    found = Some(RequestParentRef {
                        collection_id: file.folder.collection_id,
                        folder_id: Some(file.folder.folder_id),
                    });
                }
            }
        })?;
        found.ok_or_else(|| BeamError::NotFound {
            entity: "request_parent",
            id: request_id.to_string(),
        })
    }

    fn find_folder_parent(&self, folder_id: Ulid) -> Result<FolderParentRef> {
        let mut found: Option<FolderParentRef> = None;
        self.walk_files_recursive(&self.paths.collections_dir, |path| {
            if path.file_name().and_then(|n| n.to_str()) == Some("collection.toml") {
                let Ok(file) = self.read_collection_file(path) else {
                    return;
                };
                if file
                    .items
                    .iter()
                    .any(|item| item.item_id == folder_id && item.item_type == ItemType::Folder)
                {
                    found = Some(FolderParentRef {
                        collection_id: file.collection.collection_id,
                        parent_folder_id: None,
                    });
                }
            } else if path.file_name().and_then(|n| n.to_str()) == Some("folder.toml") {
                let Ok(file) = self.read_folder_file(path) else {
                    return;
                };
                if file
                    .items
                    .iter()
                    .any(|item| item.item_id == folder_id && item.item_type == ItemType::Folder)
                {
                    found = Some(FolderParentRef {
                        collection_id: file.folder.collection_id,
                        parent_folder_id: Some(file.folder.folder_id),
                    });
                }
            }
        })?;
        found.ok_or_else(|| BeamError::NotFound {
            entity: "folder_parent",
            id: folder_id.to_string(),
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

    pub fn rename_collection_with_manifest_path(
        &self,
        collection_id: Ulid,
        new_name: &str,
        known_manifest_path: Option<&Path>,
    ) -> Result<CollectionFile> {
        let next_name = new_name.trim();
        if next_name.is_empty() {
            return Err(BeamError::Validation {
                message: "Collection name cannot be empty".to_string(),
            });
        }
        let sibling_names = self.root_collection_names_from_manifests(Some(collection_id))?;
        if sibling_names
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(next_name))
        {
            return Err(BeamError::Validation {
                message: format!("A collection named '{next_name}' already exists"),
            });
        }
        let manifest_path = self
            .resolve_collection_manifest_path_for_collection(collection_id, known_manifest_path)?;
        let mut store = self.load_store_from_manifest_path(&manifest_path)?;
        let old_dir = collection_dir_path(&self.paths, &store, collection_id)?;
        let old_manifest_path = old_dir.join(COLLECTION_MANIFEST_FILE_NAME);
        let old_name = store
            .nodes
            .get(&collection_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "collection",
                id: collection_id.to_string(),
            })?
            .name
            .clone();
        if let Some(collection) = store.nodes.get_mut(&collection_id) {
            collection.name = next_name.to_string();
            collection.updated_at = Some(Utc::now());
        }

        let new_dir = collection_dir_path(&self.paths, &store, collection_id)?;
        if new_dir != old_dir {
            fs::rename(&old_dir, &new_dir).map_err(|source| BeamError::Io {
                path: old_dir.clone(),
                source,
            })?;
        }
        let write_result = write_collection_manifest(&self.paths, &store, collection_id);
        if let Err(error) = write_result {
            if new_dir != old_dir {
                let _ = fs::rename(&new_dir, &old_dir);
            }
            if let Some(collection) = store.nodes.get_mut(&collection_id) {
                collection.name = old_name;
            }
            return Err(error);
        }
        let next_manifest_path = if new_dir == old_dir {
            old_manifest_path
        } else {
            new_dir.join(COLLECTION_MANIFEST_FILE_NAME)
        };
        self.collection_file_from_store(&store, &next_manifest_path)
    }

    pub fn rename_folder_with_manifest_path(
        &self,
        folder_id: Ulid,
        new_name: &str,
        known_manifest_path: Option<&Path>,
        _known_parent_manifest_path: Option<&KnownParentManifestPath>,
    ) -> Result<FolderFile> {
        let next_name = new_name.trim();
        if next_name.is_empty() {
            return Err(BeamError::Validation {
                message: "Folder name cannot be empty".to_string(),
            });
        }
        let manifest_path =
            self.resolve_collection_manifest_path_for_folder(folder_id, known_manifest_path)?;
        let mut store = self.load_store_from_manifest_path(&manifest_path)?;
        let parent_id = store
            .nodes
            .get(&folder_id)
            .and_then(|node| node.parent_id)
            .ok_or_else(|| BeamError::Validation {
                message: format!("folder node {folder_id} is missing parent_id"),
            })?;
        assert_name_unique(
            &store.name_index,
            Some(parent_id),
            next_name,
            Some(folder_id),
        )
        .map_err(|_| BeamError::Validation {
            message: format!("A folder named '{next_name}' already exists in this scope"),
        })?;

        let old_dir = folder_dir_path(&self.paths, &store, folder_id)?;
        let old_name = store
            .nodes
            .get(&folder_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "folder",
                id: folder_id.to_string(),
            })?
            .name
            .clone();
        store
            .name_index
            .remove(&crate::tree_store::scope_key(Some(parent_id), &old_name));
        if let Some(folder) = store.nodes.get_mut(&folder_id) {
            folder.name = next_name.to_string();
            folder.updated_at = Some(Utc::now());
        }
        store.name_index.insert(
            crate::tree_store::scope_key(Some(parent_id), next_name),
            folder_id,
        );

        let new_dir = folder_dir_path(&self.paths, &store, folder_id)?;
        if new_dir != old_dir {
            fs::rename(&old_dir, &new_dir).map_err(|source| BeamError::Io {
                path: old_dir.clone(),
                source,
            })?;
        }
        let collection_id =
            root_collection_id_of(&store, folder_id).ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_folder",
                id: folder_id.to_string(),
            })?;
        if let Err(error) = write_collection_manifest(&self.paths, &store, collection_id) {
            if new_dir != old_dir {
                let _ = fs::rename(&new_dir, &old_dir);
            }
            return Err(error);
        }
        self.folder_file_from_store(&store, folder_id, &manifest_path)
    }

    pub fn delete_collection_with_manifest_path(
        &self,
        collection_id: Ulid,
        known_manifest_path: Option<&Path>,
    ) -> Result<()> {
        let collection_dir = self.resolve_collection_dir(collection_id, known_manifest_path)?;
        fs::remove_dir_all(&collection_dir).map_err(|source| BeamError::Io {
            path: collection_dir,
            source,
        })
    }

    pub fn delete_folder_with_manifest_path(
        &self,
        folder_id: Ulid,
        known_manifest_path: Option<&Path>,
        _known_parent_manifest_path: Option<&KnownParentManifestPath>,
    ) -> Result<()> {
        let manifest_path =
            self.resolve_collection_manifest_path_for_folder(folder_id, known_manifest_path)?;
        let mut store = self.load_store_from_manifest_path(&manifest_path)?;
        let folder_dir = folder_dir_path(&self.paths, &store, folder_id)?;
        let parent_id = store
            .nodes
            .get(&folder_id)
            .and_then(|node| node.parent_id)
            .ok_or_else(|| BeamError::Validation {
                message: format!("folder node {folder_id} is missing parent_id"),
            })?;
        let folder_name = store
            .nodes
            .get(&folder_id)
            .map(|node| node.name.clone())
            .ok_or_else(|| BeamError::NotFound {
                entity: "folder",
                id: folder_id.to_string(),
            })?;
        store
            .name_index
            .remove(&crate::tree_store::scope_key(Some(parent_id), &folder_name));
        let child_ids = store
            .nodes
            .get(&folder_id)
            .map(|node| node.children.clone())
            .unwrap_or_default();
        let mut stack = child_ids;
        while let Some(node_id) = stack.pop() {
            if let Some(node) = store.nodes.remove(&node_id) {
                store
                    .name_index
                    .remove(&crate::tree_store::scope_key(node.parent_id, &node.name));
                stack.extend(node.children);
            }
            store.requests.remove(&node_id);
        }
        store.nodes.remove(&folder_id);
        if let Some(parent) = store.nodes.get_mut(&parent_id) {
            parent.children.retain(|child_id| *child_id != folder_id);
        }
        let collection_id =
            root_collection_id_of(&store, parent_id).ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_folder",
                id: folder_id.to_string(),
            })?;
        write_collection_manifest(&self.paths, &store, collection_id)?;
        fs::remove_dir_all(&folder_dir).map_err(|source| BeamError::Io {
            path: folder_dir,
            source,
        })
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

    fn load_request(&self, request_id: Ulid) -> Result<RequestFile> {
        let path = self.find_request_file_by_id(request_id)?;
        self.read_request_file(&path)
    }

    fn create_request(&self, input: CreateRequestInput) -> Result<RequestFile> {
        let CreateRequestInput {
            parent,
            known_parent_manifest_path,
            name,
            method,
            url,
        } = input;
        let name = name.trim();
        if name.is_empty() {
            return Err(BeamError::Validation {
                message: "Request name cannot be empty".to_string(),
            });
        }
        let manifest_path = self.resolve_collection_manifest_path_for_request_parent(
            parent,
            known_parent_manifest_path.as_ref(),
        )?;
        let mut store = self.load_store_from_manifest_path(&manifest_path)?;
        let parent_id = parent.folder_id.unwrap_or(parent.collection_id);
        assert_name_unique(&store.name_index, Some(parent_id), name, None).map_err(|_| {
            BeamError::Validation {
                message: format!("A request named '{name}' already exists in this scope"),
            }
        })?;

        let request_file = self.default_request_file(name, method, url);
        let request_id = request_file.meta.request_id;
        let collection_id =
            root_collection_id_of(&store, parent_id).ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_request_parent",
                id: parent_id.to_string(),
            })?;
        let now = request_file.meta.created_at;
        if let Some(parent_node) = store.nodes.get_mut(&parent_id) {
            Self::insert_child(parent_node, request_id);
        }
        store.nodes.insert(
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
        store.name_index.insert(
            crate::tree_store::scope_key(Some(parent_id), &request_file.meta.name),
            request_id,
        );
        store.requests.insert(request_id, request_file.clone());

        let created_path = write_request_payload(&self.paths, &store, request_id)?;
        write_collection_manifest(&self.paths, &store, collection_id)?;
        Ok(request_file.with_file_path(created_path))
    }

    fn create_request_after(
        &self,
        input: CreateRequestInput,
        source_request_id: Ulid,
    ) -> Result<RequestFile> {
        let CreateRequestInput {
            parent,
            known_parent_manifest_path,
            name,
            method,
            url,
        } = input;
        let name = name.trim();
        if name.is_empty() {
            return Err(BeamError::Validation {
                message: "Request name cannot be empty".to_string(),
            });
        }
        let manifest_path = self.resolve_collection_manifest_path_for_request_parent(
            parent,
            known_parent_manifest_path.as_ref(),
        )?;
        let mut store = self.load_store_from_manifest_path(&manifest_path)?;
        let parent_id = parent.folder_id.unwrap_or(parent.collection_id);
        assert_name_unique(&store.name_index, Some(parent_id), name, None).map_err(|_| {
            BeamError::Validation {
                message: format!("A request named '{name}' already exists in this scope"),
            }
        })?;

        let request_file = self.default_request_file(name, method, url);
        let request_id = request_file.meta.request_id;
        let collection_id =
            root_collection_id_of(&store, parent_id).ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_request_parent",
                id: parent_id.to_string(),
            })?;
        let now = request_file.meta.created_at;
        if let Some(parent_node) = store.nodes.get_mut(&parent_id) {
            Self::insert_child_after(parent_node, source_request_id, request_id);
        }
        store.nodes.insert(
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
        store.name_index.insert(
            crate::tree_store::scope_key(Some(parent_id), &request_file.meta.name),
            request_id,
        );
        store.requests.insert(request_id, request_file.clone());

        let created_path = write_request_payload(&self.paths, &store, request_id)?;
        write_collection_manifest(&self.paths, &store, collection_id)?;
        Ok(request_file.with_file_path(created_path))
    }

    fn create_folder(&self, input: CreateFolderInput) -> Result<FolderFile> {
        let name = input.name.trim();
        if name.is_empty() {
            return Err(BeamError::Validation {
                message: "Folder name cannot be empty".to_string(),
            });
        }
        let manifest_path = self.resolve_collection_manifest_path_for_folder_parent(
            input.parent,
            input.known_parent_manifest_path.as_ref(),
        )?;
        let mut store = self.load_store_from_manifest_path(&manifest_path)?;
        let parent_id = input
            .parent
            .parent_folder_id
            .unwrap_or(input.parent.collection_id);
        assert_name_unique(&store.name_index, Some(parent_id), name, None).map_err(|_| {
            BeamError::Validation {
                message: format!("A folder named '{name}' already exists in this scope"),
            }
        })?;

        let now = Utc::now();
        let folder_id = Ulid::new();
        if let Some(parent_node) = store.nodes.get_mut(&parent_id) {
            Self::insert_child(parent_node, folder_id);
        }
        store.nodes.insert(
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
        store.name_index.insert(
            crate::tree_store::scope_key(Some(parent_id), name),
            folder_id,
        );
        let collection_id =
            root_collection_id_of(&store, parent_id).ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_folder_parent",
                id: parent_id.to_string(),
            })?;
        let folder_dir = folder_dir_path(&self.paths, &store, folder_id)?;
        fs::create_dir_all(&folder_dir).map_err(|source| BeamError::Io {
            path: folder_dir,
            source,
        })?;
        write_collection_manifest(&self.paths, &store, collection_id)?;
        self.folder_file_from_store(&store, folder_id, &manifest_path)
    }

    fn create_environment(&self, input: CreateEnvironmentInput) -> Result<EnvironmentFile> {
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

        fs::create_dir_all(&self.paths.environments_dir).map_err(|source| BeamError::Io {
            path: self.paths.environments_dir.clone(),
            source,
        })?;
        let file_path =
            self.environment_file_path_for_name(&self.paths.environments_dir, name, None)?;
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
        self.write_toml_file(&file_path, &environment_file)?;
        Ok(environment_file)
    }

    fn rename_environment(&self, environment_id: Ulid, new_name: &str) -> Result<EnvironmentFile> {
        let next_name = new_name.trim();
        if next_name.is_empty() {
            return Err(BeamError::Validation {
                message: "Environment name cannot be empty".to_string(),
            });
        }

        let existing_path = self.find_environment_file_by_id(environment_id)?;
        let mut environment_file = self.read_environment_file(&existing_path)?;
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
        let next_path = self.environment_file_path_for_name(
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
            fs::rename(&existing_path, &next_path).map_err(|source| BeamError::Io {
                path: existing_path.clone(),
                source,
            })?;
        }
        environment_file.environment.file_name = next_file_name;
        environment_file = environment_file.with_file_path(&next_path);
        self.write_toml_file(&next_path, &environment_file)?;
        Ok(environment_file)
    }

    fn update_environment_variables(
        &self,
        environment_id: Ulid,
        variables: Vec<EnvironmentVariable>,
    ) -> Result<EnvironmentFile> {
        let existing_path = self.find_environment_file_by_id(environment_id)?;
        let mut environment_file = self.read_environment_file(&existing_path)?;
        environment_file.environment.updated_at = Utc::now();
        environment_file.variables = variables;
        if environment_file.environment.file_name.trim().is_empty() {
            environment_file.environment.file_name = existing_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();
        }
        self.write_toml_file(&existing_path, &environment_file)?;
        Ok(environment_file)
    }

    fn save_request(&self, request_file: &RequestFile) -> Result<()> {
        let request_path = self.find_request_file_by_id(request_file.meta.request_id)?;
        self.write_toml_file(&request_path, request_file)
    }

    fn rename_request(&self, input: RenameRequestInput) -> Result<RequestFile> {
        let manifest_path = self.resolve_collection_manifest_path_for_request(
            input.request_id,
            input.known_request_path.as_deref(),
            input.known_parent_manifest_path.as_ref(),
        )?;
        let mut store = self.load_store_from_manifest_path(&manifest_path)?;
        let existing_path =
            self.resolve_request_file_path(input.request_id, input.known_request_path.as_deref())?;
        let mut request_file = self.read_request_file(&existing_path)?;
        let next_name = input.new_name.trim();
        if next_name.is_empty() {
            return Err(BeamError::Validation {
                message: "Request name cannot be empty".to_string(),
            });
        }
        let node = store
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
            &store.name_index,
            Some(parent_id),
            next_name,
            Some(input.request_id),
        )
        .map_err(|_| BeamError::Validation {
            message: format!("A request named '{next_name}' already exists in this scope"),
        })?;

        store
            .name_index
            .remove(&crate::tree_store::scope_key(Some(parent_id), &node.name));
        if let Some(request_node) = store.nodes.get_mut(&input.request_id) {
            request_node.name = next_name.to_string();
            request_node.updated_at = Some(Utc::now());
        }
        store.name_index.insert(
            crate::tree_store::scope_key(Some(parent_id), next_name),
            input.request_id,
        );
        request_file.meta.name = next_name.to_string();
        request_file.meta.updated_at = Utc::now();
        store
            .requests
            .insert(input.request_id, request_file.clone());

        let new_path = request_file_path(&self.paths, &store, input.request_id)?;
        write_request_payload(&self.paths, &store, input.request_id)?;
        write_collection_manifest(
            &self.paths,
            &store,
            root_collection_id_of(&store, input.request_id).ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_request",
                id: input.request_id.to_string(),
            })?,
        )?;
        if new_path != existing_path {
            fs::remove_file(&existing_path).map_err(|source| BeamError::Io {
                path: existing_path,
                source,
            })?;
        }
        Ok(request_file.with_file_path(new_path))
    }

    fn rename_collection(&self, collection_id: Ulid, new_name: &str) -> Result<CollectionFile> {
        self.rename_collection_with_manifest_path(collection_id, new_name, None)
    }

    fn rename_folder(&self, folder_id: Ulid, new_name: &str) -> Result<FolderFile> {
        self.rename_folder_with_manifest_path(folder_id, new_name, None, None)
    }

    fn duplicate_request(&self, input: DuplicateRequestInput) -> Result<RequestFile> {
        let manifest_path = self.resolve_collection_manifest_path_for_request_parent(
            input.parent,
            input.known_parent_manifest_path.as_ref(),
        )?;
        let mut store = self.load_store_from_manifest_path(&manifest_path)?;
        let source_path =
            self.resolve_request_file_path(input.request_id, input.known_request_path.as_deref())?;
        let source = self.read_request_file(&source_path)?;
        let name = input.duplicate_name.trim();
        if name.is_empty() {
            return Err(BeamError::Validation {
                message: "Duplicate request name cannot be empty".to_string(),
            });
        }
        let parent_id = input.parent.folder_id.unwrap_or(input.parent.collection_id);
        assert_name_unique(&store.name_index, Some(parent_id), name, None).map_err(|_| {
            BeamError::Validation {
                message: format!("A request named '{name}' already exists in this scope"),
            }
        })?;
        let now = Utc::now();
        let mut duplicated = source.clone();
        duplicated.meta.request_id = Ulid::new();
        duplicated.meta.name = name.to_string();
        duplicated.meta.created_at = now;
        duplicated.meta.updated_at = now;
        if let Some(parent_node) = store.nodes.get_mut(&parent_id) {
            Self::insert_child_after(parent_node, input.request_id, duplicated.meta.request_id);
        }
        store.nodes.insert(
            duplicated.meta.request_id,
            Node {
                id: duplicated.meta.request_id,
                name: duplicated.meta.name.clone(),
                kind: NodeKind::Request,
                description: duplicated.meta.description.clone(),
                created_at: Some(duplicated.meta.created_at),
                updated_at: Some(duplicated.meta.updated_at),
                parent_id: Some(parent_id),
                children: Vec::new(),
            },
        );
        store.name_index.insert(
            crate::tree_store::scope_key(Some(parent_id), &duplicated.meta.name),
            duplicated.meta.request_id,
        );
        store
            .requests
            .insert(duplicated.meta.request_id, duplicated.clone());

        let duplicated_path =
            write_request_payload(&self.paths, &store, duplicated.meta.request_id)?;
        write_collection_manifest(
            &self.paths,
            &store,
            root_collection_id_of(&store, parent_id).ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_request_parent",
                id: parent_id.to_string(),
            })?,
        )?;
        Ok(duplicated.with_file_path(duplicated_path))
    }

    fn delete_collection(&self, collection_id: Ulid) -> Result<()> {
        self.delete_collection_with_manifest_path(collection_id, None)
    }

    fn delete_folder(&self, folder_id: Ulid) -> Result<()> {
        self.delete_folder_with_manifest_path(folder_id, None, None)
    }

    fn delete_request(&self, input: DeleteRequestInput) -> Result<()> {
        let manifest_path = self.resolve_collection_manifest_path_for_request(
            input.request_id,
            input.known_request_path.as_deref(),
            input.known_parent_manifest_path.as_ref(),
        )?;
        let mut store = self.load_store_from_manifest_path(&manifest_path)?;
        let request_path =
            self.resolve_request_file_path(input.request_id, input.known_request_path.as_deref())?;
        let node = store
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
        store
            .name_index
            .remove(&crate::tree_store::scope_key(Some(parent_id), &node.name));
        store.nodes.remove(&input.request_id);
        store.requests.remove(&input.request_id);
        if let Some(parent_node) = store.nodes.get_mut(&parent_id) {
            parent_node
                .children
                .retain(|child_id| *child_id != input.request_id);
        }
        write_collection_manifest(
            &self.paths,
            &store,
            root_collection_id_of(&store, parent_id).ok_or_else(|| BeamError::NotFound {
                entity: "collection_for_request_parent",
                id: parent_id.to_string(),
            })?,
        )?;
        fs::remove_file(&request_path).map_err(|source| BeamError::Io {
            path: request_path,
            source,
        })
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

    fn reorder_collection(&self, input: ReorderCollectionInput) -> Result<()> {
        let mut root_ids = self.load_ordered_collection_ids()?;
        Self::reposition_root(&mut root_ids, input.collection_id, input.insertion_index)?;
        let mut store = SharedStore::default();
        store.root_ids = root_ids;
        write_root_order(&self.paths, &store)?;
        Ok(())
    }

    fn delete_environment(&self, environment_id: Ulid) -> Result<()> {
        let environment_path = self.find_environment_file_by_id(environment_id)?;
        fs::remove_file(&environment_path).map_err(|source| BeamError::Io {
            path: environment_path,
            source,
        })
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
    fn bootstrap_sample_workspace_if_needed_seeds_first_request() {
        let dir = tempdir().expect("tempdir");
        let storage = TomlWorkspaceStorage::new(BeamPaths::from_root(dir.path().to_path_buf()));

        storage.initialize().expect("initialize");
        storage
            .bootstrap_sample_workspace_if_needed()
            .expect("bootstrap sample workspace");

        let local_state = storage.load_local_state().expect("load local state");
        let request_id = local_state
            .local_state
            .last_opened_request_id
            .expect("sample request should be selected");
        let request = storage
            .load_request(request_id)
            .expect("load sample request");
        let manifest_path = storage
            .paths
            .collections_dir
            .join("sample-collection")
            .join(COLLECTION_MANIFEST_FILE_NAME);
        let store = shared_store_from_collection_manifest_path(&manifest_path)
            .expect("load sample collection manifest");

        assert_eq!(store.root_ids.len(), 1);
        assert_eq!(request.meta.name, "Sample Request");
        assert_eq!(request.request.method, HttpMethod::Get);
        assert_eq!(request.request.url, "https://httpbin.org/get");
        assert_eq!(
            store.root_ids[0],
            store.nodes[&request_id].parent_id.expect("parent id")
        );
        assert_eq!(store.nodes[&store.root_ids[0]].name, "Sample Collection");
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

    fn init_storage_with_collection() -> (tempfile::TempDir, TomlWorkspaceStorage, Ulid, PathBuf) {
        let dir = tempdir().expect("tempdir");
        let storage = TomlWorkspaceStorage::new(BeamPaths::from_root(dir.path().to_path_buf()));
        storage.initialize().expect("initialize");

        let collection_id = Ulid::new();
        let collection_dir = storage.paths.collections_dir.join("sample");
        fs::create_dir_all(&collection_dir).expect("create collection dir");
        let now = Utc::now();
        let collection_manifest = CollectionManifestFile {
            schema_version: crate::schema::SCHEMA_VERSION_V1,
            id: collection_id,
            name: "Sample".to_string(),
            kind: NodeKind::Collection,
            description: None,
            created_at: Some(now),
            updated_at: Some(now),
            children: Vec::new(),
        };
        storage
            .write_toml_file(
                &collection_dir.join(COLLECTION_MANIFEST_FILE_NAME),
                &collection_manifest,
            )
            .expect("save collection manifest");

        (dir, storage, collection_id, collection_dir)
    }

    fn create_collection_manifest(
        storage: &TomlWorkspaceStorage,
        dir_name: &str,
        name: &str,
    ) -> (Ulid, PathBuf) {
        let collection_id = Ulid::new();
        let collection_dir = storage.paths.collections_dir.join(dir_name);
        fs::create_dir_all(&collection_dir).expect("create collection dir");
        let now = Utc::now();
        let collection_manifest = CollectionManifestFile {
            schema_version: crate::schema::SCHEMA_VERSION_V1,
            id: collection_id,
            name: name.to_string(),
            kind: NodeKind::Collection,
            description: None,
            created_at: Some(now),
            updated_at: Some(now),
            children: Vec::new(),
        };
        storage
            .write_toml_file(
                &collection_dir.join(COLLECTION_MANIFEST_FILE_NAME),
                &collection_manifest,
            )
            .expect("save collection manifest");
        (collection_id, collection_dir)
    }

    #[test]
    fn load_helpers_hydrate_runtime_paths_for_loaded_files() {
        let (_dir, storage, collection_id, collection_dir) = init_storage_with_collection();
        let collection_manifest_path = collection_dir.join(COLLECTION_MANIFEST_FILE_NAME);
        let loaded_store = storage
            .load_store_from_manifest_path(&collection_manifest_path)
            .expect("load store from manifest");
        let loaded_collection = storage
            .collection_file_from_store(&loaded_store, &collection_manifest_path)
            .expect("load collection with path");
        assert_eq!(
            loaded_collection.manifest_path.as_deref(),
            Some(collection_manifest_path.as_path())
        );

        let folder = storage
            .create_folder(CreateFolderInput {
                parent: FolderParentRef {
                    collection_id,
                    parent_folder_id: None,
                },
                known_parent_manifest_path: None,
                name: "Auth".to_string(),
            })
            .expect("create folder");
        let folder_manifest_path = storage
            .find_collection_manifest_path_containing_node(folder.folder.folder_id)
            .expect("find folder manifest path");
        assert_eq!(
            folder.manifest_path.as_deref(),
            Some(folder_manifest_path.as_path())
        );
        let loaded_store = storage
            .load_store_from_manifest_path(&folder_manifest_path)
            .expect("load store after folder create");
        let loaded_folder = storage
            .folder_file_from_store(
                &loaded_store,
                folder.folder.folder_id,
                &folder_manifest_path,
            )
            .expect("load folder with path");
        assert_eq!(
            loaded_folder.manifest_path.as_deref(),
            Some(folder_manifest_path.as_path())
        );

        let request = storage
            .create_request(CreateRequestInput {
                parent: RequestParentRef {
                    collection_id,
                    folder_id: None,
                },
                known_parent_manifest_path: None,
                name: "List Users".to_string(),
                method: HttpMethod::Get,
                url: "https://api.example.com/users".to_string(),
            })
            .expect("create request");
        let request_path = storage
            .find_request_file_by_id(request.meta.request_id)
            .expect("find request path");
        assert_eq!(request.file_path.as_deref(), Some(request_path.as_path()));
        let loaded_request = storage
            .load_request(request.meta.request_id)
            .expect("load request with path");
        assert_eq!(
            loaded_request.file_path.as_deref(),
            Some(request_path.as_path())
        );

        let environment = storage
            .create_environment(CreateEnvironmentInput {
                name: "Global".to_string(),
                scope: EnvironmentScope::Global,
                collection_id: None,
            })
            .expect("create environment");
        let environment_path = storage
            .find_environment_file_by_id(environment.environment.environment_id)
            .expect("find environment path");
        assert_eq!(
            environment.file_path.as_deref(),
            Some(environment_path.as_path())
        );
        let loaded_environment = storage
            .read_environment_file(&environment_path)
            .expect("load environment with path");
        assert_eq!(
            loaded_environment.file_path.as_deref(),
            Some(environment_path.as_path())
        );
    }

    #[test]
    fn created_folder_and_environment_toml_do_not_persist_runtime_paths() {
        let (_dir, storage, collection_id, _) = init_storage_with_collection();

        let folder = storage
            .create_folder(CreateFolderInput {
                parent: FolderParentRef {
                    collection_id,
                    parent_folder_id: None,
                },
                known_parent_manifest_path: None,
                name: "Auth".to_string(),
            })
            .expect("create folder");
        let folder_manifest_path = folder
            .manifest_path
            .clone()
            .expect("folder manifest path should be hydrated");
        let folder_toml = fs::read_to_string(&folder_manifest_path).expect("read folder manifest");
        assert!(!folder_toml.contains("manifest_path"));

        let environment = storage
            .create_environment(CreateEnvironmentInput {
                name: "Global".to_string(),
                scope: EnvironmentScope::Global,
                collection_id: None,
            })
            .expect("create environment");
        let environment_path = environment
            .file_path
            .clone()
            .expect("environment file path should be hydrated");
        let environment_toml =
            fs::read_to_string(&environment_path).expect("read environment manifest");
        assert!(!environment_toml.contains("file_path"));
    }

    #[test]
    fn create_request_roundtrip_persists_and_links_manifest() {
        let (_dir, storage, collection_id, collection_dir) = init_storage_with_collection();
        let collection_manifest_path = collection_dir.join(COLLECTION_MANIFEST_FILE_NAME);
        let created = storage
            .create_request(CreateRequestInput {
                parent: RequestParentRef {
                    collection_id,
                    folder_id: None,
                },
                known_parent_manifest_path: None,
                name: "List Users".to_string(),
                method: HttpMethod::Get,
                url: "https://api.example.com/users".to_string(),
            })
            .expect("create request");
        let created_path = storage
            .find_request_file_by_id(created.meta.request_id)
            .expect("find created request");
        assert_eq!(created.file_path.as_deref(), Some(created_path.as_path()));

        let loaded = storage
            .load_request(created.meta.request_id)
            .expect("load request");
        assert_eq!(loaded.meta.request_id, created.meta.request_id);
        assert_eq!(loaded.meta.name, "List Users");
        assert_eq!(loaded.request.method, HttpMethod::Get);
        assert_eq!(loaded.request.url, "https://api.example.com/users");
        assert_eq!(loaded.request.headers.len(), 2);
        assert_eq!(loaded.file_path.as_deref(), Some(created_path.as_path()));

        let store = shared_store_from_collection_manifest_path(&collection_manifest_path)
            .expect("load collection manifest");
        let collection_node = store.nodes.get(&collection_id).expect("collection node");
        assert_eq!(collection_node.children, vec![created.meta.request_id]);
        let request_node = store
            .nodes
            .get(&created.meta.request_id)
            .expect("request node");
        assert_eq!(request_node.name, "List Users");
        assert_eq!(request_node.kind, NodeKind::Request);
        assert_eq!(
            storage
                .request_dir_for_parent(
                    RequestParentRef {
                        collection_id: Ulid::new(),
                        folder_id: None,
                    },
                    Some(&KnownParentManifestPath::Collection(
                        collection_manifest_path
                    )),
                )
                .expect("resolve request dir from known collection path"),
            collection_dir
        );
    }

    #[test]
    fn duplicate_request_generates_new_id_and_new_file() {
        let (_dir, storage, collection_id, _) = init_storage_with_collection();
        let collection_manifest_path = storage
            .find_collection_manifest_path_by_collection_id(collection_id)
            .expect("find collection manifest");
        let created = storage
            .create_request(CreateRequestInput {
                parent: RequestParentRef {
                    collection_id,
                    folder_id: None,
                },
                known_parent_manifest_path: None,
                name: "Get User".to_string(),
                method: HttpMethod::Get,
                url: "https://api.example.com/users/1".to_string(),
            })
            .expect("create request");
        let next_created = storage
            .create_request(CreateRequestInput {
                parent: RequestParentRef {
                    collection_id,
                    folder_id: None,
                },
                known_parent_manifest_path: None,
                name: "List Users".to_string(),
                method: HttpMethod::Get,
                url: "https://api.example.com/users".to_string(),
            })
            .expect("create second request");

        let duplicated = storage
            .duplicate_request(DuplicateRequestInput {
                request_id: created.meta.request_id,
                duplicate_name: "Get User (Copy)".to_string(),
                parent: RequestParentRef {
                    collection_id,
                    folder_id: None,
                },
                known_request_path: created.file_path.clone(),
                known_parent_manifest_path: None,
            })
            .expect("duplicate request");

        assert_ne!(duplicated.meta.request_id, created.meta.request_id);
        assert_eq!(duplicated.meta.name, "Get User (Copy)");
        let duplicated_loaded = storage
            .load_request(duplicated.meta.request_id)
            .expect("load duplicated request");
        assert_eq!(duplicated_loaded.meta.name, "Get User (Copy)");

        let store = shared_store_from_collection_manifest_path(&collection_manifest_path)
            .expect("load collection store");
        let request_ids = store
            .nodes
            .get(&collection_id)
            .expect("collection node")
            .children
            .clone();
        assert_eq!(
            request_ids,
            vec![
                created.meta.request_id,
                duplicated.meta.request_id,
                next_created.meta.request_id
            ]
        );
    }

    #[test]
    fn rename_request_updates_storage_path_safely() {
        let (_dir, storage, collection_id, _) = init_storage_with_collection();
        let created = storage
            .create_request(CreateRequestInput {
                parent: RequestParentRef {
                    collection_id,
                    folder_id: None,
                },
                known_parent_manifest_path: None,
                name: "Old Name".to_string(),
                method: HttpMethod::Get,
                url: "https://api.example.com/items".to_string(),
            })
            .expect("create request");
        let old_path = storage
            .find_request_file_by_id(created.meta.request_id)
            .expect("find old request file");

        let renamed = storage
            .rename_request(RenameRequestInput {
                request_id: created.meta.request_id,
                new_name: "New Name".to_string(),
                known_request_path: created.file_path.clone(),
                known_parent_manifest_path: None,
            })
            .expect("rename request");

        let new_path = storage
            .find_request_file_by_id(created.meta.request_id)
            .expect("find renamed request file");
        assert_eq!(renamed.meta.name, "New Name");
        assert!(new_path.exists());
        assert!(!old_path.exists());
        assert!(new_path.to_string_lossy().contains("new-name"));
        assert!(
            !new_path
                .to_string_lossy()
                .contains(&created.meta.request_id.to_string())
        );
    }

    #[test]
    fn request_file_paths_use_manifest_slug_derivation() {
        let (_dir, storage, collection_id, collection_dir) = init_storage_with_collection();

        let created = storage
            .create_request(CreateRequestInput {
                parent: RequestParentRef {
                    collection_id,
                    folder_id: None,
                },
                known_parent_manifest_path: None,
                name: "Sample".to_string(),
                method: HttpMethod::Get,
                url: "https://api.example.com/sample".to_string(),
            })
            .expect("create request");

        let created_path = storage
            .find_request_file_by_id(created.meta.request_id)
            .expect("find request path");
        assert_eq!(created_path, collection_dir.join("sample.request.toml"));

        let renamed = storage
            .rename_request(RenameRequestInput {
                request_id: created.meta.request_id,
                new_name: "Sample".to_string(),
                known_request_path: created.file_path.clone(),
                known_parent_manifest_path: None,
            })
            .expect("rename same");
        assert_eq!(renamed.meta.name, "Sample");
        let renamed_path = storage
            .find_request_file_by_id(created.meta.request_id)
            .expect("find renamed path");
        assert_eq!(created_path, renamed_path);
    }

    #[test]
    fn environment_file_paths_use_slug_and_increment_suffix_when_colliding() {
        let (_dir, storage, _, _) = init_storage_with_collection();
        fs::create_dir_all(&storage.paths.environments_dir).expect("create environments dir");

        fs::write(
            storage
                .paths
                .environments_dir
                .join("new-environment.env.toml"),
            "reserved",
        )
        .expect("seed env collision");

        let created = storage
            .create_environment(CreateEnvironmentInput {
                name: "New Environment".to_string(),
                scope: EnvironmentScope::Global,
                collection_id: None,
            })
            .expect("create environment");

        let expected = storage
            .paths
            .environments_dir
            .join("new-environment-2.env.toml");
        assert!(expected.exists());

        let loaded: EnvironmentFile = storage
            .read_toml_file(&expected)
            .expect("read created environment file");
        assert_eq!(
            loaded.environment.environment_id,
            created.environment.environment_id
        );
        assert_eq!(loaded.environment.name, "New Environment");
    }

    #[test]
    fn rename_environment_renames_file_with_slug_name() {
        let (_dir, storage, _, _) = init_storage_with_collection();
        let created = storage
            .create_environment(CreateEnvironmentInput {
                name: "Old Name".to_string(),
                scope: EnvironmentScope::Global,
                collection_id: None,
            })
            .expect("create environment");
        let old_path = storage
            .find_environment_file_by_id(created.environment.environment_id)
            .expect("find old environment path");

        let updated = storage
            .rename_environment(created.environment.environment_id, "Renamed Environment")
            .expect("rename environment");

        let new_path = storage
            .find_environment_file_by_id(created.environment.environment_id)
            .expect("find renamed environment path");
        assert_eq!(updated.environment.name, "Renamed Environment");
        assert!(new_path.exists());
        assert!(!old_path.exists());
        assert!(
            new_path
                .to_string_lossy()
                .contains("renamed-environment.env.toml")
        );
        assert_eq!(
            updated.environment.file_name,
            "renamed-environment.env.toml"
        );
        assert_eq!(updated.file_path.as_deref(), Some(new_path.as_path()));
    }

    #[test]
    fn update_environment_variables_persists_new_values_without_renaming_file() {
        let (_dir, storage, _, _) = init_storage_with_collection();
        let created = storage
            .create_environment(CreateEnvironmentInput {
                name: "Shared Env".to_string(),
                scope: EnvironmentScope::Global,
                collection_id: None,
            })
            .expect("create environment");
        let original_path = storage
            .find_environment_file_by_id(created.environment.environment_id)
            .expect("find original environment path");
        let variables = vec![EnvironmentVariable {
            name: "BASE_URL".to_string(),
            value: "https://api.example.com".to_string(),
            enabled: true,
            secret: false,
            description: None,
        }];

        let updated = storage
            .update_environment_variables(created.environment.environment_id, variables.clone())
            .expect("update environment variables");

        let updated_path = storage
            .find_environment_file_by_id(created.environment.environment_id)
            .expect("find updated environment path");
        assert_eq!(updated_path, original_path);
        assert_eq!(updated.variables, variables);
        assert_eq!(updated.environment.name, "Shared Env");
    }

    #[test]
    fn folder_request_uses_folder_root_path_and_renames_in_place() {
        let (_dir, storage, collection_id, _) = init_storage_with_collection();
        let folder = storage
            .create_folder(CreateFolderInput {
                parent: FolderParentRef {
                    collection_id,
                    parent_folder_id: None,
                },
                known_parent_manifest_path: None,
                name: "Auth".to_string(),
            })
            .expect("create folder");

        let created = storage
            .create_request(CreateRequestInput {
                parent: RequestParentRef {
                    collection_id,
                    folder_id: Some(folder.folder.folder_id),
                },
                known_parent_manifest_path: None,
                name: "Get Token".to_string(),
                method: HttpMethod::Post,
                url: "https://api.example.com/token".to_string(),
            })
            .expect("create request in folder");
        let folder_dir = storage
            .find_folder_dir_by_id(folder.folder.folder_id)
            .expect("find folder dir");
        let created_path = storage
            .find_request_file_by_id(created.meta.request_id)
            .expect("find created request path");
        assert_eq!(
            created_path.parent().expect("request parent dir"),
            folder_dir.as_path()
        );

        let renamed = storage
            .rename_request(RenameRequestInput {
                request_id: created.meta.request_id,
                new_name: "Issue Token".to_string(),
                known_request_path: created.file_path.clone(),
                known_parent_manifest_path: folder
                    .manifest_path
                    .clone()
                    .map(KnownParentManifestPath::Folder),
            })
            .expect("rename request");
        assert_eq!(renamed.meta.name, "Issue Token");
        let renamed_path = storage
            .find_request_file_by_id(created.meta.request_id)
            .expect("find renamed request path");
        assert_eq!(
            renamed_path.parent().expect("renamed request parent dir"),
            folder_dir.as_path()
        );
    }

    #[test]
    fn create_and_rename_folder_updates_manifest_and_directory() {
        let (_dir, storage, collection_id, collection_dir) = init_storage_with_collection();
        let created = storage
            .create_folder(CreateFolderInput {
                parent: FolderParentRef {
                    collection_id,
                    parent_folder_id: None,
                },
                known_parent_manifest_path: None,
                name: "Auth".to_string(),
            })
            .expect("create folder");
        let created_dir = storage
            .find_folder_dir_by_id(created.folder.folder_id)
            .expect("find created folder dir");
        assert!(created_dir.exists());
        assert_eq!(
            created_dir.file_name().and_then(|name| name.to_str()),
            Some("auth")
        );
        assert_eq!(
            created.manifest_path.as_deref(),
            Some(collection_dir.join(COLLECTION_MANIFEST_FILE_NAME).as_path())
        );
        assert_eq!(
            storage
                .folder_dir_for_parent(
                    FolderParentRef {
                        collection_id,
                        parent_folder_id: None,
                    },
                    Some(&KnownParentManifestPath::Collection(
                        collection_dir.join(COLLECTION_MANIFEST_FILE_NAME),
                    )),
                )
                .expect("resolve folder dir from known collection path"),
            collection_dir
        );

        let renamed = storage
            .rename_folder(created.folder.folder_id, "Security")
            .expect("rename folder");
        assert_eq!(renamed.folder.name, "Security");
        let renamed_dir = storage
            .find_folder_dir_by_id(created.folder.folder_id)
            .expect("find renamed folder dir");
        assert!(renamed_dir.exists());
        assert_eq!(
            renamed_dir.file_name().and_then(|name| name.to_str()),
            Some("security")
        );
        assert!(!created_dir.exists());

        let collection_manifest_path = storage
            .find_collection_manifest_path_by_collection_id(collection_id)
            .expect("find collection manifest");
        let store = shared_store_from_collection_manifest_path(&collection_manifest_path)
            .expect("load collection store");
        let folder_node = store
            .nodes
            .get(&created.folder.folder_id)
            .expect("renamed folder node");
        assert_eq!(folder_node.name, "Security");
    }

    #[test]
    fn rename_and_delete_collection_applies_on_disk() {
        let (_dir, storage, collection_id, old_collection_dir) = init_storage_with_collection();

        let renamed = storage
            .rename_collection(collection_id, "Primary Workspace")
            .expect("rename collection");
        assert_eq!(renamed.collection.name, "Primary Workspace");
        let new_collection_dir = storage
            .find_collection_dir_by_id(collection_id)
            .expect("find renamed collection dir");
        assert!(new_collection_dir.exists());
        assert!(
            new_collection_dir
                .to_string_lossy()
                .contains("primary-workspace")
        );
        assert!(!old_collection_dir.exists());

        storage
            .delete_collection(collection_id)
            .expect("delete collection");
        assert!(!new_collection_dir.exists());
    }

    #[test]
    fn delete_folder_removes_folder_from_parent_manifest() {
        let (_dir, storage, collection_id, _) = init_storage_with_collection();
        let folder = storage
            .create_folder(CreateFolderInput {
                parent: FolderParentRef {
                    collection_id,
                    parent_folder_id: None,
                },
                known_parent_manifest_path: None,
                name: "Temp".to_string(),
            })
            .expect("create folder");
        let folder_id = folder.folder.folder_id;
        let folder_dir = storage
            .find_folder_dir_by_id(folder_id)
            .expect("find folder dir");
        assert!(folder_dir.exists());

        storage.delete_folder(folder_id).expect("delete folder");
        assert!(!folder_dir.exists());

        let collection_manifest_path = storage
            .find_collection_manifest_path_by_collection_id(collection_id)
            .expect("find collection manifest");
        let store = shared_store_from_collection_manifest_path(&collection_manifest_path)
            .expect("load collection store");
        assert!(!store.nodes.contains_key(&folder_id));
        assert!(
            !store
                .nodes
                .get(&collection_id)
                .expect("collection node")
                .children
                .contains(&folder_id)
        );
    }

    #[test]
    fn move_request_reorders_within_same_parent_without_renaming_file() {
        let (_dir, storage, collection_id, collection_dir) = init_storage_with_collection();
        let manifest_path = collection_dir.join(COLLECTION_MANIFEST_FILE_NAME);
        let first = storage
            .create_request(CreateRequestInput {
                parent: RequestParentRef {
                    collection_id,
                    folder_id: None,
                },
                known_parent_manifest_path: None,
                name: "First".to_string(),
                method: HttpMethod::Get,
                url: "https://example.com/first".to_string(),
            })
            .expect("create first request");
        let second = storage
            .create_request(CreateRequestInput {
                parent: RequestParentRef {
                    collection_id,
                    folder_id: None,
                },
                known_parent_manifest_path: None,
                name: "Second".to_string(),
                method: HttpMethod::Get,
                url: "https://example.com/second".to_string(),
            })
            .expect("create second request");
        let third = storage
            .create_request(CreateRequestInput {
                parent: RequestParentRef {
                    collection_id,
                    folder_id: None,
                },
                known_parent_manifest_path: None,
                name: "Third".to_string(),
                method: HttpMethod::Get,
                url: "https://example.com/third".to_string(),
            })
            .expect("create third request");
        let first_path = first.file_path.clone().expect("first path");

        let moved = storage
            .move_request(MoveRequestInput {
                request_id: first.meta.request_id,
                new_parent: RequestParentRef {
                    collection_id,
                    folder_id: None,
                },
                insertion_index: 3,
                known_request_path: first.file_path.clone(),
                known_target_manifest_path: Some(KnownParentManifestPath::Collection(
                    manifest_path.clone(),
                )),
            })
            .expect("reorder request");

        assert_eq!(moved.file_path.as_deref(), Some(first_path.as_path()));
        assert!(first_path.exists());

        let store = shared_store_from_collection_manifest_path(&manifest_path)
            .expect("load manifest after reorder");
        assert_eq!(
            store
                .nodes
                .get(&collection_id)
                .expect("collection node")
                .children,
            vec![
                second.meta.request_id,
                third.meta.request_id,
                first.meta.request_id
            ]
        );
    }

    #[test]
    fn move_request_moves_file_between_parents() {
        let (_dir, storage, collection_id, collection_dir) = init_storage_with_collection();
        let folder = storage
            .create_folder(CreateFolderInput {
                parent: FolderParentRef {
                    collection_id,
                    parent_folder_id: None,
                },
                known_parent_manifest_path: None,
                name: "Auth".to_string(),
            })
            .expect("create folder");
        let request = storage
            .create_request(CreateRequestInput {
                parent: RequestParentRef {
                    collection_id,
                    folder_id: None,
                },
                known_parent_manifest_path: None,
                name: "Login".to_string(),
                method: HttpMethod::Post,
                url: "https://example.com/login".to_string(),
            })
            .expect("create request");
        let old_path = request.file_path.clone().expect("request path");

        let moved = storage
            .move_request(MoveRequestInput {
                request_id: request.meta.request_id,
                new_parent: RequestParentRef {
                    collection_id,
                    folder_id: Some(folder.folder.folder_id),
                },
                insertion_index: 0,
                known_request_path: request.file_path.clone(),
                known_target_manifest_path: folder
                    .manifest_path
                    .clone()
                    .map(KnownParentManifestPath::Folder),
            })
            .expect("move request");

        let new_path = moved.file_path.clone().expect("moved request path");
        assert_ne!(new_path, old_path);
        assert!(!old_path.exists());
        assert!(new_path.exists());
        let folder_dir = storage
            .find_folder_dir_by_id(folder.folder.folder_id)
            .expect("find folder dir");
        assert_eq!(new_path.parent(), Some(folder_dir.as_path()));

        let store = shared_store_from_collection_manifest_path(
            &collection_dir.join(COLLECTION_MANIFEST_FILE_NAME),
        )
        .expect("load manifest after move");
        assert_eq!(
            store
                .nodes
                .get(&folder.folder.folder_id)
                .expect("folder node")
                .children,
            vec![request.meta.request_id]
        );
    }

    #[test]
    fn move_folder_reorders_within_same_parent_without_renaming_directory() {
        let (_dir, storage, collection_id, collection_dir) = init_storage_with_collection();
        let manifest_path = collection_dir.join(COLLECTION_MANIFEST_FILE_NAME);
        let first = storage
            .create_folder(CreateFolderInput {
                parent: FolderParentRef {
                    collection_id,
                    parent_folder_id: None,
                },
                known_parent_manifest_path: None,
                name: "First".to_string(),
            })
            .expect("create first folder");
        let second = storage
            .create_folder(CreateFolderInput {
                parent: FolderParentRef {
                    collection_id,
                    parent_folder_id: None,
                },
                known_parent_manifest_path: None,
                name: "Second".to_string(),
            })
            .expect("create second folder");
        let first_dir = storage
            .find_folder_dir_by_id(first.folder.folder_id)
            .expect("find first folder dir");

        let moved = storage
            .move_folder(MoveFolderInput {
                folder_id: first.folder.folder_id,
                new_parent: FolderParentRef {
                    collection_id,
                    parent_folder_id: None,
                },
                insertion_index: 2,
                known_folder_manifest_path: first.manifest_path.clone(),
                known_target_manifest_path: Some(KnownParentManifestPath::Collection(
                    manifest_path.clone(),
                )),
            })
            .expect("reorder folder");

        assert_eq!(
            moved.manifest_path.as_deref(),
            Some(manifest_path.as_path())
        );
        assert!(first_dir.exists());
        let store = shared_store_from_collection_manifest_path(&manifest_path)
            .expect("load manifest after folder reorder");
        assert_eq!(
            store
                .nodes
                .get(&collection_id)
                .expect("collection node")
                .children,
            vec![second.folder.folder_id, first.folder.folder_id]
        );
    }

    #[test]
    fn move_folder_moves_directory_between_collections() {
        let (_dir, storage, source_collection_id, source_collection_dir) =
            init_storage_with_collection();
        let (destination_collection_id, destination_collection_dir) =
            create_collection_manifest(&storage, "target", "Target");
        let folder = storage
            .create_folder(CreateFolderInput {
                parent: FolderParentRef {
                    collection_id: source_collection_id,
                    parent_folder_id: None,
                },
                known_parent_manifest_path: None,
                name: "Moved Folder".to_string(),
            })
            .expect("create folder");
        let request = storage
            .create_request(CreateRequestInput {
                parent: RequestParentRef {
                    collection_id: source_collection_id,
                    folder_id: Some(folder.folder.folder_id),
                },
                known_parent_manifest_path: folder
                    .manifest_path
                    .clone()
                    .map(KnownParentManifestPath::Folder),
                name: "Nested Request".to_string(),
                method: HttpMethod::Get,
                url: "https://example.com/nested".to_string(),
            })
            .expect("create nested request");
        let old_folder_dir = storage
            .find_folder_dir_by_id(folder.folder.folder_id)
            .expect("find old folder dir");

        let moved = storage
            .move_folder(MoveFolderInput {
                folder_id: folder.folder.folder_id,
                new_parent: FolderParentRef {
                    collection_id: destination_collection_id,
                    parent_folder_id: None,
                },
                insertion_index: 0,
                known_folder_manifest_path: folder.manifest_path.clone(),
                known_target_manifest_path: Some(KnownParentManifestPath::Collection(
                    destination_collection_dir.join(COLLECTION_MANIFEST_FILE_NAME),
                )),
            })
            .expect("move folder across collections");

        let new_folder_dir = storage
            .find_folder_dir_by_id(folder.folder.folder_id)
            .expect("find moved folder dir");
        assert_ne!(new_folder_dir, old_folder_dir);
        assert!(!old_folder_dir.exists());
        assert!(new_folder_dir.exists());
        assert_eq!(
            moved.manifest_path.as_deref(),
            Some(
                destination_collection_dir
                    .join(COLLECTION_MANIFEST_FILE_NAME)
                    .as_path()
            )
        );

        let moved_request_path = storage
            .find_request_file_by_id(request.meta.request_id)
            .expect("find moved nested request");
        assert_eq!(moved_request_path.parent(), Some(new_folder_dir.as_path()));

        let source_store = shared_store_from_collection_manifest_path(
            &source_collection_dir.join(COLLECTION_MANIFEST_FILE_NAME),
        )
        .expect("load source manifest");
        let destination_store = shared_store_from_collection_manifest_path(
            &destination_collection_dir.join(COLLECTION_MANIFEST_FILE_NAME),
        )
        .expect("load destination manifest");
        assert!(
            !source_store.nodes.contains_key(&folder.folder.folder_id),
            "source manifest should no longer contain moved folder"
        );
        assert_eq!(
            destination_store
                .nodes
                .get(&destination_collection_id)
                .expect("destination collection node")
                .children,
            vec![folder.folder.folder_id]
        );
    }

    #[test]
    fn reorder_collection_persists_root_order() {
        let (_dir, storage, first_collection_id, _) = init_storage_with_collection();
        let (second_collection_id, _) = create_collection_manifest(&storage, "z-second", "Second");
        let mut seed_store = SharedStore::default();
        seed_store.root_ids = vec![first_collection_id, second_collection_id];
        write_root_order(&storage.paths, &seed_store).expect("seed root order");

        storage
            .reorder_collection(ReorderCollectionInput {
                collection_id: first_collection_id,
                insertion_index: 2,
            })
            .expect("reorder collection");

        let root_order: crate::tree_store::RootOrderFile = storage
            .read_toml_file(&storage.paths.collections_root_order_file)
            .expect("read root order file");
        assert_eq!(
            root_order.root_ids,
            vec![second_collection_id, first_collection_id]
        );
    }
}
