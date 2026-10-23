//! Sequential task ids `t-NNNN`: scan for the max id, then claim the next one
//! with an atomic `create_dir` (retry on `AlreadyExists`).

use crate::error::{AppError, Result};
use std::path::{Path, PathBuf};

const MAX_ATTEMPTS: u32 = 100;

pub fn parse_id(name: &str) -> Option<u32> {
    let n = name.strip_prefix("t-")?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    n.parse().ok()
}

pub fn format_id(n: u32) -> String {
    format!("t-{n:04}")
}

/// Accept `t-0042`, `t-42` and `42`; reject anything that is not an id (so a
/// CLI argument can never escape the tasks directory).
pub fn normalize_id(input: &str) -> Result<String> {
    let s = input.trim();
    let n = if s.bytes().all(|b| b.is_ascii_digit()) && !s.is_empty() {
        s.parse().ok()
    } else {
        parse_id(s)
    };
    n.map(format_id)
        .ok_or_else(|| AppError::Validation(format!("invalid task id `{input}` (expected t-NNNN)")))
}

fn max_id(tasks_dir: &Path) -> u32 {
    std::fs::read_dir(tasks_dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| parse_id(&e.file_name().to_string_lossy()))
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0)
}

/// Create and return the folder of the next free id.
pub fn allocate(tasks_dir: &Path) -> Result<(String, PathBuf)> {
    std::fs::create_dir_all(tasks_dir).map_err(|e| AppError::io(tasks_dir, e))?;
    let mut next = max_id(tasks_dir)
        .checked_add(1)
        .ok_or_else(|| AppError::Config(format!("{}: task ids exhausted", tasks_dir.display())))?;
    for _ in 0..MAX_ATTEMPTS {
        let id = format_id(next);
        let dir = tasks_dir.join(&id);
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok((id, dir)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && next < u32::MAX => next += 1,
            Err(e) => return Err(AppError::io(&dir, e)),
        }
    }
    Err(AppError::Config(format!(
        "{}: could not allocate a task id",
        tasks_dir.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_normalize() {
        assert_eq!(normalize_id("42").unwrap(), "t-0042");
        assert_eq!(normalize_id("t-7").unwrap(), "t-0007");
        assert_eq!(normalize_id("t-12345").unwrap(), "t-12345");
        assert!(normalize_id("../x").is_err());
        assert!(normalize_id("t-").is_err());
    }

    #[test]
    fn allocate_skips_taken() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("t-0003")).unwrap();
        assert_eq!(allocate(d.path()).unwrap().0, "t-0004");
        assert_eq!(allocate(d.path()).unwrap().0, "t-0005");
    }
}
