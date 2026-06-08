use std::path::PathBuf;

pub const FOLDER_MANIFEST_FILE_NAME: &str = "folder.toml";
pub const WORKSPACES_REGISTRY_FILE_NAME: &str = "workspaces.toml";
pub const APP_SETTINGS_FILE_NAME: &str = "app-settings.toml";

/// Paths rooted at the data root (`$HOME/beam/`).
/// Knows where `workspaces.toml` lives and can derive workspace-specific paths.
#[derive(Debug, Clone)]
pub struct DataRootPaths {
    /// The data root directory (e.g. `$HOME/beam`).
    pub root: PathBuf,
    /// Path to the workspaces registry file (`root/workspaces.toml`).
    pub registry_file: PathBuf,
    /// Root for all local (non-synced) state (`$HOME/beam_local`).
    pub local_root: PathBuf,
    /// Global app settings file under the local root.
    pub app_settings_file: PathBuf,
    /// Root for all application log files in the OS-native log directory.
    pub logs_root: PathBuf,
    /// Application log file under the OS-native logs root.
    pub log_file: PathBuf,
}

impl DataRootPaths {
    pub fn new(root: PathBuf, local_root: PathBuf, logs_root: PathBuf) -> Self {
        let registry_file = root.join(WORKSPACES_REGISTRY_FILE_NAME);
        let app_settings_file = local_root.join(APP_SETTINGS_FILE_NAME);
        let log_file = logs_root.join("beam.log");
        Self {
            root,
            registry_file,
            local_root,
            app_settings_file,
            logs_root,
            log_file,
        }
    }

    /// Returns per-workspace paths for the given workspace directory slug.
    pub fn workspace_paths(&self, workspace_path: &str) -> BeamPaths {
        let workspace_root = self.root.join(workspace_path);
        let local_dir = self.local_root.join(workspace_path);
        BeamPaths::from_workspace_root(
            workspace_root,
            local_dir,
            self.app_settings_file.clone(),
            self.log_file.clone(),
        )
    }

    pub fn default_user_config() -> Self {
        let home_dir = dirs::home_dir();
        let root = home_dir
            .as_ref()
            .map(|h| h.join("beam"))
            .unwrap_or_else(|| PathBuf::from("./beam"));
        let local_root = home_dir
            .map(|h| h.join("beam_local"))
            .unwrap_or_else(|| PathBuf::from("./beam_local"));
        let logs_root = default_logs_root();
        Self::new(root, local_root, logs_root)
    }
}

/// Per-workspace paths. Rooted at a specific workspace directory.
#[derive(Debug, Clone)]
pub struct BeamPaths {
    /// Workspace data root (e.g. `$HOME/beam/my-workspace`).
    pub root: PathBuf,
    /// `root/environments`
    pub environments_dir: PathBuf,
    /// Local state directory for this workspace (e.g. `$HOME/beam_local/my-workspace`).
    pub local_dir: PathBuf,
    /// `local_dir/local-state.toml`
    pub local_state_file: PathBuf,
    /// Global app settings file shared by all workspaces.
    pub app_settings_file: PathBuf,
    /// Application log file in the OS-native logs directory.
    pub log_file: PathBuf,
    /// `root/beam.workspace.toml`
    pub workspace_file: PathBuf,
}

impl BeamPaths {
    /// Build workspace paths given a workspace data root and a local state directory.
    pub fn from_workspace_root(
        root: PathBuf,
        local_dir: PathBuf,
        app_settings_file: PathBuf,
        log_file: PathBuf,
    ) -> Self {
        let environments_dir = root.join("environments");
        let local_state_file = local_dir.join("local-state.toml");
        let workspace_file = root.join("beam.workspace.toml");
        Self {
            root,
            environments_dir,
            local_dir,
            local_state_file,
            app_settings_file,
            log_file,
            workspace_file,
        }
    }

    /// Derives paths from a single root, placing local state under `root/.beam/`.
    /// Used by tests that want a self-contained workspace fixture.
    pub fn from_root(root: PathBuf) -> Self {
        let local_dir = root.join(".beam");
        let app_settings_file = local_dir.join(APP_SETTINGS_FILE_NAME);
        let log_file = root.join(".beam_logs").join("beam.log");
        Self::from_workspace_root(root, local_dir, app_settings_file, log_file)
    }

    /// Default user config for the default workspace slug.
    pub fn default_user_config() -> Self {
        let data_root = DataRootPaths::default_user_config();
        data_root.workspace_paths("default")
    }
}

#[cfg(test)]
fn default_root_from_home_dir(home_dir: Option<PathBuf>) -> PathBuf {
    home_dir
        .map(|home| home.join("beam"))
        .unwrap_or_else(|| PathBuf::from("./beam"))
}

