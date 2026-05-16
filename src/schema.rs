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
}

impl std::fmt::Display for SchemaKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Workspace => "workspace",
            Self::Folder => "folder",
            Self::Request => "request",
            Self::Environment => "environment",
            Self::LocalState => "local_state",
        };
        write!(f, "{value}")
    }
}

pub fn validate_schema_version(kind: SchemaKind, found: u32) -> Result<()> {
    let expected = SCHEMA_VERSION_V1;
    let accepted = match kind {
        // Legacy local-state files use schema_version = 2.
        // TODO: only support schema_version = 1 for now.
        SchemaKind::LocalState => found == expected || found == 2,
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
