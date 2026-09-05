//! Save/export failures that never mention LibreDWG.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

// ------------------------------------------------------------
// Enum: ExportError
// Purpose: Failures from native DXF write and PDF export.
// ------------------------------------------------------------
#[derive(Debug, Error)]
pub enum ExportError {
    #[error("path is not valid UTF-8")]
    InvalidPath,
    #[error("cannot write {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{0}")]
    Invalid(&'static str),
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Validation(String),
}

impl ExportError {
    pub fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
