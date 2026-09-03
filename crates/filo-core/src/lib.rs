pub mod dedupe;
pub mod error;
pub mod history;
pub mod model;
pub mod ops;
pub mod organize;
pub mod rename;
pub mod rules;
pub mod scan;
pub mod undo;
pub mod util;

pub use dedupe::DuplicateGroup;
pub use error::{FiloError, Result};
pub use history::History;
pub use model::{FileEntry, Operation, OperationKind};
pub use organize::OrganizeStrategy;
pub use rename::{RenamePlan, RenameSpec};

use directories::ProjectDirs;
use std::path::{Path, PathBuf};

pub struct Filo {
    data_dir: PathBuf,
    trash_dir: PathBuf,
    history: History,
    redo: History,
}

impl Filo {
    pub fn new() -> Result<Self> {
        let dirs = ProjectDirs::from("dev", "filo", "filo").ok_or(FiloError::NoDataDir)?;
        Self::with_data_dir(dirs.data_dir().to_path_buf())
    }

    pub fn with_data_dir(dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&dir)?;
        let trash_dir = dir.join("trash");
        let history = History::new(dir.join("history.jsonl"));
        let redo = History::new(dir.join("redo.jsonl"));
        Ok(Filo {
            data_dir: dir,
            trash_dir,
            history,
            redo,
        })
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn trash_dir(&self) -> &Path {
        &self.trash_dir
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    /// The redo stack (operations that have been undone).
    pub fn redo_stack(&self) -> &History {
        &self.redo
    }

    /// Internal accessor used by undo/redo.
    pub(crate) fn redo_log(&self) -> &History {
        &self.redo
    }

    /// Append a *new* operation to the change log. Doing so clears the redo
    /// stack, because a new action invalidates anything that was undone.
    pub(crate) fn record(&self, op: Operation) -> Result<Operation> {
        self.history.append(&op)?;
        self.redo.clear()?;
        Ok(op)
    }
}