#[cfg(target_os = "macos")]
fn default_logs_root() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join("Library").join("Logs").join("Beam"))
        .unwrap_or_else(|| PathBuf::from("./beam_logs"))
}

#[cfg(target_os = "windows")]
fn default_logs_root() -> PathBuf {
    dirs::data_local_dir()
        .map(|dir| dir.join("Beam").join("Logs"))
        .unwrap_or_else(|| PathBuf::from("./beam_logs"))
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn default_logs_root() -> PathBuf {
    if let Some(state_home) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(state_home).join("beam").join("logs");
    }

    dirs::home_dir()
        .map(|home| home.join(".local").join("state").join("beam").join("logs"))
        .unwrap_or_else(|| PathBuf::from("./beam_logs"))
}

pub fn slugify(name: &str) -> String {
    let raw: String = name
        .chars()
        .map(|c| match c {
            'A'..='Z' => c.to_ascii_lowercase(),
            'a'..='z' | '0'..='9' => c,
            _ => '-',
        })
        .collect();

    raw.split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::{
        APP_SETTINGS_FILE_NAME, BeamPaths, DataRootPaths, default_logs_root,
        default_root_from_home_dir, slugify,
    };

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
        let paths = BeamPaths::from_root(dir.path().to_path_buf());
        assert_eq!(paths.workspace_file, dir.path().join("beam.workspace.toml"));
    }

    #[test]
    fn local_state_file_is_under_dot_beam_by_default() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().to_path_buf());
        assert_eq!(
            paths.local_state_file,
            dir.path().join(".beam").join("local-state.toml")
        );
    }

    #[test]
    fn log_file_is_under_test_log_directory() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().to_path_buf());
        assert_eq!(
            paths.log_file,
            dir.path().join(".beam_logs").join("beam.log")
        );
    }

    #[test]
    fn data_root_workspace_paths_uses_separate_local_root() {
        let dir = tempdir().expect("tempdir");
        let data_root = dir.path().join("beam");
        let local_root = dir.path().join("beam_local");
        let logs_root = dir.path().join("beam_logs");
        let paths = DataRootPaths::new(data_root.clone(), local_root.clone(), logs_root.clone());
        let ws_paths = paths.workspace_paths("my-workspace");
        let other_ws_paths = paths.workspace_paths("other-workspace");
        assert_eq!(ws_paths.root, data_root.join("my-workspace"));
        assert_eq!(ws_paths.local_dir, local_root.join("my-workspace"));
        assert_eq!(
            ws_paths.local_state_file,
            local_root.join("my-workspace").join("local-state.toml")
        );
        assert_eq!(
            ws_paths.app_settings_file,
            local_root.join(APP_SETTINGS_FILE_NAME)
        );
        assert_eq!(ws_paths.log_file, logs_root.join("beam.log"));
        assert_eq!(other_ws_paths.log_file, logs_root.join("beam.log"));
    }

    #[test]
    fn registry_file_is_at_data_root() {
        let dir = tempdir().expect("tempdir");
        let data_root = dir.path().join("beam");
        let local_root = dir.path().join("beam_local");
        let logs_root = dir.path().join("beam_logs");
        let paths = DataRootPaths::new(data_root.clone(), local_root, logs_root.clone());
        assert_eq!(paths.registry_file, data_root.join("workspaces.toml"));
        assert_eq!(
            paths.app_settings_file,
            paths.local_root.join(APP_SETTINGS_FILE_NAME)
        );
        assert_eq!(paths.log_file, logs_root.join("beam.log"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn default_logs_root_uses_library_logs_on_macos() {
        let home = dirs::home_dir().expect("home dir");
        assert_eq!(
            default_logs_root(),
            home.join("Library").join("Logs").join("Beam")
        );
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    #[test]
    fn default_logs_root_uses_xdg_state_on_linux_like_platforms() {
        let expected = if let Some(state_home) = std::env::var_os("XDG_STATE_HOME") {
            PathBuf::from(state_home).join("beam").join("logs")
        } else {
            dirs::home_dir()
                .expect("home dir")
                .join(".local")
                .join("state")
                .join("beam")
                .join("logs")
        };
        assert_eq!(default_logs_root(), expected);
    }

    #[test]
    fn slugify_replaces_spaces_and_specials_with_hyphens() {
        assert_eq!(slugify("My Workspace"), "my-workspace");
        assert_eq!(slugify("Work/API"), "work-api");
        assert_eq!(slugify("hello world!"), "hello-world");
        assert_eq!(slugify("  spaces  "), "spaces");
        assert_eq!(slugify("already-slugified"), "already-slugified");
    }
}
