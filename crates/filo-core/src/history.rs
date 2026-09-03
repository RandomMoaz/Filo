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

    fn read_lines(&self) -> Result<Vec<String>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file = File::open(&self.path)?;
        let mut lines = Vec::new();
        for line in BufReader::new(file).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            lines.push(line);
        }
        Ok(lines)
    }

    pub fn read_all(&self) -> Result<Vec<Operation>> {
        Ok(self
            .read_lines()?
            .iter()
            .filter_map(|line| serde_json::from_str::<Operation>(line).ok())
            .collect())
    }

    /// The newest entry. Parses backwards from the end and stops at the first
    /// readable line, so a long log costs one line of JSON, not all of them.
    pub fn last(&self) -> Result<Option<Operation>> {
        Ok(self
            .read_lines()?
            .iter()
            .rev()
            .find_map(|line| serde_json::from_str::<Operation>(line).ok()))
    }

    /// The most recent `limit` entries, oldest first. Only those lines are
    /// parsed — a display that shows the tail of the log should use this
    /// rather than reading the whole history back every time.
    pub fn read_recent(&self, limit: usize) -> Result<Vec<Operation>> {
        let lines = self.read_lines()?;
        let start = lines.len().saturating_sub(limit);
        Ok(lines[start..]
            .iter()
            .filter_map(|line| serde_json::from_str::<Operation>(line).ok())
            .collect())
    }

    /// How many entries the log holds, without parsing any of them.
    pub fn len(&self) -> Result<usize> {
        Ok(self.read_lines()?.len())
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    pub fn pop(&self) -> Result<Option<Operation>> {
        let mut lines = self.read_lines()?;
        let idx = match lines
            .iter()
            .rposition(|line| serde_json::from_str::<Operation>(line).is_ok())
        {
            Some(i) => i,
            None => return Ok(None),
        };
        let popped: Operation = serde_json::from_str(&lines[idx])?;
        lines.remove(idx);
        self.rewrite(&lines)?;
        Ok(Some(popped))
    }

    pub fn clear(&self) -> Result<()> {
        if self.path.exists() {
            std::fs::remove_file(&self.path)?;
        }
        Ok(())
    }

    fn rewrite(&self, lines: &[String]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = File::create(&self.path)?;
        for line in lines {
            writeln!(file, "{}", line)?;
        }
        Ok(())
    }
}
