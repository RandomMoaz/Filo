use crate::error::{FiloError, Result};
use crate::model::{Operation, OperationKind};
use crate::{util, Filo};

impl Filo {
    pub fn undo(&self) -> Result<Operation> {
        let op = self.history().last()?.ok_or(FiloError::NothingToUndo)?;
        self.reverse(&op)?;
        self.history().pop()?;
        Ok(op)
    }

    fn reverse(&self, op: &Operation) -> Result<()> {
        match &op.kind {
            OperationKind::Create { path } => {
                if path.exists() {
                    util::remove_path(path)?;
                }
            }
            OperationKind::Delete { from, trashed_to } => {
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
                for (_, to) in batch {
                    if let Some(dir) = to.parent() {
                        let _ = std::fs::remove_dir(dir);
                    }
                }
            }
        }
        Ok(())
    }
}
