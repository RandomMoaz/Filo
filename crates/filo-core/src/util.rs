use crate::error::{FiloError, Result};
use std::path::Path;

pub fn make_parent_dir(dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        if parent.as_os_str().is_empty() {
            return Ok(());
        }
        std::fs::create_dir_all(parent).map_err(|e| {
            FiloError::PathIo(
                format!("could not create the folder {}", parent.display()),
                e,
            )
        })?;
    }
    Ok(())
}

pub fn copy_path(src: &Path, dst: &Path) -> Result<()> {
    let meta = std::fs::metadata(src)?;
    if meta.is_dir() {
        copy_dir_all(src, dst)
    } else {
        make_parent_dir(dst)?;
        std::fs::copy(src, dst)?;
        Ok(())
    }
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

pub fn move_path(src: &Path, dst: &Path) -> Result<()> {
    make_parent_dir(dst)?;
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_path(src, dst)?;
            remove_path(src)?;
            Ok(())
        }
    }
}

pub fn remove_path(path: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

pub fn require_exists(path: &Path) -> Result<()> {
    if path.exists() {
        Ok(())
    } else {
        Err(FiloError::NotFound(path.to_path_buf()))
    }
}

pub fn move_path_no_clobber(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        return Err(FiloError::WouldOverwrite(dst.to_path_buf()));
    }
    move_path(src, dst)
}
