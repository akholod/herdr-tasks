//! GitHub sync: explicit command only, per-task last-write-wins.
//!
//! Phase 1 (no lock): fetch issues, plan from a task snapshot (pure
//! `plan_sync`). Phase 2 (no lock): remote writes. Phase 3: per task, take the
//! project lock, re-read, skip into `conflicts` if `updated` moved, else write.

pub mod github;
pub mod state;

use crate::config::{Config, SyncMode};
use crate::error::{AppError, Result};
use crate::store::task::{self, GithubLink, NewTask, TaskEntry, TaskMeta};
use crate::store::{lock, Project, Store};
use chrono::{DateTime, Utc};
use github::{GitHubApi, Issue};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

/// Room for other trackers (Jira/Linear) later; GitHub is the only provider.
pub trait SyncProvider {
    fn name(&self) -> &'static str;
}

pub fn body_sha(body: &str) -> String {
    Sha256::digest(body.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Cut on a char boundary; an over-limit body ends with `…`.
pub fn truncate_chars(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_string();
    }
    let mut out: String = s.chars().take(limit.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn sorted(v: &[String]) -> Vec<String> {
    let mut v = v.to_vec();
    v.sort();
    v
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalUpdate {
    pub id: String,
    pub issue: Issue,
    pub title: Option<String>,
    pub status: Option<String>,
    pub tags: Option<Vec<String>>,
    /// Rewrite `description.md` from the body (body changed, no local edit).
    pub rewrite_description: bool,
    pub seen_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PushPlan {
    pub id: String,
    pub issue: u64,
    pub title: Option<String>,
    pub close: bool,
    pub reopen: bool,
    pub seen_updated: DateTime<Utc>,
}

impl PushPlan {
    fn actions(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.title.is_some() {
            v.push("edit");
        }
        if self.close {
            v.push("close");
        }
        if self.reopen {
            v.push("reopen");
        }
        v
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncPlan {
    pub imports: Vec<Issue>,
    pub local: Vec<LocalUpdate>,
    pub push: Vec<PushPlan>,
    pub conflicts: Vec<String>,
    pub skipped: Vec<String>,
}

impl SyncPlan {
    pub fn is_empty(&self) -> bool {
        self.imports.is_empty() && self.local.is_empty() && self.push.is_empty()
    }
}

/// Pure planner. `issues` holds the current state of every linked issue that
/// could be fetched; `imports` are the unlinked issues selected for import.
pub fn plan_sync(
    tasks: &[TaskMeta],
    issues: &BTreeMap<u64, Issue>,
    imports: Vec<Issue>,
    mode: SyncMode,
    statuses: &[String],
) -> SyncPlan {
    let mut plan = SyncPlan::default();
    let first = statuses.first().map(String::as_str).unwrap_or("todo");
    let done = statuses.last().map(String::as_str).unwrap_or("done");
    if mode.pulls() {
        plan.imports = imports;
    }
    for t in tasks {
        let Some(link) = &t.github else { continue };
        let Some(issue) = issues.get(&link.issue) else {
            plan.skipped
                .push(format!("{}: issue #{} not available", t.id, link.issue));
            continue;
        };
        let local_changed = link.synced_at.is_none_or(|s| t.updated > s);
        let task_done = t.status == done;
        let title_diff = issue.title != t.title;
        let state_diff = issue.closed != task_done;
        let labels_diff = sorted(&issue.labels) != sorted(&t.tags);
        let body_diff = link.body_sha.as_deref() != Some(body_sha(&issue.body).as_str());
        let remote_newer = link.remote_updated_at.is_none_or(|r| issue.updated_at > r);
        let remote_changed = remote_newer && (title_diff || state_diff || labels_diff || body_diff);

        if remote_changed && mode.pulls() {
            if local_changed {
                plan.conflicts.push(t.id.clone());
            }
            let status = if issue.closed && !task_done {
                Some(done.to_string())
            } else if !issue.closed && task_done {
                Some(first.to_string())
            } else {
                None
            };
            plan.local.push(LocalUpdate {
                id: t.id.clone(),
                issue: issue.clone(),
                title: title_diff.then(|| issue.title.clone()),
                status,
                tags: labels_diff.then(|| issue.labels.clone()),
                rewrite_description: body_diff && !local_changed,
                seen_updated: t.updated,
            });
        } else if local_changed && mode.pushes() {
            let p = PushPlan {
                id: t.id.clone(),
                issue: link.issue,
                title: title_diff.then(|| t.title.clone()),
                close: task_done && !issue.closed,
                reopen: !task_done && issue.closed,
                seen_updated: t.updated,
            };
            if !p.actions().is_empty() {
                plan.push.push(p);
            }
        }
    }
    plan
}

#[derive(Debug, Clone, Serialize)]
pub struct Pushed {
    pub id: String,
    pub issue: u64,
    pub action: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncReport {
    pub mode: String,
    pub repo: String,
    pub client: String,
    pub dry_run: bool,
    pub imported: Vec<String>,
    pub updated_local: Vec<String>,
    pub pushed: Vec<Pushed>,
    pub conflicts: Vec<String>,
    pub skipped: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// `(task id, issue number)` for each import.
    #[serde(skip)]
    pub imported_links: Vec<(String, u64)>,
}

pub struct Session {
    pub api: Box<dyn GitHubApi>,
    pub repo: String,
    pub warnings: Vec<String>,
}

impl Session {
    pub fn connect(project: &Project, cfg: &Config, root: &std::path::Path) -> Result<Session> {
        let config_path = crate::config::project_path(root, &project.folder);
        let client = github::select_client(&cfg.github, &|k| std::env::var(k).ok())?;
        let repo = github::resolve_repo(&cfg.github, &project.key, &config_path)?;
        Ok(Session {
            api: client.api,
            repo,
            warnings: client.warnings,
        })
    }
}

pub struct Fetched {
    pub open: Vec<Issue>,
    pub linked: BTreeMap<u64, Issue>,
    pub tasks: Vec<TaskMeta>,
    pub skipped: Vec<String>,
}

impl Fetched {
    /// Open issues not linked to any task and not ignored.
    pub fn candidates(&self, ignored: &[u64]) -> Vec<Issue> {
        let linked: HashSet<u64> = self
            .tasks
            .iter()
            .filter_map(|t| t.github.as_ref().map(|g| g.issue))
            .collect();
        self.open
            .iter()
            .filter(|i| !linked.contains(&i.number) && !ignored.contains(&i.number))
            .cloned()
            .collect()
    }

    pub fn linked_numbers(&self) -> HashSet<u64> {
        self.tasks
            .iter()
            .filter_map(|t| t.github.as_ref().map(|g| g.issue))
            .collect()
    }
}

pub fn task_snapshot(project: &Project, cfg: &Config) -> Vec<TaskMeta> {
    task::load_tasks(project, cfg)
        .into_iter()
        .filter_map(|e| match e {
            TaskEntry::Ok(t) => Some(t.meta),
            TaskEntry::Broken(_) => None,
        })
        .collect()
}

/// Phase 1: list open issues, then view linked issues missing from the list.
pub fn fetch(session: &Session, project: &Project, cfg: &Config) -> Result<Fetched> {
    let tasks = task_snapshot(project, cfg);
    let open = session
        .api
        .list_open_issues(&session.repo, cfg.github.import_limit)?;
    let mut linked = BTreeMap::new();
    let mut skipped = Vec::new();
    for t in &tasks {
        let Some(g) = &t.github else { continue };
        if let Some(i) = open.iter().find(|i| i.number == g.issue) {
            linked.insert(g.issue, i.clone());
            continue;
        }
        match session.api.view_issue(&session.repo, g.issue) {
            Ok(i) => {
                linked.insert(g.issue, i);
            }
            Err(e) => skipped.push(format!("{}: {e}", t.id)),
        }
    }
    Ok(Fetched {
        open,
        linked,
        tasks,
        skipped,
    })
}

/// Which unlinked issues to import.
#[derive(Debug, Clone)]
pub enum Selection {
    /// Every candidate not in `ignored_issues` (CLI `sync`).
    AllCandidates,
    /// Exactly these numbers (CLI `--import`, TUI review); removed from
    /// `ignored_issues`.
    Only(Vec<u64>),
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub selection: Selection,
    /// Issue numbers to add to `ignored_issues` (TUI review: unchecked).
    pub ignore_add: Vec<u64>,
    pub dry_run: bool,
    /// Only import; leave linked tasks alone (CLI `--import`).
    pub imports_only: bool,
}

impl RunOptions {
    pub fn all(dry_run: bool) -> RunOptions {
        RunOptions {
            selection: Selection::AllCandidates,
            ignore_add: Vec::new(),
            dry_run,
            imports_only: false,
        }
    }
}

/// Phases 1b-3 after a fetch: select imports, plan, apply, update ignores.
pub fn run_fetched(
    store: &Store,
    project: &Project,
    cfg: &Config,
    session: &Session,
    fetched: Fetched,
    opts: &RunOptions,
) -> Result<SyncReport> {
    let dry_run = opts.dry_run;
    let mode = cfg.github.mode;
    let mut report = SyncReport {
        mode: mode.as_str().into(),
        repo: session.repo.clone(),
        client: session.api.kind().into(),
        dry_run,
        warnings: session.warnings.clone(),
        skipped: fetched.skipped.clone(),
        ..Default::default()
    };
    let ignored = state::load(project)?.ignored_issues;
    let linked = fetched.linked_numbers();
    let mut unignore = Vec::new();
    let imports: Vec<Issue> = match &opts.selection {
        Selection::AllCandidates => fetched.candidates(&ignored),
        Selection::Only(numbers) => {
            let mut v = Vec::new();
            for n in numbers {
                if linked.contains(n) {
                    report.skipped.push(format!("#{n}: already linked"));
                    continue;
                }
                let issue = match fetched.open.iter().find(|i| i.number == *n) {
                    Some(i) => Ok(i.clone()),
                    None => session.api.view_issue(&session.repo, *n),
                };
                match issue {
                    Ok(i) => {
                        unignore.push(*n);
                        v.push(i);
                    }
                    Err(e) => report.skipped.push(format!("#{n}: {e}")),
                }
            }
            v
        }
    };
    let linked_tasks: &[TaskMeta] = if opts.imports_only {
        &[]
    } else {
        &fetched.tasks
    };
    let plan = plan_sync(linked_tasks, &fetched.linked, imports, mode, &cfg.statuses);
    report.conflicts = plan.conflicts.clone();
    report.skipped.extend(plan.skipped.iter().cloned());
    if dry_run {
        report.imported = plan
            .imports
            .iter()
            .map(|i| format!("#{}", i.number))
            .collect();
        report.updated_local = plan.local.iter().map(|u| u.id.clone()).collect();
        report.pushed = plan
            .push
            .iter()
            .flat_map(|p| {
                p.actions().into_iter().map(|a| Pushed {
                    id: p.id.clone(),
                    issue: p.issue,
                    action: a.into(),
                })
            })
            .collect();
        return Ok(report);
    }
    apply(store, project, cfg, session, &plan, &mut report)?;
    if !opts.ignore_add.is_empty() || !unignore.is_empty() {
        state::edit(project, &opts.ignore_add, &unignore)?;
    }
    Ok(report)
}

/// Phases 2 and 3.
fn apply(
    store: &Store,
    project: &Project,
    cfg: &Config,
    session: &Session,
    plan: &SyncPlan,
    report: &mut SyncReport,
) -> Result<()> {
    let api = session.api.as_ref();
    let repo = session.repo.as_str();
    // Phase 2: remote writes, no lock held.
    for p in &plan.push {
        let mut res = Ok(());
        if let Some(t) = &p.title {
            res = api.edit_title(repo, p.issue, t);
        }
        if res.is_ok() && p.close {
            res = api.close(repo, p.issue);
        }
        if res.is_ok() && p.reopen {
            res = api.reopen(repo, p.issue);
        }
        let remote = res.and_then(|_| api.view_issue(repo, p.issue));
        let remote = match remote {
            Ok(i) => i,
            Err(e) => {
                report.skipped.push(format!("{}: {e}", p.id));
                continue;
            }
        };
        for a in p.actions() {
            report.pushed.push(Pushed {
                id: p.id.clone(),
                issue: p.issue,
                action: a.into(),
            });
        }
        // Phase 3 for this task.
        let wrote = write_locked(project, &p.id, p.seen_updated, |meta| {
            let at = task::bump(meta.updated);
            if let Some(g) = meta.github.as_mut() {
                g.synced_at = Some(at);
                g.remote_updated_at = Some(remote.updated_at);
            }
            meta.updated = at;
            Ok(())
        })?;
        if !wrote {
            report.conflicts.push(p.id.clone());
        }
    }
    // Phase 3: pulled changes to linked tasks.
    for u in &plan.local {
        let dir = project.task_dir(&u.id);
        let wrote = write_locked(project, &u.id, u.seen_updated, |meta| {
            let at = task::bump(meta.updated);
            if let Some(t) = &u.title {
                meta.title = t.clone();
            }
            if let Some(s) = &u.status {
                meta.status = s.clone();
            }
            if let Some(tags) = &u.tags {
                meta.tags = tags.clone();
            }
            let sha = body_sha(&u.issue.body);
            if let Some(g) = meta.github.as_mut() {
                if u.rewrite_description {
                    task::write_text(
                        &dir,
                        task::DESCRIPTION_FILE,
                        &truncate_chars(&u.issue.body, cfg.description_limit),
                    )?;
                }
                g.body_sha = Some(sha);
                g.synced_at = Some(at);
                g.remote_updated_at = Some(u.issue.updated_at);
            }
            meta.updated = at;
            Ok(())
        })?;
        if wrote {
            report.updated_local.push(u.id.clone());
        } else if !report.conflicts.contains(&u.id) {
            report.conflicts.push(u.id.clone());
        }
    }
    // Imports: new tasks, `details.md` = full body (first and only time).
    for i in &plan.imports {
        let at = task::now();
        let new = NewTask {
            title: i.title.clone(),
            description: truncate_chars(&i.body, cfg.description_limit),
            details: i.body.clone(),
            tags: i.labels.clone(),
            status: Some(
                if i.closed {
                    cfg.done_status()
                } else {
                    cfg.first_status()
                }
                .to_string(),
            ),
            github: Some(GithubLink {
                issue: i.number,
                synced_at: Some(at),
                body_sha: Some(body_sha(&i.body)),
                remote_updated_at: Some(i.updated_at),
            }),
            at: Some(at),
            ..Default::default()
        };
        match task::create_task(store, project, cfg, new) {
            Ok(t) => {
                report.imported_links.push((t.meta.id.clone(), i.number));
                report.imported.push(t.meta.id);
            }
            Err(e) => report.skipped.push(format!("#{}: {e}", i.number)),
        }
    }
    Ok(())
}

/// Re-read under the lock; write only if `updated` still equals `seen`.
fn write_locked(
    project: &Project,
    id: &str,
    seen: DateTime<Utc>,
    f: impl FnOnce(&mut TaskMeta) -> Result<()>,
) -> Result<bool> {
    let dir = project.task_dir(id);
    lock::with_lock(&project.dir, || {
        let (mut meta, _) = task::read_meta(&dir).map_err(AppError::Config)?;
        if meta.updated != seen {
            return Ok(false);
        }
        f(&mut meta)?;
        task::write_meta(&dir, &meta)?;
        Ok(true)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::task::Priority;
    use chrono::Duration;

    fn ts(s: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000 + s, 0).unwrap()
    }

    fn statuses() -> Vec<String> {
        Config::default().statuses
    }

    fn issue(n: u64, title: &str, closed: bool, at: i64) -> Issue {
        Issue {
            number: n,
            title: title.into(),
            body: "body".into(),
            closed,
            labels: vec![],
            updated_at: ts(at),
        }
    }

    /// A task in sync with `issue(n, title, false, remote_at)` as of `synced`.
    fn linked(id: &str, n: u64, title: &str, synced: i64, remote_at: i64) -> TaskMeta {
        TaskMeta {
            id: id.into(),
            title: title.into(),
            status: "todo".into(),
            priority: Priority::P1,
            pinned: false,
            tags: vec![],
            created: ts(0),
            updated: ts(synced),
            github: Some(GithubLink {
                issue: n,
                synced_at: Some(ts(synced)),
                body_sha: Some(body_sha("body")),
                remote_updated_at: Some(ts(remote_at)),
            }),
        }
    }

    fn map(issues: Vec<Issue>) -> BTreeMap<u64, Issue> {
        issues.into_iter().map(|i| (i.number, i)).collect()
    }

    #[test]
    fn in_sync_is_empty_in_every_mode() {
        let t = vec![linked("t-0001", 1, "a", 100, 50)];
        let i = map(vec![issue(1, "a", false, 50)]);
        for m in [SyncMode::Pull, SyncMode::Push, SyncMode::TwoWay] {
            assert!(
                plan_sync(&t, &i, vec![], m, &statuses()).is_empty(),
                "{m:?}"
            );
        }
    }

    #[test]
    fn pull_never_pushes_push_never_writes_locally() {
        let mut t = linked("t-0001", 1, "local", 100, 50);
        t.updated = ts(200);
        let i = map(vec![issue(1, "remote", false, 60)]);
        let pull = plan_sync(
            std::slice::from_ref(&t),
            &i,
            vec![issue(2, "new", false, 1)],
            SyncMode::Pull,
            &statuses(),
        );
        assert!(pull.push.is_empty());
        assert_eq!(pull.local.len(), 1);
        assert_eq!(pull.imports.len(), 1);
        let push = plan_sync(
            &[t],
            &i,
            vec![issue(2, "new", false, 1)],
            SyncMode::Push,
            &statuses(),
        );
        assert!(push.local.is_empty());
        assert!(push.imports.is_empty());
        assert_eq!(push.push[0].title.as_deref(), Some("local"));
    }

    #[test]
    fn both_changed_remote_wins_and_conflict_listed() {
        let mut t = linked("t-0001", 1, "local", 100, 50);
        t.updated = ts(200);
        let i = map(vec![issue(1, "remote", false, 60)]);
        let p = plan_sync(&[t], &i, vec![], SyncMode::TwoWay, &statuses());
        assert!(p.push.is_empty());
        assert_eq!(p.local[0].title.as_deref(), Some("remote"));
        assert_eq!(p.conflicts, vec!["t-0001"]);
    }

    #[test]
    fn skewed_local_clock_does_not_matter() {
        // Local clock one hour ahead: synced_at/updated far beyond GitHub time.
        let t = linked("t-0001", 1, "a", 3600 + 100, 50);
        let i = map(vec![issue(1, "a", false, 50)]);
        assert!(plan_sync(
            std::slice::from_ref(&t),
            &i,
            vec![],
            SyncMode::TwoWay,
            &statuses()
        )
        .is_empty());
        let i2 = map(vec![issue(1, "b", false, 51)]);
        let p = plan_sync(&[t], &i2, vec![], SyncMode::TwoWay, &statuses());
        assert_eq!(p.local.len(), 1);
        assert!(p.conflicts.is_empty());
    }

    #[test]
    fn comment_only_change_is_ignored() {
        let t = linked("t-0001", 1, "a", 100, 50);
        let i = map(vec![issue(1, "a", false, 999)]);
        for m in [SyncMode::Pull, SyncMode::TwoWay] {
            assert!(plan_sync(std::slice::from_ref(&t), &i, vec![], m, &statuses()).is_empty());
        }
    }

    #[test]
    fn body_change_rewrites_description_unless_edited_locally() {
        let t = linked("t-0001", 1, "a", 100, 50);
        let mut iss = issue(1, "a", false, 60);
        iss.body = "new body".into();
        let p = plan_sync(
            std::slice::from_ref(&t),
            &map(vec![iss.clone()]),
            vec![],
            SyncMode::Pull,
            &statuses(),
        );
        assert!(p.local[0].rewrite_description);
        let mut edited = t;
        edited.updated = ts(150);
        let p = plan_sync(
            &[edited],
            &map(vec![iss]),
            vec![],
            SyncMode::Pull,
            &statuses(),
        );
        assert!(!p.local[0].rewrite_description);
    }

    #[test]
    fn closed_maps_to_last_status_and_back() {
        let t = linked("t-0001", 1, "a", 100, 50);
        let p = plan_sync(
            std::slice::from_ref(&t),
            &map(vec![issue(1, "a", true, 60)]),
            vec![],
            SyncMode::Pull,
            &statuses(),
        );
        assert_eq!(p.local[0].status.as_deref(), Some("done"));
        let mut d = t;
        d.status = "done".into();
        d.updated = ts(200);
        let p = plan_sync(
            &[d.clone()],
            &map(vec![issue(1, "a", false, 50)]),
            vec![],
            SyncMode::Push,
            &statuses(),
        );
        assert!(p.push[0].close);
        let p = plan_sync(
            &[d],
            &map(vec![issue(1, "a", false, 50)]),
            vec![],
            SyncMode::Pull,
            &statuses(),
        );
        assert!(p.local.is_empty());
    }

    #[test]
    fn local_change_without_pushable_diff_plans_nothing() {
        let mut t = linked("t-0001", 1, "a", 100, 50);
        t.updated = ts(200);
        t.pinned = true;
        let p = plan_sync(
            &[t],
            &map(vec![issue(1, "a", false, 50)]),
            vec![],
            SyncMode::TwoWay,
            &statuses(),
        );
        assert!(p.is_empty());
    }

    #[test]
    fn truncation_on_char_boundary() {
        assert_eq!(truncate_chars("жжжж", 3), "жж…");
        assert_eq!(truncate_chars("abc", 3), "abc");
    }

    #[test]
    fn missing_issue_is_skipped() {
        let t = linked("t-0001", 9, "a", 100, 50);
        let p = plan_sync(&[t], &BTreeMap::new(), vec![], SyncMode::Pull, &statuses());
        assert_eq!(p.skipped.len(), 1);
    }

    #[test]
    fn edit_between_plan_and_apply_is_a_conflict() {
        let d = tempfile::tempdir().unwrap();
        let store = Store::at(d.path());
        let project = store.project_for_key(std::path::Path::new("/x/p"));
        let cfg = Config::default();
        let t = task::create_task(
            &store,
            &project,
            &cfg,
            NewTask {
                title: "a".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let seen = t.meta.updated;
        task::set_pin(&project, &cfg, "t-0001", true).unwrap();
        let wrote = write_locked(&project, "t-0001", seen, |m| {
            m.title = "sync".into();
            Ok(())
        })
        .unwrap();
        assert!(!wrote);
        let wrote =
            write_locked(&project, "t-0001", seen + Duration::days(9999), |_| Ok(())).unwrap();
        assert!(!wrote);
        assert_eq!(
            task::load_task(&project, &cfg, "1").unwrap().meta.title,
            "a"
        );
    }
}
