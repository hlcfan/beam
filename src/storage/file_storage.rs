use super::{
    CollectionMetadata, CollectionStorage, EnvironmentsMetadata, PersistentEnvironments,
    PersistentRequest, StorageError,
};
use crate::storage::RequestMetadata;
use crate::types::{Environment, RequestCollection, RequestConfig, SerializableRequestConfig};
use log::{error, info};
use serde::Serialize;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

/// TOML-based file storage implementation
pub struct TomlFileStorage {
    base_path: PathBuf,
    collections_path: PathBuf,
    environments_path: PathBuf,
}

impl TomlFileStorage {
    /// Create a new TOML file storage instance
    pub fn new(base_path: PathBuf) -> Self {
        let collections_path = base_path.join("collections");
        let environments_path = base_path.join("environments.toml");

        Self {
            base_path,
            collections_path,
            environments_path,
        }
    }

    /// Get the path for a collection directory
    fn collection_path(&self, collection_name: &str) -> PathBuf {
        self.collections_path
            .join(sanitize_filename(collection_name))
    }

    /// Get the path for a collection metadata file
    fn collection_metadata_path(&self, collection_name: &str) -> PathBuf {
        self.collection_path(collection_name)
            .join("collection.toml")
    }

    /// Find the actual collection directory by searching for directories that match the collection name
    fn find_collection_directory(
        &self,
        collection_name: &str,
    ) -> Result<Option<PathBuf>, StorageError> {
        let entries = match fs::read_dir(&self.collections_path) {
            Ok(entries) => entries,
            Err(_) => return Ok(None),
        };

        for entry in entries {
            let entry = entry.map_err(|e| StorageError::IoError(e))?;
            if entry
                .file_type()
                .map_err(|e| StorageError::IoError(e))?
                .is_dir()
            {
                let dir_name = entry.file_name().to_string_lossy().to_string();
                // Check if the directory name matches the collection name (with or without prefix)
                if dir_name == collection_name
                    || remove_numeric_prefix(&dir_name) == collection_name
                {
                    return Ok(Some(entry.path()));
                }
            }
        }

        Ok(None)
    }

    /// Get the path for the last opened request file
    fn last_opened_request_path(&self) -> PathBuf {
        self.base_path.join("last_opened_request.toml")
    }

    /// Save a collection to disk (metadata only)
    fn save_collection_to_disk(&self, collection: &RequestCollection) -> Result<(), StorageError> {
        // Try to find existing collection directory by name first
        let collection_dir = match self.find_collection_directory_by_name(&collection.name)? {
            Some(existing_dir) => existing_dir,
            None => {
                // If no existing directory found, create a new one with numeric name
                let numeric_folder = self.find_next_collection_number()?;
                let new_dir = self.collections_path.join(&numeric_folder);
                fs::create_dir_all(&new_dir)?;
                new_dir
            }
        };

        // Save collection metadata with the collection name stored in TOML
        let metadata = CollectionMetadata {
            name: collection.name.clone(),
            description: None,
            expanded: collection.expanded,
        };

        let metadata_content = toml::to_string_pretty(&metadata)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;

        let metadata_path = collection_dir.join("collection.toml");
        fs::write(&metadata_path, metadata_content)?;

        Ok(())
    }

    /// Save a collection with all its requests to disk (for initial creation)
    fn save_collection_with_requests(
        &self,
        collection: &RequestCollection,
    ) -> Result<(), StorageError> {
        // First save the collection metadata
        self.save_collection_to_disk(collection)?;

        // Then save each request as a separate file
        for request in &collection.requests {
            // let persistent_request = PersistentRequest {
            //     name: request.name.clone(),
            //     method: request.method.to_string(),
            //     url: request.url.clone(),
            //     headers: Vec::new(),
            //     params: Vec::new(),
            //     body: String::new(),
            //     content_type: "application/json".to_string(),
            //     auth_type: "None".to_string(),
            //     bearer_token: None,
            //     basic_username: None,
            //     basic_password: None,
            //     api_key: None,
            //     api_key_header: None,
            //     collection_index: 0,
            //     request_index: 0,
            //     metadata: Some(super::persistent_types::RequestMetadata::default()),
            // };

            self.save_request(&collection.name, &request)?;
        }

        Ok(())
    }

