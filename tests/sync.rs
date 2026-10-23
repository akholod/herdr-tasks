//! GitHub sync through a fake `gh` on PATH (AC-6a..6e, 6g..6i).

mod support;

use serde_json::Value;
use support::cli_env::{issue, write_calls, CliEnv};

fn two_issues(e: &CliEnv) {
    e.set_issues(&[
        issue(
            1,
            "Crash on start",
            "Steps:\n1. run",
            &["bug"],
            "2025-12-31T10:00:00Z",
        ),
        issue(2, "Docs typo", "", &[], "2025-12-31T11:00:00Z"),
    ]);
}

#[test]
fn off_mode_is_a_clear_no_op() {
    let e = CliEnv::new();
    let out = e.cmd().arg("sync").output().unwrap();
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.starts_with("GitHub sync is off for "), "{s}");
    assert!(s.contains("set github.mode in "), "{s}");
    assert_eq!(
        e.json(&["sync"]),
        serde_json::json!({"mode": "off", "skipped": true})
    );
    assert!(e.take_log().is_empty());
}

#[test]
fn pull_imports_and_never_writes_to_github() {
    let e = CliEnv::new();
    e.set_mode("pull");
    two_issues(&e);
    let r = e.json(&["sync"]);
    assert_eq!(r["client"], "gh");
    assert_eq!(r["imported"].as_array().unwrap().len(), 2);
    let log = e.take_log();
    assert!(write_calls(&log).is_empty(), "{log:?}");
    assert!(
        log.iter().all(|l| l.starts_with("auth status")
            || l.starts_with("issue list")
            || l.starts_with("issue view")),
        "{log:?}"
    );
    let t = e.json(&["show", "t-0001", "--details"]);
    assert_eq!(t["title"], "Crash on start");
    assert_eq!(t["tags"][0], "bug");
    assert_eq!(t["github"]["issue"], 1);
    assert_eq!(t["details"], "Steps:\n1. run");
    assert_eq!(t["description"], "Steps:\n1. run");
}

#[test]
fn long_body_truncated_full_body_in_details() {
    let e = CliEnv::new();
    e.write_config("description_limit = 10\n[github]\nmode = \"pull\"\nrepo = \"o/r\"\n");
    e.set_issues(&[issue(
        7,
        "Long",
        "0123456789abcdef",
        &[],
        "2025-12-31T10:00:00Z",
    )]);
    e.json(&["sync"]);
    assert_eq!(e.read("t-0001", "description.md"), "012345678…");
    assert_eq!(e.read("t-0001", "details.md"), "0123456789abcdef");
}

#[test]
fn two_way_pushes_title_and_close() {
    let e = CliEnv::new();
    e.set_mode("pull");
    two_issues(&e);
    e.json(&["sync"]);
    e.set_mode("two-way");
    e.take_log();
    e.json(&["update", "t-0001", "--title", "Renamed"]);
    e.json(&["move", "t-0001", "done"]);
    let r = e.json(&["sync"]);
    let log = e.take_log();
    assert!(
        log.iter()
            .any(|l| l.starts_with("issue edit 1") && l.contains("--title Renamed")),
        "{log:?}"
    );
    assert!(
        log.iter().any(|l| l.starts_with("issue close 1")),
        "{log:?}"
    );
    assert!(log.iter().any(|l| l.starts_with("issue view 1")), "{log:?}");
    assert_eq!(r["pushed"].as_array().unwrap().len(), 2);
    let iss = e.issues();
    assert_eq!(iss[0]["title"], "Renamed");
    assert_eq!(iss[0]["state"], "CLOSED");
    // Reopen locally -> reopen remotely.
    e.json(&["move", "t-0001", "todo"]);
    e.json(&["sync"]);
    assert!(e.take_log().iter().any(|l| l.starts_with("issue reopen 1")));
    assert_eq!(e.issues()[0]["state"], "OPEN");
}

