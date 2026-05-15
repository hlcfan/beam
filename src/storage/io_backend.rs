use std::path::Path;

use serde::{Serialize, de::DeserializeOwned};

use crate::error::Result;
use crate::paths::BeamPaths;

pub trait StorageIoBackend: Send + 'static {
    fn paths(&self) -> &BeamPaths;

    fn read_toml_file<T: DeserializeOwned>(&self, path: &Path) -> Result<T>;
    fn write_toml_file<T: Serialize>(&self, path: &Path, value: &T) -> Result<()>;

    fn create_dir_all(&self, path: &Path) -> Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> Result<()>;
    fn remove_file(&self, path: &Path) -> Result<()>;
    fn remove_dir_all(&self, path: &Path) -> Result<()>;

    fn read_dir(&self, path: &Path) -> Result<Vec<std::path::PathBuf>>;
}
