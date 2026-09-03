use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: DateTime<Utc>,
    pub extension: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub id: Uuid,
    pub kind: OperationKind,
    pub timestamp: DateTime<Utc>,
}

impl Operation {
    pub fn new(kind: OperationKind) -> Self {
        Operation {
            id: Uuid::new_v4(),
            kind,
            timestamp: Utc::now(),
        }
    }

    pub fn summary(&self) -> String {
        match &self.kind {
            OperationKind::Create { path, .. } => format!("create  {}", path.display()),
            OperationKind::Delete { from, .. } => format!("delete  {}", from.display()),
            OperationKind::DeleteMany { batch } => {
                format!("delete  {} item(s)", batch.len())
            }
            OperationKind::Move { from, to } => {
                format!("move    {} -> {}", from.display(), to.display())
            }
            OperationKind::Copy { to, .. } => format!("copy    {}", to.display()),
            OperationKind::Rename { from, to } => {
                let to_name = to.file_name().map(|s| s.to_string_lossy().into_owned());
                format!(
                    "rename  {} -> {}",
                    from.display(),
                    to_name.unwrap_or_else(|| to.display().to_string())
                )
            }
            OperationKind::Organize { batch } => {
                format!("organize {} file(s)", batch.len())
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OperationKind {
    /// A file or directory was created at `path`. `is_dir` records which, so
    /// redo can recreate the right kind. (`serde(default)` keeps old logs readable.)
    Create {
        path: PathBuf,
        #[serde(default)]
        is_dir: bool,
    },
    Delete { from: PathBuf, trashed_to: PathBuf },
    DeleteMany { batch: Vec<(PathBuf, PathBuf)> },
    Move { from: PathBuf, to: PathBuf },
    Copy { from: PathBuf, to: PathBuf },
    Rename { from: PathBuf, to: PathBuf },
    Organize { batch: Vec<(PathBuf, PathBuf)> },
}
