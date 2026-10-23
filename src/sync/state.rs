//! `projects/<folder>/sync-state.toml`: issues the user chose to skip.

use crate::error::{AppError, Result};
use crate::store::{fsutil, lock, Project};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const FILE: &str = "sync-state.toml";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SyncState {
    #[serde(default)]
    pub ignored_issues: Vec<u64>,
}

pub fn path(project: &Project) -> PathBuf {
    project.dir.join(FILE)
}

pub fn load(project: &Project) -> Result<SyncState> {
    let p = path(project);
    match std::fs::read_to_string(&p) {
        Ok(s) => toml::from_str(&s).map_err(|e| AppError::toml(&p, &s, &e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SyncState::default()),
        Err(e) => Err(AppError::io(&p, e)),
    }
}

/// Add `ignore` and remove `unignore` under the project lock; writes only when
/// the list actually changes.
pub fn edit(project: &Project, ignore: &[u64], unignore: &[u64]) -> Result<SyncState> {
    lock::with_lock(&project.dir, || {
        let mut st = load(project)?;
        let before = st.clone();
        for n in ignore {
            if !st.ignored_issues.contains(n) {
                st.ignored_issues.push(*n);
            }
        }
        st.ignored_issues.retain(|n| !unignore.contains(n));
        st.ignored_issues.sort_unstable();
        if st != before {
            let text = toml::to_string(&st).map_err(|e| AppError::Config(e.to_string()))?;
            let p = path(project);
            fsutil::atomic_write(&p, text.as_bytes()).map_err(|e| AppError::io(&p, e))?;
        }
        Ok(st)
    })
}