#[test]
fn second_sync_is_a_no_op_in_every_mode() {
    for mode in ["pull", "push", "two-way"] {
        let e = CliEnv::new();
        e.set_mode("pull");
        two_issues(&e);
        e.json(&["sync"]);
        e.set_mode(mode);
        if mode != "pull" {
            e.json(&["update", "t-0001", "--title", "Local"]);
            e.json(&["move", "t-0002", "done"]);
        } else {
            let mut iss = e.issues();
            iss[0]["title"] = "Remote".into();
            iss[0]["updatedAt"] = "2025-12-31T12:00:00Z".into();
            e.set_issues(&iss);
        }
        e.json(&["sync"]);
        e.take_log();
        let before = e.mtimes();
        let r = e.json(&["sync"]);
        let log = e.take_log();
        assert!(write_calls(&log).is_empty(), "{mode}: {log:?}");
        assert_eq!(e.mtimes(), before, "{mode}: files rewritten");
        for k in ["imported", "updated_local", "pushed", "conflicts"] {
            assert!(
                r[k].as_array().unwrap().is_empty(),
                "{mode}: {k} = {}",
                r[k]
            );
        }
    }
}

#[test]
fn comment_only_change_touches_nothing() {
    let e = CliEnv::new();
    e.set_mode("two-way");
    two_issues(&e);
    e.json(&["sync"]);
    e.json(&["update", "t-0001", "--append-details", "my notes"]);
    let desc = e.read("t-0001", "description.md");
    let det = e.read("t-0001", "details.md");
    let mut iss = e.issues();
    iss[0]["updatedAt"] = "2025-12-31T23:00:00Z".into();
    e.set_issues(&iss);
    e.json(&["sync"]);
    assert_eq!(e.read("t-0001", "description.md"), desc);
    assert_eq!(e.read("t-0001", "details.md"), det);
}

#[test]
fn remote_body_change_rewrites_description_not_details() {
    let e = CliEnv::new();
    e.set_mode("pull");
    two_issues(&e);
    e.json(&["sync"]);
    let mut iss = e.issues();
    iss[0]["body"] = "New body".into();
    iss[0]["updatedAt"] = "2025-12-31T23:00:00Z".into();
    e.set_issues(&iss);
    let r = e.json(&["sync"]);
    assert_eq!(r["updated_local"][0], "t-0001");
    assert_eq!(e.read("t-0001", "description.md"), "New body");
    assert_eq!(e.read("t-0001", "details.md"), "Steps:\n1. run");
}

#[test]
fn remote_close_moves_task_to_done() {
    let e = CliEnv::new();
    e.set_mode("pull");
    two_issues(&e);
    e.json(&["sync"]);
    let mut iss = e.issues();
    iss[1]["state"] = "CLOSED".into();
    iss[1]["updatedAt"] = "2025-12-31T23:00:00Z".into();
    e.set_issues(&iss);
    e.json(&["sync"]);
    assert_eq!(e.json(&["show", "t-0002"])["status"], "done");
}

#[test]
fn ignore_then_sync_skips_issue() {
    let e = CliEnv::new();
    e.set_mode("pull");
    two_issues(&e);
    assert_eq!(
        e.json(&["sync", "--ignore", "1"])["ignored_issues"],
        serde_json::json!([1])
    );
    assert!(e.take_log().is_empty(), "--ignore needs no network");
    let r = e.json(&["sync"]);
    assert_eq!(r["imported"].as_array().unwrap().len(), 1);
    assert_eq!(e.json(&["show", "t-0001"])["github"]["issue"], 2);
    let l = e.json(&["sync", "--list-new"]);
    assert!(l["candidates"].as_array().unwrap().is_empty());
    assert_eq!(l["ignored"], serde_json::json!([1]));
    assert_eq!(
        e.json(&["sync", "--unignore", "1"])["ignored_issues"],
        serde_json::json!([])
    );
    assert_eq!(
        e.json(&["sync", "--list-new"])["candidates"][0]["number"],
        1
    );
}

