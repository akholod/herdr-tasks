//! Filesystem and process helpers: atomic writes, stale temp cleanup, sizes,
//! and a PATH that still finds tools when herdr starts us with a minimal one.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// Directories appended to `PATH` when missing (herdr launches plugin
/// processes with a minimal PATH; see herdr-beads `scripts/lib.sh`).
pub const DEFAULT_PATH_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/home/linuxbrew/.linuxbrew/bin",
    "/usr/local/bin",
    "/usr/bin",
    "/bin",
];

/// Marker inside temp file names; the watcher and loaders skip these.
pub const TMP_MARKER: &str = ".tmp-";

/// `$PATH` with the default tool directories appended. Appending (not
/// prefixing) keeps the caller's order, so a test's fake `gh` still wins.
pub fn tool_path() -> OsString {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&current).collect();
    for d in DEFAULT_PATH_DIRS {
        let p = PathBuf::from(d);
        if !dirs.contains(&p) {
            dirs.push(p);
        }
    }
    std::env::join_paths(dirs).unwrap_or(current)
}

/// A `Command` whose program lookup and child env use `tool_path()`.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut c = Command::new(program);
    c.env("PATH", tool_path());
    c
}

/// Write via a temp file in the same directory, then rename over the target.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(
        "{name}{TMP_MARKER}{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    if let Err(e) = std::fs::write(&tmp, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

pub fn is_tmp_name(name: &str) -> bool {
    name.contains(TMP_MARKER)
}

/// Delete crash leftovers (`*.tmp-*` older than `max_age`) in the project
/// directory and in each task folder.
pub fn clean_stale_tmp(project_dir: &Path, max_age: Duration) {
    let mut dirs = vec![project_dir.to_path_buf()];
    if let Ok(rd) = std::fs::read_dir(project_dir.join("tasks")) {
        dirs.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
    }
    let now = SystemTime::now();
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            if !is_tmp_name(&e.file_name().to_string_lossy()) {
                continue;
            }
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .is_some_and(|age| age > max_age);
            if old {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64 / 1024.0;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", UNITS[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(human_size(12), "12 B");
        assert_eq!(human_size(12_595), "12.3 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn atomic_write_leaves_no_tmp() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.toml");
        atomic_write(&p, b"x").unwrap();
        atomic_write(&p, b"y").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"y");
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    }

    #[test]
    fn stale_tmp_removed_fresh_kept() {
        let d = tempfile::tempdir().unwrap();
        let t = d.path().join("tasks/t-0001");
        std::fs::create_dir_all(&t).unwrap();
        let old = t.join("task.toml.tmp-x");
        let fresh = t.join("task.toml.tmp-y");
        std::fs::write(&old, "").unwrap();
        std::fs::write(&fresh, "").unwrap();
        let past =
            filetime::FileTime::from_system_time(SystemTime::now() - Duration::from_secs(120));
        filetime::set_file_mtime(&old, past).unwrap();
        clean_stale_tmp(d.path(), Duration::from_secs(60));
        assert!(!old.exists());
        assert!(fresh.exists());
    }

    #[test]
    fn tool_path_appends_defaults_after_caller_dirs() {
        let p = tool_path();
        let dirs: Vec<PathBuf> = std::env::split_paths(&p).collect();
        assert!(dirs.contains(&PathBuf::from("/usr/bin")));
    }
}
