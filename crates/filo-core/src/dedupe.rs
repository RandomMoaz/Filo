use crate::error::Result;
use crate::model::{FileEntry, Operation};
use crate::{scan, Filo};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Files are first compared on their opening bytes only. Two files that differ
/// early — the common case — never get read past this.
const HEAD_BYTES: u64 = 16 * 1024;
const MAX_HASH_THREADS: usize = 8;

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateGroup {
    pub hash: String,
    pub size: u64,
    pub paths: Vec<PathBuf>,
}

fn hash_file(path: &Path, limit: Option<u64>) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut read = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        read += n as u64;
        if limit.is_some_and(|cap| read >= cap) {
            break;
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Hash a batch across a few threads. Hashing is CPU- and IO-bound in equal
/// measure, so even a handful of workers makes a large difference.
fn hash_many(paths: &[PathBuf], limit: Option<u64>) -> Vec<Option<String>> {
    if paths.len() < 2 {
        return paths.iter().map(|p| hash_file(p, limit).ok()).collect();
    }
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, MAX_HASH_THREADS)
        .min(paths.len());
    let chunk = paths.len().div_ceil(workers);

    let mut hashes = Vec::with_capacity(paths.len());
    std::thread::scope(|scope| {
        let handles: Vec<_> = paths
            .chunks(chunk)
            .map(|slice| {
                scope.spawn(move || {
                    slice
                        .iter()
                        .map(|p| hash_file(p, limit).ok())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for handle in handles {
            match handle.join() {
                Ok(part) => hashes.extend(part),
                Err(_) => hashes.extend(std::iter::repeat_n(None, chunk)),
            }
        }
    });
    hashes.truncate(paths.len());
    hashes
}

fn group_by_hash(paths: &[PathBuf], limit: Option<u64>) -> HashMap<String, Vec<PathBuf>> {
    let mut grouped: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for (path, hash) in paths.iter().zip(hash_many(paths, limit)) {
        if let Some(hash) = hash {
            grouped.entry(hash).or_default().push(path.clone());
        }
    }
    grouped
}

pub fn find_duplicates(dir: &Path) -> Result<Vec<DuplicateGroup>> {
    Ok(find_duplicates_in(&scan::walk_files(dir, None)))
}

/// Group already-scanned files by identical content, so callers that have
/// walked the tree for their own reasons do not have to walk it again.
///
/// Three passes, each cheaper than the one it feeds: bucket by size, then by a
/// hash of the opening bytes, and only then hash the surviving candidates in
/// full. Empty files are skipped — they all "match", which is useless.
pub fn find_duplicates_in(entries: &[FileEntry]) -> Vec<DuplicateGroup> {
    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for entry in entries {
        if entry.size == 0 || entry.is_dir {
            continue;
        }
        by_size
            .entry(entry.size)
            .or_default()
            .push(entry.path.clone());
    }

    let mut groups = Vec::new();
    for (size, paths) in by_size {
        if paths.len() < 2 {
            continue;
        }
        // For a file no bigger than the head, the head hash *is* the full hash.
        let head_limit = (size > HEAD_BYTES).then_some(HEAD_BYTES);

        for (head, candidates) in group_by_hash(&paths, head_limit) {
            if candidates.len() < 2 {
                continue;
            }
            if head_limit.is_none() {
                let mut paths = candidates;
                paths.sort();
                groups.push(DuplicateGroup {
                    hash: head,
                    size,
                    paths,
                });
                continue;
            }
            for (hash, mut paths) in group_by_hash(&candidates, None) {
                if paths.len() > 1 {
                    paths.sort();
                    groups.push(DuplicateGroup { hash, size, paths });
                }
            }
        }
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.size));
    groups
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
