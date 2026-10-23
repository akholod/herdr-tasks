//! Task folders: `task.toml`, `description.md`, `details.md`, `attachments/`.
//! Every read-modify-write runs under the project lock; every file write is
//! atomic (temp + rename).

use crate::config::Config;
use crate::error::{AppError, Result};
use crate::store::attach::{self, Attachment};
use crate::store::{fsutil, ids, lock, Project, Store};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const META_FILE: &str = "task.toml";
pub const DESCRIPTION_FILE: &str = "description.md";
pub const DETAILS_FILE: &str = "details.md";
pub const ATTACHMENTS_DIR: &str = "attachments";
const KNOWN_KEYS: &[&str] = &[
    "id", "title", "status", "priority", "pinned", "tags", "created", "updated", "github",
];
const STALE_TMP: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Priority {
    P0,
    P1,
    P2,
}

impl Priority {
    pub const ALL: [Priority; 3] = [Priority::P0, Priority::P1, Priority::P2];

    pub fn as_str(self) -> &'static str {
        match self {
            Priority::P0 => "P0",
            Priority::P1 => "P1",
            Priority::P2 => "P2",
        }
    }

    pub fn next(self) -> Priority {
        match self {
            Priority::P0 => Priority::P1,
            Priority::P1 => Priority::P2,
            Priority::P2 => Priority::P0,
        }
    }
}

impl std::str::FromStr for Priority {
    type Err = AppError;
    fn from_str(s: &str) -> Result<Priority> {
        match s.trim().to_ascii_uppercase().as_str() {
            "P0" | "0" => Ok(Priority::P0),
            "P1" | "1" => Ok(Priority::P1),
            "P2" | "2" => Ok(Priority::P2),
            _ => Err(AppError::Validation(format!(
                "invalid priority `{s}` (expected P0, P1 or P2)"
            ))),
        }
    }
}

/// TOML datetimes (unquoted, millisecond precision) <-> chrono.
mod tomldt {
    use chrono::{DateTime, Utc};
    use serde::{
        de::Error as _, ser::Error as _, Deserialize, Deserializer, Serialize, Serializer,
    };

    pub fn to_toml(dt: &DateTime<Utc>) -> String {
        dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
    }

    pub fn serialize<S: Serializer>(dt: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
        let t: toml::value::Datetime = to_toml(dt).parse().map_err(S::Error::custom)?;
        t.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
        let t = toml::value::Datetime::deserialize(d)?;
        DateTime::parse_from_rfc3339(&t.to_string())
            .map(|v| v.with_timezone(&Utc))
            .map_err(|e| D::Error::custom(format!("datetime `{t}`: {e}")))
    }

    pub mod opt {
        use super::*;

