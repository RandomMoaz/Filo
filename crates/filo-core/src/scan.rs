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

/// Directory names that hold generated or vendored files. Walking them is slow
/// and nothing inside is worth a tidy-up suggestion.
pub const NOISE_DIRS: [&str; 9] = [
    ".git",
    "target",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    "dist",
    "build",
];

pub fn walk_files_skipping(dir: &Path, skip: &[&str]) -> Vec<FileEntry> {
    WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 || !e.file_type().is_dir() {
                return true;
            }
            let name = e.file_name().to_string_lossy().to_lowercase();
            !skip.iter().any(|noise| *noise == name)
        })
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| entry_for(e.path()).ok())
        .collect()
}

pub fn walk_dirs_skipping(dir: &Path, skip: &[&str]) -> Vec<std::path::PathBuf> {
    WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 || !e.file_type().is_dir() {
                return true;
            }
            let name = e.file_name().to_string_lossy().to_lowercase();
            !skip.iter().any(|noise| *noise == name)
        })
        .filter_map(|e| e.ok())
        .filter(|e| e.depth() > 0 && e.file_type().is_dir())
        .map(|e| e.path().to_path_buf())
        .collect()
}

pub fn is_empty_dir(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|mut it| it.next().is_none())
        .unwrap_or(false)
}

/// Files that mark a directory as the root of a project or repository.
/// Organizing such a folder scatters the very files its tooling looks for.
const PROJECT_MARKERS: [&str; 10] = [
    ".git",
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "Gemfile",
    "composer.json",
    "CMakeLists.txt",
];

/// The marker that makes `dir` look like a project root, if any.
pub fn project_marker(dir: &Path) -> Option<&'static str> {
    PROJECT_MARKERS
        .iter()
        .find(|marker| dir.join(marker).exists())
        .copied()
}
