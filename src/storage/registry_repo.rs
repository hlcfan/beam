use std::fs;
use std::path::Path;

use chrono::Utc;
use ulid::Ulid;

use crate::error::{BeamError, Result};
use crate::models::{WorkspaceEntry, WorkspacesRegistryFile};
use crate::paths::{BeamPaths, DataRootPaths, slugify};
use crate::schema::validate_workspaces_registry_version;

pub struct RegistryRepository {
    data_root: DataRootPaths,
}

impl RegistryRepository {
    pub fn new(data_root: DataRootPaths) -> Self {
        Self { data_root }
    }

    /// Load the registry from disk. Returns an error if the file exists but cannot be parsed.
    pub fn load(&self) -> Result<WorkspacesRegistryFile> {
        let path = &self.data_root.registry_file;
        let content = fs::read_to_string(path).map_err(|source| BeamError::Io {
            path: path.clone(),
            source,
        })?;
        let registry: WorkspacesRegistryFile =
            toml::from_str(&content).map_err(|source| BeamError::TomlDecode {
                path: path.clone(),
                source,
            })?;
        validate_workspaces_registry_version(registry.schema_version)?;
        Ok(registry)
    }

    /// Persist the registry to disk atomically.
    pub fn save(&self, registry: &WorkspacesRegistryFile) -> Result<()> {
        let path = &self.data_root.registry_file;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| BeamError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let encoded = toml::to_string_pretty(registry)?;
        atomic_write(path, encoded.as_bytes())
    }

    /// Load the registry if it exists, or create a new one with a default "default" workspace.
    /// Returns `(registry, created_new)`.
    pub fn initialize(&self) -> Result<(WorkspacesRegistryFile, bool)> {
        if self.data_root.registry_file.exists() {
            let registry = self.load()?;
            return Ok((registry, false));
        }

        let registry = WorkspacesRegistryFile::new_with_default_workspace("default", "default");
        // Create the workspace directory and its beam.workspace.toml if needed.
        let ws_paths = self.workspace_paths(registry.registry.workspaces.first().unwrap());
        fs::create_dir_all(&ws_paths.root).map_err(|source| BeamError::Io {
            path: ws_paths.root.clone(),
            source,
        })?;
        fs::create_dir_all(&ws_paths.local_dir).map_err(|source| BeamError::Io {
            path: ws_paths.local_dir.clone(),
            source,
        })?;
        self.save(&registry)?;
        Ok((registry, true))
    }

    /// Create a new workspace: generates a unique slug, creates directories, and adds to registry.
    pub fn create_workspace(
        &self,
        registry: &mut WorkspacesRegistryFile,
        name: &str,
    ) -> Result<WorkspaceEntry> {
        let slug = self.unique_workspace_slug(registry, name);
        let ws_paths = self.data_root.workspace_paths(&slug);

        fs::create_dir_all(&ws_paths.root).map_err(|source| BeamError::Io {
            path: ws_paths.root.clone(),
            source,
        })?;
        fs::create_dir_all(&ws_paths.local_dir).map_err(|source| BeamError::Io {
            path: ws_paths.local_dir.clone(),
            source,
        })?;

        let entry = WorkspaceEntry {
            workspace_id: Ulid::new(),
            name: name.to_string(),
            path: slug,
            created_at: Utc::now(),
        };
        registry.registry.workspaces.push(entry.clone());
        self.save(registry)?;
        Ok(entry)
    }

    /// Delete a workspace from the registry. If `delete_data` is true, also removes the
    /// workspace directory from disk.
    pub fn delete_workspace(
        &self,
        registry: &mut WorkspacesRegistryFile,
        workspace_id: Ulid,
        delete_data: bool,
    ) -> Result<()> {
        let entry = registry
            .registry
            .workspaces
            .iter()
            .find(|e| e.workspace_id == workspace_id)
            .cloned()
            .ok_or_else(|| BeamError::NotFound {
                entity: "workspace",
                id: workspace_id.to_string(),
            })?;

        if delete_data {
            let ws_paths = self.workspace_paths(&entry);
            if ws_paths.root.exists() {
                fs::remove_dir_all(&ws_paths.root).map_err(|source| BeamError::Io {
                    path: ws_paths.root.clone(),
                    source,
                })?;
            }
            if ws_paths.local_dir.exists() {
                fs::remove_dir_all(&ws_paths.local_dir).map_err(|source| BeamError::Io {
                    path: ws_paths.local_dir.clone(),
                    source,
                })?;
            }
        }

        registry
            .registry
            .workspaces
            .retain(|e| e.workspace_id != workspace_id);

        // If the deleted workspace was active, pick another one.
        if registry.registry.active_workspace_id == Some(workspace_id) {
            registry.registry.active_workspace_id =
                registry.registry.workspaces.first().map(|e| e.workspace_id);
        }

        self.save(registry)?;
        Ok(())
    }