        pub fn serialize<S: Serializer>(
            dt: &Option<DateTime<Utc>>,
            s: S,
        ) -> Result<S::Ok, S::Error> {
            match dt {
                Some(v) => super::serialize(v, s),
                None => s.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            d: D,
        ) -> Result<Option<DateTime<Utc>>, D::Error> {
            super::deserialize(d).map(Some)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GithubLink {
    pub issue: u64,
    #[serde(default, with = "tomldt::opt", skip_serializing_if = "Option::is_none")]
    pub synced_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_sha: Option<String>,
    #[serde(default, with = "tomldt::opt", skip_serializing_if = "Option::is_none")]
    pub remote_updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskMeta {
    pub id: String,
    pub title: String,
    pub status: String,
    pub priority: Priority,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(with = "tomldt")]
    pub created: DateTime<Utc>,
    #[serde(with = "tomldt")]
    pub updated: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<GithubLink>,
}

#[derive(Debug, Clone)]
pub struct Task {
    pub meta: TaskMeta,
    pub description: String,
    pub attachments: Vec<Attachment>,
    pub dir: PathBuf,
    pub status_valid: bool,
    pub warnings: Vec<String>,
}

impl Task {
    pub fn id(&self) -> &str {
        &self.meta.id
    }

    pub fn details(&self) -> String {
        std::fs::read_to_string(self.dir.join(DETAILS_FILE)).unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub struct BrokenTask {
    pub id: String,
    pub path: PathBuf,
    pub error: String,
}

#[derive(Debug, Clone)]
pub enum TaskEntry {
    Ok(Box<Task>),
    Broken(BrokenTask),
}

impl TaskEntry {
    pub fn id(&self) -> &str {
        match self {
            TaskEntry::Ok(t) => t.id(),
            TaskEntry::Broken(b) => &b.id,
        }
    }

    pub fn task(&self) -> Option<&Task> {
        match self {
            TaskEntry::Ok(t) => Some(t),
            TaskEntry::Broken(_) => None,
        }
    }
}

/// `max(now, prev + 1 ms)`: `updated` strictly increases on every write.
pub fn bump(prev: DateTime<Utc>) -> DateTime<Utc> {
    let now = now();
    let min = prev + ChronoDuration::milliseconds(1);
    if now > min {
        now
    } else {
        min
    }
}

/// Now, truncated to the millisecond precision we persist.
pub fn now() -> DateTime<Utc> {
    let n = Utc::now();
    DateTime::from_timestamp_millis(n.timestamp_millis()).unwrap_or(n)
}

pub fn read_meta(dir: &Path) -> std::result::Result<(TaskMeta, Vec<String>), String> {
    let path = dir.join(META_FILE);
    let src = std::fs::read_to_string(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "missing task.toml".to_string(),
        _ => format!("cannot read task.toml: {e}"),
    })?;
    let meta: TaskMeta = toml::from_str(&src).map_err(|e| {
        let msg = e.message().split_whitespace().collect::<Vec<_>>().join(" ");
        format!("invalid task.toml: {msg}")
    })?;
    let id = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if meta.id != id {
        return Err(format!(
            "invalid task.toml: id `{}` != folder `{id}`",
            meta.id
        ));
    }
    if meta.title.trim().is_empty() {
        return Err("invalid task.toml: empty title".into());
    }
    let mut warnings = Vec::new();
    if let Ok(table) = toml::from_str::<toml::Table>(&src) {
        for k in table.keys() {
            if !KNOWN_KEYS.contains(&k.as_str()) {
                warnings.push(format!("unknown key `{k}` in task.toml"));
            }
        }
    }
    Ok((meta, warnings))
}

pub fn write_meta(dir: &Path, meta: &TaskMeta) -> Result<()> {
    let text =
        toml::to_string(meta).map_err(|e| AppError::Config(format!("{}: {e}", dir.display())))?;
    let path = dir.join(META_FILE);
    fsutil::atomic_write(&path, text.as_bytes()).map_err(|e| AppError::io(&path, e))
}

pub fn write_text(dir: &Path, file: &str, text: &str) -> Result<()> {
    let path = dir.join(file);
    fsutil::atomic_write(&path, text.as_bytes()).map_err(|e| AppError::io(&path, e))
}

pub fn load_dir(dir: &Path, cfg: &Config) -> TaskEntry {
    let id = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (meta, mut warnings) = match read_meta(dir) {
        Ok(v) => v,
        Err(error) => {
            return TaskEntry::Broken(BrokenTask {
                id,
                path: dir.join(META_FILE),
                error,
            })
        }
    };
    let description = match std::fs::read_to_string(dir.join(DESCRIPTION_FILE)) {
        Ok(s) => s,
        Err(_) => {
            warnings.push("missing description.md".into());
            String::new()
        }
    };
    let len = description.chars().count();
    if len > cfg.description_limit {
        warnings.push(format!(
            "description is {len} chars, over description_limit {}",
            cfg.description_limit
        ));
    }
    let status_valid = cfg.statuses.iter().any(|s| s == &meta.status);
    if !status_valid {
        warnings.push(format!("unknown status `{}`", meta.status));
    }
    TaskEntry::Ok(Box::new(Task {
        attachments: attach::list(dir),
        meta,
        description,
        dir: dir.to_path_buf(),
        status_valid,
        warnings,
    }))
}

/// All tasks of a project, sorted by id; removes stale temp files first.
pub fn load_tasks(project: &Project, cfg: &Config) -> Vec<TaskEntry> {
    fsutil::clean_stale_tmp(&project.dir, STALE_TMP);
    let mut dirs: Vec<(u32, PathBuf)> = std::fs::read_dir(project.tasks_dir())
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let n = ids::parse_id(&e.file_name().to_string_lossy())?;
                    e.path().is_dir().then(|| (n, e.path()))
                })
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs.iter().map(|(_, d)| load_dir(d, cfg)).collect()
}

/// Load one task; missing -> NotFound (2), broken -> Config (3).
pub fn load_task(project: &Project, cfg: &Config, id: &str) -> Result<Task> {
    let id = ids::normalize_id(id)?;
    let dir = project.task_dir(&id);
    if !dir.is_dir() {
        return Err(AppError::NotFound(format!(
            "task {id} not found in {}",
            project.key.display()
        )));
    }
    match load_dir(&dir, cfg) {
        TaskEntry::Ok(t) => Ok(*t),
        TaskEntry::Broken(b) => Err(AppError::Config(format!(
            "{}: {}",
            b.path.display(),
            b.error
        ))),
    }
}

#[derive(Debug, Clone, Default)]
pub struct NewTask {
    pub title: String,
    pub description: String,
    pub details: String,
    pub priority: Option<Priority>,
    pub status: Option<String>,
    pub tags: Vec<String>,
    pub pinned: bool,
    /// Sync imports: link + timestamps set by the caller.
    pub github: Option<GithubLink>,
    pub at: Option<DateTime<Utc>>,
}

fn check_title(title: &str) -> Result<String> {
    let t = title.trim();
    if t.is_empty() {
        return Err(AppError::Validation("title must not be empty".into()));
    }
    if t.contains('\n') {
        return Err(AppError::Validation("title must be a single line".into()));
    }
    Ok(t.to_string())
}

fn check_description(text: &str, cfg: &Config) -> Result<()> {
    let len = text.chars().count();
    if len > cfg.description_limit {
        return Err(AppError::Validation(format!(
            "description is {len} chars, over description_limit {} (put long content in details.md: --details / --details-file)",
            cfg.description_limit
        )));
    }
    Ok(())
}

fn check_status(status: &str, cfg: &Config) -> Result<()> {
    if cfg.statuses.iter().any(|s| s == status) {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "unknown status `{status}` (expected one of: {})",
            cfg.statuses.join(", ")
        )))
    }
}

