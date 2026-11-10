use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{BeamError, Result};
use crate::paths::DataRootPaths;

/// Machine-local storage locations, loaded before opening the workspace registry.
#[derive(Clone, Debug)]
pub struct StorageConfig {
    file: PathBuf,
    paths: DataRootPaths,
}

#[derive(Default, Deserialize, Serialize)]
struct StorageFolders {
    data_root: Option<PathBuf>,
    local_root: Option<PathBuf>,
    #[serde(flatten)]
    other: toml::Table,
}

impl StorageConfig {
    pub fn load_user_config() -> Result<Self> {
        let home = dirs::home_dir().ok_or_else(|| invalid("Cannot find your home folder."))?;
        Self::load_or_create(
            home.join(".config/beam/config.toml"),
            DataRootPaths::default_user_config(),
        )
    }

    pub fn load_or_create(file: PathBuf, defaults: DataRootPaths) -> Result<Self> {
        let folders = match fs::read_to_string(&file) {
            Ok(content) => toml::from_str::<StorageFolders>(&content).map_err(|source| {
                BeamError::TomlDecode {
                    path: file.clone(),
                    source,
                }
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let folders = StorageFolders {
                    data_root: Some(defaults.root.clone()),
                    local_root: Some(defaults.local_root.clone()),
                    ..StorageFolders::default()
                };
                write_config(&file, &folders)?;
                folders
            }
            Err(source) => return Err(BeamError::Io { path: file, source }),
        };
        let root = resolve_folder(folders.data_root.as_deref().unwrap_or(&defaults.root))?;
        let local_root = resolve_folder(
            folders
                .local_root
                .as_deref()
                .unwrap_or(&defaults.local_root),
        )?;
        validate_separate_folders(&root, &local_root)?;
        Ok(Self {
            file,
            paths: DataRootPaths::new(root, local_root, defaults.logs_root),
        })
    }

    pub fn paths(&self) -> &DataRootPaths {
        &self.paths
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Save locations for the next launch. Does not move data or change active repositories.
    pub fn save_folders(&self, data_root: &str, local_root: &str) -> Result<Self> {
        let root = resolve_folder(Path::new(data_root.trim()))?;
        let local_root = resolve_folder(Path::new(local_root.trim()))?;
        validate_separate_folders(&root, &local_root)?;
        check_writable(&root)?;
        check_writable(&local_root)?;

        // Read again so a malformed file is never silently overwritten.
        let content = fs::read_to_string(&self.file).map_err(|source| BeamError::Io {
            path: self.file.clone(),
            source,
        })?;
        let mut folders: StorageFolders =
            toml::from_str(&content).map_err(|source| BeamError::TomlDecode {
                path: self.file.clone(),
                source,
            })?;
        folders.data_root = Some(root.clone());
        folders.local_root = Some(local_root.clone());
        write_config(&self.file, &folders)?;
        Ok(Self {
            file: self.file.clone(),
            paths: DataRootPaths::new(root, local_root, self.paths.logs_root.clone()),
        })
    }
}

fn resolve_folder(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(invalid("Storage folders cannot be empty."));
    }
    let expanded = if path.starts_with("~") {
        dirs::home_dir()
            .ok_or_else(|| invalid("Cannot find your home folder."))?
            .join(path.strip_prefix("~").map_err(|_| invalid("Invalid folder path."))?)
    } else {
        path.to_path_buf()
    };
    if !expanded.is_absolute() {
        return Err(invalid("Use an absolute folder path or a path starting with ~/ ."));
    }
    // Resolve symlinks before comparing folders, including paths not yet created.
    let mut ancestor = expanded.as_path();
    let mut missing = Vec::new();
    loop {
        match fs::canonicalize(ancestor) {
            Ok(mut resolved) => {
                if !resolved.is_dir() {
                    return Err(invalid("Choose a folder, not a file."));
                }
                for component in missing.iter().rev() {
                    match component {
                        Component::ParentDir => { resolved.pop(); }
                        Component::Normal(name) => resolved.push(name),
                        _ => {}
                    }
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let component = ancestor.components().next_back()
                    .ok_or_else(|| invalid("Invalid folder path."))?;
                missing.push(component);
                ancestor = ancestor.parent().ok_or_else(|| invalid("Invalid folder path."))?;
            }
            Err(source) => return Err(BeamError::Io { path: expanded, source }),
        }
    }
}

fn validate_separate_folders(root: &Path, local_root: &Path) -> Result<()> {
    if root.starts_with(local_root) || local_root.starts_with(root) {
        return Err(invalid("The workspace and local state folders must be separate and cannot contain each other."));
    }
    Ok(())
}

fn check_writable(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|source| BeamError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let probe = path.join(format!(".beam-write-check-{}", uuid::Uuid::new_v4()));
    let file = fs::OpenOptions::new().write(true).create_new(true).open(&probe)
        .map_err(|source| BeamError::Io { path: path.to_path_buf(), source })?;
    drop(file);
    fs::remove_file(&probe).map_err(|source| BeamError::Io { path: probe, source })
}

fn write_config(file: &Path, folders: &StorageFolders) -> Result<()> {
    let parent = file.parent().ok_or_else(|| invalid("Invalid configuration file path."))?;
    fs::create_dir_all(parent).map_err(|source| BeamError::Io {
        path: parent.to_path_buf(), source,
    })?;
    let temporary = parent.join(format!(".config-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        fs::write(&temporary, toml::to_string_pretty(folders)?).map_err(|source| BeamError::Io {
            path: temporary.clone(), source,
        })?;
        fs::rename(&temporary, file).map_err(|source| BeamError::Io {
            path: file.to_path_buf(), source,
        })
    })();
    if result.is_err() { let _ = fs::remove_file(temporary); }
    result
}

fn invalid(message: &str) -> BeamError {
    BeamError::Validation { message: message.to_string() }
}

#[cfg(test)]
mod tests;
