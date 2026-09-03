use crate::error::{FiloError, Result};
use crate::model::{Operation, OperationKind};
use crate::{scan, util, Filo};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct RenamePlan {
    pub from: PathBuf,
    pub to: PathBuf,
}

pub enum RenameSpec {
    Pattern(String),
    Regex { find: String, replace: String },
}

impl Filo {
    pub fn plan_bulk_rename(&self, dir: &Path, spec: &RenameSpec) -> Result<Vec<RenamePlan>> {
        let mut files: Vec<_> = scan::list_dir(dir)?
            .into_iter()
            .filter(|e| !e.is_dir)
            .collect();
        files.sort_by(|a, b| a.name.cmp(&b.name));

        let re = match spec {
            RenameSpec::Regex { find, .. } => Some(regex::Regex::new(find)?),
            RenameSpec::Pattern(_) => None,
        };

        let mut plans = Vec::new();
        for (i, entry) in files.iter().enumerate() {
            let stem = Path::new(&entry.name)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| entry.name.clone());
            let ext = entry.extension.clone().unwrap_or_default();

            let new_name = match spec {
                RenameSpec::Pattern(tpl) => {
                    let mut name = tpl
                        .replace("{n}", &(i + 1).to_string())
                        .replace("{name}", &stem)
                        .replace("{ext}", &ext)
                        .replace("{date}", &entry.modified.format("%Y%m%d").to_string());
                    let suffix = format!(".{}", ext.to_lowercase());
                    if !ext.is_empty()
                        && !name.to_lowercase().ends_with(&suffix)
                        && !tpl.contains("{ext}")
                    {
                        name = format!("{}.{}", name, ext);
                    }
                    name
                }
                RenameSpec::Regex { replace, .. } => re
                    .as_ref()
                    .unwrap()
                    .replace_all(&entry.name, replace.as_str())
                    .into_owned(),
            };

            if new_name == entry.name || new_name.is_empty() {
                continue;
            }
            plans.push(RenamePlan {
                from: entry.path.clone(),
                to: dir.join(new_name),
            });
        }

        check_collisions(&plans)?;
        Ok(plans)
    }

    pub fn bulk_rename(&self, dir: &Path, spec: &RenameSpec) -> Result<Vec<Operation>> {
        let plans = self.plan_bulk_rename(dir, spec)?;
        let targets: HashSet<PathBuf> = plans.iter().map(|p| p.to.clone()).collect();

        let mut staged: HashMap<PathBuf, PathBuf> = HashMap::new();
        for plan in &plans {
            if targets.contains(&plan.from) {
                let temp = dir.join(format!(".filo-rename-{}", Uuid::new_v4()));
                util::move_path(&plan.from, &temp)?;
                staged.insert(plan.from.clone(), temp);
            }
        }

        let mut ops = Vec::new();
        for plan in &plans {
            let current = staged
                .get(&plan.from)
                .cloned()
                .unwrap_or_else(|| plan.from.clone());
            if let Err(e) = util::move_path_no_clobber(&current, &plan.to) {
                for (from, temp) in &staged {
                    if temp.exists() {
                        let _ = util::move_path(temp, from);
                    }
                }
                return Err(e);
            }
            staged.remove(&plan.from);
            ops.push(self.record(Operation::new(OperationKind::Rename {
                from: plan.from.clone(),
                to: plan.to.clone(),
            }))?);
        }
        Ok(ops)
    }
}

fn check_collisions(plans: &[RenamePlan]) -> Result<()> {
    let sources: HashSet<PathBuf> = plans.iter().map(|p| p.from.clone()).collect();
    let mut targets: HashSet<PathBuf> = HashSet::new();
    for plan in plans {
        if !targets.insert(plan.to.clone()) {
            return Err(FiloError::NameCollision(plan.to.clone()));
        }
        if plan.to.exists() && !sources.contains(&plan.to) {
            return Err(FiloError::NameCollision(plan.to.clone()));
        }
    }
    Ok(())
}