fn push_tags(tags: &mut Vec<String>, add: &[String]) {
    for t in add {
        let t = t.trim();
        if !t.is_empty() && !tags.iter().any(|x| x == t) {
            tags.push(t.to_string());
        }
    }
}

pub fn create_task(store: &Store, project: &Project, cfg: &Config, new: NewTask) -> Result<Task> {
    let title = check_title(&new.title)?;
    check_description(&new.description, cfg)?;
    let status = new.status.unwrap_or_else(|| cfg.first_status().to_string());
    check_status(&status, cfg)?;
    store.register(project)?;
    let dir = lock::with_lock(&project.dir, || {
        let (id, dir) = ids::allocate(&project.tasks_dir())?;
        let at = new.at.unwrap_or_else(now);
        let mut tags = Vec::new();
        push_tags(&mut tags, &new.tags);
        let meta = TaskMeta {
            id,
            title: title.clone(),
            status: status.clone(),
            priority: new.priority.unwrap_or(Priority::P1),
            pinned: new.pinned,
            tags,
            created: at,
            updated: at,
            github: new.github.clone(),
        };
        let att = dir.join(ATTACHMENTS_DIR);
        std::fs::create_dir_all(&att).map_err(|e| AppError::io(&att, e))?;
        write_text(&dir, DESCRIPTION_FILE, &new.description)?;
        write_text(&dir, DETAILS_FILE, &new.details)?;
        write_meta(&dir, &meta)?;
        Ok(dir)
    })?;
    match load_dir(&dir, cfg) {
        TaskEntry::Ok(t) => Ok(*t),
        TaskEntry::Broken(b) => Err(AppError::Config(b.error)),
    }
}

