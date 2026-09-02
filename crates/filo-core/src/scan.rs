use crate::error::Result;
use crate::model::FileEntry;
use chrono::{DateTime, Utc};
use std::path::Path;
use walkdir::WalkDir;

pub fn entry_for(path: &Path) -> Result<FileEntry> {
    let meta = std::fs::metadata(path)?;
    let modified: DateTime<Utc> = meta
        .modified()
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(|_| Utc::now());
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let extension = path
        .extension()
        .map(|s| s.to_string_lossy().to_lowercase());
    Ok(FileEntry {
        path: path.to_path_buf(),
        name,
        is_dir: meta.is_dir(),
        size: meta.len(),
        modified,
        extension,
    })
}

pub fn list_dir(dir: &Path) -> Result<Vec<FileEntry>> {
    let mut entries = Vec::new();
    for dirent in std::fs::read_dir(dir)? {
        let dirent = dirent?;
        if let Ok(entry) = entry_for(&dirent.path()) {
            entries.push(entry);
        }
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(entries)
}

pub fn walk_files(dir: &Path, max_depth: Option<usize>) -> Vec<FileEntry> {
    let mut walker = WalkDir::new(dir);
    if let Some(d) = max_depth {
        walker = walker.max_depth(d);
    }
    walker
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| entry_for(e.path()).ok())
        .collect()
}
