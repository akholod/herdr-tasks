//! Attachments: copies under `attachments/`, listed as name + size.

use crate::error::{AppError, Result};
use crate::store::fsutil;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    pub size: u64,
    pub path: PathBuf,
}

pub fn list(task_dir: &Path) -> Vec<Attachment> {
    let dir = task_dir.join("attachments");
    let mut out: Vec<Attachment> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .filter(|e| !fsutil::is_tmp_name(&e.file_name().to_string_lossy()))
                .map(|e| Attachment {
                    name: e.file_name().to_string_lossy().into_owned(),
                    size: e.metadata().map(|m| m.len()).unwrap_or(0),
                    path: e.path(),
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// `log.txt` -> `log-1.txt`, `log-2.txt`, ... until the name is free.
fn free_name(dir: &Path, name: &str) -> String {
    if !dir.join(name).exists() {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (1..)
        .map(|n| format!("{stem}-{n}{ext}"))
        .find(|n| !dir.join(n).exists())
        .unwrap_or_else(|| name.to_string())
}

/// Copy `files` into the task's `attachments/`; returns the new entries.
pub fn copy_in(task_dir: &Path, files: &[PathBuf]) -> Result<Vec<Attachment>> {
    for f in files {
        if f.is_dir() {
            return Err(AppError::Validation(format!(
                "{}: is a directory, attach files",
                f.display()
            )));
        }
        if !f.is_file() {
            return Err(AppError::NotFound(format!("{}: no such file", f.display())));
        }
    }
    let dir = task_dir.join("attachments");
    std::fs::create_dir_all(&dir).map_err(|e| AppError::io(&dir, e))?;
    let mut added = Vec::new();
    for f in files {
        let name = f
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| AppError::Validation(format!("{}: not a file name", f.display())))?;
        let name = free_name(&dir, &name);
        let dest = dir.join(&name);
        let size = std::fs::copy(f, &dest).map_err(|e| AppError::io(f, e))?;
        added.push(Attachment {
            name,
            size,
            path: dest,
        });
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clash_gets_numeric_suffix() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("log.txt");
        std::fs::write(&src, "hello").unwrap();
        let task = d.path().join("t-0001");
        std::fs::create_dir_all(&task).unwrap();
        copy_in(&task, std::slice::from_ref(&src)).unwrap();
        let second = copy_in(&task, &[src]).unwrap();
        assert_eq!(second[0].name, "log-1.txt");
        let names: Vec<_> = list(&task).into_iter().map(|a| (a.name, a.size)).collect();
        assert_eq!(names, vec![("log-1.txt".into(), 5), ("log.txt".into(), 5)]);
    }
}
