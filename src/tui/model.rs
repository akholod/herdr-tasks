//! Pure view model: tasks of several projects -> groups -> flat rows.

use crate::store::task::{Priority, TaskEntry};
use std::cmp::Reverse;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Grouping {
    Priority,
    Status,
    Project,
}

impl Grouping {
    pub const ALL: [Grouping; 3] = [Grouping::Priority, Grouping::Status, Grouping::Project];

    pub fn label(self) -> &'static str {
        match self {
            Grouping::Priority => "Priority",
            Grouping::Status => "Status",
            Grouping::Project => "Project",
        }
    }

    pub fn cycle(self, delta: i32) -> Grouping {
        let i = Grouping::ALL.iter().position(|g| *g == self).unwrap_or(0) as i32;
        Grouping::ALL[(i + delta).rem_euclid(3) as usize]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GroupKey {
    Pinned,
    Priority(Priority),
    Status(String),
    Project(String),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemRef {
    pub folder: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub key: GroupKey,
    pub title: String,
    pub items: Vec<ItemRef>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Header {
        key: GroupKey,
        title: String,
        count: usize,
        collapsed: bool,
    },
    Task(ItemRef),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RowId {
    Header(GroupKey),
    Task(ItemRef),
}

impl Row {
    pub fn id(&self) -> RowId {
        match self {
            Row::Header { key, .. } => RowId::Header(key.clone()),
            Row::Task(i) => RowId::Task(i.clone()),
        }
    }
}

pub struct ProjectInput<'a> {
    pub folder: &'a str,
    pub label: &'a str,
    pub entries: &'a [TaskEntry],
    /// Show this project's group even with zero tasks (open herdr space).
    pub is_space: bool,
}

/// Order inside a group: priority asc, then `updated` desc; broken last.
fn sort_items(items: &mut [(ItemRef, Option<(Priority, i64)>)]) {
    items.sort_by_key(|(r, k)| match k {
        Some((p, upd)) => (0, *p, Reverse(*upd), r.id.clone()),
        None => (1, Priority::P2, Reverse(0), r.id.clone()),
    });
}

/// Build groups. Pinned first (all pinned tasks, removed from their regular
/// group). Empty groups are kept only for space projects in Project grouping
/// and, when `keep_empty_statuses`, for status columns (kanban).
pub fn build_groups(
    projects: &[ProjectInput],
    grouping: Grouping,
    statuses: &[String],
    keep_empty_statuses: bool,
) -> Vec<Group> {
    type Bucket = Vec<(ItemRef, Option<(Priority, i64)>)>;
    let mut pinned: Bucket = Vec::new();
    let mut keyed: Vec<(GroupKey, String, Bucket, bool)> = Vec::new();
    match grouping {
        Grouping::Priority => {
            for p in Priority::ALL {
                keyed.push((GroupKey::Priority(p), p.as_str().into(), Vec::new(), false));
            }
        }
        Grouping::Status => {
            for s in statuses {
                keyed.push((
                    GroupKey::Status(s.clone()),
                    s.clone(),
                    Vec::new(),
                    keep_empty_statuses,
                ));
            }
        }
        Grouping::Project => {
            let mut ps: Vec<&ProjectInput> = projects.iter().collect();
            ps.sort_by_key(|p| (p.label.to_lowercase(), p.folder.to_string()));
            for p in ps {
                keyed.push((
                    GroupKey::Project(p.folder.into()),
                    p.label.into(),
                    Vec::new(),
                    p.is_space,
                ));
            }
        }
    }
    let mut unknown: Bucket = Vec::new();
    for p in projects {
        for e in p.entries {
            let r = ItemRef {
                folder: p.folder.into(),
                id: e.id().into(),
            };
            let key = match e {
                TaskEntry::Ok(t) => {
                    let sort = Some((t.meta.priority, t.meta.updated.timestamp_millis()));
                    if t.meta.pinned {
                        pinned.push((r, sort));
                        continue;
                    }
                    let k = match grouping {
                        Grouping::Priority => GroupKey::Priority(t.meta.priority),
                        Grouping::Status if statuses.contains(&t.meta.status) && t.status_valid => {
                            GroupKey::Status(t.meta.status.clone())
                        }
                        Grouping::Status => GroupKey::Unknown,
                        Grouping::Project => GroupKey::Project(p.folder.into()),
                    };
                    (k, sort)
                }
                TaskEntry::Broken(_) => match grouping {
                    Grouping::Project => (GroupKey::Project(p.folder.into()), None),
                    _ => (GroupKey::Unknown, None),
                },
            };
            match keyed.iter_mut().find(|g| g.0 == key.0) {
                Some(g) => g.2.push((r, key.1)),
                None => unknown.push((r, key.1)),
            }
        }
    }
    let mut out = Vec::new();
    if !pinned.is_empty() {
        sort_items(&mut pinned);
        out.push(Group {
            key: GroupKey::Pinned,
            title: "Pinned".into(),
            items: pinned.into_iter().map(|x| x.0).collect(),
        });
    }
    for (key, title, mut items, keep_empty) in keyed {
        if items.is_empty() && !keep_empty {
            continue;
        }
        sort_items(&mut items);
        out.push(Group {
            key,
            title,
            items: items.into_iter().map(|x| x.0).collect(),
        });
    }
    if !unknown.is_empty() {
        sort_items(&mut unknown);
        out.push(Group {
            key: GroupKey::Unknown,
            title: "unknown".into(),
            items: unknown.into_iter().map(|x| x.0).collect(),
        });
    }
    out
}

pub fn flatten(groups: &[Group], collapsed: &HashSet<GroupKey>) -> Vec<Row> {
    let mut rows = Vec::new();
    for g in groups {
        let c = collapsed.contains(&g.key);
        rows.push(Row::Header {
            key: g.key.clone(),
            title: g.title.clone(),
            count: g.items.len(),
            collapsed: c,
        });
        if !c {
            rows.extend(g.items.iter().cloned().map(Row::Task));
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::task::{BrokenTask, Task, TaskMeta};
    use chrono::DateTime;

    fn t(id: &str, prio: Priority, status: &str, pinned: bool, upd: i64) -> TaskEntry {
        TaskEntry::Ok(Box::new(Task {
            meta: TaskMeta {
                id: id.into(),
                title: id.into(),
                status: status.into(),
                priority: prio,
                pinned,
                tags: vec![],
                created: DateTime::from_timestamp(0, 0).unwrap(),
                updated: DateTime::from_timestamp(upd, 0).unwrap(),
                github: None,
            },
            description: String::new(),
            attachments: vec![],
            dir: "/x".into(),
            status_valid: status != "weird",
            warnings: vec![],
        }))
    }

    fn statuses() -> Vec<String> {
        ["todo", "in-progress", "testing", "done"]
            .map(String::from)
            .to_vec()
    }

    fn titles(gs: &[Group]) -> Vec<String> {
        gs.iter()
            .map(|g| format!("{} {}", g.title, g.items.len()))
            .collect()
    }

    #[test]
    fn pinned_first_in_every_grouping() {
        let a = vec![
            t("t-0001", Priority::P2, "todo", true, 1),
            t("t-0002", Priority::P0, "done", false, 1),
        ];
        let p = [ProjectInput {
            folder: "a",
            label: "A",
            entries: &a,
            is_space: true,
        }];
        for g in Grouping::ALL {
            let gs = build_groups(&p, g, &statuses(), false);
            assert_eq!(gs[0].key, GroupKey::Pinned, "{g:?}");
            assert_eq!(gs[0].items.len(), 1);
            assert_eq!(gs.iter().map(|g| g.items.len()).sum::<usize>(), 2);
        }
    }

    #[test]
    fn ordering_priority_then_updated_desc() {
        let a = vec![
            t("t-0001", Priority::P1, "todo", false, 1),
            t("t-0002", Priority::P0, "todo", false, 1),
            t("t-0003", Priority::P1, "todo", false, 5),
        ];
        let p = [ProjectInput {
            folder: "a",
            label: "A",
            entries: &a,
            is_space: false,
        }];
        let gs = build_groups(&p, Grouping::Status, &statuses(), false);
        let ids: Vec<_> = gs[0].items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["t-0002", "t-0003", "t-0001"]);
    }

    #[test]
    fn empty_space_group_only_in_project_grouping() {
        let a = vec![t("t-0001", Priority::P1, "todo", false, 1)];
        let p = [
            ProjectInput {
                folder: "a",
                label: "alpha",
                entries: &a,
                is_space: true,
            },
            ProjectInput {
                folder: "b",
                label: "beta",
                entries: &[],
                is_space: true,
            },
        ];
        assert_eq!(
            titles(&build_groups(&p, Grouping::Project, &statuses(), false)),
            vec!["alpha 1", "beta 0"]
        );
        assert_eq!(
            titles(&build_groups(&p, Grouping::Priority, &statuses(), false)),
            vec!["P1 1"]
        );
        assert_eq!(
            titles(&build_groups(&p, Grouping::Status, &statuses(), false)),
            vec!["todo 1"]
        );
        assert_eq!(
            build_groups(&p, Grouping::Status, &statuses(), true).len(),
            4
        );
    }

    #[test]
    fn unknown_status_and_broken_go_to_unknown() {
        let a = vec![
            t("t-0001", Priority::P1, "weird", false, 1),
            TaskEntry::Broken(BrokenTask {
                id: "t-0002".into(),
                path: "/x".into(),
                error: "bad".into(),
            }),
        ];
        let p = [ProjectInput {
            folder: "a",
            label: "A",
            entries: &a,
            is_space: false,
        }];
        let gs = build_groups(&p, Grouping::Status, &statuses(), false);
        assert_eq!(titles(&gs), vec!["unknown 2"]);
        let gs = build_groups(&p, Grouping::Project, &statuses(), false);
        assert_eq!(titles(&gs), vec!["A 2"]);
    }

    #[test]
    fn collapsed_group_has_no_task_rows() {
        let a = vec![t("t-0001", Priority::P1, "todo", false, 1)];
        let p = [ProjectInput {
            folder: "a",
            label: "A",
            entries: &a,
            is_space: false,
        }];
        let gs = build_groups(&p, Grouping::Priority, &statuses(), false);
        let mut c = HashSet::new();
        assert_eq!(flatten(&gs, &c).len(), 2);
        c.insert(GroupKey::Priority(Priority::P1));
        let rows = flatten(&gs, &c);
        assert_eq!(rows.len(), 1);
        assert!(matches!(
            rows[0],
            Row::Header {
                collapsed: true,
                count: 1,
                ..
            }
        ));
    }

    #[test]
    fn grouping_cycles() {
        assert_eq!(Grouping::Priority.cycle(1), Grouping::Status);
        assert_eq!(Grouping::Project.cycle(1), Grouping::Priority);
        assert_eq!(Grouping::Priority.cycle(-1), Grouping::Project);
    }
}
