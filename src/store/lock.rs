//! Per-project advisory write lock (`<project>/.lock`, std `File::lock`).
//! Held only around local read-modify-write, never across subprocess calls.

use crate::error::{AppError, Result};
use std::fs::OpenOptions;
use std::path::Path;

pub const LOCK_FILE: &str = ".lock";

pub fn with_lock<T>(dir: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
    let path = dir.join(LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| AppError::io(&path, e))?;
    file.lock().map_err(|e| AppError::io(&path, e))?;
    let out = f();
    let _ = file.unlock();
    out
}
