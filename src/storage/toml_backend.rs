use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::error::{BeamError, Result};
use crate::models::{LocalStateFile, WorkspaceFile};
use crate::paths::BeamPaths;
use crate::schema::{SchemaKind, validate_schema_version};
use crate::storage::WorkspaceStorage;


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


#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::models::{
        AuthConfig, BodyConfig, HttpMethod, RequestDefinition, RequestFile, RequestMeta,
        ScriptConfig,
    };

    use chrono::Utc;
    use ulid::Ulid;

    #[test]
    fn workspace_roundtrip_preserves_data() {
        let dir = tempdir().expect("tempdir");
        let storage = TomlWorkspaceStorage::new(BeamPaths::from_root(dir.path().to_path_buf()));

        let workspace = WorkspaceFile::default();
        storage.save_workspace(&workspace).expect("save workspace");
        let loaded = storage.load_workspace().expect("load workspace");

        assert_eq!(workspace, loaded);
    }

    #[test]
    fn load_local_state_ignores_nested_expanded_and_selection_fields() {
        let dir = tempdir().expect("tempdir");
        let storage = TomlWorkspaceStorage::new(BeamPaths::from_root(dir.path().to_path_buf()));

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
        fs::create_dir_all(storage.paths.local_state_file.parent().unwrap()).expect("create beam_local dir");
        fs::write(&storage.paths.local_state_file, local_state_toml).expect("write local state");

        let loaded = storage.load_local_state().expect("load local state");
        assert!(loaded.tree_state.expanded_item_ids.is_empty());
        assert!(loaded.collection_environment_selection.is_empty());
    }

    #[test]
    fn persist_theme_state_updates_theme_fields() {
        let dir = tempdir().expect("tempdir");
        let storage = TomlWorkspaceStorage::new(BeamPaths::from_root(dir.path().to_path_buf()));
        storage.save_local_state(&LocalStateFile::default()).expect("save local state");

        storage
            .persist_theme_state("One Dark")
            .expect("persist theme state");
        let loaded = storage.load_local_state().expect("load local state");

        assert_eq!(loaded.local_state.theme_name.as_deref(), Some("One Dark"));
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

}
