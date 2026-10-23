//! herdr integration through its CLI (`herdr api snapshot`, falling back to
//! `pane list` + `workspace list`) and project discovery: project = space.

use crate::store::fsutil;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaneInfo {
    pub pane_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub cwd: Option<String>,
    pub focused: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorkspaceInfo {
    pub workspace_id: String,
    pub label: String,
    pub number: u64,
    pub active_tab_id: Option<String>,
    pub focused: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub panes: Vec<PaneInfo>,
    pub workspaces: Vec<WorkspaceInfo>,
    /// tab_id -> focused pane of that tab (snapshot `layouts` only).
    pub tab_focus: HashMap<String, String>,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn parse_panes(arr: &[Value]) -> Vec<PaneInfo> {
    arr.iter()
        .map(|p| PaneInfo {
            pane_id: s(p, "pane_id"),
            workspace_id: s(p, "workspace_id"),
            tab_id: s(p, "tab_id"),
            cwd: p
                .get("cwd")
                .or_else(|| p.get("foreground_cwd"))
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())
                .map(str::to_string),
            focused: p.get("focused").and_then(Value::as_bool).unwrap_or(false),
        })
        .collect()
}

fn parse_workspaces(arr: &[Value]) -> Vec<WorkspaceInfo> {
    arr.iter()
        .map(|w| WorkspaceInfo {
            workspace_id: s(w, "workspace_id"),
            label: s(w, "label"),
            number: w.get("number").and_then(Value::as_u64).unwrap_or(u64::MAX),
            active_tab_id: w
                .get("active_tab_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            focused: w.get("focused").and_then(Value::as_bool).unwrap_or(false),
        })
        .collect()
}

/// `herdr api snapshot` -> `result.snapshot.{panes,workspaces,layouts}`.
/// `None` when panes lack `cwd` or workspaces lack `label` (older shape).
pub fn parse_api_snapshot(v: &Value) -> Option<Snapshot> {
    let snap = v.get("result")?.get("snapshot")?;
    let panes = snap.get("panes")?.as_array()?;
    let wss = snap.get("workspaces")?.as_array()?;
    if panes.iter().any(|p| p.get("cwd").is_none()) || wss.iter().any(|w| w.get("label").is_none())
    {
        return None;
    }
    let mut tab_focus = HashMap::new();
    for l in snap
        .get("layouts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (tab, pane) = (s(l, "tab_id"), s(l, "focused_pane_id"));
        if !tab.is_empty() && !pane.is_empty() {
            tab_focus.insert(tab, pane);
        }
    }
    Some(Snapshot {
        panes: parse_panes(panes),
        workspaces: parse_workspaces(wss),
        tab_focus,
    })
}

/// `herdr pane list` + `herdr workspace list`.
pub fn parse_lists(panes: &Value, workspaces: &Value) -> Option<Snapshot> {
    let p = panes.get("result")?.get("panes")?.as_array()?;
    let w = workspaces.get("result")?.get("workspaces")?.as_array()?;
    Some(Snapshot {
        panes: parse_panes(p),
        workspaces: parse_workspaces(w),
        tab_focus: HashMap::new(),
    })
}

fn run_json(args: &[&str]) -> Option<Value> {
    let out = fsutil::command(crate::store::herdr_bin())
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

/// Read panes and workspaces from herdr; `None` outside herdr.
pub fn snapshot() -> Option<Snapshot> {
    if let Some(s) = run_json(&["api", "snapshot"])
        .as_ref()
        .and_then(parse_api_snapshot)
    {
        return Some(s);
    }
    parse_lists(
        &run_json(&["pane", "list"])?,
        &run_json(&["workspace", "list"])?,
    )
}

/// The plugin directory of a binary at `<plugin>/target/<profile>/herdr-tasks`.
pub fn plugin_dir_of(exe: &Path) -> Option<PathBuf> {
    let profile = exe.parent()?;
    let target = profile.parent()?;
    (target.file_name()? == "target").then(|| target.parent().map(Path::to_path_buf))?
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpaceProject {
    pub key: PathBuf,
    pub label: String,
    pub number: u64,
    pub workspace_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Discovery {
    /// One per distinct space key, ordered by space number.
    pub spaces: Vec<SpaceProject>,
    /// Keys of minority panes (not any space's key).
    pub other_keys: Vec<PathBuf>,
    /// Key of the focused space, if any.
    pub focused_key: Option<PathBuf>,
}

/// Project = herdr space. Per space: drop our own pane and panes in the
/// plugin dir, map pane cwds to keys, take the majority key; a tie goes to
/// the space's focused pane. Spaces sharing a key collapse into one project
/// labelled by the lowest-numbered space.
pub fn discover_projects(
    snap: &Snapshot,
    own_pane: Option<&str>,
    plugin_dir: Option<&Path>,
    resolve: &mut dyn FnMut(&str) -> Option<PathBuf>,
) -> Discovery {
    let mut wss = snap.workspaces.clone();
    wss.sort_by_key(|w| w.number);
    let mut by_key: BTreeMap<PathBuf, usize> = BTreeMap::new();
    let mut spaces: Vec<SpaceProject> = Vec::new();
    let mut pane_keys: Vec<PathBuf> = Vec::new();
    let mut focused_key = None;
    for ws in &wss {
        let panes: Vec<(&PaneInfo, PathBuf)> = snap
            .panes
            .iter()
            .filter(|p| p.workspace_id == ws.workspace_id)
            .filter(|p| own_pane != Some(p.pane_id.as_str()))
            .filter_map(|p| {
                let cwd = p.cwd.as_deref()?;
                if plugin_dir.is_some_and(|d| Path::new(cwd).starts_with(d)) {
                    return None;
                }
                Some((p, resolve(cwd)?))
            })
            .filter(|(_, k)| !plugin_dir.is_some_and(|d| k.starts_with(d)))
            .collect();
        if panes.is_empty() {
            continue;
        }
        let mut counts: Vec<(PathBuf, usize)> = Vec::new();
        for (_, k) in &panes {
            match counts.iter_mut().find(|(c, _)| c == k) {
                Some(e) => e.1 += 1,
                None => counts.push((k.clone(), 1)),
            }
            pane_keys.push(k.clone());
        }
        let max = counts.iter().map(|c| c.1).max().unwrap_or(0);
        let tied: Vec<&PathBuf> = counts.iter().filter(|c| c.1 == max).map(|c| &c.0).collect();
        let key = if tied.len() == 1 {
            tied[0].clone()
        } else {
            let tab_focus = ws
                .active_tab_id
                .as_ref()
                .and_then(|t| snap.tab_focus.get(t));
            let pick = panes
                .iter()
                .find(|(p, k)| p.focused && tied.contains(&k))
                .or_else(|| {
                    panes
                        .iter()
                        .find(|(p, k)| Some(&p.pane_id) == tab_focus && tied.contains(&k))
                })
                .or_else(|| panes.iter().find(|(_, k)| tied.contains(&k)));
            pick.map(|(_, k)| k.clone())
                .unwrap_or_else(|| tied[0].clone())
        };
        if ws.focused {
            focused_key = Some(key.clone());
        }
        match by_key.get(&key) {
            Some(&i) => spaces[i].workspace_ids.push(ws.workspace_id.clone()),
            None => {
                let label = if ws.label.trim().is_empty() {
                    key.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| key.display().to_string())
                } else {
                    ws.label.trim().to_string()
                };
                by_key.insert(key.clone(), spaces.len());
                spaces.push(SpaceProject {
                    key,
                    label,
                    number: ws.number,
                    workspace_ids: vec![ws.workspace_id.clone()],
                });
            }
        }
    }
    // Duplicate labels for different keys: "name", "name (2)", ...
    let mut seen: HashMap<String, usize> = HashMap::new();
    for sp in &mut spaces {
        let n = seen.entry(sp.label.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            sp.label = format!("{} ({n})", sp.label);
        }
    }
    let mut other_keys: Vec<PathBuf> = Vec::new();
    for k in pane_keys {
        if !by_key.contains_key(&k) && !other_keys.contains(&k) {
            other_keys.push(k);
        }
    }
    Discovery {
        spaces,
        other_keys,
        focused_key,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/herdr")
            .join(name);
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    /// Test resolver: `/work/<repo>/...` -> `/work/<repo>` (a "git root").
    fn resolver() -> impl FnMut(&str) -> Option<PathBuf> {
        |cwd: &str| {
            let p = Path::new(cwd);
            let parts: Vec<_> = p.components().take(3).collect();
            Some(parts.iter().collect())
        }
    }

    fn keys(d: &Discovery) -> Vec<(String, String)> {
        d.spaces
            .iter()
            .map(|s| (s.label.clone(), s.key.display().to_string()))
            .collect()
    }

    #[test]
    fn snapshot_and_lists_give_same_projects() {
        let a = parse_api_snapshot(&fixture("api-snapshot.json")).unwrap();
        let b = parse_lists(&fixture("pane-list.json"), &fixture("workspace-list.json")).unwrap();
        let plugin = Path::new("/work/herdr-tasks");
        let da = discover_projects(&a, Some("w1:p9"), Some(plugin), &mut resolver());
        let db = discover_projects(&b, Some("w1:p9"), Some(plugin), &mut resolver());
        assert_eq!(keys(&da), keys(&db));
        // Two panes in subdirs of one repo + a non-git dir: 2 keys, plugin pane excluded.
        assert_eq!(
            keys(&da),
            vec![
                ("alpha".to_string(), "/work/alpha".to_string()),
                ("notes".to_string(), "/work/notes".to_string()),
                ("empty-space".to_string(), "/work/idle".to_string()),
            ]
        );
        assert_eq!(da.focused_key, Some(PathBuf::from("/work/alpha")));
    }

    #[test]
    fn majority_key_wins() {
        let s = parse_api_snapshot(&fixture("space-majority.json")).unwrap();
        let d = discover_projects(
            &s,
            Some("w1:p9"),
            Some(Path::new("/work/herdr-tasks")),
            &mut resolver(),
        );
        assert_eq!(
            keys(&d),
            vec![("space".to_string(), "/work/repoA".to_string())]
        );
        assert_eq!(d.other_keys, vec![PathBuf::from("/work/dirB")]);
    }

    #[test]
    fn tie_goes_to_focused_pane() {
        let s = parse_api_snapshot(&fixture("space-tie.json")).unwrap();
        let d = discover_projects(
            &s,
            Some("w1:p9"),
            Some(Path::new("/work/herdr-tasks")),
            &mut resolver(),
        );
        assert_eq!(
            keys(&d),
            vec![("space".to_string(), "/work/dirB".to_string())]
        );
    }

    #[test]
    fn same_repo_in_two_spaces_shown_once() {
        let s = parse_api_snapshot(&fixture("two-spaces-one-repo.json")).unwrap();
        let d = discover_projects(&s, None, None, &mut resolver());
        assert_eq!(d.spaces.len(), 1);
        assert_eq!(d.spaces[0].label, "first");
        assert_eq!(d.spaces[0].workspace_ids, vec!["w1", "w2"]);
    }

    #[test]
    fn plugin_dir_from_exe() {
        assert_eq!(
            plugin_dir_of(Path::new("/x/herdr-tasks/target/release/herdr-tasks")),
            Some(PathBuf::from("/x/herdr-tasks"))
        );
        assert_eq!(plugin_dir_of(Path::new("/usr/bin/herdr-tasks")), None);
    }
}
