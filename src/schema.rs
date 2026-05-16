use serde::{Deserialize, Serialize};

use crate::error::{BeamError, Result};

pub const SCHEMA_VERSION_V1: u32 = 1;
pub const SCHEMA_VERSION_V3: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaKind {
    Workspace,
    Folder,
    Request,
    Environment,
    LocalState,
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
            Self::WorkspacesRegistry => "workspaces_registry",
        };
        write!(f, "{value}")
    }
}

pub fn validate_workspaces_registry_version(found: u32) -> crate::error::Result<()> {
    if found == SCHEMA_VERSION_V3 {
        return Ok(());
    }
    Err(crate::error::BeamError::SchemaVersion {
        kind: SchemaKind::WorkspacesRegistry,
        expected: SCHEMA_VERSION_V3,
        found,
    })
}

pub fn validate_schema_version(kind: SchemaKind, found: u32) -> Result<()> {
    let expected = SCHEMA_VERSION_V1;
    let accepted = match kind {
        // Legacy local-state files use schema_version = 2.
        // TODO: only support schema_version = 1 for now.
        SchemaKind::LocalState => found == expected || found == 2,
        SchemaKind::WorkspacesRegistry => found == SCHEMA_VERSION_V3,
        _ => found == expected,
    };

    if accepted {
        return Ok(());
    }

    Err(BeamError::SchemaVersion {
        kind,
        expected,
        found,
    })
}
