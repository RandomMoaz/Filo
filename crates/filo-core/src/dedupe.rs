use crate::error::Result;
use crate::model::Operation;
use crate::{scan, Filo};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateGroup {
    pub hash: String,
    pub size: u64,
    pub paths: Vec<PathBuf>,
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn find_duplicates(dir: &Path) -> Result<Vec<DuplicateGroup>> {
    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for entry in scan::walk_files(dir, None) {
        if entry.size == 0 {
            continue;
        }
        by_size.entry(entry.size).or_default().push(entry.path);
    }

    let mut groups = Vec::new();
    for (size, paths) in by_size {
        if paths.len() < 2 {
            continue;
        }
        let mut by_hash: HashMap<String, Vec<PathBuf>> = HashMap::new();
        for p in paths {
            if let Ok(h) = hash_file(&p) {
                by_hash.entry(h).or_default().push(p);
            }
        }
        for (hash, mut paths) in by_hash {
            if paths.len() > 1 {
                paths.sort();
                groups.push(DuplicateGroup { hash, size, paths });
            }
        }
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.size));
    Ok(groups)
}

impl Filo {
    pub fn find_duplicates(&self, dir: &Path) -> Result<Vec<DuplicateGroup>> {
        find_duplicates(dir)
    }

    pub fn dedupe_delete(&self, dir: &Path) -> Result<(Vec<DuplicateGroup>, Vec<Operation>)> {
        let groups = find_duplicates(dir)?;
        let mut ops = Vec::new();
        for group in &groups {
            for path in group.paths.iter().skip(1) {
                ops.push(self.delete(path, false)?);
            }
        }
        Ok((groups, ops))
    }
}