    /// Rename a workspace (updates name and saves registry).
    pub fn rename_workspace(
        &self,
        registry: &mut WorkspacesRegistryFile,
        workspace_id: Ulid,
        new_name: &str,
    ) -> Result<WorkspaceEntry> {
        let entry = registry
            .registry
            .workspaces
            .iter_mut()
            .find(|e| e.workspace_id == workspace_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "workspace",
                id: workspace_id.to_string(),
            })?;
        entry.name = new_name.to_string();
        let updated = entry.clone();
        self.save(registry)?;
        Ok(updated)
    }

    /// Mark the given workspace as active and save the registry.
    pub fn set_active_workspace(
        &self,
        registry: &mut WorkspacesRegistryFile,
        workspace_id: Ulid,
    ) -> Result<()> {
        if !registry
            .registry
            .workspaces
            .iter()
            .any(|e| e.workspace_id == workspace_id)
        {
            return Err(BeamError::NotFound {
                entity: "workspace",
                id: workspace_id.to_string(),
            });
        }
        registry.registry.active_workspace_id = Some(workspace_id);
        self.save(registry)?;
        Ok(())
    }

    /// Returns the `BeamPaths` for a given workspace entry.
    pub fn workspace_paths(&self, entry: &WorkspaceEntry) -> BeamPaths {
        self.data_root.workspace_paths(&entry.path)
    }

    /// Find the active workspace entry, falling back to the first entry if active_workspace_id
    /// is unset or no longer present.
    pub fn active_workspace_entry<'r>(
        &self,
        registry: &'r WorkspacesRegistryFile,
    ) -> Option<&'r WorkspaceEntry> {
        if let Some(active_id) = registry.registry.active_workspace_id {
            if let Some(entry) = registry
                .registry
                .workspaces
                .iter()
                .find(|e| e.workspace_id == active_id)
            {
                return Some(entry);
            }
        }
        registry.registry.workspaces.first()
    }

    fn unique_workspace_slug(&self, registry: &WorkspacesRegistryFile, name: &str) -> String {
        let base = slugify(name);
        let base = if base.is_empty() {
            "workspace".to_string()
        } else {
            base
        };

        let existing_paths: std::collections::HashSet<&str> = registry
            .registry
            .workspaces
            .iter()
            .map(|e| e.path.as_str())
            .collect();

        if !existing_paths.contains(base.as_str()) {
            return base;
        }

        for i in 2u32.. {
            let candidate = format!("{base}-{i}");
            if !existing_paths.contains(candidate.as_str()) {
                return candidate;
            }
        }
        unreachable!("slug uniquifier overflowed u32")
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp_name = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "tmp".to_string());
    tmp_name.push_str(".tmp");
    let tmp_path = path.with_file_name(tmp_name);

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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_repo(dir: &std::path::Path) -> RegistryRepository {
        let data_root = dir.join("beam");
        let local_root = dir.join("beam_local");
        let logs_root = dir.join("beam_logs");
        RegistryRepository::new(DataRootPaths::new(data_root, local_root, logs_root))
    }

    #[test]
    fn initialize_creates_registry_when_absent() {
        let dir = tempdir().expect("tempdir");
        let repo = make_repo(dir.path());
        let (registry, created) = repo.initialize().expect("initialize");
        assert!(created);
        assert_eq!(registry.registry.workspaces.len(), 1);
        assert_eq!(registry.registry.workspaces[0].name, "default");
        assert!(repo.data_root.registry_file.exists());
    }

    #[test]
    fn initialize_loads_existing_registry() {
        let dir = tempdir().expect("tempdir");
        let repo = make_repo(dir.path());
        repo.initialize().expect("first init");
        let (registry, created) = repo.initialize().expect("second init");
        assert!(!created);
        assert_eq!(registry.registry.workspaces.len(), 1);
    }

    #[test]
    fn create_workspace_adds_entry_and_creates_directory() {
        let dir = tempdir().expect("tempdir");
        let repo = make_repo(dir.path());
        let (mut registry, _) = repo.initialize().expect("init");
        let entry = repo
            .create_workspace(&mut registry, "My API Workspace")
            .expect("create workspace");
        assert_eq!(entry.name, "My API Workspace");
        assert_eq!(entry.path, "my-api-workspace");
        assert_eq!(registry.registry.workspaces.len(), 2);
        let ws_paths = repo.workspace_paths(&entry);
        assert!(ws_paths.root.exists());
    }

    #[test]
    fn create_workspace_deduplicates_slugs() {
        let dir = tempdir().expect("tempdir");
        let repo = make_repo(dir.path());
        let (mut registry, _) = repo.initialize().expect("init");
        let e1 = repo
            .create_workspace(&mut registry, "Work")
            .expect("create 1");
        let e2 = repo
            .create_workspace(&mut registry, "Work")
            .expect("create 2");
        assert_ne!(e1.path, e2.path);
        assert_eq!(e2.path, "work-2");
    }

    #[test]
    fn delete_workspace_removes_from_registry() {
        let dir = tempdir().expect("tempdir");
        let repo = make_repo(dir.path());
        let (mut registry, _) = repo.initialize().expect("init");
        let entry = repo
            .create_workspace(&mut registry, "Extra")
            .expect("create");
        let id = entry.workspace_id;
        repo.delete_workspace(&mut registry, id, true)
            .expect("delete");
        assert!(
            !registry
                .registry
                .workspaces
                .iter()
                .any(|e| e.workspace_id == id)
        );
    }

    #[test]
    fn rename_workspace_updates_name() {
        let dir = tempdir().expect("tempdir");
        let repo = make_repo(dir.path());
        let (mut registry, _) = repo.initialize().expect("init");
        let id = registry.registry.workspaces[0].workspace_id;
        let updated = repo
            .rename_workspace(&mut registry, id, "Production")
            .expect("rename");
        assert_eq!(updated.name, "Production");
        assert_eq!(registry.registry.workspaces[0].name, "Production");
    }

    #[test]
    fn set_active_workspace_updates_active_id() {
        let dir = tempdir().expect("tempdir");
        let repo = make_repo(dir.path());
        let (mut registry, _) = repo.initialize().expect("init");
        let entry = repo
            .create_workspace(&mut registry, "Second")
            .expect("create second");
        repo.set_active_workspace(&mut registry, entry.workspace_id)
            .expect("set active");
        assert_eq!(
            registry.registry.active_workspace_id,
            Some(entry.workspace_id)
        );
    }
}
