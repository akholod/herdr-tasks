//! Shared helpers for TUI tests.
#![allow(dead_code)]

pub mod cli_env;
pub mod http_stub;

use herdr_tasks::cli::Surface;
use herdr_tasks::config::Config;
use herdr_tasks::herdr::{Discovery, SpaceProject};
use herdr_tasks::store::task::{self, NewTask, Priority};
use herdr_tasks::store::{Project, Store};
use herdr_tasks::tui::app::{App, Deps, Opener};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub struct Env {
    pub _dir: tempfile::TempDir,
    pub store: Store,
    pub spaces: Vec<(PathBuf, String)>,
}

impl Env {
    /// A store plus one canonical directory per space label.
    pub fn new(labels: &[&str]) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path().join("store"));
        store.ensure().unwrap();
        let spaces = labels
            .iter()
            .map(|l| {
                let p = dir.path().join("work").join(l);
                std::fs::create_dir_all(&p).unwrap();
                (std::fs::canonicalize(&p).unwrap(), l.to_string())
            })
            .collect();
        Env {
            _dir: dir,
            store,
            spaces,
        }
    }

    pub fn project(&self, i: usize) -> Project {
        self.store.project_for_key(&self.spaces[i].0)
    }

    pub fn create(
        &self,
        i: usize,
        title: &str,
        prio: Priority,
        status: &str,
        pinned: bool,
    ) -> task::Task {
        let p = self.project(i);
        let cfg = Config::load(&self.store.root, Some(&p.folder)).unwrap();
        task::create_task(
            &self.store,
            &p,
            &cfg,
            NewTask {
                title: title.into(),
                priority: Some(prio),
                status: Some(status.into()),
                pinned,
                description: format!("About {title}."),
                ..Default::default()
            },
        )
        .unwrap()
    }

    pub fn discovery(&self) -> Discovery {
        Discovery {
            spaces: self
                .spaces
                .iter()
                .enumerate()
                .map(|(i, (k, l))| SpaceProject {
                    key: k.clone(),
                    label: l.clone(),
                    number: i as u64 + 1,
                    workspace_ids: vec![format!("w{i}")],
                })
                .collect(),
            other_keys: vec![],
            focused_key: self.spaces.first().map(|s| s.0.clone()),
        }
    }

    pub fn app(&self, surface: Surface) -> (App, Opened) {
        let d = self.discovery();
        let opened: Opened = Arc::new(Mutex::new(Vec::new()));
        let app = App::new(Deps {
            store: self.store.clone(),
            surface,
            discover: Box::new(move || Some(d.clone())),
            opener: Box::new(Recorder(opened.clone())),
        });
        (app, opened)
    }
}

pub type Opened = Arc<Mutex<Vec<(Vec<String>, PathBuf)>>>;

struct Recorder(Opened);

impl Opener for Recorder {
    fn open(&mut self, command: &[String], path: &Path) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .push((command.to_vec(), path.to_path_buf()));
        Ok(())
    }
}

/// Render and return the screen as text (one line per row, right-trimmed).
pub fn draw(app: &mut App, w: u16, h: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| herdr_tasks::tui::view::render(f, app)).unwrap();
    let buf = t.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..h {
        let mut line = String::new();
        for x in 0..w {
            line.push_str(buf[(x, y)].symbol());
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}
