use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{BeamError, Result};
use crate::paths::BeamPaths;

#[derive(Debug, Clone)]
pub struct FileSystemStorage {
    pub paths: BeamPaths,
}

impl FileSystemStorage {
    pub fn new(paths: BeamPaths) -> Self {
        Self { paths }
    }
}

impl crate::storage::io_backend::StorageIoBackend for FileSystemStorage {
    fn paths(&self) -> &BeamPaths {
        &self.paths
    }

    fn read_toml_file<T: serde::de::DeserializeOwned>(&self, path: &Path) -> Result<T> {
        let content = fs::read_to_string(path).map_err(|source| BeamError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        toml::from_str(&content).map_err(|source| BeamError::TomlDecode {
            path: path.to_path_buf(),
            source,
        })
    }

    fn write_toml_file<T: serde::Serialize>(&self, path: &Path, value: &T) -> Result<()> {
        let encoded = toml::to_string_pretty(value)?;
        atomic_write(path, encoded.as_bytes())
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        fs::create_dir_all(path).map_err(|source| BeamError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        fs::rename(from, to).map_err(|source| BeamError::Io {
            path: from.to_path_buf(),
            source,
        })
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        fs::remove_file(path).map_err(|source| BeamError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    fn remove_dir_all(&self, path: &Path) -> Result<()> {
        fs::remove_dir_all(path).map_err(|source| BeamError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>> {
        let entries = fs::read_dir(path).map_err(|source| BeamError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| BeamError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            paths.push(entry.path());
        }
        Ok(paths)
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| BeamError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
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
