use crate::error::Result;
use crate::model::{FileEntry, Operation, OperationKind};
use crate::rules::RuleSet;
use crate::{scan, util, Filo};
use chrono::Datelike;
use std::path::{Path, PathBuf};

pub enum OrganizeStrategy {
    Extension,
    Date,
    Size,
    Rules(RuleSet),
}

impl OrganizeStrategy {
    fn destination_for(&self, entry: &FileEntry) -> Option<String> {
        match self {
            OrganizeStrategy::Extension => Some(match &entry.extension {
                Some(ext) if !ext.is_empty() => ext.clone(),
                _ => "no_extension".to_string(),
            }),
            OrganizeStrategy::Date => Some(format!(
                "{}/{:02}",
                entry.modified.year(),
                entry.modified.month()
            )),
            OrganizeStrategy::Size => Some(
                match entry.size {
                    s if s < 1024 * 1024 => "small",
                    s if s < 100 * 1024 * 1024 => "medium",
                    _ => "large",
                }
                .to_string(),
            ),
            OrganizeStrategy::Rules(set) => set.destination_for(entry),
        }
    }
}

impl Filo {
    pub fn plan_organize(&self, dir: &Path, strategy: &OrganizeStrategy) -> Result<Vec<(PathBuf, PathBuf)>> {
        let mut batch = Vec::new();
        for entry in scan::list_dir(dir)? {
            if entry.is_dir {
                continue;
            }
            if let Some(folder) = strategy.destination_for(&entry) {
                let dest_dir = dir.join(&folder);
                let to = dest_dir.join(&entry.name);
                if to == entry.path || to.exists() {
                    continue;
                }
                batch.push((entry.path.clone(), to));
            }
        }
        Ok(batch)
    }

    pub fn organize(&self, dir: &Path, strategy: &OrganizeStrategy) -> Result<Operation> {
        let batch = self.plan_organize(dir, strategy)?;
        let mut done = Vec::new();
        for (from, to) in batch {
            match util::move_path_no_clobber(&from, &to) {
                Ok(()) => done.push((from, to)),
                Err(e) => {
                    if !done.is_empty() {
                        self.record(Operation::new(OperationKind::Organize { batch: done }))?;
                    }
                    return Err(e);
                }
            }
        }
        self.record(Operation::new(OperationKind::Organize { batch: done }))
    }
}
