use std::path::PathBuf;

pub mod fs_backend;
pub mod io_backend;
pub mod registry_repo;
pub mod workspace_repo;

use crate::error::Result;
use crate::models::{LocalStateFile, WorkspaceFile};
use ulid::Ulid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BootstrapReport {
    pub created_workspace_file: bool,
    pub created_local_state_file: bool,
}

/// Reference to a parent scope when creating or moving a request.
/// `folder_id: None` means the workspace root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestParentRef {
    pub folder_id: Option<Ulid>,
}

/// The known manifest path for the parent scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownParentManifestPath(pub PathBuf);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRequestInput {
    pub parent: RequestParentRef,
    pub known_parent_manifest_path: Option<KnownParentManifestPath>,
    pub name: String,
    pub method: crate::models::HttpMethod,
    pub url: String,
}

/// Reference to a parent scope when creating or moving a folder.
/// `folder_id: None` means the workspace root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderParentRef {
    pub folder_id: Option<Ulid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateFolderInput {
    pub parent: FolderParentRef,
    pub known_parent_manifest_path: Option<KnownParentManifestPath>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateRequestInput {
    pub request_id: Ulid,
    pub duplicate_name: String,
    pub parent: RequestParentRef,
    pub known_request_path: Option<PathBuf>,
    pub known_parent_manifest_path: Option<KnownParentManifestPath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameRequestInput {
    pub request_id: Ulid,
    pub new_name: String,
    pub known_request_path: Option<PathBuf>,
    pub known_parent_manifest_path: Option<KnownParentManifestPath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteRequestInput {
    pub request_id: Ulid,
    pub known_request_path: Option<PathBuf>,
    pub known_parent_manifest_path: Option<KnownParentManifestPath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveRequestInput {
    pub request_id: Ulid,
    pub new_parent: RequestParentRef,
    pub insertion_index: usize,
    pub known_request_path: Option<PathBuf>,
    pub known_target_manifest_path: Option<KnownParentManifestPath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveFolderInput {
    pub folder_id: Ulid,
    pub new_parent: FolderParentRef,
    pub insertion_index: usize,
    pub known_folder_manifest_path: Option<PathBuf>,
    pub known_target_manifest_path: Option<KnownParentManifestPath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateEnvironmentInput {
    pub name: String,
}

pub trait WorkspaceStorage {
    fn load_workspace(&self) -> Result<WorkspaceFile>;
    fn save_workspace(&self, workspace_file: &WorkspaceFile) -> Result<()>;
    fn load_local_state(&self) -> Result<LocalStateFile>;
    fn save_local_state(&self, local_state_file: &LocalStateFile) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateWorkspaceInput {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteWorkspaceInput {
    pub workspace_id: ulid::Ulid,
    /// Whether to also delete the workspace directory from disk.
    pub delete_data: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameWorkspaceInput {
    pub workspace_id: ulid::Ulid,
    pub new_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchWorkspaceInput {
    pub workspace_id: ulid::Ulid,
}
