//! TUI state and every action (keys and mouse call the same methods).

use super::model::{self, GroupKey, Grouping, ItemRef, ProjectInput, Row, RowId};
use super::mouse::HitMap;
use crate::cli::Surface;
use crate::config::Config;
use crate::herdr::Discovery;
use crate::store::task::{self, NewTask, Priority, Task, TaskEntry, TaskPatch};
use crate::store::{Project, Store};
use crate::sync::github::Issue;
use crate::sync::{self, RunOptions, Selection, Session, SyncReport};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

/// Opens an attachment with `open_command` (or xdg-open / open).
pub trait Opener {
    fn open(&mut self, command: &[String], path: &Path) -> Result<(), String>;
}

pub struct SystemOpener;

impl Opener for SystemOpener {
    fn open(&mut self, command: &[String], path: &Path) -> Result<(), String> {
        let default = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let (prog, args) = match command.split_first() {
            Some((p, rest)) => (p.clone(), rest.to_vec()),
            None => (default.to_string(), Vec::new()),
        };
        let mut child = crate::store::fsutil::command(&prog)
            .args(&args)
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("{prog}: {e}"))?;
        std::thread::spawn(move || child.wait());
        Ok(())
    }
}

pub type DiscoverFn = Box<dyn FnMut() -> Option<Discovery>>;

pub struct Deps {
    pub store: Store,
    pub surface: Surface,
    /// `None` = herdr unavailable (show all registered projects with tasks).
    pub discover: DiscoverFn,
    pub opener: Box<dyn Opener>,
}

pub struct ProjectView {
    pub project: Project,
    pub label: String,
    pub cfg: Config,
    pub cfg_errors: Vec<String>,
    pub entries: Vec<TaskEntry>,
    pub is_space: bool,
}

impl ProjectView {
    pub fn task(&self, id: &str) -> Option<&Task> {
        self.entries
            .iter()
            .find(|e| e.id() == id)
            .and_then(TaskEntry::task)
    }
}

