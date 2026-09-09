//! Errors raised while reading and interpreting a SCIP index.

use std::path::PathBuf;

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, ScipError>;

#[derive(Debug, thiserror::Error)]
pub enum ScipError {
    #[error("failed to read SCIP index {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The file exists but is not a SCIP protobuf payload — usually a JSON
    /// index that was never converted, or a truncated CI artifact.
    #[error("{path} is not a valid SCIP index: {message}")]
    Decode { path: PathBuf, message: String },

    /// A decoded index with no documents indexes nothing. Almost always a
    /// build that compiled zero sources, which is worth failing loudly on
    /// rather than silently producing an empty graph.
    #[error("SCIP index {path} contains no documents")]
    Empty { path: PathBuf },
}