#[derive(Debug, Clone, Default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub details: Option<String>,
    pub append_details: Option<String>,
    pub priority: Option<Priority>,
    pub status: Option<String>,
    pub pinned: Option<bool>,
    pub add_tags: Vec<String>,
    pub remove_tags: Vec<String>,
}

/// Apply a user/agent edit; validation happens before anything is written.
pub fn update_task(project: &Project, cfg: &Config, id: &str, patch: TaskPatch) -> Result<Task> {
    let id = ids::normalize_id(id)?;
    let title = patch.title.as_deref().map(check_title).transpose()?;
    if let Some(d) = &patch.description {
        check_description(d, cfg)?;
    }
    if let Some(s) = &patch.status {
        check_status(s, cfg)?;
    }
    let dir = project.task_dir(&id);
    lock::with_lock(&project.dir, || {
        let mut meta = load_task(project, cfg, &id)?.meta;
        if let Some(t) = &title {
            meta.title = t.clone();
        }
        if let Some(p) = patch.priority {
            meta.priority = p;
        }
        if let Some(s) = &patch.status {
            meta.status = s.clone();
        }
        if let Some(p) = patch.pinned {
            meta.pinned = p;
        }
        push_tags(&mut meta.tags, &patch.add_tags);
        meta.tags
            .retain(|t| !patch.remove_tags.iter().any(|r| r.trim() == t));
        if let Some(d) = &patch.description {
            write_text(&dir, DESCRIPTION_FILE, d)?;
        }
        if let Some(d) = &patch.details {
            write_text(&dir, DETAILS_FILE, d)?;
        }
        if let Some(extra) = &patch.append_details {
            let mut cur = std::fs::read_to_string(dir.join(DETAILS_FILE)).unwrap_or_default();
            if !cur.is_empty() && !cur.ends_with('\n') {
                cur.push('\n');
            }
            cur.push_str(extra);
            if !cur.ends_with('\n') {
                cur.push('\n');
            }
            write_text(&dir, DETAILS_FILE, &cur)?;
        }
        meta.updated = bump(meta.updated);
        write_meta(&dir, &meta)
    })?;
    load_task(project, cfg, &id)
}

pub fn move_task(project: &Project, cfg: &Config, id: &str, status: &str) -> Result<Task> {
    update_task(
        project,
        cfg,
        id,
        TaskPatch {
            status: Some(status.to_string()),
            ..Default::default()
        },
    )
}

pub fn set_pin(project: &Project, cfg: &Config, id: &str, pinned: bool) -> Result<Task> {
    update_task(
        project,
        cfg,
        id,
        TaskPatch {
            pinned: Some(pinned),
            ..Default::default()
        },
    )
}

