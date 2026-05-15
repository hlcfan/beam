use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{BeamError, Result};
use crate::paths::BeamPaths;
#[derive(Debug, Clone)]
pub struct TomlWorkspaceStorage {
    pub paths: BeamPaths,
}

impl TomlWorkspaceStorage {
    pub fn new(paths: BeamPaths) -> Self {
        Self { paths }
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
