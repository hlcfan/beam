use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct BeamPaths {
    pub root: PathBuf,
    pub collections_dir: PathBuf,
    pub collections_root_order_file: PathBuf,
    pub environments_dir: PathBuf,
    pub local_dir: PathBuf,
    pub local_state_file: PathBuf,
    pub workspace_file: PathBuf,
}

impl BeamPaths {
    pub fn from_root(root: PathBuf) -> Self {
        let collections_dir = root.join("collections");
        let collections_root_order_file =
            collections_dir.join(crate::tree_store::COLLECTION_ROOT_ORDER_FILE_NAME);
        let environments_dir = root.join("environments");
        let local_dir = root.join(".beam");
        let local_state_file = local_dir.join("local-state.toml");
        let workspace_file = root.join("beam.workspace.toml");

        Self {
            root,
            collections_dir,
            collections_root_order_file,
            environments_dir,
            local_dir,
            local_state_file,
            workspace_file,
        }
    }

    pub fn default_user_config() -> Self {
        let legacy_root = dirs::home_dir()
            .map(|home| home.join(".config").join("beam"))
            .unwrap_or_else(|| PathBuf::from(".config/beam"));
        let platform_root = dirs::config_dir()
            .map(|dir| dir.join("beam"))
            .unwrap_or_else(|| PathBuf::from("./beam"));

        // Prefer whichever location already contains a workspace.
        let root = if has_workspace_data(&legacy_root) {
            legacy_root
        } else if has_workspace_data(&platform_root) {
            platform_root
        } else {
            // Default to ~/.config/beam to match Beam docs and existing storage layout.
            legacy_root
        };

        Self::from_root(root)
    }
}

fn has_workspace_data(root: &Path) -> bool {
    root.join("beam.workspace.toml").exists() || root.join("collections").exists()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::has_workspace_data;

    #[test]
    fn detects_workspace_via_workspace_file() {
        let dir = tempdir().expect("tempdir");
        assert!(!has_workspace_data(dir.path()));
        fs::write(
            dir.path().join("beam.workspace.toml"),
            "[workspace]\nschema_version = 1\n",
        )
        .expect("write workspace");
        assert!(has_workspace_data(dir.path()));
    }

    #[test]
    fn detects_workspace_via_collections_dir() {
        let dir = tempdir().expect("tempdir");
        assert!(!has_workspace_data(dir.path()));
        fs::create_dir_all(dir.path().join("collections")).expect("create collections dir");
        assert!(has_workspace_data(dir.path()));
    }

    #[test]
    fn derives_collections_root_order_file_path() {
        let dir = tempdir().expect("tempdir");
        let paths = super::BeamPaths::from_root(dir.path().to_path_buf());
        assert_eq!(
            paths.collections_root_order_file,
            dir.path().join("collections").join(".root-order.toml")
        );
    }
}