#[derive(Debug, Clone, Default)]
pub struct TitlePrompt {
    pub folder: String,
    pub label: String,
    pub buffer: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewIssue {
    pub number: u64,
    pub title: String,
    pub labels: Vec<String>,
}

impl From<&Issue> for ReviewIssue {
    fn from(i: &Issue) -> Self {
        ReviewIssue {
            number: i.number,
            title: i.title.clone(),
            labels: i.labels.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReviewLine {
    Candidate(usize),
    IgnoredHeader,
    Ignored(usize),
}

#[derive(Debug, Clone)]
pub struct ImportReview {
    pub label: String,
    pub candidates: Vec<(ReviewIssue, bool)>,
    pub ignored: Vec<ReviewIssue>,
    pub ignored_open: bool,
    pub cursor: usize,
}

impl ImportReview {
    pub fn lines(&self) -> Vec<ReviewLine> {
        let mut v: Vec<ReviewLine> = (0..self.candidates.len())
            .map(ReviewLine::Candidate)
            .collect();
        if !self.ignored.is_empty() {
            v.push(ReviewLine::IgnoredHeader);
            if self.ignored_open {
                v.extend((0..self.ignored.len()).map(ReviewLine::Ignored));
            }
        }
        v
    }
}

pub enum SyncMsg {
    Review {
        candidates: Vec<Issue>,
        ignored: Vec<Issue>,
    },
    Done(Result<SyncReport, String>),
}

pub enum ReviewDecision {
    Import { import: Vec<u64>, ignore: Vec<u64> },
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chip {
    Status,
    Priority,
    Pin,
    Details,
}

pub struct App {
    pub store: Store,
    pub surface: Surface,
    pub grouping: Grouping,
    pub collapsed: HashSet<GroupKey>,
    pub projects: Vec<ProjectView>,
    pub global_cfg: Config,
    pub global_errors: Vec<String>,
    pub herdr_ok: bool,
    pub focused_key: Option<PathBuf>,
    pub rows: Vec<Row>,
    pub cursor: usize,
    pub list_offset: usize,
    pub detail_open: bool,
    pub details_expanded: bool,
    pub attachment_cursor: usize,
    pub detail_scroll: u16,
    pub prompt: Option<TitlePrompt>,
    pub review: Option<ImportReview>,
    pub help: bool,
    pub status: Option<(String, bool)>,
    pub hits: HitMap,
    pub width: u16,
    pub height: u16,
    pub watch_mode: &'static str,
    pub should_quit: bool,
    pub syncing: bool,
    pub clipboard: Option<String>,
    pub pending_g: bool,
    /// Rows visible in the list viewport (set by the renderer).
    pub list_height: usize,
    sync_rx: Option<Receiver<SyncMsg>>,
    sync_reply: Option<Sender<ReviewDecision>>,
    discover: DiscoverFn,
    opener: Box<dyn Opener>,
}

impl App {
    pub fn new(deps: Deps) -> App {
        let (global_cfg, global_errors) = Config::load_soft(&deps.store.root, None);
        let mut app = App {
            store: deps.store,
            surface: deps.surface,
            grouping: Grouping::Priority,
            collapsed: HashSet::new(),
            projects: Vec::new(),
            global_cfg,
            global_errors,
            herdr_ok: true,
            focused_key: None,
            rows: Vec::new(),
            cursor: 0,
            list_offset: 0,
            detail_open: false,
            details_expanded: false,
            attachment_cursor: 0,
            detail_scroll: 0,
            prompt: None,
            review: None,
            help: false,
            status: None,
            hits: HitMap::default(),
            width: 0,
            height: 0,
            watch_mode: "inotify",
            should_quit: false,
            syncing: false,
            clipboard: None,
            pending_g: false,
            list_height: 10,
            sync_rx: None,
            sync_reply: None,
            discover: deps.discover,
            opener: deps.opener,
        };
        app.refresh_projects();
        app
    }

    pub fn wide(&self) -> bool {
        self.surface == Surface::Board || self.width >= self.global_cfg.kanban_min_width
    }

    pub fn kanban(&self) -> bool {
        self.wide() && self.grouping == Grouping::Status
    }

    pub fn banners(&self) -> Vec<String> {
        let mut v = self.global_errors.clone();
        for p in &self.projects {
            v.extend(p.cfg_errors.iter().cloned());
        }
        if !self.herdr_ok {
            v.push("herdr unavailable: showing all projects".into());
        }
        v
    }

    // ---- data ----------------------------------------------------------

    fn load_view(&self, project: Project, label: String, is_space: bool) -> ProjectView {
        let (cfg, cfg_errors) = Config::load_soft(&self.store.root, Some(&project.folder));
        let entries = load_entries(&project, &cfg);
        ProjectView {
            project,
            label,
            cfg,
            cfg_errors,
            entries,
            is_space,
        }
    }

    /// Re-run discovery and rebuild the project list, reusing loaded tasks.
    pub fn refresh_projects(&mut self) {
        let (cfg, errs) = Config::load_soft(&self.store.root, None);
        self.global_cfg = cfg;
        self.global_errors = errs;
        let registry = self.store.registry().unwrap_or_default();
        let has_tasks = |p: &Project| {
            std::fs::read_dir(p.tasks_dir())
                .map(|mut rd| rd.next().is_some())
                .unwrap_or(false)
        };
        let mut wanted: Vec<(Project, String, bool)> = Vec::new();
        match (self.discover)() {
            Some(d) => {
                self.herdr_ok = true;
                self.focused_key = d.focused_key.clone();
                for s in &d.spaces {
                    wanted.push((self.store.project_for_key(&s.key), s.label.clone(), true));
                }
                for k in &d.other_keys {
                    if registry.contains_key(&k.to_string_lossy().into_owned()) {
                        let p = self.store.project_for_key(k);
                        if has_tasks(&p) {
                            let label = p.basename();
                            wanted.push((p, label, false));
                        }
                    }
                }
            }
            None => {
                self.herdr_ok = false;
                for p in self.store.registered_projects().unwrap_or_default() {
                    if has_tasks(&p) {
                        let label = p.basename();
                        wanted.push((p, label, false));
                    }
                }
            }
        }
        let mut old: Vec<ProjectView> = std::mem::take(&mut self.projects);
        let mut next = Vec::new();
        for (project, label, is_space) in wanted {
            if next
                .iter()
                .any(|v: &ProjectView| v.project.folder == project.folder)
            {
                continue;
            }
            match old.iter().position(|v| v.project.folder == project.folder) {
                Some(i) => {
                    let mut v = old.swap_remove(i);
                    v.label = label;
                    v.is_space = is_space;
                    next.push(v);
                }
                None => next.push(self.load_view(project, label, is_space)),
            }
        }
        next.sort_by_key(|v| (v.label.to_lowercase(), v.project.folder.clone()));
        self.projects = next;
        self.rebuild();
    }

    pub fn reload_project(&mut self, folder: &str) -> bool {
        let Some(i) = self
            .projects
            .iter()
            .position(|p| p.project.folder == folder)
        else {
            return false;
        };
        let (project, label, is_space) = {
            let p = &self.projects[i];
            (p.project.clone(), p.label.clone(), p.is_space)
        };
        self.projects[i] = self.load_view(project, label, is_space);
        self.rebuild();
        true
    }

    pub fn reload_all(&mut self) {
        let folders: Vec<String> = self
            .projects
            .iter()
            .map(|p| p.project.folder.clone())
            .collect();
        for f in folders {
            self.reload_project(&f);
        }
        self.refresh_projects();
    }

    pub fn statuses(&self) -> Vec<String> {
        let mut v = self.global_cfg.statuses.clone();
        for p in &self.projects {
            for s in &p.cfg.statuses {
                if !v.contains(s) {
                    v.push(s.clone());
                }
            }
        }
        v
    }

    pub fn groups(&self) -> Vec<model::Group> {
        let inputs: Vec<ProjectInput> = self
            .projects
            .iter()
            .map(|p| ProjectInput {
                folder: &p.project.folder,
                label: &p.label,
                entries: &p.entries,
                is_space: p.is_space,
            })
            .collect();
        model::build_groups(&inputs, self.grouping, &self.statuses(), self.kanban())
    }

    /// Recompute rows, keeping the cursor on the same row id when possible.
    pub fn rebuild(&mut self) {
        let keep = self.rows.get(self.cursor).map(Row::id);
        self.rows = model::flatten(&self.groups(), &self.collapsed);
        if let Some(id) = keep {
            if let Some(i) = self.rows.iter().position(|r| r.id() == id) {
                self.cursor = i;
            }
        }
        if self.cursor >= self.rows.len() {
            self.cursor = self.rows.len().saturating_sub(1);
        }
        if let Some(t) = self.selected_task() {
            let n = t.attachments.len();
            if self.attachment_cursor >= n {
                self.attachment_cursor = n.saturating_sub(1);
            }
        }
    }

    pub fn project(&self, folder: &str) -> Option<&ProjectView> {
        self.projects.iter().find(|p| p.project.folder == folder)
    }

    pub fn entry(&self, r: &ItemRef) -> Option<&TaskEntry> {
        self.project(&r.folder)?
            .entries
            .iter()
            .find(|e| e.id() == r.id)
    }

    pub fn selected_ref(&self) -> Option<&ItemRef> {
        match self.rows.get(self.cursor)? {
            Row::Task(r) => Some(r),
            Row::Header { .. } => None,
        }
    }

    pub fn selected_task(&self) -> Option<&Task> {
        let r = self.selected_ref()?;
        self.project(&r.folder)?.task(&r.id)
    }

    // ---- navigation ----------------------------------------------------

    pub fn move_cursor(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let max = self.rows.len() as isize - 1;
        let next = (self.cursor as isize + delta).clamp(0, max) as usize;
        self.select_row(next);
    }

    pub fn select_row(&mut self, i: usize) {
        if i < self.rows.len() && i != self.cursor {
            self.cursor = i;
            self.attachment_cursor = 0;
            self.detail_scroll = 0;
        }
    }

    pub fn goto_top(&mut self) {
        self.select_row(0);
    }

    pub fn goto_bottom(&mut self) {
        self.select_row(self.rows.len().saturating_sub(1));
    }

    pub fn half_page(&mut self, down: bool) {
        let n = (self.list_height / 2).max(1) as isize;
        self.move_cursor(if down { n } else { -n });
    }

    pub fn set_grouping(&mut self, g: Grouping) {
        if self.grouping != g {
            self.grouping = g;
            self.rebuild();
        }
    }

    pub fn cycle_grouping(&mut self, delta: i32) {
        self.set_grouping(self.grouping.cycle(delta));
    }

    /// The group key of the cursor row (its header or the group it is in).
    pub fn current_group(&self) -> Option<GroupKey> {
        self.rows[..=self.cursor.min(self.rows.len().checked_sub(1)?)]
            .iter()
            .rev()
            .find_map(|r| match r {
                Row::Header { key, .. } => Some(key.clone()),
                Row::Task(_) => None,
            })
    }

    pub fn set_collapsed(&mut self, key: GroupKey, collapsed: bool) {
        if collapsed {
            self.collapsed.insert(key.clone());
        } else {
            self.collapsed.remove(&key);
        }
        self.rebuild();
        if collapsed {
            if let Some(i) = self
                .rows
                .iter()
                .position(|r| r.id() == RowId::Header(key.clone()))
            {
                self.cursor = i;
            }
        }
    }

    pub fn toggle_group(&mut self, key: GroupKey) {
        let c = self.collapsed.contains(&key);
        self.set_collapsed(key, !c);
    }

    pub fn collapse_current(&mut self, collapsed: bool) {
        if let Some(k) = self.current_group() {
            self.set_collapsed(k, collapsed);
        }
    }

    /// `C`: collapse every group, or expand all if any is collapsed.
    pub fn toggle_all(&mut self) {
        if self.collapsed.is_empty() {
            self.collapsed = self.groups().into_iter().map(|g| g.key).collect();
        } else {
            self.collapsed.clear();
        }
        self.rebuild();
    }

    // ---- detail --------------------------------------------------------

    pub fn open_detail(&mut self) {
        if self.selected_task().is_some() {
            self.detail_open = true;
        }
    }

    pub fn close_detail(&mut self) {
        self.detail_open = false;
        self.details_expanded = false;
        self.detail_scroll = 0;
    }

    /// Enter: header -> toggle collapse; task -> open detail, or open the
    /// attachment under the cursor when the detail is already open.
    pub fn activate(&mut self) {
        match self.rows.get(self.cursor).cloned() {
            Some(Row::Header { key, .. }) => self.toggle_group(key),
            Some(Row::Task(_)) => {
                if !self.detail_open {
                    self.open_detail();
                } else if self
                    .selected_task()
                    .is_some_and(|t| !t.attachments.is_empty())
                {
                    self.open_attachment(None);
                }
            }
            None => {}
        }
    }

    pub fn toggle_details(&mut self) {
        if self.selected_task().is_some() {
            self.detail_open = true;
            self.details_expanded = !self.details_expanded;
        }
    }

    pub fn move_attachment(&mut self, delta: isize) {
        let n = self.selected_task().map_or(0, |t| t.attachments.len());
        if n == 0 {
            return;
        }
        self.attachment_cursor =
            (self.attachment_cursor as isize + delta).clamp(0, n as isize - 1) as usize;
    }

    pub fn open_attachment(&mut self, index: Option<usize>) {
        let Some(r) = self.selected_ref().cloned() else {
            return;
        };
        let Some(pv) = self.project(&r.folder) else {
            return;
        };
        let Some(t) = pv.task(&r.id) else { return };
        let i = index.unwrap_or(self.attachment_cursor);
        let Some(a) = t.attachments.get(i) else {
            self.flash("no attachment", true);
            return;
        };
        let (cmd, path, name) = (pv.cfg.open_command.clone(), a.path.clone(), a.name.clone());
        self.attachment_cursor = i;
        match self.opener.open(&cmd, &path) {
            Ok(()) => self.flash(&format!("opened {name}"), false),
            Err(e) => self.flash(&format!("open failed: {e}"), true),
        }
    }

    // ---- edits ---------------------------------------------------------

    pub fn flash(&mut self, msg: &str, error: bool) {
        self.status = Some((msg.to_string(), error));
    }

    fn edit(&mut self, f: impl FnOnce(&ProjectView, &Task) -> TaskPatch) {
        let Some(r) = self.selected_ref().cloned() else {
            return;
        };
        let Some(pv) = self.project(&r.folder) else {
            return;
        };
        let Some(t) = pv.task(&r.id) else {
            self.flash(&format!("{} is broken; fix task.toml first", r.id), true);
            return;
        };
        let patch = f(pv, t);
        let res = task::update_task(&pv.project, &pv.cfg, &r.id, patch);
        match res {
            Ok(t) => {
                let msg = format!(
                    "{} {} {} {}",
                    t.meta.id,
                    t.meta.status,
                    t.meta.priority.as_str(),
                    if t.meta.pinned { "pinned" } else { "" }
                );
                self.reload_project(&r.folder);
                self.flash(msg.trim_end(), false);
            }
            Err(e) => self.flash(&e.to_string(), true),
        }
    }

    pub fn cycle_status(&mut self, delta: isize) {
        self.edit(|pv, t| {
            let st = &pv.cfg.statuses;
            let next = match st.iter().position(|s| s == &t.meta.status) {
                Some(i) => st[(i as isize + delta).rem_euclid(st.len() as isize) as usize].clone(),
                None => st[0].clone(),
            };
            TaskPatch {
                status: Some(next),
                ..Default::default()
            }
        });
    }

    pub fn cycle_priority(&mut self) {
        self.edit(|_, t| TaskPatch {
            priority: Some(t.meta.priority.next()),
            ..Default::default()
        });
    }

    pub fn toggle_pin(&mut self) {
        self.edit(|_, t| TaskPatch {
            pinned: Some(!t.meta.pinned),
            ..Default::default()
        });
    }

    pub fn chip(&mut self, c: Chip) {
        match c {
            Chip::Status => self.cycle_status(1),
            Chip::Priority => self.cycle_priority(),
            Chip::Pin => self.toggle_pin(),
            Chip::Details => self.toggle_details(),
        }
    }

    pub fn yank(&mut self) {
        if let Some(r) = self.selected_ref().cloned() {
            self.clipboard = Some(r.id.clone());
            self.flash(&format!("copied {}", r.id), false);
        }
    }

    // ---- create --------------------------------------------------------

    /// Target project for `n`: Project grouping -> group under cursor; else
    /// the selected task's project; else the focused space; else the first.
    pub fn create_target(&self) -> Option<&ProjectView> {
        if self.grouping == Grouping::Project {
            if let Some(GroupKey::Project(f)) = self.current_group() {
                return self.project(&f);
            }
        }
        if let Some(r) = self.selected_ref() {
            return self.project(&r.folder);
        }
        if let Some(k) = &self.focused_key {
            if let Some(p) = self.projects.iter().find(|p| &p.project.key == k) {
                return Some(p);
            }
        }
        self.projects.first()
    }

    pub fn start_prompt(&mut self) {
        match self.create_target() {
            Some(p) => {
                self.prompt = Some(TitlePrompt {
                    folder: p.project.folder.clone(),
                    label: p.label.clone(),
                    ..Default::default()
                })
            }
            None => self.flash("no project: open a space in herdr first", true),
        }
    }

    pub fn prompt_submit(&mut self) {
        let Some(pr) = self.prompt.clone() else {
            return;
        };
        if pr.buffer.trim().is_empty() {
            if let Some(p) = self.prompt.as_mut() {
                p.error = Some("title must not be empty".into());
            }
            return;
        }
        let Some(pv) = self.project(&pr.folder) else {
            self.prompt = None;
            return;
        };
        let res = task::create_task(
            &self.store,
            &pv.project,
            &pv.cfg,
            NewTask {
                title: pr.buffer.clone(),
                priority: Some(Priority::P1),
                ..Default::default()
            },
        );
        match res {
            Ok(t) => {
                self.prompt = None;
                self.reload_project(&pr.folder);
                let want = RowId::Task(ItemRef {
                    folder: pr.folder.clone(),
                    id: t.meta.id.clone(),
                });
                if !self.rows.iter().any(|r| r.id() == want) {
                    // The new task may sit in a collapsed group: expand it.
                    for g in self.groups() {
                        if g.items
                            .iter()
                            .any(|i| i.folder == pr.folder && i.id == t.meta.id)
                        {
                            self.collapsed.remove(&g.key);
                        }
                    }
                    self.rebuild();
                }
                if let Some(i) = self.rows.iter().position(|r| r.id() == want) {
                    self.select_row(i);
                }
                self.flash(&format!("created {} in {}", t.meta.id, pr.label), false);
            }
            Err(e) => {
                if let Some(p) = self.prompt.as_mut() {
                    p.error = Some(e.to_string());
                }
            }
        }
    }

    pub fn prompt_cancel(&mut self) {
        self.prompt = None;
    }

    // ---- sync ----------------------------------------------------------

    /// Project for `R`: selected task, else group under cursor, else focused.
    fn sync_target(&self) -> Option<&ProjectView> {
        if let Some(r) = self.selected_ref() {
            return self.project(&r.folder);
        }
        if let Some(GroupKey::Project(f)) = self.current_group() {
            return self.project(&f);
        }
        self.create_target()
    }

    pub fn start_sync(&mut self) {
        if self.syncing {
            return;
        }
        let Some(pv) = self.sync_target() else {
            self.flash("no project to sync", true);
            return;
        };
        if pv.cfg.github.mode == crate::config::SyncMode::Off {
            let path = crate::config::project_path(&self.store.root, &pv.project.folder);
            let msg = format!(
                "GitHub sync is off for {} (set github.mode in {})",
                pv.label,
                path.display()
            );
            self.flash(&msg, true);
            return;
        }
        let (store, project, cfg, label) = (
            self.store.clone(),
            pv.project.clone(),
            pv.cfg.clone(),
            pv.label.clone(),
        );
        let (tx, rx) = channel();
        let (reply_tx, reply_rx) = channel();
        self.sync_rx = Some(rx);
        self.sync_reply = Some(reply_tx);
        self.syncing = true;
        self.flash(&format!("syncing {label}…"), false);
        std::thread::spawn(move || {
            let res = sync_worker(&store, &project, &cfg, &tx, &reply_rx);
            let _ = tx.send(SyncMsg::Done(res.map_err(|e| e.to_string())));
        });
    }

    /// Called every tick: handle worker messages.
    pub fn poll_sync(&mut self) {
        let Some(rx) = &self.sync_rx else { return };
        let mut msgs = Vec::new();
        while let Ok(m) = rx.try_recv() {
            msgs.push(m);
        }
        for m in msgs {
            match m {
                SyncMsg::Review {
                    candidates,
                    ignored,
                } => {
                    let label = self
                        .sync_target()
                        .map(|p| p.label.clone())
                        .unwrap_or_default();
                    self.review = Some(ImportReview {
                        label,
                        candidates: candidates
                            .iter()
                            .map(|i| (ReviewIssue::from(i), true))
                            .collect(),
                        ignored: ignored.iter().map(ReviewIssue::from).collect(),
                        ignored_open: false,
                        cursor: 0,
                    });
                }
                SyncMsg::Done(res) => {
                    self.syncing = false;
                    self.sync_rx = None;
                    self.sync_reply = None;
                    self.review = None;
                    match res {
                        Ok(r) => self.flash(
                            &crate::cli::commands::summary(&r).replace('\n', "; "),
                            false,
                        ),
                        Err(e) => self.flash(&format!("sync failed: {e}"), true),
                    }
                }
            }
        }
    }

    /// Show a review without a worker (tests and previews).
    pub fn open_review(&mut self, review: ImportReview) {
        self.review = Some(review);
    }

    pub fn review_move(&mut self, delta: isize) {
        if let Some(r) = self.review.as_mut() {
            let n = r.lines().len() as isize;
            if n > 0 {
                r.cursor = (r.cursor as isize + delta).clamp(0, n - 1) as usize;
            }
        }
    }

    pub fn review_toggle(&mut self, line: Option<usize>) {
        let Some(r) = self.review.as_mut() else {
            return;
        };
        let lines = r.lines();
        let i = line.unwrap_or(r.cursor);
        r.cursor = i.min(lines.len().saturating_sub(1));
        match lines.get(i) {
            Some(ReviewLine::Candidate(c)) => r.candidates[*c].1 = !r.candidates[*c].1,
            Some(ReviewLine::IgnoredHeader) => r.ignored_open = !r.ignored_open,
            _ => {}
        }
    }

    pub fn review_toggle_all(&mut self) {
        if let Some(r) = self.review.as_mut() {
            let all = r.candidates.iter().all(|c| c.1);
            for c in &mut r.candidates {
                c.1 = !all;
            }
        }
    }

    /// `u` on an ignored issue: offer it again (checked).
    pub fn review_unignore(&mut self) {
        let Some(r) = self.review.as_mut() else {
            return;
        };
        if let Some(ReviewLine::Ignored(i)) = r.lines().get(r.cursor).cloned() {
            let issue = r.ignored.remove(i);
            r.candidates.push((issue, true));
            r.cursor = r.candidates.len() - 1;
        }
    }

    pub fn review_finish(&mut self, import: bool) {
        let Some(r) = self.review.take() else { return };
        let decision = if import {
            ReviewDecision::Import {
                import: r
                    .candidates
                    .iter()
                    .filter(|c| c.1)
                    .map(|c| c.0.number)
                    .collect(),
                ignore: r
                    .candidates
                    .iter()
                    .filter(|c| !c.1)
                    .map(|c| c.0.number)
                    .collect(),
            }
        } else {
            ReviewDecision::Cancel
        };
        if let Some(tx) = &self.sync_reply {
            let _ = tx.send(decision);
        } else {
            self.apply_review_offline(decision);
        }
    }

    /// Without a worker (tests): only the ignore list is written.
    fn apply_review_offline(&mut self, d: ReviewDecision) {
        if let (ReviewDecision::Import { ignore, .. }, Some(pv)) = (d, self.sync_target()) {
            let project = pv.project.clone();
            if let Err(e) = sync::state::edit(&project, &ignore, &[]) {
                self.flash(&e.to_string(), true);
            }
        }
    }
}

/// Load tasks; if any is broken (maybe half-written), retry once after 100 ms.
fn load_entries(project: &Project, cfg: &Config) -> Vec<TaskEntry> {
    let e = task::load_tasks(project, cfg);
    let fresh = |p: &Path| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .map(|t| t.elapsed().map_or(true, |d| d < Duration::from_secs(2)))
            .unwrap_or(true)
    };
    if e.iter()
        .any(|x| matches!(x, TaskEntry::Broken(b) if fresh(&b.path)))
    {
        std::thread::sleep(Duration::from_millis(100));
        return task::load_tasks(project, cfg);
    }
    e
}

fn sync_worker(
    store: &Store,
    project: &Project,
    cfg: &Config,
    tx: &Sender<SyncMsg>,
    reply: &Receiver<ReviewDecision>,
) -> crate::error::Result<SyncReport> {
    let session = Session::connect(project, cfg, &store.root)?;
    let fetched = sync::fetch(&session, project, cfg)?;
    let mut opts = RunOptions {
        selection: Selection::Only(Vec::new()),
        ignore_add: Vec::new(),
        dry_run: false,
        imports_only: false,
    };
    if cfg.github.mode.pulls() {
        let ignored_nums = sync::state::load(project)?.ignored_issues;
        let candidates = fetched.candidates(&ignored_nums);
        let linked = fetched.linked_numbers();
        let ignored: Vec<Issue> = fetched
            .open
            .iter()
            .filter(|i| ignored_nums.contains(&i.number) && !linked.contains(&i.number))
            .cloned()
            .collect();
        if !candidates.is_empty() {
            let _ = tx.send(SyncMsg::Review {
                candidates,
                ignored,
            });
            match reply.recv() {
                Ok(ReviewDecision::Import { import, ignore }) => {
                    opts.selection = Selection::Only(import);
                    opts.ignore_add = ignore;
                }
                Ok(ReviewDecision::Cancel) | Err(_) => {}
            }
        }
    }
    sync::run_fetched(store, project, cfg, &session, fetched, &opts)
}
