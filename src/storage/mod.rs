use crate::types::{RequestCollection, Environment, RequestConfig};
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

pub mod persistent_types;
pub mod file_storage;
pub mod conversions;

// Re-export for convenience
pub use persistent_types::*;
pub use file_storage::TomlFileStorage;

/// Main storage trait that abstracts persistence operations
#[allow(dead_code)]
#[async_trait::async_trait]
pub trait CollectionStorage: Send + Sync {
    /// Load all collections from storage
    async fn load_collections(&self) -> Result<Vec<RequestCollection>, StorageError>;

    /// Save a collection to storage (metadata only)
    async fn save_collection(&self, collection: &RequestCollection) -> Result<(), StorageError>;

    /// Save a collection with all its requests to storage (for initial creation)
    async fn save_collection_with_requests(&self, collection: &RequestCollection) -> Result<(), StorageError>;

    /// Delete a collection from storage
    async fn delete_collection(&self, collection_name: &str) -> Result<(), StorageError>;

    /// Rename a collection
    async fn rename_collection(&self, old_name: &str, new_name: &str) -> Result<(), StorageError>;

    /// Save a request within a collection
    async fn save_request(&self, collection_name: &str, request: &RequestConfig) -> Result<(), StorageError>;

    /// Save a serializable request config directly (optimized version)
    async fn save_serializable_request(&self, collection_name: &str, request_name: &str, request_config: &RequestConfig) -> Result<(), StorageError>;

    /// Save a request directly to a file path (simplified version)
    async fn save_request_by_path(&self, request_config: &RequestConfig) -> Result<(), StorageError>;

    /// Delete a request from a collection
    async fn delete_request(&self, collection_name: &str, request_name: &str) -> Result<(), StorageError>;

    /// Rename a request within a collection
    async fn rename_request(&self, collection_name: &str, old_name: &str, new_name: &str) -> Result<(), StorageError>;

    /// Load environments from storage
    async fn load_environments(&self) -> Result<Vec<Environment>, StorageError>;

    /// Save environments to storage
    async fn save_environments(&self, environments: &[Environment]) -> Result<(), StorageError>;

    /// Save environments with active environment information
    async fn save_environments_with_active(&self, environments: &[Environment], active_environment: Option<&str>) -> Result<(), StorageError>;

    /// Load active environment name from storage
    async fn load_active_environment(&self) -> Result<Option<String>, StorageError>;

    /// Save the last opened request
    async fn save_last_opened_request(&self, collection_index: usize, request_index: usize) -> Result<(), StorageError>;

    /// Load the last opened request
    async fn load_last_opened_request(&self) -> Result<Option<(usize, usize)>, StorageError>;

    /// Load a specific request by collection and request indices
    async fn load_request_by_indices(&self, collections: &[RequestCollection], collection_index: usize, request_index: usize) -> Result<Option<RequestConfig>, StorageError>;

    /// Initialize storage (create directories, etc.)
    async fn initialize_storage(&self) -> Result<(), StorageError>;

    /// Create a backup of the storage
    async fn backup_storage(&self, backup_path: &str) -> Result<(), StorageError>;
}

/// Storage configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub storage_type: StorageType,
    pub base_path: PathBuf,
    pub auto_save: bool,
    pub backup_enabled: bool,
    pub backup_interval_hours: u64,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            storage_type: StorageType::TomlFiles,
            // TODO: change the base path to ~/.config/.beam
            base_path: dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".beam"),
            auto_save: true,
            // TODO: do we need this?
            backup_enabled: true,
            backup_interval_hours: 24,
        }
    }
}

/// Available storage types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StorageType {
    TomlFiles,
    #[allow(dead_code)]
    Sqlite,
    #[allow(dead_code)]
    Json,
}

/// Storage errors
#[allow(dead_code)]
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Collection not found: {0}")]
    CollectionNotFound(String),

    #[error("Request not found: {0}")]
    RequestNotFound(String),

    #[error("Invalid format: {0}")]
    InvalidFormat(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Storage not initialized")]
    NotInitialized,
}



/// Storage manager that provides a unified interface
pub struct StorageManager {
    storage: Box<dyn CollectionStorage>,
    config: StorageConfig,
}

impl std::fmt::Debug for StorageManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageManager")
            .field("config", &self.config)
            .finish()
    }
}

impl StorageManager {
    /// Create a new storage manager with the specified configuration
    pub async fn new(config: StorageConfig) -> Result<Self, StorageError> {
        let storage: Box<dyn CollectionStorage> = match config.storage_type {
            StorageType::TomlFiles => Box::new(TomlFileStorage::new(config.base_path.clone())),
            StorageType::Sqlite => {
                return Err(StorageError::InvalidFormat("SQLite not implemented yet".to_string()));
            }
            StorageType::Json => {
                return Err(StorageError::InvalidFormat("JSON not implemented yet".to_string()));
            }
        };

        // Initialize storage
        storage.initialize_storage().await?;

        Ok(Self { storage, config })
    }

    /// Create a new storage manager with default configuration
    pub async fn with_default_config() -> Result<Self, StorageError> {
        Self::new(StorageConfig::default()).await
    }

    /// Get a reference to the storage implementation
    pub fn storage(&self) -> &dyn CollectionStorage {
        self.storage.as_ref()
    }

    /// Get the storage configuration
    pub fn config(&self) -> &StorageConfig {
        &self.config
    }
}
