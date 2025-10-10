use super::{CollectionStorage, StorageError, PersistentRequest, PersistentCollection, PersistentEnvironments, PersistentEnvironment, CollectionMetadata, EnvironmentsMetadata};
use crate::types::{RequestCollection, Environment};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::AsyncWriteExt;

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
        self.collections_path.join(sanitize_filename(collection_name))
    }
    
    /// Get the path for a collection metadata file
    fn collection_metadata_path(&self, collection_name: &str) -> PathBuf {
        self.collection_path(collection_name).join("collection.toml")
    }

    /// Find the actual collection directory by searching for directories that match the collection name
    async fn find_collection_directory(&self, collection_name: &str) -> Result<Option<PathBuf>, StorageError> {
        let mut entries = match fs::read_dir(&self.collections_path).await {
            Ok(entries) => entries,
            Err(_) => return Ok(None),
        };

        while let Some(entry) = entries.next_entry().await.map_err(|e| StorageError::IoError(e))? {
            if entry.file_type().await.map_err(|e| StorageError::IoError(e))?.is_dir() {
                let dir_name = entry.file_name().to_string_lossy().to_string();
                // Check if the directory name matches the collection name (with or without prefix)
                if dir_name == collection_name || remove_numeric_prefix(&dir_name) == collection_name {
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
    
    /// Load a collection from disk
    async fn load_collection_from_disk(&self, collection_name: &str) -> Result<RequestCollection, StorageError> {
        let collection_dir = self.collection_path(collection_name);
        
        if !collection_dir.exists() {
            return Err(StorageError::CollectionNotFound(collection_name.to_string()));
        }
        
        // Load collection metadata
        let metadata_path = self.collection_metadata_path(collection_name);
        let metadata = if metadata_path.exists() {
            let content = fs::read_to_string(&metadata_path).await?;
            toml::from_str::<CollectionMetadata>(&content)
                .map_err(|e| StorageError::SerializationError(e.to_string()))?
        } else {
            CollectionMetadata::default()
        };
        
        // First, collect all request files with their filenames
        let mut request_files = Vec::new();
        let mut entries = fs::read_dir(&collection_dir).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "toml") {
                let filename = path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");
                
                // Skip collection metadata file
                if filename == "collection" {
                    continue;
                }
                
                request_files.push((filename.to_string(), path));
            }
        }
        
        // Sort request files by numeric ID (filename without extension)
        request_files.sort_by(|a, b| {
            // Try to parse filenames as numbers (e.g., "0001" -> 1)
            let num_a = a.0.parse::<u32>().unwrap_or(u32::MAX);
            let num_b = b.0.parse::<u32>().unwrap_or(u32::MAX);
            
            match (num_a == u32::MAX, num_b == u32::MAX) {
                (false, false) => num_a.cmp(&num_b), // Both are valid numbers
                (false, true) => std::cmp::Ordering::Less, // a is number, b is not
                (true, false) => std::cmp::Ordering::Greater, // a is not number, b is
                (true, true) => a.0.cmp(&b.0), // Both are not numbers, sort alphabetically
            }
        });
        
        // Load requests in sorted order
        let mut requests = Vec::new();
        for (filename, path) in request_files {
            let content = fs::read_to_string(&path).await?;
            let persistent_request: PersistentRequest = toml::from_str(&content)
                .map_err(|e| StorageError::SerializationError(e.to_string()))?;
            
            // Convert to SavedRequest (simplified version)
            let method = match persistent_request.method.to_uppercase().as_str() {
                "GET" => crate::types::HttpMethod::GET,
                "POST" => crate::types::HttpMethod::POST,
                "PUT" => crate::types::HttpMethod::PUT,
                "DELETE" => crate::types::HttpMethod::DELETE,
                "PATCH" => crate::types::HttpMethod::PATCH,
                "HEAD" => crate::types::HttpMethod::HEAD,
                "OPTIONS" => crate::types::HttpMethod::OPTIONS,
                _ => crate::types::HttpMethod::GET, // Default to GET for unknown methods
            };
            
            let saved_request = crate::types::SavedRequest {
                name: persistent_request.name.clone(), // Use the name directly from TOML content
                method,
                url: persistent_request.url.clone(),
            };
            
            requests.push(saved_request);
        }
        
        Ok(RequestCollection {
            name: collection_name.to_string(),
            requests,
            expanded: metadata.expanded,
        })
    }
    
    /// Save a collection to disk (metadata only)
    async fn save_collection_to_disk(&self, collection: &RequestCollection) -> Result<(), StorageError> {
        // Try to find existing collection directory first
        let collection_dir = match self.find_collection_directory(&collection.name).await? {
            Some(existing_dir) => existing_dir,
            None => {
                // If no existing directory found, create a new one using the display name
                let new_dir = self.collection_path(&collection.name);
                fs::create_dir_all(&new_dir).await?;
                new_dir
            }
        };
        
        // Save collection metadata only
        let metadata = CollectionMetadata {
            created_at: chrono::Utc::now().to_rfc3339(),
            modified_at: chrono::Utc::now().to_rfc3339(),
            description: None,
            version: "1.0".to_string(),
            expanded: collection.expanded,
        };
        
        let metadata_content = toml::to_string_pretty(&metadata)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        
        let metadata_path = collection_dir.join("collection.toml");
        fs::write(&metadata_path, metadata_content).await?;
        
        Ok(())
    }

    /// Save a collection with all its requests to disk (for initial creation)
    async fn save_collection_with_requests(&self, collection: &RequestCollection) -> Result<(), StorageError> {
        // First save the collection metadata
        self.save_collection_to_disk(collection).await?;
        
        // Then save each request as a separate file
        for request in &collection.requests {
            let persistent_request = PersistentRequest {
                name: request.name.clone(),
                method: request.method.to_string(),
                url: request.url.clone(),
                headers: Vec::new(),
                params: Vec::new(),
                body: String::new(),
                content_type: "application/json".to_string(),
                auth_type: "None".to_string(),
                bearer_token: None,
                basic_username: None,
                basic_password: None,
                api_key: None,
                api_key_header: None,
                metadata: super::persistent_types::RequestMetadata::default(),
            };
            
            self.save_request(&collection.name, &persistent_request).await?;
        }
        
        Ok(())
    }
    
    /// Find the next available numeric prefix for a new request in the collection
    async fn find_next_numeric_prefix(&self, collection_dir: &Path) -> Result<u32, StorageError> {
        let mut max_prefix = 0u32;
        let mut entries = fs::read_dir(collection_dir).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "toml") {
                let filename = path.file_stem()
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
}

#[async_trait::async_trait]
impl CollectionStorage for TomlFileStorage {
    async fn load_collections(&self) -> Result<Vec<RequestCollection>, StorageError> {
        if !self.collections_path.exists() {
            return Ok(Vec::new());
        }
        
        // First, collect all collection directory names
        let mut collection_names = Vec::new();
        let mut entries = fs::read_dir(&self.collections_path).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_dir() {
                if let Some(collection_name) = path.file_name().and_then(|s| s.to_str()) {
                    collection_names.push(collection_name.to_string());
                }
            }
        }
        
        // Sort collection names by numeric prefix
        sort_by_numeric_prefix(&mut collection_names);
        
        // Load collections in sorted order
        let mut collections = Vec::new();
        for collection_name in collection_names {
            match self.load_collection_from_disk(&collection_name).await {
                Ok(mut collection) => {
                    // Remove numeric prefix from collection name for display
                    collection.name = remove_numeric_prefix(&collection.name).to_string();
                    collections.push(collection);
                },
                Err(e) => {
                    eprintln!("Warning: Failed to load collection '{}': {}", collection_name, e);
                }
            }
        }
        
        Ok(collections)
    }
    
    async fn save_collection(&self, collection: &RequestCollection) -> Result<(), StorageError> {
        self.save_collection_to_disk(collection).await
    }
    
    async fn save_collection_with_requests(&self, collection: &RequestCollection) -> Result<(), StorageError> {
        self.save_collection_with_requests(collection).await
    }
    
    async fn delete_collection(&self, collection_name: &str) -> Result<(), StorageError> {
        let collection_dir = self.collection_path(collection_name);
        
        if !collection_dir.exists() {
            return Err(StorageError::CollectionNotFound(collection_name.to_string()));
        }
        
        fs::remove_dir_all(&collection_dir).await?;
        Ok(())
    }
    
    async fn rename_collection(&self, old_name: &str, new_name: &str) -> Result<(), StorageError> {
        let old_path = self.collection_path(old_name);
        let new_path = self.collection_path(new_name);
        
        if !old_path.exists() {
            return Err(StorageError::CollectionNotFound(old_name.to_string()));
        }
        
        if new_path.exists() {
            return Err(StorageError::InvalidFormat(format!("Collection '{}' already exists", new_name)));
        }
        
        fs::rename(&old_path, &new_path).await?;
        Ok(())
    }
    
    async fn save_request(&self, collection_name: &str, request: &PersistentRequest) -> Result<(), StorageError> {
        let collection_dir = match self.find_collection_directory(collection_name).await? {
            Some(dir) => dir,
            None => return Err(StorageError::CollectionNotFound(collection_name.to_string())),
        };
        
        let request_content = toml::to_string_pretty(request)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        
        // Find existing file that contains this request by checking file content
        let mut existing_file_path = None;
        let mut entries = fs::read_dir(&collection_dir).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "toml") {
                let filename = path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");
                
                // Skip collection metadata file
                if filename == "collection" {
                    continue;
                }
                
                // Read the file content and check if it matches this request
                if let Ok(content) = fs::read_to_string(&path).await {
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
            let next_prefix = self.find_next_numeric_prefix(&collection_dir).await?;
            let filename = format!("{:04}.toml", next_prefix);
            collection_dir.join(filename)
        };
        
        fs::write(&request_path, request_content).await?;
        
        Ok(())
    }
    
    async fn delete_request(&self, collection_name: &str, request_name: &str) -> Result<(), StorageError> {
        let collection_dir = match self.find_collection_directory(collection_name).await? {
            Some(dir) => dir,
            None => return Err(StorageError::CollectionNotFound(collection_name.to_string())),
        };
        
        // Find the file that contains this request by checking the content
        let mut target_file_path = None;
        let mut entries = fs::read_dir(&collection_dir).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "toml") {
                let filename = path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");
                
                // Skip collection metadata file
                if filename == "collection" {
                    continue;
                }
                
                // Read the file content to check if it matches this request
                if let Ok(content) = fs::read_to_string(&path).await {
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
                fs::remove_file(&file_path).await?;
                Ok(())
            }
            None => Err(StorageError::RequestNotFound(request_name.to_string())),
        }
    }
    
    async fn rename_request(&self, collection_name: &str, old_name: &str, new_name: &str) -> Result<(), StorageError> {
        let collection_dir = match self.find_collection_directory(collection_name).await? {
            Some(dir) => dir,
            None => return Err(StorageError::CollectionNotFound(collection_name.to_string())),
        };
        
        // Find the file that contains this request by checking the content
        let mut existing_file_path = None;
        let mut entries = fs::read_dir(&collection_dir).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "toml") {
                let filename = path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");
                
                // Skip collection metadata file
                if filename == "collection" {
                    continue;
                }
                
                // Read the file content to check if it matches the old request name
                if let Ok(content) = fs::read_to_string(&path).await {
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
        let content = fs::read_to_string(&file_path).await?;
        let mut persistent_request: PersistentRequest = toml::from_str(&content)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        
        // Update the name in the file content
        persistent_request.name = new_name.to_string();
        
        // Write the updated content back to the same file (filename stays the same)
        let updated_content = toml::to_string_pretty(&persistent_request)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        
        fs::write(&file_path, updated_content).await?;
        
        Ok(())
    }
    
    async fn load_environments(&self) -> Result<Vec<Environment>, StorageError> {
        if !self.environments_path.exists() {
            return Ok(Vec::new());
        }
        
        let content = fs::read_to_string(&self.environments_path).await?;
        let persistent_envs: PersistentEnvironments = toml::from_str(&content)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        
        let environments = persistent_envs.environments
            .into_iter()
            .map(|env| Environment {
                name: env.name,
                variables: env.variables,
                description: env.description,
            })
            .collect();
        
        Ok(environments)
    }
    
    async fn save_environments(&self, environments: &[Environment]) -> Result<(), StorageError> {
        // Create base directory only when saving
        fs::create_dir_all(&self.base_path).await?;
        
        let persistent_envs: Vec<PersistentEnvironment> = environments
            .iter()
            .map(|env| PersistentEnvironment {
                name: env.name.clone(),
                variables: env.variables.clone(),
                description: env.description.clone(),
                metadata: super::persistent_types::EnvironmentMetadata::default(),
            })
            .collect();
        
        let persistent_data = PersistentEnvironments {
            environments: persistent_envs,
            active_environment: None,
            metadata: EnvironmentsMetadata::default(),
        };
        
        let content = toml::to_string_pretty(&persistent_data)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        
        fs::write(&self.environments_path, content).await?;
        Ok(())
    }
    
    async fn save_last_opened_request(&self, collection_index: usize, request_index: usize) -> Result<(), StorageError> {
        use serde::Serialize;
        
        // Create base directory only when saving
        fs::create_dir_all(&self.base_path).await?;
        
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
        
        fs::write(self.last_opened_request_path(), content).await?;
        Ok(())
    }
    
    async fn load_last_opened_request(&self) -> Result<Option<(usize, usize)>, StorageError> {
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
        
        let content = fs::read_to_string(&path).await?;
        let data: LastOpenedRequest = toml::from_str(&content)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        
        Ok(Some((data.collection_index, data.request_index)))
    }
    
    async fn load_request_by_indices(&self, collections: &[RequestCollection], collection_index: usize, request_index: usize) -> Result<Option<PersistentRequest>, StorageError> {
        eprintln!("DEBUG: load_request_by_indices - collections.len(): {}, collection_index: {}, request_index: {}", collections.len(), collection_index, request_index);
        
        if collection_index >= collections.len() {
            eprintln!("DEBUG: collection_index {} >= collections.len() {}", collection_index, collections.len());
            return Ok(None);
        }
        
        let collection = &collections[collection_index];
        eprintln!("DEBUG: collection.name: {}, collection.requests.len(): {}", collection.name, collection.requests.len());
        
        if request_index >= collection.requests.len() {
            eprintln!("DEBUG: request_index {} >= collection.requests.len() {}", request_index, collection.requests.len());
            return Ok(None);
        }
        
        // Instead of using the display name, we need to find the actual file by index
        // Find the actual collection directory (which may have a numeric prefix)
        let collection_dir = self.find_collection_directory(&collection.name).await?;
        if collection_dir.is_none() {
            eprintln!("DEBUG: collection directory not found for: {}", collection.name);
            return Ok(None);
        }
        let collection_dir = collection_dir.unwrap();
        
        // Collect all request files with their filenames
        let mut request_files = Vec::new();
        let mut entries = fs::read_dir(&collection_dir).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "toml") {
                let filename = path.file_stem()
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
            eprintln!("DEBUG: request_index {} >= request_files.len() {}", request_index, request_files.len());
            return Ok(None);
        }
        
        // Get the actual file path for the request at the given index
        let (filename, request_path) = &request_files[request_index];
        eprintln!("DEBUG: Loading request file: {}, path: {:?}", filename, request_path);
        
        if !request_path.exists() {
            eprintln!("DEBUG: request_path does not exist: {:?}", request_path);
            return Ok(None);
        }
        
        let content = fs::read_to_string(&request_path).await?;
        let persistent_request: PersistentRequest = toml::from_str(&content)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        
        eprintln!("DEBUG: Successfully loaded request: {}", persistent_request.name);
        Ok(Some(persistent_request))
    }
    
    async fn initialize_storage(&self) -> Result<(), StorageError> {
        // Don't create directories automatically on startup
        // Directories will be created only when needed (e.g., when saving data)
        Ok(())
    }
    
    async fn backup_storage(&self, backup_path: &str) -> Result<(), StorageError> {
        let backup_dir = Path::new(backup_path);
        fs::create_dir_all(backup_dir).await?;
        
        // Copy the entire storage directory
        copy_dir_all(&self.base_path, backup_dir).await?;
        
        Ok(())
    }
}

/// Sanitize a filename by removing invalid characters
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' || c == ' ' {
            c
        } else {
            '_'
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
            (Some(_), None) => std::cmp::Ordering::Less,  // Prefixed comes before non-prefixed
            (None, Some(_)) => std::cmp::Ordering::Greater, // Non-prefixed comes after prefixed
            (None, None) => a.cmp(b), // Both non-prefixed, sort alphabetically
        }
    });
}

/// Recursively copy a directory
async fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    fs::create_dir_all(dst).await?;
    
    let mut entries = fs::read_dir(src).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let dst_path = dst.join(entry.file_name());
        
        if path.is_dir() {
            Box::pin(copy_dir_all(&path, &dst_path)).await?;
        } else {
            fs::copy(&path, &dst_path).await?;
        }
    }
    
    Ok(())
}