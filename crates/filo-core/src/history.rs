use crate::error::Result;
use crate::model::Operation;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub struct History {
    path: PathBuf,
}

impl History {
    pub fn new(path: PathBuf) -> Self {
        History { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, op: &Operation) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let line = serde_json::to_string(op)?;
        writeln!(file, "{}", line)?;
        Ok(())
    }

    pub fn read_all(&self) -> Result<Vec<Operation>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let mut ops = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(op) = serde_json::from_str::<Operation>(&line) {
                ops.push(op);
            }
        }
        Ok(ops)
    }

    pub fn last(&self) -> Result<Option<Operation>> {
        Ok(self.read_all()?.pop())
    }

    pub fn pop(&self) -> Result<Option<Operation>> {
        let mut ops = self.read_all()?;
        let popped = ops.pop();
        self.rewrite(&ops)?;
        Ok(popped)
    }

    /// Remove every entry (used to clear the redo stack after a new action).
    pub fn clear(&self) -> Result<()> {
        if self.path.exists() {
            std::fs::remove_file(&self.path)?;
        }
        Ok(())
    }

    fn rewrite(&self, ops: &[Operation]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = File::create(&self.path)?;
        for op in ops {
            writeln!(file, "{}", serde_json::to_string(op)?)?;
        }
        Ok(())
    }
}
