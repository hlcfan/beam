use std::path::PathBuf;

use thiserror::Error;

use crate::schema::SchemaKind;

#[derive(Debug, Error)]
pub enum BeamError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("toml decode error at {path}: {source}")]
    TomlDecode {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("toml encode error: {0}")]
    TomlEncode(#[from] toml::ser::Error),
    #[error("schema version mismatch for {kind}: expected {expected}, found {found}")]
    SchemaVersion {
        kind: SchemaKind,
        expected: u32,
        found: u32,
    },
    #[error("{entity} not found: {id}")]
    NotFound { entity: &'static str, id: String },
    #[error("validation error: {message}")]
    Validation { message: String },
}

pub type Result<T> = std::result::Result<T, BeamError>;
