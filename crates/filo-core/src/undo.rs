//! Undo & redo. Every `OperationKind` stores what it needs to be reversed
//! (undo) or re-played (redo), so each direction is a simple match.
//!
//! Model: the change log (`history`) is the undo stack; a separate `redo` stack
//! holds operations that have been undone. Undoing pops from history, reverses
//! it, and pushes it onto redo. Redoing pops from redo, re-applies it, and
//! pushes it back onto history. Performing any *new* operation clears the redo
//! stack (standard editor semantics) — that happens in `Filo::record`.

use crate::error::{FiloError, Result};
use crate::model::{Operation, OperationKind};
use crate::{util, Filo};

impl Filo {
    /// Reverse the most recent operation. Returns the operation that was undone.
    pub fn undo(&self) -> Result<Operation> {
        let op = self.history().last()?.ok_or(FiloError::NothingToUndo)?;
        self.reverse(&op)?;
        // Move it from the undo stack to the redo stack (only after success).
        self.history().pop()?;
        self.redo_log().append(&op)?;
        Ok(op)
    }

    /// Re-apply the most recently undone operation. Returns the redone operation.
    pub fn redo(&self) -> Result<Operation> {
        let op = self.redo_log().last()?.ok_or(FiloError::NothingToRedo)?;
        self.apply(&op)?;
        // Move it from the redo stack back onto the undo stack. Note: we append
        // directly (NOT via `record`) so the rest of the redo stack is preserved.
        self.redo_log().pop()?;
        self.history().append(&op)?;
        Ok(op)
    }

    /// Undo up to `count` operations, stopping early if there is nothing left.
    /// Returns the operations undone, in the order they were undone.
    pub fn undo_many(&self, count: usize) -> Result<Vec<Operation>> {
        let mut done = Vec::new();
        for _ in 0..count {
            match self.undo() {
                Ok(op) => done.push(op),
                Err(FiloError::NothingToUndo) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(done)
    }

    /// Redo up to `count` operations, stopping early if there is nothing left.
    pub fn redo_many(&self, count: usize) -> Result<Vec<Operation>> {
        let mut done = Vec::new();
        for _ in 0..count {
            match self.redo() {
                Ok(op) => done.push(op),
                Err(FiloError::NothingToRedo) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(done)
    }

    /// Undo everything currently on the undo stack.
    pub fn undo_all(&self) -> Result<Vec<Operation>> {
        self.undo_many(usize::MAX)
    }

    /// Redo everything currently on the redo stack.
    pub fn redo_all(&self) -> Result<Vec<Operation>> {
        self.redo_many(usize::MAX)
    }

    /// Is there anything to undo?
    pub fn can_undo(&self) -> bool {
        matches!(self.history().last(), Ok(Some(_)))
    }

    /// Is there anything to redo?
    pub fn can_redo(&self) -> bool {
        matches!(self.redo_log().last(), Ok(Some(_)))
    }

    /// Reverse a single operation (undo direction).
    fn reverse(&self, op: &Operation) -> Result<()> {
        match &op.kind {
            OperationKind::Create { path, .. } => {
                if path.exists() {
                    util::remove_path(path)?;
                }
            }
            OperationKind::Delete { from, trashed_to } => {
                // Restore from the trash back to the original location.
                util::require_exists(trashed_to)?;
                util::move_path(trashed_to, from)?;
            }
            OperationKind::Move { from, to } => {
                util::move_path(to, from)?;
            }
            OperationKind::Copy { to, .. } => {
                if to.exists() {
                    util::remove_path(to)?;
                }
            }
            OperationKind::Rename { from, to } => {
                util::move_path(to, from)?;
            }
            OperationKind::Organize { batch } => {
                for (from, to) in batch.iter().rev() {
                    if to.exists() {
                        util::move_path(to, from)?;
                    }
                }
                // Remove any destination folders left empty by the reversal.
                for (_, to) in batch {
                    if let Some(dir) = to.parent() {
                        let _ = std::fs::remove_dir(dir); // no-op if not empty
                    }
                }
            }
        }
        Ok(())
    }

    /// Re-play a single operation forward (redo direction).
    fn apply(&self, op: &Operation) -> Result<()> {
        match &op.kind {
            OperationKind::Create { path, is_dir } => {
                if *is_dir {
                    std::fs::create_dir_all(path)?;
                } else {
                    if let Some(parent) = path.parent() {
                        if !parent.as_os_str().is_empty() {
                            std::fs::create_dir_all(parent)?;
                        }
                    }
                    std::fs::File::create(path)?;
                }
            }
            OperationKind::Delete { from, trashed_to } => {
                util::move_path(from, trashed_to)?;
            }
            OperationKind::Move { from, to } => {
                util::move_path(from, to)?;
            }
            OperationKind::Copy { from, to } => {
                util::copy_path(from, to)?;
            }
            OperationKind::Rename { from, to } => {
                util::move_path(from, to)?;
            }
            OperationKind::Organize { batch } => {
                for (from, to) in batch {
                    util::move_path(from, to)?;
                }
            }
        }
        Ok(())
    }
}
