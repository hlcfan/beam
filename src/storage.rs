pub mod toml_backend;

use crate::error::Result;
use crate::models::{
    EnvironmentFile, EnvironmentScope, EnvironmentVariable, LocalStateFile, RequestFile,
    WorkspaceFile,
};
use ulid::Ulid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BootstrapReport {
    pub created_workspace_file: bool,
    pub created_local_state_file: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestParentRef {
    pub collection_id: Ulid,
    pub folder_id: Option<Ulid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRequestInput {
    pub parent: RequestParentRef,
    pub name: String,
    pub method: crate::models::HttpMethod,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderParentRef {
    pub collection_id: Ulid,
    pub parent_folder_id: Option<Ulid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateFolderInput {
    pub parent: FolderParentRef,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateEnvironmentInput {
    pub name: String,
    pub scope: EnvironmentScope,
    pub collection_id: Option<Ulid>,
}

pub trait WorkspaceStorage {
    fn initialize(&self) -> Result<BootstrapReport>;
    fn load_workspace(&self) -> Result<WorkspaceFile>;
    fn save_workspace(&self, workspace_file: &WorkspaceFile) -> Result<()>;
    fn load_local_state(&self) -> Result<LocalStateFile>;
    fn save_local_state(&self, local_state_file: &LocalStateFile) -> Result<()>;
    fn load_request(&self, request_id: Ulid) -> Result<RequestFile>;
    fn create_request(&self, input: CreateRequestInput) -> Result<RequestFile>;
    fn create_request_after(
        &self,
        input: CreateRequestInput,
        source_request_id: Ulid,
    ) -> Result<RequestFile>;
    fn create_folder(&self, input: CreateFolderInput) -> Result<crate::models::FolderFile>;
    fn create_environment(&self, input: CreateEnvironmentInput) -> Result<EnvironmentFile>;
    fn rename_environment(&self, environment_id: Ulid, new_name: &str) -> Result<EnvironmentFile>;
    fn update_environment_variables(
        &self,
        environment_id: Ulid,
        variables: Vec<EnvironmentVariable>,
    ) -> Result<EnvironmentFile>;
    fn delete_environment(&self, environment_id: Ulid) -> Result<()>;
    fn save_request(&self, request_file: &RequestFile) -> Result<()>;
    fn rename_request(&self, request_id: Ulid, new_name: &str) -> Result<RequestFile>;
    fn rename_collection(
        &self,
        collection_id: Ulid,
        new_name: &str,
    ) -> Result<crate::models::CollectionFile>;
    fn rename_folder(&self, folder_id: Ulid, new_name: &str) -> Result<crate::models::FolderFile>;
    fn duplicate_request(
        &self,
        request_id: Ulid,
        duplicate_name: &str,
        parent: RequestParentRef,
    ) -> Result<RequestFile>;
    fn delete_collection(&self, collection_id: Ulid) -> Result<()>;
    fn delete_folder(&self, folder_id: Ulid) -> Result<()>;
    fn delete_request(&self, request_id: Ulid) -> Result<()>;
}