    /// Find the next available numeric prefix for a new request in the collection
    fn find_next_numeric_prefix(&self, collection_dir: &Path) -> Result<u32, StorageError> {
        let mut max_prefix = 0u32;
        let entries = fs::read_dir(collection_dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "toml") {
                let filename = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");

                // Skip collection metadata file
                if filename == "collection" {
                    continue;
                }

                // Parse filename as numeric ID
                if let Ok(id) = filename.parse::<u32>() {
                    max_prefix = max_prefix.max(id);
                }
            }
        }

        Ok(max_prefix + 1)
    }

    /// Find the next available 4-digit numeric folder name for collections
    fn find_next_collection_number(&self) -> Result<String, StorageError> {
        if !self.collections_path.exists() {
            return Ok("0001".to_string());
        }

        let mut max_number = 0u32;
        let entries = fs::read_dir(&self.collections_path)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                if let Some(folder_name) = path.file_name().and_then(|s| s.to_str()) {
                    // Check if folder name is a 4-digit number
                    if folder_name.len() == 4 && folder_name.chars().all(|c| c.is_ascii_digit()) {
                        if let Ok(number) = folder_name.parse::<u32>() {
                            max_number = max_number.max(number);
                        }
                    }
                }
            }
        }

        Ok(format!("{:04}", max_number + 1))
    }

    /// Find collection directory by searching for the collection name in TOML metadata
    fn find_collection_directory_by_name(
        &self,
        collection_name: &str,
    ) -> Result<Option<PathBuf>, StorageError> {
        if !self.collections_path.exists() {
            return Ok(None);
        }

        let entries = fs::read_dir(&self.collections_path)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let metadata_path = path.join("collection.toml");
                if metadata_path.exists() {
                    if let Ok(content) = fs::read_to_string(&metadata_path) {
                        if let Ok(metadata) = toml::from_str::<CollectionMetadata>(&content) {
                            if metadata.name == collection_name {
                                return Ok(Some(path));
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Migrate existing collections from folder-based names to numeric folders with TOML metadata
    pub fn migrate_collections_to_numeric_folders(&self) -> Result<(), StorageError> {
        if !self.collections_path.exists() {
            return Ok(());
        }

        let entries = fs::read_dir(&self.collections_path)?;
        let mut collections_to_migrate = Vec::new();
        let mut collections_to_fix = Vec::new();

        // Find collections that need migration or fixing
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                if let Some(folder_name) = path.file_name().and_then(|s| s.to_str()) {
                    let metadata_path = path.join("collection.toml");

                    if folder_name.parse::<u32>().is_err() || !metadata_path.exists() {
                        // Non-numeric folder or missing collection.toml - needs full migration
                        collections_to_migrate.push((folder_name.to_string(), path));
                    } else if metadata_path.exists() {
                        // Numeric folder with collection.toml - check if name field is missing
                        if let Ok(content) = fs::read_to_string(&metadata_path) {
                            if let Ok(metadata) = toml::from_str::<CollectionMetadata>(&content) {
                                if metadata.name.is_empty() || metadata.name == "New Collection" {
                                    // Name field is missing or default - needs fixing
                                    collections_to_fix.push((folder_name.to_string(), path));
                                }
                            } else {
                                // Invalid TOML - needs fixing
                                collections_to_fix.push((folder_name.to_string(), path));
                            }
                        }
                    }
                }
            }
        }

        // Migrate collections that need full migration (non-numeric folders)
        for (original_name, old_path) in collections_to_migrate {
            // Get next available numeric folder name
            let numeric_folder = self.find_next_collection_number()?;
            let new_path = self.collections_path.join(&numeric_folder);

            // Create new numeric folder
            fs::create_dir_all(&new_path)?;

            // Create collection.toml with the original folder name as the collection name
            let metadata = CollectionMetadata {
                name: original_name.clone(),
                description: None,
                expanded: false,
            };

            let metadata_content = toml::to_string_pretty(&metadata)
                .map_err(|e| StorageError::SerializationError(e.to_string()))?;
            let metadata_path = new_path.join("collection.toml");
            fs::write(&metadata_path, metadata_content)?;

            // Copy all request files from old to new location
            let old_entries = fs::read_dir(&old_path)?;
            for entry in old_entries {
                let entry = entry?;
                let file_path = entry.path();
                if file_path.is_file() {
                    if let Some(file_name) = file_path.file_name() {
                        let new_file_path = new_path.join(file_name);
                        fs::copy(&file_path, &new_file_path)?;
                    }
                }
            }

            // Remove the old folder
            fs::remove_dir_all(&old_path)?;

            println!(
                "Migrated collection '{}' from folder-based to numeric storage",
                original_name
            );
        }

        // Fix collections that have missing or invalid name fields
        for (folder_name, collection_path) in collections_to_fix {
            let metadata_path = collection_path.join("collection.toml");

            // Try to determine the collection name from existing data
            let _collection_name = if let Ok(content) = fs::read_to_string(&metadata_path) {
                if let Ok(mut metadata) = toml::from_str::<CollectionMetadata>(&content) {
                    // If name is missing or default, use folder name or "My Requests"
                    if metadata.name.is_empty() || metadata.name == "New Collection" {
                        metadata.name = if folder_name == "0001" {
                            "My Requests".to_string()
                        } else {
                            folder_name.clone()
                        };
                        // metadata.modified_at = chrono::Utc::now().to_rfc3339();

                        // Write the updated metadata back
                        let metadata_content = toml::to_string_pretty(&metadata)
                            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
                        fs::write(&metadata_path, metadata_content)?;

                        println!(
                            "Fixed collection metadata for folder '{}' with name '{}'",
                            folder_name, metadata.name
                        );
                        metadata.name
                    } else {
                        metadata.name
                    }
                } else {
                    // Invalid TOML, create new metadata
                    let collection_name = if folder_name == "0001" {
                        "My Requests".to_string()
                    } else {
                        folder_name.clone()
                    };
                    let metadata = CollectionMetadata {
                        name: collection_name.clone(),
                        description: None,
                        expanded: false,
                    };

                    let metadata_content = toml::to_string_pretty(&metadata)
                        .map_err(|e| StorageError::SerializationError(e.to_string()))?;
                    fs::write(&metadata_path, metadata_content)?;

                    println!(
                        "Created new collection metadata for folder '{}' with name '{}'",
                        folder_name, collection_name
                    );
                    collection_name
                }
            } else {
                // No metadata file, create one
                let collection_name = if folder_name == "0001" {
                    "My Requests".to_string()
                } else {
                    folder_name.clone()
                };
                let metadata = CollectionMetadata {
                    name: collection_name.clone(),
                    description: None,
                    expanded: false,
                };

                let metadata_content = toml::to_string_pretty(&metadata)
                    .map_err(|e| StorageError::SerializationError(e.to_string()))?;
                fs::write(&metadata_path, metadata_content)?;

                println!(
                    "Created collection metadata for folder '{}' with name '{}'",
                    folder_name, collection_name
                );
                collection_name
            };
        }

        Ok(())
    }
}

impl CollectionStorage for TomlFileStorage {
    fn load_collections(&self) -> Result<Vec<RequestCollection>, StorageError> {
        if !self.collections_path.exists() {
            self.initialize_default_collections()?;
        }

        // First, collect all collection directories with their metadata
        let mut collection_data = Vec::new();
        let collection_entries = fs::read_dir(&self.collections_path)?;

        let mut collecion_index = 0;
        let mut request_index = 0;

        for entry in collection_entries {
            let entry = entry?;
            let collection_path = entry.path();
            if !collection_path.is_dir() {
                continue;
            }

            let metadata_path = collection_path.join("collection.toml");
            if !metadata_path.exists() {
                continue;
            }

            // let folder_name = collection_path.file_name().and_then(|s| s.to_str());
            let folder_name =
                if let Some(folder_name) = collection_path.file_name().and_then(|s| s.to_str()) {
                    folder_name.to_string()
                } else {
                    continue;
                };

            let mut collection_name: String = String::new();

            let mut requests = Vec::new();

            let request_files = fs::read_dir(&collection_path)?;
            for request_file in request_files {
                let request_file = request_file?;
                let request_path = request_file.path();
                if request_path.is_dir() {
                    continue;
                }

                if request_path.extension().map_or(false, |ext| ext != "toml") {
                    continue;
                }

                // TODO: any simple way to compare the file name?
                if request_path.file_name() == Some(OsStr::new("collection.toml")) {
                    if let Ok(content) = fs::read_to_string(&metadata_path) {
                        if let Ok(metadata) = toml::from_str::<CollectionMetadata>(&content) {
                            collection_name = metadata.name;
                        }
                    }
                } else {
                    // request toml files
                    if let Ok(content) = fs::read_to_string(&request_path) {
                        match toml::from_str::<SerializableRequestConfig>(&content) {
                            Ok(r) => {
                                requests.push(RequestConfig {
                                    name: r.name.unwrap(),
                                    path: request_path.clone(),
                                    method: r.method,
                                    url: r.url.unwrap_or_default(),
                                    headers: r.headers,
                                    params: r.params,
                                    body: r.body.unwrap_or_default(),
                                    content_type: r.content_type.unwrap_or_default(),
                                    auth_type: r.auth_type.unwrap_or_default(),
                                    body_format: r.body_format.unwrap_or_default(),
                                    bearer_token: r.bearer_token.unwrap_or_default(),
                                    basic_username: r.basic_username.unwrap_or_default(),
                                    basic_password: r.basic_password.unwrap_or_default(),
                                    api_key: r.api_key.unwrap_or_default(),
                                    api_key_header: r.api_key_header.unwrap_or_default(),
                                    metadata: r.metadata,
                                    collection_index: collecion_index,
                                    request_index: request_index,
                                    post_request_script: r.post_request_script,
                                    last_response: r.last_response,
                                });

                                request_index += 1;
                            }
                            Err(e) => {
                                error!("===fail to parse: {:?}", e);
                            }
                        }
                    }
                }
            }

            // Sort requests by file name alphanumerically
            requests.sort_by(|a, b| {
                let file_name_a = a
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");
                let file_name_b = b
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");
                file_name_a.cmp(file_name_b)
            });

            collection_data.push(RequestCollection {
                folder_name: folder_name,
                name: collection_name,
                requests: requests,
                expanded: false,
            });

            collecion_index += 1;
            request_index = 0;
        }

        // Sort by numeric folder names
        collection_data.sort_by(|a, b| {
            let num_a = a.folder_name.parse::<u32>().unwrap_or(u32::MAX);
            let num_b = b.folder_name.parse::<u32>().unwrap_or(u32::MAX);
            num_a.cmp(&num_b)
        });

        // Update collection_index and request_index after sorting
        for (collection_idx, collection) in collection_data.iter_mut().enumerate() {
            for (request_idx, request) in collection.requests.iter_mut().enumerate() {
                request.collection_index = collection_idx;
                request.request_index = request_idx;
            }
        }

        // Load collections in sorted order using their actual names from metadata
        // let mut collections = Vec::new();
        // for (index, (_folder_name, collection)) in collection_data.iter().enumerate() {
        //     match self.load_collection_from_disk(collection).await {
        //         Ok(collection) => {
        //             collections.push(collection);
        //         }
        //         Err(e) => {
        //             error!("Warning: Failed to load collection, err: {}", e);
        //         }
        //     }
        // }

        Ok(collection_data)
    }

    fn save_collection(&self, collection: &RequestCollection) -> Result<(), StorageError> {
        self.save_collection_to_disk(collection)
    }

    fn save_collection_with_requests(
        &self,
        collection: &RequestCollection,
    ) -> Result<(), StorageError> {
        self.save_collection_with_requests(collection)
    }

    fn delete_collection_by_folder_name(&self, folder_name: &str) {
        let base_path = self.base_path.to_str().unwrap().to_string();
        let dir = format!("{}/collections/{}", base_path, folder_name);

        match fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(e) => {
                error!("fail to delete folder: {:?}, error: {}", folder_name, e);
            }
        }
    }

    fn rename_collection(&self, old_name: &str, new_name: &str) -> Result<(), StorageError> {
        // Find collection directory by old name
        let collection_dir = match self.find_collection_directory_by_name(old_name)? {
            Some(dir) => dir,
            None => return Err(StorageError::CollectionNotFound(old_name.to_string())),
        };

        // Check if new name already exists
        if self.find_collection_directory_by_name(new_name)?.is_some() {
            return Err(StorageError::InvalidFormat(format!(
                "Collection '{}' already exists",
                new_name
            )));
        }

        // Load existing metadata
        let metadata_path = collection_dir.join("collection.toml");
        let mut metadata = if metadata_path.exists() {
            let content = fs::read_to_string(&metadata_path)?;
            toml::from_str::<CollectionMetadata>(&content)
                .map_err(|e| StorageError::SerializationError(e.to_string()))?
        } else {
            CollectionMetadata::default()
        };

        // Update the name and modified_at fields
        metadata.name = new_name.to_string();
        // metadata.modified_at = chrono::Utc::now().to_rfc3339();

        // Save updated metadata
        let metadata_content = toml::to_string_pretty(&metadata)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        fs::write(&metadata_path, metadata_content)?;

        Ok(())
    }

    fn save_request(
        &self,
        collection_name: &str,
        request: &RequestConfig,
    ) -> Result<(), StorageError> {
        let collection_dir = match self.find_collection_directory_by_name(collection_name)? {
            Some(dir) => dir,
            None => {
                return Err(StorageError::CollectionNotFound(
                    collection_name.to_string(),
                ));
            }
        };

        let request_content = toml::to_string_pretty(request)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;

        // Find existing file that contains this request by checking file content
        let mut existing_file_path = None;
        let entries = fs::read_dir(&collection_dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "toml") {
                let filename = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");

                // Skip collection metadata file
                if filename == "collection" {
                    continue;
                }

                // Read the file content and check if it matches this request
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(existing_request) = toml::from_str::<PersistentRequest>(&content) {
                        if existing_request.name == request.name {
                            existing_file_path = Some(path);
                            break;
                        }
                    }
                }
            }
        }

        // Use existing file path if found, otherwise create new file with next available numeric prefix
        let request_path = if let Some(existing_path) = existing_file_path {
            existing_path
        } else {
            // Find next available numeric prefix for new request
            let next_prefix = self.find_next_numeric_prefix(&collection_dir)?;
            let filename = format!("{:04}.toml", next_prefix);
            collection_dir.join(filename)
        };

        fs::write(&request_path, request_content)?;

        Ok(())
    }

    fn save_serializable_request(
        &self,
        collection_name: &str,
        request_name: &str,
        request_config: &RequestConfig,
    ) -> Result<(), StorageError> {
        let collection_dir = match self.find_collection_directory_by_name(collection_name)? {
            Some(dir) => dir,
            None => {
                return Err(StorageError::CollectionNotFound(
                    collection_name.to_string(),
                ));
            }
        };

        let request_content = toml::to_string_pretty(request_config)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;

        // Find existing file that contains this request by checking file content
        let mut existing_file_path = None;
        let entries = fs::read_dir(&collection_dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "toml") {
                let filename = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");

                // Skip collection metadata file
                if filename == "collection" {
                    continue;
                }

                // Read the file content and check if it matches this request
                if let Ok(content) = fs::read_to_string(&path) {
                    // Try to parse as SerializableRequestConfig first (new format)
                    if let Ok(existing_request) = toml::from_str::<RequestConfig>(&content) {
                        if existing_request.name == request_name {
                            existing_file_path = Some(path);
                            break;
                        }
                    }
                    // If that fails, try to parse as PersistentRequest (old format)
                    else if let Ok(existing_request) =
                        toml::from_str::<PersistentRequest>(&content)
                    {
                        if existing_request.name == request_name {
                            existing_file_path = Some(path);
                            break;
                        }
                    }
                }
            }
        }

        // Use existing file path if found, otherwise create new file with next available numeric prefix
        let request_path = if let Some(existing_path) = existing_file_path {
            existing_path
        } else {
            // Find next available numeric prefix for new request
            let next_prefix = self.find_next_numeric_prefix(&collection_dir)?;
            let filename = format!("{:04}.toml", next_prefix);
            collection_dir.join(filename)
        };

        fs::write(&request_path, request_content)?;

        Ok(())
    }

    fn save_request_by_path(&self, request_config: &RequestConfig) -> Result<(), StorageError> {
        // Check if the path is empty (new request without a file path)
        if request_config.path.as_os_str().is_empty() {
            return Err(StorageError::InvalidFormat(
                "Request path is empty".to_string(),
            ));
        }

        // Serialize the request config to TOML
        let request_content = toml::to_string_pretty(request_config)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;

        if let Some(parent) = request_config.path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(&request_config.path, request_content)?;

        Ok(())
    }

    fn get_new_request_path_from_collection(&self, collection: &RequestCollection) -> String {
        // If collection is empty, update the request path and save
        // else deduce the new file name, and save
        // Check if the path is empty (new request without a file path)
        let base_path = self.base_path.to_str().unwrap().to_string();
        if collection.requests.is_empty() {
            return format!(
                "{}/collections/{}/{}.toml",
                base_path, collection.folder_name, "0001"
            );
        } else {
            if let Some(last_request) = collection.requests.last() {
                let path = last_request.path.clone();

                let file_name: usize = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("0001")
                    .parse()
                    .unwrap();

                let file_path = path.parent().and_then(|s| s.to_str()).unwrap_or("0001");

                format!("{}/{:04}.toml", file_path, file_name + 1)
            } else {
                format!("{}/{}/{}.toml", base_path, collection.folder_name, "0001")
            }
        }
    }

    fn delete_request(
        &self,
        collection_name: &str,
        request_name: &str,
    ) -> Result<(), StorageError> {
        let collection_dir = match self.find_collection_directory_by_name(collection_name)? {
            Some(dir) => dir,
            None => {
                return Err(StorageError::CollectionNotFound(
                    collection_name.to_string(),
                ));
            }
        };

        // Find the file that contains this request by checking the content
        let mut target_file_path = None;
        let entries = fs::read_dir(&collection_dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "toml") {
                let filename = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");

                // Skip collection metadata file
                if filename == "collection" {
                    continue;
                }

                // Read the file content to check if it matches this request
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(existing_request) = toml::from_str::<PersistentRequest>(&content) {
                        if existing_request.name == request_name {
                            target_file_path = Some(path);
                            break;
                        }
                    }
                }
            }
        }

        // Delete the file if found
        match target_file_path {
            Some(file_path) => {
                fs::remove_file(&file_path)?;
                Ok(())
            }
            None => Err(StorageError::RequestNotFound(request_name.to_string())),
        }
    }

    /// Delete a request by its file path directly (more efficient when path is known)
    fn delete_request_by_path(&self, request_path: &Path) -> Result<(), StorageError> {
        fs::remove_file(request_path)?;
        Ok(())
    }

    fn rename_request(
        &self,
        collection_name: &str,
        old_name: &str,
        new_name: &str,
    ) -> Result<(), StorageError> {
        let collection_dir = match self.find_collection_directory_by_name(collection_name)? {
            Some(dir) => dir,
            None => {
                return Err(StorageError::CollectionNotFound(
                    collection_name.to_string(),
                ));
            }
        };

        // Find the file that contains this request by checking the content
        let mut existing_file_path = None;
        let entries = fs::read_dir(&collection_dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "toml") {
                let filename = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");

                // Skip collection metadata file
                if filename == "collection" {
                    continue;
                }

                // Read the file content to check if it matches the old request name
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(existing_request) = toml::from_str::<PersistentRequest>(&content) {
                        if existing_request.name == old_name {
                            existing_file_path = Some(path);
                            break;
                        }
                    }
                }
            }
        }

        let file_path = existing_file_path
            .ok_or_else(|| StorageError::RequestNotFound(old_name.to_string()))?;

        // Read the existing file content
        let content = fs::read_to_string(&file_path)?;
        let mut persistent_request: PersistentRequest = toml::from_str(&content)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;

        // Update the name in the file content
        persistent_request.name = new_name.to_string();

        // Write the updated content back to the same file (filename stays the same)
        let updated_content = toml::to_string_pretty(&persistent_request)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;

        fs::write(&file_path, updated_content)?;

        Ok(())
    }

    fn load_environments(&self) -> Result<PersistentEnvironments, StorageError> {
        if !self.environments_path.exists() {
            return Ok(PersistentEnvironments {
                environments: Vec::new(),
                active_environment: None,
                metadata: EnvironmentsMetadata::default(),
            });
        }

        let content = fs::read_to_string(&self.environments_path)?;
        let persistent_envs: PersistentEnvironments = toml::from_str(&content)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;

        Ok(persistent_envs)
    }

    fn save_environments(&self, environments: &[Environment]) -> Result<(), StorageError> {
        self.save_environments_with_active(environments, None)
    }

    fn save_environments_with_active(
        &self,
        environments: &[Environment],
        active_environment: Option<&str>,
    ) -> Result<(), StorageError> {
        // Create base directory only when saving
        fs::create_dir_all(&self.base_path)?;

        let persistent_data = PersistentEnvironments {
            environments: environments.to_vec(),
            active_environment: active_environment.map(|s| s.to_string()),
            metadata: EnvironmentsMetadata::default(),
        };

        let content = toml::to_string_pretty(&persistent_data)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;

        fs::write(&self.environments_path, content)?;
        Ok(())
    }

    fn load_active_environment(&self) -> Result<Option<String>, StorageError> {
        if !self.environments_path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&self.environments_path)?;
        let persistent_envs: PersistentEnvironments = toml::from_str(&content)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;

        Ok(persistent_envs.active_environment)
    }

    fn save_last_opened_request(
        &self,
        collection_index: usize,
        request_index: usize,
    ) -> Result<(), StorageError> {
        // Create base directory only when saving
        fs::create_dir_all(&self.base_path)?;

        #[derive(Serialize)]
        struct LastOpenedRequest {
            collection_index: usize,
            request_index: usize,
        }

        let data = LastOpenedRequest {
            collection_index,
            request_index,
        };

        let content = toml::to_string_pretty(&data)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;

        fs::write(self.last_opened_request_path(), content)?;
        Ok(())
    }

    fn load_last_opened_request(&self) -> Result<Option<(usize, usize)>, StorageError> {
        use serde::Deserialize;

        #[derive(Deserialize)]
        struct LastOpenedRequest {
            collection_index: usize,
            request_index: usize,
        }

        let path = self.last_opened_request_path();

        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&path)?;
        let data: LastOpenedRequest = toml::from_str(&content)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;

        Ok(Some((data.collection_index, data.request_index)))
    }

    fn load_request_by_indices(
        &self,
        collections: &[RequestCollection],
        collection_index: usize,
        request_index: usize,
    ) -> Result<Option<RequestConfig>, StorageError> {
        eprintln!(
            "DEBUG: load_request_by_indices - collections.len(): {}, collection_index: {}, request_index: {}",
            collections.len(),
            collection_index,
            request_index
        );

        if collection_index >= collections.len() {
            eprintln!(
                "DEBUG: collection_index {} >= collections.len() {}",
                collection_index,
                collections.len()
            );
            return Ok(None);
        }

        let collection = &collections[collection_index];
        eprintln!(
            "DEBUG: collection.name: {}, collection.requests.len(): {}",
            collection.name,
            collection.requests.len()
        );

        if request_index >= collection.requests.len() {
            eprintln!(
                "DEBUG: request_index {} >= collection.requests.len() {}",
                request_index,
                collection.requests.len()
            );
            return Ok(None);
        }

        // Instead of using the display name, we need to find the actual file by index
        // Find the actual collection directory (which may have a numeric prefix)
        let collection_dir = self.find_collection_directory_by_name(&collection.name)?;
        if collection_dir.is_none() {
            eprintln!(
                "DEBUG: collection directory not found for: {}",
                collection.name
            );
            return Ok(None);
        }
        let collection_dir = collection_dir.unwrap();

        // Collect all request files with their filenames
        let mut request_files = Vec::new();
        let entries = fs::read_dir(&collection_dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "toml") {
                let filename = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");

                // Skip collection metadata file
                if filename == "collection" {
                    continue;
                }

                request_files.push((filename.to_string(), path));
            }
        }

        // Sort request files by numeric ID (same as in load_collection_from_disk)
        request_files.sort_by(|a, b| {
            let id_a = a.0.parse::<u32>().ok();
            let id_b = b.0.parse::<u32>().ok();

            match (id_a, id_b) {
                (Some(num_a), Some(num_b)) => num_a.cmp(&num_b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.0.cmp(&b.0),
            }
        });

        // Check if the request_index is valid
        if request_index >= request_files.len() {
            eprintln!(
                "DEBUG: request_index {} >= request_files.len() {}",
                request_index,
                request_files.len()
            );
            return Ok(None);
        }

        // Get the actual file path for the request at the given index
        let (filename, request_path) = &request_files[request_index];
        eprintln!(
            "DEBUG: Loading request file: {}, path: {:?}",
            filename, request_path
        );

        if !request_path.exists() {
            eprintln!("DEBUG: request_path does not exist: {:?}", request_path);
            return Ok(None);
        }

        let content = fs::read_to_string(&request_path)?;
        let persistent_request: RequestConfig = toml::from_str(&content)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;

        eprintln!(
            "DEBUG: Successfully loaded request: {}",
            persistent_request.name
        );
        Ok(Some(persistent_request))
    }

    fn initialize_storage(&self) -> Result<(), StorageError> {
        // Don't create directories automatically on startup
        // Directories will be created only when needed (e.g., when saving data)

        // Run migration for existing collections to convert them to numeric folders
        // if self.collections_path.exists() {
        //     self.migrate_collections_to_numeric_folders()?;
        // }

        Ok(())
    }

    /// Initialize default collections and files when the app is launched for the first time
    /// Creates:
    /// - collections/0001/0001.toml (default request)
    /// - collections/0001/collection.toml (collection metadata)
    /// - last_opened_request.toml (tracks last opened request)
    fn initialize_default_collections(&self) -> Result<(), StorageError> {
        // Check if collections already exist - if so, don't create defaults
        if self.collections_path.exists() && self.collections_path.read_dir()?.next().is_some() {
            info!("Collections already exist, skipping default initialization");
            return Ok(());
        }

        // Create the first collection directory: collections/0001
        let first_collection_dir = self.collections_path.join("0001");
        fs::create_dir_all(&first_collection_dir)?;

        // Create the first request file: collections/0001/0001.toml
        let first_request_path = first_collection_dir.join("0001.toml");
        let default_request = RequestConfig {
            name: "My First Request".to_string(),
            path: first_request_path.clone(),
            method: crate::types::HttpMethod::GET,
            url: "https://httpbin.org/get".to_string(),
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            params: vec![],
            body: String::new(),
            content_type: "application/json".to_string(),
            auth_type: crate::types::AuthType::None,
            body_format: crate::types::BodyFormat::Json,
            bearer_token: String::new(),
            basic_username: String::new(),
            basic_password: String::new(),
            api_key: String::new(),
            api_key_header: String::new(),
            collection_index: 0,
            request_index: 0,
            metadata: Some(RequestMetadata::default()),
            post_request_script: None,
            last_response: None,
        };

        let request_content = toml::to_string_pretty(&default_request)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        fs::write(&first_request_path, request_content)?;

        // Create the collection metadata file: collections/0001/collection.toml
        let collection_metadata_path = first_collection_dir.join("collection.toml");
        let default_collection = RequestCollection {
            name: "Default Collection".to_string(),
            folder_name: "0001".to_string(),
            requests: vec![],
            // requests: vec![default_request], // Include the first request
            expanded: true,
        };

        let collection_content = toml::to_string_pretty(&default_collection)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        fs::write(&collection_metadata_path, collection_content)?;

        // Create the last opened request file: last_opened_request.toml
        #[derive(Serialize)]
        struct LastOpenedRequestData {
            collection_index: usize,
            request_index: usize,
        }

        let last_opened_data = LastOpenedRequestData {
            collection_index: 0,
            request_index: 0,
        };

        let last_opened_content = toml::to_string_pretty(&last_opened_data)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;

        let last_opened_path = self.last_opened_request_path();
        fs::write(&last_opened_path, last_opened_content)?;

        info!("Default collections initialized successfully");
        Ok(())
    }

    fn backup_storage(&self, backup_path: &str) -> Result<(), StorageError> {
        let backup_dir = Path::new(backup_path);
        fs::create_dir_all(backup_dir)?;

        // Copy the entire storage directory
        copy_dir_all(&self.base_path, backup_dir)?;

        Ok(())
    }
}

