//! Project identity: key = git root of a directory, else its canonical path;
//! storage folder = `<slug>-<hash8>`.

use crate::error::{AppError, Result};
use crate::store::fsutil;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The project key of `dir`: `git rev-parse --show-toplevel`, else the
/// canonical path (symlinks resolved).
pub fn resolve_key(dir: &Path) -> Result<PathBuf> {
    let canon = std::fs::canonicalize(dir)
        .map_err(|e| AppError::NotFound(format!("{}: {e}", dir.display())))?;
    if let Some(root) = git_root(&canon) {
        return Ok(root);
    }
    Ok(canon)
}

fn git_root(dir: &Path) -> Option<PathBuf> {
    let out = fsutil::command("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    std::fs::canonicalize(s).ok()
}

/// `<slug>-<hash8>`: slug = lowercased basename restricted to `[a-z0-9-]`
/// (max 32 chars), hash8 = first 8 hex chars of SHA-256 of the key path.
pub fn folder_name(key: &Path) -> String {
    let base = key
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mut slug = String::new();
    for c in base.chars() {
        let c = if c.is_ascii_alphanumeric() { c } else { '-' };
        if c == '-' && (slug.is_empty() || slug.ends_with('-')) {
            continue;
        }
        slug.push(c);
    }
    let mut slug: String = slug.trim_end_matches('-').chars().take(32).collect();
    if slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug = "root".into();
    }
    let hash = Sha256::digest(key.to_string_lossy().as_bytes());
    let hex: String = hash.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("{slug}-{hex}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn folder_name_is_slug_and_hash() {
        let f = folder_name(Path::new("/home/u/My Repo.rs"));
        assert!(f.starts_with("my-repo-rs-"), "{f}");
        assert_eq!(f.len(), "my-repo-rs-".len() + 8);
        assert_ne!(f, folder_name(Path::new("/other/My Repo.rs")));
    }

    #[test]
    fn subdirs_of_one_repo_share_a_key() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("repo");
        std::fs::create_dir_all(repo.join("a/b")).unwrap();
        let ok = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let k1 = resolve_key(&repo).unwrap();
        let k2 = resolve_key(&repo.join("a/b")).unwrap();
        assert_eq!(k1, k2);
        assert_eq!(k1, std::fs::canonicalize(&repo).unwrap());
    }

    #[test]
    fn non_git_dir_and_symlink_use_canonical_path() {
        let d = tempfile::tempdir().unwrap();
        let plain = d.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        let link = d.path().join("link");
        std::os::unix::fs::symlink(&plain, &link).unwrap();
        let canon = std::fs::canonicalize(&plain).unwrap();
        assert_eq!(resolve_key(&plain).unwrap(), canon);
        assert_eq!(resolve_key(&link).unwrap(), canon);
    }
}
