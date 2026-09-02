use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FiloError {
    #[error("path does not exist: {0}")]
    NotFound(PathBuf),

    #[error("path already exists: {0}")]
    AlreadyExists(PathBuf),

    #[error("destination is not a directory: {0}")]
    NotADirectory(PathBuf),

    #[error("refusing to overwrite existing path: {0}")]
    WouldOverwrite(PathBuf),

    #[error("bulk rename would create a name collision: {0}")]
    NameCollision(PathBuf),

    #[error("nothing to undo")]
    NothingToUndo,

    #[error("invalid rename pattern: {0}")]
    BadPattern(String),

    #[error("could not determine a data directory for filo")]
    NoDataDir,

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Toml(#[from] toml::de::Error),

    #[error(transparent)]
    Regex(#[from] regex::Error),

    #[error(transparent)]
    WalkDir(#[from] walkdir::Error),
}

pub type Result<T> = std::result::Result<T, FiloError>;
