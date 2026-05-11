use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use ulid::Ulid;

use crate::error::{BeamError, Result};
use crate::models::{
    AuthConfig, BodyConfig, CollectionFile, CollectionItemRef, EnvironmentFile, EnvironmentMeta,
    EnvironmentScope, EnvironmentVariable, FolderFile, FolderMeta, HeaderField, ItemType,
    LocalState, LocalStateFile, QueryParamField, RequestDefinition, RequestFile, RequestMeta,
    ScriptConfig, TreeState, WorkspaceFile, WorkspaceMeta,
};
use crate::paths::BeamPaths;
use crate::schema::{SCHEMA_VERSION_V1, SchemaKind, validate_schema_version};
use crate::storage::{
    BootstrapReport, CreateEnvironmentInput, CreateFolderInput, CreateRequestInput,
    FolderParentRef, RequestParentRef, WorkspaceStorage,
};

#[derive(Debug, Clone)]
pub struct TomlWorkspaceStorage {
    pub paths: BeamPaths,
}

impl TomlWorkspaceStorage {
    pub fn new(paths: BeamPaths) -> Self {
        Self { paths }
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

    fn write_toml_file<T: serde::Serialize>(&self, path: &Path, value: &T) -> Result<()> {
        let encoded = toml::to_string_pretty(value)?;
        atomic_write(path, encoded.as_bytes())
    }

    fn read_toml_file<T: for<'de> serde::Deserialize<'de>>(&self, path: &Path) -> Result<T> {
        let content = fs::read_to_string(path).map_err(|source| BeamError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        toml::from_str(&content).map_err(|source| BeamError::TomlDecode {
            path: path.to_path_buf(),
            source,
        })
    }

    fn merge_nested_local_state_fields(&self, local_state: &mut LocalStateFile) {
        let Ok(parsed_file) =
            self.read_toml_file::<LocalStateNestedFile>(&self.paths.local_state_file)
        else {
            return;
        };

        if local_state.tree_state.expanded_item_ids.is_empty()
            && !parsed_file.local_state.expanded_item_ids.is_empty()
        {
            local_state.tree_state.expanded_item_ids = parsed_file.local_state.expanded_item_ids;
        }

        if local_state.collection_environment_selection.is_empty()
            && !parsed_file
                .local_state
                .collection_environment_selections
                .is_empty()
        {
            local_state.collection_environment_selection = parsed_file
                .local_state
                .collection_environment_selections
                .into_iter()
                .map(|entry| (entry.collection_id, entry.environment_id))
                .collect();
        }
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
        let mut found: Option<PathBuf> = None;
        self.walk_files_recursive(&self.paths.collections_dir, |path| {
            if path.file_name().and_then(|n| n.to_str()) != Some("collection.toml") {
                return;
            }
            let Ok(file) = self.read_collection_file(path) else {
                return;
            };
            if file.collection.collection_id == collection_id {
                found = path.parent().map(Path::to_path_buf);
            }
        })?;
        found.ok_or_else(|| BeamError::NotFound {
            entity: "collection",
            id: collection_id.to_string(),
        })
    }

    fn find_folder_dir_by_id(&self, folder_id: Ulid) -> Result<PathBuf> {
        let mut found: Option<PathBuf> = None;
        self.walk_files_recursive(&self.paths.collections_dir, |path| {
            if path.file_name().and_then(|n| n.to_str()) != Some("folder.toml") {
                return;
            }
            let Ok(file) = self.read_folder_file(path) else {
                return;
            };
            if file.folder.folder_id == folder_id {
                found = path.parent().map(Path::to_path_buf);
            }
        })?;
        found.ok_or_else(|| BeamError::NotFound {
            entity: "folder",
            id: folder_id.to_string(),
        })
    }

    fn request_dir_for_parent(&self, parent: RequestParentRef) -> Result<PathBuf> {
        if let Some(folder_id) = parent.folder_id {
            return self.find_folder_dir_by_id(folder_id);
        }
        let collection_dir = self.find_collection_dir_by_id(parent.collection_id)?;
        Ok(collection_dir.join("requests"))
    }

    fn folder_dir_for_parent(&self, parent: FolderParentRef) -> Result<PathBuf> {
        if let Some(parent_folder_id) = parent.parent_folder_id {
            return self.find_folder_dir_by_id(parent_folder_id);
        }
        self.find_collection_dir_by_id(parent.collection_id)
    }

    fn append_folder_to_parent_manifest(
        &self,
        parent: FolderParentRef,
        folder_id: Ulid,
        name: &str,
    ) -> Result<()> {
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
        request_id: Ulid,
        name: &str,
    ) -> Result<()> {
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

    fn update_folder_name_in_parent_manifest(&self, folder_id: Ulid, new_name: &str) -> Result<()> {
        let parent = self.find_folder_parent(folder_id)?;
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
        request_id: Ulid,
    ) -> Result<()> {
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

    fn remove_folder_from_parent_manifest(&self, folder_id: Ulid) -> Result<()> {
        let parent = self.find_folder_parent(folder_id)?;
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
        skip_folder_id: Option<Ulid>,
    ) -> Result<Vec<String>> {
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
    ) -> Result<PathBuf> {
        let request_dir = self.request_dir_for_parent(parent)?;
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
                let Ok(file) = self.read_toml_file::<EnvironmentFile>(path) else {
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
        let mut found: Option<PathBuf> = None;
        self.walk_files_recursive(&self.paths.collections_dir, |path| {
            if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
                return;
            }
            let Ok(file) = self.read_request_meta_by_path(path) else {
                return;
            };
            if file.meta.request_id == request_id {
                found = Some(path.to_path_buf());
            }
        })?;
        found.ok_or_else(|| BeamError::NotFound {
            entity: "request",
            id: request_id.to_string(),
        })
    }

    fn find_request_file_in_dir(&self, dir: &Path, request_id: Ulid) -> Result<PathBuf> {
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
            if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
                continue;
            }
            let Ok(file) = self.read_request_meta_by_path(&path) else {
                continue;
            };
            if file.meta.request_id == request_id {
                return Ok(path);
            }
        }
        Err(BeamError::NotFound {
            entity: "request",
            id: request_id.to_string(),
        })
    }

    fn read_request_meta_by_path(&self, path: &Path) -> Result<RequestMetaIdFile> {
        if let Ok(file) = self.read_toml_file::<RequestMetaIdFile>(path) {
            return Ok(file);
        }

        let request_file: RequestFile = self.read_toml_file(path)?;
        Ok(RequestMetaIdFile {
            meta: RequestMetaIdOnly {
                request_id: request_file.meta.request_id,
            },
        })
    }

    fn read_request_file(&self, path: &Path) -> Result<RequestFile> {
        self.read_toml_file(path)
    }

    fn read_collection_file(&self, path: &Path) -> Result<CollectionFile> {
        self.read_toml_file(path)
    }

    fn read_folder_file(&self, path: &Path) -> Result<FolderFile> {
        self.read_toml_file(path)
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
        if let Ok(workspace) = self.read_toml_file::<WorkspaceFile>(&self.paths.workspace_file) {
            validate_schema_version(SchemaKind::Workspace, workspace.schema_version)?;
            return Ok(workspace);
        }

        // TODO: the workspace file is read twice, can consolidate this.
        let parsed_file: WorkspaceTomlFile = self.read_toml_file(&self.paths.workspace_file)?;
        validate_schema_version(SchemaKind::Workspace, parsed_file.workspace.schema_version)?;
        Ok(WorkspaceFile {
            schema_version: parsed_file.workspace.schema_version,
            workspace: WorkspaceMeta {
                workspace_id: parsed_file.workspace.workspace_id,
                name: parsed_file.workspace.name,
                description: parsed_file.workspace.description,
                created_at: parsed_file.workspace.created_at,
                updated_at: parsed_file.workspace.updated_at,
            },
        })
    }

    fn save_workspace(&self, workspace_file: &WorkspaceFile) -> Result<()> {
        validate_schema_version(SchemaKind::Workspace, workspace_file.schema_version)?;
        self.write_toml_file(&self.paths.workspace_file, workspace_file)
    }

    fn load_local_state(&self) -> Result<LocalStateFile> {
        if let Ok(mut local_state) =
            self.read_toml_file::<LocalStateFile>(&self.paths.local_state_file)
        {
            validate_schema_version(SchemaKind::LocalState, local_state.schema_version)?;
            self.merge_nested_local_state_fields(&mut local_state);
            return Ok(local_state);
        }

        // TODO: the local-state file is read twice, consolidate this.
        let parsed_file: LocalStateTomlFile = self.read_toml_file(&self.paths.local_state_file)?;
        validate_schema_version(
            SchemaKind::LocalState,
            parsed_file.local_state.schema_version,
        )?;
        Ok(LocalStateFile {
            schema_version: parsed_file.local_state.schema_version,
            local_state: LocalState {
                active_global_environment_id: parsed_file.local_state.active_global_environment_id,
                last_opened_request_id: parsed_file.local_state.last_opened_request_id,
                theme_name: parsed_file.local_state.theme_name,
                updated_at: parsed_file.local_state.updated_at,
            },
            collection_environment_selection: parsed_file
                .local_state
                .collection_environment_selections
                .into_iter()
                .map(|entry| (entry.collection_id, entry.environment_id))
                .collect(),
            // TODO: tree_state isn't better named?
            tree_state: TreeState {
                expanded_item_ids: parsed_file.local_state.expanded_item_ids,
            },
        })
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
        let name = input.name.trim();
        if name.is_empty() {
            return Err(BeamError::Validation {
                message: "Request name cannot be empty".to_string(),
            });
        }
        let now = Utc::now();
        let request_file = RequestFile {
            meta: RequestMeta {
                request_id: Ulid::new(),
                name: name.to_string(),
                description: None,
                created_at: now,
                updated_at: now,
            },
            request: RequestDefinition {
                method: input.method,
                url: input.url,
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
        };
        self.write_request_new_path(&request_file, input.parent)?;
        self.append_request_to_parent_manifest(
            input.parent,
            request_file.meta.request_id,
            &request_file.meta.name,
        )?;
        Ok(request_file)
    }

    fn create_request_after(
        &self,
        input: CreateRequestInput,
        source_request_id: Ulid,
    ) -> Result<RequestFile> {
        let name = input.name.trim();
        if name.is_empty() {
            return Err(BeamError::Validation {
                message: "Request name cannot be empty".to_string(),
            });
        }
        let now = Utc::now();
        let request_file = RequestFile {
            meta: RequestMeta {
                request_id: Ulid::new(),
                name: name.to_string(),
                description: None,
                created_at: now,
                updated_at: now,
            },
            request: RequestDefinition {
                method: input.method,
                url: input.url,
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
        };
        self.write_request_new_path(&request_file, input.parent)?;
        self.insert_request_to_parent_manifest_after(
            input.parent,
            source_request_id,
            request_file.meta.request_id,
            &request_file.meta.name,
        )?;
        Ok(request_file)
    }

    fn create_folder(&self, input: CreateFolderInput) -> Result<FolderFile> {
        let name = input.name.trim();
        if name.is_empty() {
            return Err(BeamError::Validation {
                message: "Folder name cannot be empty".to_string(),
            });
        }
        let sibling_names = self.sibling_folder_names(input.parent, None)?;
        if sibling_names
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(name))
        {
            return Err(BeamError::Validation {
                message: format!("A folder named '{name}' already exists in this scope"),
            });
        }

        let now = Utc::now();
        let folder_file = FolderFile {
            folder: FolderMeta {
                folder_id: Ulid::new(),
                collection_id: input.parent.collection_id,
                parent_folder_id: input.parent.parent_folder_id,
                name: name.to_string(),
                description: None,
                created_at: now,
                updated_at: now,
            },
            items: Vec::new(),
        };

        let parent_dir = self.folder_dir_for_parent(input.parent)?;
        let folder_dir_name = format!("{}-{}", slugify(name), folder_file.folder.folder_id);
        let folder_dir = parent_dir.join(folder_dir_name);
        fs::create_dir_all(&folder_dir).map_err(|source| BeamError::Io {
            path: folder_dir.clone(),
            source,
        })?;
        self.write_toml_file(&folder_dir.join("folder.toml"), &folder_file)?;
        self.append_folder_to_parent_manifest(input.parent, folder_file.folder.folder_id, name)?;
        Ok(folder_file)
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
        };
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
        let mut environment_file: EnvironmentFile = self.read_toml_file(&existing_path)?;
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
        self.write_toml_file(&next_path, &environment_file)?;
        Ok(environment_file)
    }

    fn update_environment_variables(
        &self,
        environment_id: Ulid,
        variables: Vec<EnvironmentVariable>,
    ) -> Result<EnvironmentFile> {
        let existing_path = self.find_environment_file_by_id(environment_id)?;
        let mut environment_file: EnvironmentFile = self.read_toml_file(&existing_path)?;
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

    fn rename_request(&self, request_id: Ulid, new_name: &str) -> Result<RequestFile> {
        let existing_path = self.find_request_file_by_id(request_id)?;
        let mut request_file = self.read_request_file(&existing_path)?;
        let parent = self.find_request_parent(request_id)?;
        let next_name = new_name.trim();
        if next_name.is_empty() {
            return Err(BeamError::Validation {
                message: "Request name cannot be empty".to_string(),
            });
        }

        request_file.meta.name = next_name.to_string();
        request_file.meta.updated_at = Utc::now();
        let request_dir = existing_path.parent().ok_or_else(|| BeamError::NotFound {
            entity: "request_parent_dir",
            id: existing_path.to_string_lossy().to_string(),
        })?;
        enum OwnerManifest {
            Folder(FolderFile, PathBuf),
            Collection(CollectionFile, PathBuf),
        }
        let owner_manifest = if let Some(folder_id) = parent.folder_id {
            let folder_dir = self.find_folder_dir_by_id(folder_id)?;
            let folder_manifest = folder_dir.join("folder.toml");
            OwnerManifest::Folder(self.read_folder_file(&folder_manifest)?, folder_manifest)
        } else {
            let collection_dir = self.find_collection_dir_by_id(parent.collection_id)?;
            let collection_manifest = collection_dir.join("collection.toml");
            OwnerManifest::Collection(
                self.read_collection_file(&collection_manifest)?,
                collection_manifest,
            )
        };
        let has_duplicate_name = match &owner_manifest {
            OwnerManifest::Folder(file, _) => file.items.iter().any(|item| {
                item.item_type == ItemType::Request
                    && item.item_id != request_id
                    && item.name.eq_ignore_ascii_case(next_name)
            }),
            OwnerManifest::Collection(file, _) => file.items.iter().any(|item| {
                item.item_type == ItemType::Request
                    && item.item_id != request_id
                    && item.name.eq_ignore_ascii_case(next_name)
            }),
        };
        if has_duplicate_name {
            return Err(BeamError::Validation {
                message: format!("A request named '{next_name}' already exists in this scope"),
            });
        }
        let new_path =
            self.request_file_path_for_name(request_dir, next_name, Some(&existing_path))?;
        if new_path != existing_path {
            fs::rename(&existing_path, &new_path).map_err(|source| BeamError::Io {
                path: existing_path.clone(),
                source,
            })?;
        }
        self.write_toml_file(&new_path, &request_file)?;
        match owner_manifest {
            OwnerManifest::Folder(mut file, manifest_path) => {
                for item in &mut file.items {
                    if item.item_id == request_id && item.item_type == ItemType::Request {
                        item.name = next_name.to_string();
                    }
                }
                self.write_toml_file(&manifest_path, &file)?;
            }
            OwnerManifest::Collection(mut file, manifest_path) => {
                for item in &mut file.items {
                    if item.item_id == request_id && item.item_type == ItemType::Request {
                        item.name = next_name.to_string();
                    }
                }
                self.write_toml_file(&manifest_path, &file)?;
            }
        }
        Ok(request_file)
    }

    fn rename_collection(&self, collection_id: Ulid, new_name: &str) -> Result<CollectionFile> {
        let next_name = new_name.trim();
        if next_name.is_empty() {
            return Err(BeamError::Validation {
                message: "Collection name cannot be empty".to_string(),
            });
        }
        let sibling_names = self.sibling_collection_names(Some(collection_id))?;
        if sibling_names
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(next_name))
        {
            return Err(BeamError::Validation {
                message: format!("A collection named '{next_name}' already exists"),
            });
        }

        let old_dir = self.find_collection_dir_by_id(collection_id)?;
        let path = old_dir.join("collection.toml");
        let mut collection_file = self.read_collection_file(&path)?;
        collection_file.collection.name = next_name.to_string();
        collection_file.collection.updated_at = Utc::now();
        self.write_toml_file(&path, &collection_file)?;

        let parent_dir = old_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.paths.collections_dir.clone());
        let old_name = old_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let new_dir_name = slugify(next_name);
        if !new_dir_name.is_empty() && new_dir_name != old_name {
            let new_dir = parent_dir.join(&new_dir_name);
            if !new_dir.exists() {
                fs::rename(&old_dir, &new_dir).map_err(|source| BeamError::Io {
                    path: old_dir,
                    source,
                })?;
            }
        }
        Ok(collection_file)
    }

    fn rename_folder(&self, folder_id: Ulid, new_name: &str) -> Result<FolderFile> {
        let next_name = new_name.trim();
        if next_name.is_empty() {
            return Err(BeamError::Validation {
                message: "Folder name cannot be empty".to_string(),
            });
        }
        let parent = self.find_folder_parent(folder_id)?;
        let sibling_names = self.sibling_folder_names(parent, Some(folder_id))?;
        if sibling_names
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(next_name))
        {
            return Err(BeamError::Validation {
                message: format!("A folder named '{next_name}' already exists in this scope"),
            });
        }

        let old_dir = self.find_folder_dir_by_id(folder_id)?;
        let path = old_dir.join("folder.toml");
        let mut folder_file = self.read_folder_file(&path)?;
        folder_file.folder.name = next_name.to_string();
        folder_file.folder.updated_at = Utc::now();
        self.write_toml_file(&path, &folder_file)?;
        self.update_folder_name_in_parent_manifest(folder_id, next_name)?;

        let parent_dir =
            old_dir
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| BeamError::NotFound {
                    entity: "folder_parent_dir",
                    id: folder_id.to_string(),
                })?;
        let old_name = old_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let new_dir_name = format!("{}-{}", slugify(next_name), folder_id);
        if new_dir_name != old_name {
            let new_dir = parent_dir.join(&new_dir_name);
            if !new_dir.exists() {
                fs::rename(&old_dir, &new_dir).map_err(|source| BeamError::Io {
                    path: old_dir,
                    source,
                })?;
            }
        }
        Ok(folder_file)
    }

    fn duplicate_request(
        &self,
        request_id: Ulid,
        duplicate_name: &str,
        parent: RequestParentRef,
    ) -> Result<RequestFile> {
        let request_dir = self.request_dir_for_parent(parent)?;
        let source_path = self.find_request_file_in_dir(&request_dir, request_id)?;
        let source = self.read_request_file(&source_path)?;
        let name = duplicate_name.trim();
        if name.is_empty() {
            return Err(BeamError::Validation {
                message: "Duplicate request name cannot be empty".to_string(),
            });
        }
        let now = Utc::now();
        let mut duplicated = source.clone();
        duplicated.meta.request_id = Ulid::new();
        duplicated.meta.name = name.to_string();
        duplicated.meta.created_at = now;
        duplicated.meta.updated_at = now;

        self.write_request_new_path_with_dir(&duplicated, &request_dir)?;
        self.insert_request_to_parent_manifest_after(
            parent,
            request_id,
            duplicated.meta.request_id,
            &duplicated.meta.name,
        )?;
        Ok(duplicated)
    }

    fn delete_collection(&self, collection_id: Ulid) -> Result<()> {
        let collection_dir = self.find_collection_dir_by_id(collection_id)?;
        fs::remove_dir_all(&collection_dir).map_err(|source| BeamError::Io {
            path: collection_dir,
            source,
        })
    }

    fn delete_folder(&self, folder_id: Ulid) -> Result<()> {
        let folder_dir = self.find_folder_dir_by_id(folder_id)?;
        self.remove_folder_from_parent_manifest(folder_id)?;
        fs::remove_dir_all(&folder_dir).map_err(|source| BeamError::Io {
            path: folder_dir,
            source,
        })
    }

    fn delete_request(&self, request_id: Ulid) -> Result<()> {
        let request_path = self.find_request_file_by_id(request_id)?;
        let parent = self.find_request_parent(request_id)?;

        fs::remove_file(&request_path).map_err(|source| BeamError::Io {
            path: request_path,
            source,
        })?;
        self.remove_request_from_parent_manifest(parent, request_id)
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct WorkspaceTomlFile {
    workspace: WorkspaceTomlMeta,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct WorkspaceTomlMeta {
    schema_version: u32,
    workspace_id: Ulid,
    name: String,
    description: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct RequestMetaIdFile {
    meta: RequestMetaIdOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct RequestMetaIdOnly {
    request_id: Ulid,
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::models::{
        AuthConfig, BodyConfig, CollectionFile, CollectionMeta, HttpMethod, RequestDefinition,
        RequestFile, RequestMeta, ScriptConfig,
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
    fn load_local_state_recovers_nested_expanded_and_selection_fields() {
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
        assert_eq!(loaded.tree_state.expanded_item_ids, vec![expanded_id]);
        assert_eq!(
            loaded
                .collection_environment_selection
                .get(&collection_id)
                .copied(),
            Some(environment_id)
        );
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
        let collection_file = CollectionFile {
            collection: CollectionMeta {
                collection_id,
                name: "Sample".to_string(),
                description: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            items: Vec::new(),
        };
        storage
            .write_toml_file(&collection_dir.join("collection.toml"), &collection_file)
            .expect("save collection file");

        (dir, storage, collection_id, collection_dir)
    }

    #[test]
    fn create_request_roundtrip_persists_and_links_manifest() {
        let (_dir, storage, collection_id, _collection_dir) = init_storage_with_collection();
        let created = storage
            .create_request(CreateRequestInput {
                parent: RequestParentRef {
                    collection_id,
                    folder_id: None,
                },
                name: "List Users".to_string(),
                method: HttpMethod::Get,
                url: "https://api.example.com/users".to_string(),
            })
            .expect("create request");

        let loaded = storage
            .load_request(created.meta.request_id)
            .expect("load request");
        assert_eq!(loaded.meta.request_id, created.meta.request_id);
        assert_eq!(loaded.meta.name, "List Users");
        assert_eq!(loaded.request.method, HttpMethod::Get);
        assert_eq!(loaded.request.url, "https://api.example.com/users");
        assert_eq!(loaded.request.headers.len(), 2);

        let collection_dir = storage
            .find_collection_dir_by_id(collection_id)
            .expect("find collection dir");
        let collection_file: CollectionFile = storage
            .read_toml_file(&collection_dir.join("collection.toml"))
            .expect("load collection");
        assert!(collection_file.items.iter().any(|item| {
            item.item_id == created.meta.request_id
                && item.item_type == ItemType::Request
                && item.name == "List Users"
        }));
    }

    #[test]
    fn duplicate_request_generates_new_id_and_new_file() {
        let (_dir, storage, collection_id, _) = init_storage_with_collection();
        let created = storage
            .create_request(CreateRequestInput {
                parent: RequestParentRef {
                    collection_id,
                    folder_id: None,
                },
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
                name: "List Users".to_string(),
                method: HttpMethod::Get,
                url: "https://api.example.com/users".to_string(),
            })
            .expect("create second request");

        let duplicated = storage
            .duplicate_request(
                created.meta.request_id,
                "Get User (Copy)",
                RequestParentRef {
                    collection_id,
                    folder_id: None,
                },
            )
            .expect("duplicate request");

        assert_ne!(duplicated.meta.request_id, created.meta.request_id);
        assert_eq!(duplicated.meta.name, "Get User (Copy)");
        let duplicated_loaded = storage
            .load_request(duplicated.meta.request_id)
            .expect("load duplicated request");
        assert_eq!(duplicated_loaded.meta.name, "Get User (Copy)");

        let collection_dir = storage
            .find_collection_dir_by_id(collection_id)
            .expect("find collection dir");
        let mut ordered_items = storage
            .read_collection_file(&collection_dir.join("collection.toml"))
            .expect("load collection")
            .items;
        ordered_items.sort_by(|a, b| {
            a.order
                .cmp(&b.order)
                .then_with(|| a.item_id.to_string().cmp(&b.item_id.to_string()))
        });
        let request_ids: Vec<Ulid> = ordered_items
            .into_iter()
            .filter(|item| item.item_type == ItemType::Request)
            .map(|item| item.item_id)
            .collect();
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
                name: "Old Name".to_string(),
                method: HttpMethod::Get,
                url: "https://api.example.com/items".to_string(),
            })
            .expect("create request");
        let old_path = storage
            .find_request_file_by_id(created.meta.request_id)
            .expect("find old request file");

        let renamed = storage
            .rename_request(created.meta.request_id, "New Name")
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
    fn request_file_paths_use_slug_and_increment_suffix_when_colliding() {
        let (_dir, storage, collection_id, collection_dir) = init_storage_with_collection();
        let request_dir = collection_dir.join("requests");
        fs::create_dir_all(&request_dir).expect("create requests dir");

        // Reserve the default file name to force numeric suffix allocation.
        fs::write(request_dir.join("sample.request.toml"), "reserved").expect("seed collision");

        let created = storage
            .create_request(CreateRequestInput {
                parent: RequestParentRef {
                    collection_id,
                    folder_id: None,
                },
                name: "Sample".to_string(),
                method: HttpMethod::Get,
                url: "https://api.example.com/sample".to_string(),
            })
            .expect("create request");

        let created_path = storage
            .find_request_file_by_id(created.meta.request_id)
            .expect("find request path");
        assert!(
            created_path
                .to_string_lossy()
                .contains("sample-2.request.toml")
        );

        let renamed = storage
            .rename_request(created.meta.request_id, "Sample")
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
                name: "Auth".to_string(),
            })
            .expect("create folder");

        let created = storage
            .create_request(CreateRequestInput {
                parent: RequestParentRef {
                    collection_id,
                    folder_id: Some(folder.folder.folder_id),
                },
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
            .rename_request(created.meta.request_id, "Issue Token")
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
        let (_dir, storage, collection_id, _) = init_storage_with_collection();
        let created = storage
            .create_folder(CreateFolderInput {
                parent: FolderParentRef {
                    collection_id,
                    parent_folder_id: None,
                },
                name: "Auth".to_string(),
            })
            .expect("create folder");
        let created_dir = storage
            .find_folder_dir_by_id(created.folder.folder_id)
            .expect("find created folder dir");
        assert!(created_dir.exists());

        let renamed = storage
            .rename_folder(created.folder.folder_id, "Security")
            .expect("rename folder");
        assert_eq!(renamed.folder.name, "Security");
        let renamed_dir = storage
            .find_folder_dir_by_id(created.folder.folder_id)
            .expect("find renamed folder dir");
        assert!(renamed_dir.exists());
        assert!(renamed_dir.to_string_lossy().contains("security"));
        assert!(!created_dir.exists());

        let collection_dir = storage
            .find_collection_dir_by_id(collection_id)
            .expect("find collection dir");
        let collection_file: CollectionFile = storage
            .read_toml_file(&collection_dir.join("collection.toml"))
            .expect("load collection");
        assert!(collection_file.items.iter().any(|item| {
            item.item_id == created.folder.folder_id
                && item.item_type == ItemType::Folder
                && item.name == "Security"
        }));
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

        let collection_dir = storage
            .find_collection_dir_by_id(collection_id)
            .expect("find collection dir");
        let collection_file: CollectionFile = storage
            .read_toml_file(&collection_dir.join("collection.toml"))
            .expect("load collection");
        assert!(
            !collection_file
                .items
                .iter()
                .any(|item| item.item_id == folder_id && item.item_type == ItemType::Folder)
        );
    }
}
