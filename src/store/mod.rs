//! The file store: root resolution, `projects.toml` registry, projects.

pub mod attach;
pub mod fsutil;
pub mod ids;
pub mod lock;
pub mod project;
pub mod task;

use crate::error::{AppError, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const PLUGIN_ID: &str = "herdr-tasks";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootSource {
    EnvOverride,
    PluginEnv,
    HerdrConfigDir,
    Fallback,
}

impl RootSource {
    pub fn as_str(self) -> &'static str {
        match self {
            RootSource::EnvOverride => "env-override",
            RootSource::PluginEnv => "plugin-env",
            RootSource::HerdrConfigDir => "herdr-config-dir",
            RootSource::Fallback => "fallback",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Store {
    pub root: PathBuf,
    pub root_source: RootSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub key: PathBuf,
    pub folder: String,
    pub dir: PathBuf,
}

impl Project {
    pub fn tasks_dir(&self) -> PathBuf {
        self.dir.join("tasks")
    }

    pub fn task_dir(&self, id: &str) -> PathBuf {
        self.tasks_dir().join(id)
    }

    /// Basename of the key, used when no herdr space label is known.
    pub fn basename(&self) -> String {
        self.key
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.key.display().to_string())
    }
}

/// Resolve the storage root:
/// 1. `$HERDR_TASKS_HOME`; 2. `$HERDR_PLUGIN_CONFIG_DIR`;
/// 3. `herdr plugin config-dir herdr-tasks` (2 s budget);
/// 4. `${XDG_CONFIG_HOME:-~/.config}/herdr/plugins/config/herdr-tasks`.
pub fn resolve_root_with(
    env: impl Fn(&str) -> Option<String>,
    herdr_config_dir: impl FnOnce() -> Option<PathBuf>,
) -> (PathBuf, RootSource) {
    let set = |k: &str| env(k).filter(|v| !v.trim().is_empty());
    if let Some(v) = set("HERDR_TASKS_HOME") {
        return (PathBuf::from(v), RootSource::EnvOverride);
    }
    if let Some(v) = set("HERDR_PLUGIN_CONFIG_DIR") {
        return (PathBuf::from(v), RootSource::PluginEnv);
    }
    if let Some(p) = herdr_config_dir() {
        return (p, RootSource::HerdrConfigDir);
    }
    let base = set("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(set("HOME").unwrap_or_default()).join(".config"));
    (
        base.join("herdr/plugins/config").join(PLUGIN_ID),
        RootSource::Fallback,
    )
}

pub fn herdr_bin() -> String {
    std::env::var("HERDR_BIN_PATH")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "herdr".into())
}

/// Ask herdr for our config dir; `None` if herdr is missing, fails, or takes
/// longer than 2 s.
fn herdr_config_dir() -> Option<PathBuf> {
    let mut child = fsutil::command(herdr_bin())
        .args(["plugin", "config-dir", PLUGIN_ID])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut s = String::new();
                use std::io::Read;
                child.stdout.take()?.read_to_string(&mut s).ok()?;
                let s = s.trim();
                return (!s.is_empty() && Path::new(s).is_absolute()).then(|| PathBuf::from(s));
            }
            Ok(None) if start.elapsed() < Duration::from_secs(2) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

impl Store {
    pub fn resolve() -> Store {
        let (root, root_source) = resolve_root_with(|k| std::env::var(k).ok(), herdr_config_dir);
        Store { root, root_source }
    }

    pub fn at(root: impl Into<PathBuf>) -> Store {
        Store {
            root: root.into(),
            root_source: RootSource::EnvOverride,
        }
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.root.join("projects")
    }

    pub fn registry_path(&self) -> PathBuf {
        self.root.join("projects.toml")
    }

    /// Create `<root>/projects/` so the watcher never fails on a fresh install.
    pub fn ensure(&self) -> Result<()> {
        let p = self.projects_dir();
        std::fs::create_dir_all(&p).map_err(|e| AppError::io(&p, e))
    }

    /// `projects.toml`: key path -> folder.
    pub fn registry(&self) -> Result<BTreeMap<String, String>> {
        let path = self.registry_path();
        let src = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(e) => return Err(AppError::io(&path, e)),
        };
        toml::from_str(&src).map_err(|e| AppError::toml(&path, &src, &e))
    }

    pub fn project_for_key(&self, key: &Path) -> Project {
        let k = key.to_string_lossy().into_owned();
        let folder = self
            .registry()
            .ok()
            .and_then(|r| r.get(&k).cloned())
            .unwrap_or_else(|| project::folder_name(key));
        Project {
            key: key.to_path_buf(),
            dir: self.projects_dir().join(&folder),
            folder,
        }
    }

    pub fn project_for_dir(&self, dir: &Path) -> Result<Project> {
        let key = project::resolve_key(dir)?;
        Ok(self.project_for_key(&key))
    }

    pub fn registered_projects(&self) -> Result<Vec<Project>> {
        Ok(self
            .registry()?
            .into_iter()
            .map(|(k, folder)| Project {
                key: PathBuf::from(k),
                dir: self.projects_dir().join(&folder),
                folder,
            })
            .collect())
    }

    /// Add the project to `projects.toml` (idempotent, atomic, root-locked).
    pub fn register(&self, project: &Project) -> Result<()> {
        let key = project.key.to_string_lossy().into_owned();
        if self.registry()?.get(&key) == Some(&project.folder) {
            return Ok(());
        }
        self.ensure()?;
        lock::with_lock(&self.root, || {
            let mut reg = self.registry()?;
            if reg.get(&key) == Some(&project.folder) {
                return Ok(());
            }
            reg.insert(key.clone(), project.folder.clone());
            let text = toml::to_string(&reg)
                .map_err(|e| AppError::Config(format!("projects.toml: {e}")))?;
            let path = self.registry_path();
            fsutil::atomic_write(&path, text.as_bytes()).map_err(|e| AppError::io(&path, e))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn root_order() {
        let herdr = || Some(PathBuf::from("/h/cfg"));
        let all = env(&[
            ("HERDR_TASKS_HOME", "/t"),
            ("HERDR_PLUGIN_CONFIG_DIR", "/p"),
            ("HOME", "/home/u"),
        ]);
        assert_eq!(
            resolve_root_with(&all, herdr),
            (PathBuf::from("/t"), RootSource::EnvOverride)
        );
        let plug = env(&[("HERDR_PLUGIN_CONFIG_DIR", "/p"), ("HOME", "/home/u")]);
        assert_eq!(
            resolve_root_with(&plug, herdr),
            (PathBuf::from("/p"), RootSource::PluginEnv)
        );
        let none = env(&[("HOME", "/home/u")]);
        assert_eq!(
            resolve_root_with(&none, herdr),
            (PathBuf::from("/h/cfg"), RootSource::HerdrConfigDir)
        );
        assert_eq!(
            resolve_root_with(&none, || None),
            (
                PathBuf::from("/home/u/.config/herdr/plugins/config/herdr-tasks"),
                RootSource::Fallback
            )
        );
        let xdg = env(&[("HOME", "/home/u"), ("XDG_CONFIG_HOME", "/x")]);
        assert_eq!(
            resolve_root_with(&xdg, || None).0,
            PathBuf::from("/x/herdr/plugins/config/herdr-tasks")
        );
    }

    #[test]
    fn register_is_idempotent() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::at(d.path());
        let p = s.project_for_key(Path::new("/some/proj"));
        s.register(&p).unwrap();
        s.register(&p).unwrap();
        let reg = s.registry().unwrap();
        assert_eq!(reg.len(), 1);
        assert_eq!(reg["/some/proj"], p.folder);
    }
}
