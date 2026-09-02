use crate::error::{FiloError, Result};
use crate::model::{Operation, OperationKind};
use crate::util;
use crate::Filo;
use std::path::{Path, PathBuf};
use uuid::Uuid;

impl Filo {
    pub fn create(&self, path: &Path, as_dir: bool) -> Result<Operation> {
        if path.exists() {
            return Err(FiloError::AlreadyExists(path.to_path_buf()));
        }
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        if as_dir {
            std::fs::create_dir(path)?;
        } else {
            std::fs::File::create(path)?;
        }
        self.record(Operation::new(OperationKind::Create {
            path: path.to_path_buf(),
        }))
    }

    pub fn delete(&self, path: &Path, permanent: bool) -> Result<Operation> {
        util::require_exists(path)?;
        let name = path
            .file_name()
            .map(|s| s.to_owned())
            .unwrap_or_else(|| "unnamed".into());
        let target: PathBuf = self.trash_dir().join(Uuid::new_v4().to_string()).join(&name);
        util::move_path(path, &target)?;

        let op = self.record(Operation::new(OperationKind::Delete {
            from: path.to_path_buf(),
            trashed_to: target.clone(),
        }))?;

        if permanent {
            let _ = util::remove_path(&target);
        }
        Ok(op)
    }

    pub fn add(&self, sources: &[PathBuf], dest_dir: &Path, move_them: bool) -> Result<Vec<Operation>> {
        if !dest_dir.exists() {
            std::fs::create_dir_all(dest_dir)?;
        }
        if !dest_dir.is_dir() {
            return Err(FiloError::NotADirectory(dest_dir.to_path_buf()));
        }
        let mut ops = Vec::new();
        for src in sources {
            util::require_exists(src)?;
            let name = src
                .file_name()
                .ok_or_else(|| FiloError::NotFound(src.clone()))?;
            let to = dest_dir.join(name);
            if to.exists() {
                return Err(FiloError::WouldOverwrite(to));
            }
            let op = if move_them {
                util::move_path(src, &to)?;
                Operation::new(OperationKind::Move {
                    from: src.clone(),
                    to,
                })
            } else {
                util::copy_path(src, &to)?;
                Operation::new(OperationKind::Copy {
                    from: src.clone(),
                    to,
                })
            };
            ops.push(self.record(op)?);
        }
        Ok(ops)
    }

    pub fn rename(&self, path: &Path, new_name: &str) -> Result<Operation> {
        util::require_exists(path)?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let to = parent.join(new_name);
        if to.exists() {
            return Err(FiloError::WouldOverwrite(to));
        }
        util::move_path(path, &to)?;
        self.record(Operation::new(OperationKind::Rename {
            from: path.to_path_buf(),
            to,
        }))
    }

    pub fn empty_trash(&self) -> Result<usize> {
        let trash = self.trash_dir();
        if !trash.exists() {
            return Ok(0);
        }
        let mut count = 0;
        for entry in std::fs::read_dir(trash)? {
            let entry = entry?;
            util::remove_path(&entry.path())?;
            count += 1;
        }
        Ok(count)
    }
}