pub fn attach_files(
    project: &Project,
    cfg: &Config,
    id: &str,
    files: &[PathBuf],
) -> Result<Vec<Attachment>> {
    let id = ids::normalize_id(id)?;
    let dir = project.task_dir(&id);
    lock::with_lock(&project.dir, || {
        let mut meta = load_task(project, cfg, &id)?.meta;
        let added = attach::copy_in(&dir, files)?;
        meta.updated = bump(meta.updated);
        write_meta(&dir, &meta)?;
        Ok(added)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn setup() -> (tempfile::TempDir, Store, Project, Config) {
        let d = tempfile::tempdir().unwrap();
        let store = Store::at(d.path());
        let project = store.project_for_key(Path::new("/x/proj"));
        (d, store, project, Config::default())
    }

    fn new(title: &str) -> NewTask {
        NewTask {
            title: title.into(),
            ..Default::default()
        }
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn create_writes_exactly_four_entries() {
        let (_d, s, p, c) = setup();
        let t = create_task(&s, &p, &c, new("hello")).unwrap();
        assert_eq!(t.id(), "t-0001");
        assert_eq!(t.meta.status, "todo");
        assert_eq!(t.meta.priority, Priority::P1);
        assert_eq!(
            entries(&t.dir),
            vec!["attachments", "description.md", "details.md", "task.toml"]
        );
        assert_eq!(s.registry().unwrap().len(), 1);
    }

    #[test]
    fn meta_roundtrips_with_toml_datetimes() {
        let (_d, s, p, c) = setup();
        let t = create_task(&s, &p, &c, new("x")).unwrap();
        let text = std::fs::read_to_string(t.dir.join(META_FILE)).unwrap();
        assert!(text.contains("created = 20"), "{text}");
        assert!(!text.contains("created = \""), "{text}");
        let (m, w) = read_meta(&t.dir).unwrap();
        assert_eq!(m, t.meta);
        assert!(w.is_empty());
    }

    #[test]
    fn over_limit_create_writes_nothing() {
        let (_d, s, p, c) = setup();
        let err = create_task(
            &s,
            &p,
            &c,
            NewTask {
                description: "a".repeat(501),
                ..new("x")
            },
        )
        .unwrap_err();
        assert_eq!(err.code(), 1);
        assert!(!p.tasks_dir().exists() || entries(&p.tasks_dir()).is_empty());
    }

    #[test]
    fn limit_counts_chars_not_bytes() {
        let (_d, s, p, c) = setup();
        let ok = NewTask {
            description: "ж".repeat(500),
            ..new("x")
        };
        assert!(create_task(&s, &p, &c, ok).is_ok());
    }

    #[test]
    fn over_limit_update_is_byte_identical() {
        let (_d, s, p, c) = setup();
        let t = create_task(
            &s,
            &p,
            &c,
            NewTask {
                description: "short".into(),
                ..new("x")
            },
        )
        .unwrap();
        let before_meta = std::fs::read(t.dir.join(META_FILE)).unwrap();
        let before_desc = std::fs::read(t.dir.join(DESCRIPTION_FILE)).unwrap();
        let err = update_task(
            &p,
            &c,
            "t-0001",
            TaskPatch {
                description: Some("a".repeat(501)),
                title: Some("new".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(err.code(), 1);
        assert_eq!(std::fs::read(t.dir.join(META_FILE)).unwrap(), before_meta);
        assert_eq!(
            std::fs::read(t.dir.join(DESCRIPTION_FILE)).unwrap(),
            before_desc
        );
    }

    #[test]
    fn concurrent_creates_get_unique_ids() {
        let (_d, s, p, c) = setup();
        let (s, p, c) = (Arc::new(s), Arc::new(p), Arc::new(c));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let (s, p, c) = (s.clone(), p.clone(), c.clone());
                std::thread::spawn(move || {
                    (0..50)
                        .map(|i| {
                            create_task(&s, &p, &c, new(&format!("t{i}")))
                                .unwrap()
                                .meta
                                .id
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut all: Vec<String> = handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 100);
    }

    #[test]
    fn concurrent_updates_lose_nothing() {
        let (_d, s, p, c) = setup();
        create_task(&s, &p, &c, new("x")).unwrap();
        let (p, c) = (Arc::new(p), Arc::new(c));
        let a = {
            let (p, c) = (p.clone(), c.clone());
            std::thread::spawn(move || {
                for i in 0..100 {
                    let patch = TaskPatch {
                        title: Some(format!("title {i}")),
                        ..Default::default()
                    };
                    update_task(&p, &c, "t-0001", patch).unwrap();
                }
            })
        };
        let b = {
            let (p, c) = (p.clone(), c.clone());
            std::thread::spawn(move || {
                for i in 0..100 {
                    let patch = TaskPatch {
                        pinned: Some(i % 2 == 0),
                        add_tags: vec![format!("tag{i}")],
                        ..Default::default()
                    };
                    update_task(&p, &c, "t-0001", patch).unwrap();
                }
            })
        };
        a.join().unwrap();
        b.join().unwrap();
        let t = load_task(&p, &c, "t-0001").unwrap();
        assert_eq!(t.meta.tags.len(), 100);
        assert_eq!(t.meta.title, "title 99");
    }

    #[test]
    fn broken_task_does_not_hide_siblings() {
        let (_d, s, p, c) = setup();
        create_task(&s, &p, &c, new("a")).unwrap();
        let b = create_task(&s, &p, &c, new("b")).unwrap();
        create_task(&s, &p, &c, new("c")).unwrap();
        std::fs::write(b.dir.join(META_FILE), "id = \"t-0002\"\nstatus = \n").unwrap();
        let all = load_tasks(&p, &c);
        assert_eq!(all.len(), 3);
        assert!(matches!(all[1], TaskEntry::Broken(_)));
        assert!(all[0].task().is_some() && all[2].task().is_some());
        assert_eq!(load_task(&p, &c, "t-0002").unwrap_err().code(), 3);
    }

    #[test]
    fn missing_title_is_broken() {
        let (_d, s, p, c) = setup();
        let t = create_task(&s, &p, &c, new("a")).unwrap();
        let text = std::fs::read_to_string(t.dir.join(META_FILE)).unwrap();
        let text: String = text
            .lines()
            .filter(|l| !l.starts_with("title"))
            .map(|l| format!("{l}\n"))
            .collect();
        std::fs::write(t.dir.join(META_FILE), text).unwrap();
        assert!(matches!(load_tasks(&p, &c)[0], TaskEntry::Broken(_)));
    }

    #[test]
    fn unknown_status_loads_and_move_fixes_it() {
        let (_d, s, p, c) = setup();
        let t = create_task(&s, &p, &c, new("a")).unwrap();
        let text = std::fs::read_to_string(t.dir.join(META_FILE))
            .unwrap()
            .replace("status = \"todo\"", "status = \"blocked\"");
        std::fs::write(t.dir.join(META_FILE), text).unwrap();
        let t = load_task(&p, &c, "t-0001").unwrap();
        assert!(!t.status_valid);
        let t = move_task(&p, &c, "t-0001", "done").unwrap();
        assert!(t.status_valid);
    }

    #[test]
    fn unknown_keys_are_warnings() {
        let (_d, s, p, c) = setup();
        let t = create_task(&s, &p, &c, new("a")).unwrap();
        let mut text = std::fs::read_to_string(t.dir.join(META_FILE)).unwrap();
        text = format!("future = 1\n{text}");
        std::fs::write(t.dir.join(META_FILE), text).unwrap();
        let t = load_task(&p, &c, "t-0001").unwrap();
        assert_eq!(t.warnings, vec!["unknown key `future` in task.toml"]);
    }

    #[test]
    fn updated_strictly_increases() {
        let (_d, s, p, c) = setup();
        let t0 = create_task(&s, &p, &c, new("a")).unwrap();
        let t1 = set_pin(&p, &c, "1", true).unwrap();
        let t2 = set_pin(&p, &c, "1", false).unwrap();
        assert!(t1.meta.updated > t0.meta.updated);
        assert!(t2.meta.updated > t1.meta.updated);
    }

    #[test]
    fn append_details() {
        let (_d, s, p, c) = setup();
        create_task(
            &s,
            &p,
            &c,
            NewTask {
                details: "one".into(),
                ..new("a")
            },
        )
        .unwrap();
        let patch = TaskPatch {
            append_details: Some("two".into()),
            ..Default::default()
        };
        let t = update_task(&p, &c, "1", patch).unwrap();
        assert_eq!(t.details(), "one\ntwo\n");
    }
}