#[test]
fn import_only_selected_and_unignore() {
    let e = CliEnv::new();
    e.set_mode("two-way");
    two_issues(&e);
    e.json(&["sync", "--ignore", "2"]);
    e.json(&["sync"]);
    // t-0001 <- #1. Make a local change that a full sync would push.
    e.json(&["update", "t-0001", "--title", "Local"]);
    e.take_log();
    let r = e.json(&["sync", "--import", "2"]);
    assert_eq!(
        r["imported"],
        serde_json::json!([{"id": "t-0002", "issue": 2}])
    );
    assert!(
        write_calls(&e.take_log()).is_empty(),
        "linked tasks untouched"
    );
    assert_eq!(e.json(&["show", "t-0001"])["title"], "Local");
    assert_eq!(
        e.json(&["sync", "--list-new"])["ignored"],
        serde_json::json!([])
    );
}

#[test]
fn list_new_needs_a_pull_mode() {
    let e = CliEnv::new();
    e.set_mode("push");
    let out = e
        .cmd()
        .args(["sync", "--list-new", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("push"));
    let out = e
        .cmd()
        .args(["sync", "--list-new", "--ignore", "1"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64));
}

#[test]
fn push_mode_never_imports_or_creates() {
    let e = CliEnv::new();
    e.set_mode("push");
    two_issues(&e);
    e.json(&["create", "local only"]);
    let r = e.json(&["sync"]);
    assert!(r["imported"].as_array().unwrap().is_empty());
    assert!(write_calls(&e.take_log()).is_empty());
}

#[test]
fn dry_run_writes_nothing() {
    let e = CliEnv::new();
    e.set_mode("two-way");
    two_issues(&e);
    let r = e.json(&["sync", "--dry-run"]);
    assert_eq!(r["dry_run"], true);
    assert_eq!(r["imported"], serde_json::json!(["#1", "#2"]));
    assert!(!e.project_dir().join("tasks").exists());
    e.json(&["sync"]);
    e.json(&["move", "t-0001", "done"]);
    e.take_log();
    let before = e.mtimes();
    let r: Value = e.json(&["sync", "--dry-run"]);
    assert_eq!(r["pushed"][0]["action"], "close");
    assert!(write_calls(&e.take_log()).is_empty());
    assert_eq!(e.mtimes(), before);
}

#[test]
fn both_sides_changed_remote_wins_with_conflict() {
    let e = CliEnv::new();
    e.set_mode("two-way");
    two_issues(&e);
    e.json(&["sync"]);
    e.json(&["update", "t-0001", "--title", "Local"]);
    let mut iss = e.issues();
    iss[0]["title"] = "Remote".into();
    iss[0]["updatedAt"] = "2025-12-31T23:00:00Z".into();
    e.set_issues(&iss);
    let r = e.json(&["sync"]);
    assert_eq!(r["conflicts"], serde_json::json!(["t-0001"]));
    assert_eq!(e.json(&["show", "t-0001"])["title"], "Remote");
    assert!(write_calls(&e.take_log()).is_empty());
}

#[test]
fn pull_reports_conflict_when_local_edit_is_overwritten() {
    let e = CliEnv::new();
    e.set_mode("pull");
    two_issues(&e);
    e.json(&["sync"]);
    e.json(&["move", "t-0001", "done"]);
    let mut iss = e.issues();
    iss[0]["labels"] = serde_json::json!([{"name": "bug"}, {"name": "new"}]);
    iss[0]["updatedAt"] = "2025-12-31T23:00:00Z".into();
    e.set_issues(&iss);
    let r = e.json(&["sync"]);
    assert_eq!(r["conflicts"], serde_json::json!(["t-0001"]));
}

#[test]
fn import_of_missing_issue_exits_2() {
    let e = CliEnv::new();
    e.set_mode("pull");
    two_issues(&e);
    let out = e.cmd().args(["sync", "--import", "99"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}
