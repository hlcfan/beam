use serde::{Deserialize, Serialize};

use crate::error::{BeamError, Result};

pub const SCHEMA_VERSION_V1: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaKind {
    Workspace,
    Folder,
    Request,
    Environment,
    LocalState,
    AppSettings,
    WorkspacesRegistry,
}

impl std::fmt::Display for SchemaKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Workspace => "workspace",
            Self::Folder => "folder",
            Self::Request => "request",
            Self::Environment => "environment",
            Self::LocalState => "local_state",
            Self::AppSettings => "app_settings",
            Self::WorkspacesRegistry => "workspaces_registry",
        };
        write!(f, "{value}")
    }
}

pub fn validate_workspaces_registry_version(found: u32) -> crate::error::Result<()> {
    if found == SCHEMA_VERSION_V1 {
        return Ok(());
    }
    Err(crate::error::BeamError::SchemaVersion {
        kind: SchemaKind::WorkspacesRegistry,
        expected: SCHEMA_VERSION_V1,
        found,
    })
}

pub fn validate_schema_version(kind: SchemaKind, found: u32) -> Result<()> {
    let expected = SCHEMA_VERSION_V1;

    if found == expected {
        return Ok(());
    }

    Err(BeamError::SchemaVersion {
        kind,
        expected,
        found,
    })
}
