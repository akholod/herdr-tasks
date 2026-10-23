//! Human vs JSON rendering and the JSON error envelope.

use crate::error::AppError;
use crate::store::fsutil::human_size;
use crate::store::task::{Task, TaskEntry};
use crate::store::Project;
use serde_json::{json, Value};
use std::io::Write;

pub fn print_json(v: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = serde_json::to_writer(&mut out, v);
    let _ = writeln!(out);
}

/// `{"error":{"code","kind","message"}}` on stderr, stdout empty.
pub fn print_error(e: &AppError, json: bool) {
    if json {
        let v = json!({"error": {"code": e.code(), "kind": e.kind(), "message": e.to_string()}});
        eprintln!("{v}");
    } else {
        eprintln!("herdr-tasks: {e}");
    }
}

fn ts(t: &chrono::DateTime<chrono::Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn task_json(project: &Project, t: &Task) -> Value {
    let m = &t.meta;
    json!({
        "id": m.id,
        "project": project.key.display().to_string(),
        "title": m.title,
        "status": m.status,
        "status_valid": t.status_valid,
        "priority": m.priority.as_str(),
        "pinned": m.pinned,
        "tags": m.tags,
        "created": ts(&m.created),
        "updated": ts(&m.updated),
        "description": t.description,
        "github": m.github.as_ref().map(|g| json!({
            "issue": g.issue,
            "synced_at": g.synced_at.as_ref().map(ts),
        })),
        "attachments": t.attachments.iter().map(|a| json!({"name": a.name, "size": a.size})).collect::<Vec<_>>(),
        "path": t.dir.display().to_string(),
        "warnings": t.warnings,
    })
}

pub fn broken_json(entry: &TaskEntry) -> Option<Value> {
    match entry {
        TaskEntry::Broken(b) => Some(json!({
            "id": b.id,
            "path": b.path.display().to_string(),
            "message": b.error,
        })),
        TaskEntry::Ok(_) => None,
    }
}

pub fn project_json(p: &Project) -> Value {
    json!({
        "key": p.key.display().to_string(),
        "folder": p.folder,
        "path": p.dir.display().to_string(),
        "label": p.basename(),
    })
}

/// One line: `t-0042  in-progress  P1 *  title`.
pub fn task_line(t: &Task) -> String {
    let m = &t.meta;
    let pin = if m.pinned { "*" } else { " " };
    let status = if t.status_valid {
        m.status.clone()
    } else {
        format!("{}!", m.status)
    };
    format!(
        "{}  {:<12} {} {}  {}",
        m.id,
        status,
        m.priority.as_str(),
        pin,
        m.title
    )
}

pub fn task_block(t: &Task, details: Option<&str>) -> String {
    let m = &t.meta;
    let mut s = task_line(t);
    s.push('\n');
    if !m.tags.is_empty() {
        s.push_str(&format!("  tags: {}\n", m.tags.join(", ")));
    }
    if let Some(g) = &m.github {
        s.push_str(&format!("  github: #{}\n", g.issue));
    }
    if !t.description.trim().is_empty() {
        for l in t.description.trim_end().lines() {
            s.push_str(&format!("  {l}\n"));
        }
    }
    if !t.attachments.is_empty() {
        s.push_str(&format!("  attachments ({}):\n", t.attachments.len()));
        for a in &t.attachments {
            s.push_str(&format!("    {}  {}\n", a.name, human_size(a.size)));
        }
    }
    for w in &t.warnings {
        s.push_str(&format!("  warning: {w}\n"));
    }
    if let Some(d) = details {
        s.push_str("  --- details.md ---\n");
        for l in d.trim_end().lines() {
            s.push_str(&format!("  {l}\n"));
        }
    }
    s.push_str(&format!("  path: {}\n", t.dir.display()));
    s
}
