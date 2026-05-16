use std::path::PathBuf;

pub const FOLDER_MANIFEST_FILE_NAME: &str = "folder.toml";

#[derive(Debug, Clone)]
pub struct BeamPaths {
    pub root: PathBuf,
    pub environments_dir: PathBuf,
    pub local_dir: PathBuf,
    pub local_state_file: PathBuf,
    pub workspace_file: PathBuf,
}

impl BeamPaths {
    pub fn from_root(root: PathBuf) -> Self {
        let environments_dir = root.join("environments");
        let local_dir = root.join(".beam");
        let local_state_file = local_dir.join("local-state.toml");
        let workspace_file = root.join("beam.workspace.toml");

        Self {
            root,
            environments_dir,
            local_dir,
            local_state_file,
            workspace_file,
        }
    }

    pub fn default_user_config() -> Self {
        let home_dir = dirs::home_dir();
        let mut paths = Self::from_root(default_root_from_home_dir(home_dir.clone()));
        paths.local_dir = home_dir
            .map(|home| home.join("beam_local"))
            .unwrap_or_else(|| PathBuf::from("./beam_local"));
        paths.local_state_file = paths.local_dir.join("local-state.toml");
        paths
    }
}

fn default_root_from_home_dir(home_dir: Option<PathBuf>) -> PathBuf {
    home_dir
        .map(|home| home.join("beam"))
        .unwrap_or_else(|| PathBuf::from("./beam"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::default_root_from_home_dir;

    #[test]
    fn defaults_to_home_beam_directory() {
        let dir = tempdir().expect("tempdir");
        assert_eq!(
            default_root_from_home_dir(Some(dir.path().to_path_buf())),
            dir.path().join("beam")
        );
    }

    #[test]
    fn falls_back_to_relative_beam_directory_without_home() {
        assert_eq!(default_root_from_home_dir(None), PathBuf::from("./beam"));
    }

    #[test]
    fn workspace_file_is_at_root() {
        let dir = tempdir().expect("tempdir");
        let paths = super::BeamPaths::from_root(dir.path().to_path_buf());
        assert_eq!(
            paths.workspace_file,
            dir.path().join("beam.workspace.toml")
        );
    }

    #[test]
    fn local_state_file_is_under_dot_beam_by_default() {
        let dir = tempdir().expect("tempdir");
        let paths = super::BeamPaths::from_root(dir.path().to_path_buf());
        assert_eq!(
            paths.local_state_file,
            dir.path().join(".beam").join("local-state.toml")
        );
    }
}
