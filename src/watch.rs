//! Store watcher: `notify` recursive on `<root>/projects` plus `<root>`
//! non-recursive, debounced 150 ms; falls back to a 500 ms `PollWatcher`.

use crate::store::{fsutil, lock::LOCK_FILE};
use notify::{RecursiveMode, Watcher};
use notify_debouncer_mini::{new_debouncer_opt, Config, DebounceEventResult, Debouncer};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// Something under `projects/<folder>/` changed.
    Project(String),
    /// `projects.toml` or the global `config.toml` changed.
    Root,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchMode {
    Native,
    Poll,
}

impl WatchMode {
    pub fn as_str(self) -> &'static str {
        match self {
            WatchMode::Native => "inotify",
            WatchMode::Poll => "poll",
        }
    }
}

/// Held only to keep the debouncer (and its watcher thread) alive.
#[allow(dead_code)]
enum Inner {
    Native(Debouncer<notify::RecommendedWatcher>),
    Poll(Debouncer<notify::PollWatcher>),
}

pub struct StoreWatcher {
    pub rx: Receiver<WatchEvent>,
    pub mode: WatchMode,
    /// Set when the native watcher reported an error (switch to poll).
    pub failed: Receiver<()>,
    _inner: Inner,
}

/// Map a changed path to an event; `None` for temp files, locks and noise.
pub fn classify(root: &Path, path: &Path) -> Option<WatchEvent> {
    let name = path.file_name()?.to_string_lossy();
    if fsutil::is_tmp_name(&name) || name == LOCK_FILE {
        return None;
    }
    let rel = path.strip_prefix(root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    match parts.as_slice() {
        [f] if f == "projects.toml" || f == "config.toml" => Some(WatchEvent::Root),
        [p, folder, ..] if p == "projects" => Some(WatchEvent::Project(folder.clone())),
        _ => None,
    }
}

fn handler(
    root: PathBuf,
    tx: Sender<WatchEvent>,
    fail: Sender<()>,
) -> impl FnMut(DebounceEventResult) {
    move |res: DebounceEventResult| match res {
        Ok(events) => {
            let mut sent: Vec<WatchEvent> = Vec::new();
            for e in events {
                if let Some(ev) = classify(&root, &e.path) {
                    if !sent.contains(&ev) {
                        let _ = tx.send(ev.clone());
                        sent.push(ev);
                    }
                }
            }
        }
        Err(_) => {
            let _ = fail.send(());
        }
    }
}

fn add_watches(w: &mut dyn Watcher, root: &Path) -> notify::Result<()> {
    w.watch(&root.join("projects"), RecursiveMode::Recursive)?;
    w.watch(root, RecursiveMode::NonRecursive)
}

/// Start watching `root` (creates `<root>/projects` first). Uses the native
/// watcher unless `force_poll` or it fails to start.
pub fn start(root: &Path, force_poll: bool) -> std::io::Result<StoreWatcher> {
    std::fs::create_dir_all(root.join("projects"))?;
    let (tx, rx) = channel();
    let (ftx, frx) = channel();
    let timeout = Duration::from_millis(150);
    if !force_poll {
        let cfg = Config::default().with_timeout(timeout);
        if let Ok(mut d) = new_debouncer_opt::<_, notify::RecommendedWatcher>(
            cfg,
            handler(root.into(), tx.clone(), ftx.clone()),
        ) {
            if add_watches(d.watcher(), root).is_ok() {
                return Ok(StoreWatcher {
                    rx,
                    mode: WatchMode::Native,
                    failed: frx,
                    _inner: Inner::Native(d),
                });
            }
        }
    }
    let cfg = Config::default().with_timeout(timeout).with_notify_config(
        notify::Config::default().with_poll_interval(Duration::from_millis(500)),
    );
    let mut d = new_debouncer_opt::<_, notify::PollWatcher>(cfg, handler(root.into(), tx, ftx))
        .map_err(std::io::Error::other)?;
    add_watches(d.watcher(), root).map_err(std::io::Error::other)?;
    Ok(StoreWatcher {
        rx,
        mode: WatchMode::Poll,
        failed: frx,
        _inner: Inner::Poll(d),
    })
}

impl StoreWatcher {
    /// Drain pending events (deduplicated).
    pub fn drain(&self) -> Vec<WatchEvent> {
        let mut out: Vec<WatchEvent> = Vec::new();
        while let Ok(e) = self.rx.try_recv() {
            if !out.contains(&e) {
                out.push(e);
            }
        }
        out
    }

    pub fn native_failed(&self) -> bool {
        self.mode == WatchMode::Native && self.failed.try_recv().is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_paths() {
        let r = Path::new("/r");
        assert_eq!(
            classify(r, Path::new("/r/projects/p-1/tasks/t-0001/task.toml")),
            Some(WatchEvent::Project("p-1".into()))
        );
        assert_eq!(
            classify(r, Path::new("/r/projects.toml")),
            Some(WatchEvent::Root)
        );
        assert_eq!(
            classify(r, Path::new("/r/config.toml")),
            Some(WatchEvent::Root)
        );
        assert_eq!(classify(r, Path::new("/r/projects/p-1/.lock")), None);
        assert_eq!(
            classify(r, Path::new("/r/projects/p-1/tasks/t-1/task.toml.tmp-1-2")),
            None
        );
        assert_eq!(classify(r, Path::new("/r/projects")), None);
    }
}