/// Sanitize a filename by removing invalid characters
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Extract numeric prefix from a name (e.g., "0001_Collection Name" -> Some(1))
/// Returns None if no numeric prefix is found
fn extract_numeric_prefix(name: &str) -> Option<u32> {
    if let Some(underscore_pos) = name.find('_') {
        let prefix = &name[..underscore_pos];
        if prefix.len() == 4 && prefix.chars().all(|c| c.is_ascii_digit()) {
            return prefix.parse::<u32>().ok();
        }
    }
    None
}

/// Remove numeric prefix from a name (e.g., "0001_Collection Name" -> "Collection Name")
/// Returns the original name if no prefix is found
fn remove_numeric_prefix(name: &str) -> &str {
    if let Some(underscore_pos) = name.find('_') {
        let prefix = &name[..underscore_pos];
        if prefix.len() == 4 && prefix.chars().all(|c| c.is_ascii_digit()) {
            return &name[underscore_pos + 1..];
        }
    }
    name
}

/// Sort a vector of names by their numeric prefixes
/// Names without prefixes are sorted alphabetically and placed after prefixed names
fn sort_by_numeric_prefix(names: &mut Vec<String>) {
    names.sort_by(|a, b| {
        let prefix_a = extract_numeric_prefix(a);
        let prefix_b = extract_numeric_prefix(b);

        match (prefix_a, prefix_b) {
            (Some(num_a), Some(num_b)) => num_a.cmp(&num_b),
            (Some(_), None) => std::cmp::Ordering::Less, // Prefixed comes before non-prefixed
            (None, Some(_)) => std::cmp::Ordering::Greater, // Non-prefixed comes after prefixed
            (None, None) => a.cmp(b),                    // Both non-prefixed, sort alphabetically
        }
    });
}

/// Recursively copy a directory
#[allow(dead_code)]
fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    fs::create_dir_all(dst)?;

    let entries = fs::read_dir(src)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if path.is_dir() {
            copy_dir_all(&path, &dst_path)?;
        } else {
            fs::copy(&path, &dst_path)?;
        }
    }

    Ok(())
}
