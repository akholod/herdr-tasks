//! CLI contract: JSON output, exit codes, limits, project keys, broken tasks.

mod support;

use serde_json::Value;
use support::cli_env::CliEnv;

fn err_json(stderr: &[u8]) -> Value {
    serde_json::from_slice(stderr)
        .unwrap_or_else(|_| panic!("stderr not JSON: {}", String::from_utf8_lossy(stderr)))
}

#[test]
fn every_subcommand_prints_one_json_document() {
    let e = CliEnv::new();
    let f = e.dir.path().join("note.txt");
    std::fs::write(&f, "hello").unwrap();
    let t = e.json(&[
        "create",
        "First",
        "-d",
        "short",
        "--tag",
        "ui",
        "--priority",
        "P0",
    ]);
    assert_eq!(t["id"], "t-0001");
    assert_eq!(t["priority"], "P0");
    assert_eq!(t["tags"][0], "ui");
    assert_eq!(e.json(&["list"])["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(e.json(&["show", "t-0001"])["title"], "First");
    assert_eq!(e.json(&["show", "1", "--details"])["details"], "");
    assert_eq!(
        e.json(&[
            "update",
            "t-0001",
            "--title",
            "Renamed",
            "--append-details",
            "ctx"
        ])["title"],
        "Renamed"
    );
    assert_eq!(e.json(&["move", "t-0001", "testing"])["status"], "testing");
    assert_eq!(e.json(&["pin", "t-0001"])["pinned"], true);
    assert_eq!(e.json(&["pin", "t-0001", "--off"])["pinned"], false);
    let a = e.json(&["attach", "t-0001", f.to_str().unwrap()]);
    assert_eq!(a["attachments"][0]["name"], "note.txt");
    assert_eq!(a["attachments"][0]["size"], 5);
    assert_eq!(e.json(&["sync"])["skipped"], true);
    let p = e.json(&["project"]);
    assert_eq!(p["root_source"], "env-override");
    assert_eq!(p["config"]["description_limit"], 500);
    let skill = e.cmd().args(["skill", "--json"]).output().unwrap();
    assert_eq!(
        skill.stdout,
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/skill/SKILL.md")).unwrap()
    );
    assert_eq!(e.read("t-0001", "details.md"), "ctx\n");
}

#[test]
fn over_limit_create_fails_and_writes_nothing() {
    let e = CliEnv::new();
    let long = "a".repeat(501);
    let out = e
        .cmd()
        .args(["create", "x", "--json", "-d", &long])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let v = err_json(&out.stderr);
    assert_eq!(v["error"]["code"], 1);
    assert_eq!(v["error"]["kind"], "validation");
    let tasks = e.project_dir().join("tasks");
    assert!(!tasks.exists() || std::fs::read_dir(&tasks).unwrap().count() == 0);
}

#[test]
fn over_limit_update_leaves_files_byte_identical() {
    let e = CliEnv::new();
    e.json(&["create", "x", "-d", "short"]);
    let meta = std::fs::read(e.task_dir("t-0001").join("task.toml")).unwrap();
    let desc = std::fs::read(e.task_dir("t-0001").join("description.md")).unwrap();
    let long = "a".repeat(501);
    let out = e
        .cmd()
        .args(["update", "t-0001", "-d", &long, "--title", "changed"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        std::fs::read(e.task_dir("t-0001").join("task.toml")).unwrap(),
        meta
    );
    assert_eq!(
        std::fs::read(e.task_dir("t-0001").join("description.md")).unwrap(),
        desc
    );
}

#[test]
fn broken_project_config_exits_3_with_path_and_position() {
    let e = CliEnv::new();
    e.write_config("description_limit = 10\nstatuses = [\n");
    let out = e.cmd().args(["list"]).output().unwrap();
    assert_eq!(out.status.code(), Some(3));
    let msg = String::from_utf8_lossy(&out.stderr);
    assert!(msg.contains("config.toml"), "{msg}");
    assert!(msg.contains("line 3, column 1"), "{msg}");
}

#[test]
fn project_key_is_git_root_for_subdirs() {
    let e = CliEnv::new();
    let repo = e.proj.join("repo");
    std::fs::create_dir_all(repo.join("a/b")).unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    let top = e
        .cmd()
        .current_dir(&repo)
        .args(["project", "--json"])
        .output()
        .unwrap();
    let sub = e
        .cmd()
        .current_dir(repo.join("a/b"))
        .args(["project", "--json"])
        .output()
        .unwrap();
    let (top, sub): (Value, Value) = (
        serde_json::from_slice(&top.stdout).unwrap(),
        serde_json::from_slice(&sub.stdout).unwrap(),
    );
    assert_eq!(top["key"], sub["key"]);
    assert_eq!(top["folder"], sub["folder"]);
    assert_eq!(top["key"], repo.display().to_string());
    let via_flag: Value = serde_json::from_slice(
        &e.cmd()
            .current_dir("/")
            .args(["project", "--json", "--project"])
            .arg(repo.join("a"))
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(via_flag["key"], top["key"]);
}

#[test]
fn non_git_dir_and_symlink_use_canonical_key() {
    let e = CliEnv::new();
    let link = e.dir.path().join("link");
    std::os::unix::fs::symlink(&e.proj, &link).unwrap();
    let a = e.json(&["project"]);
    let b: Value = serde_json::from_slice(
        &e.cmd()
            .current_dir(&link)
            .args(["project", "--json"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(a["key"], e.proj.display().to_string());
    assert_eq!(a["key"], b["key"]);
    e.json(&["create", "here"]);
    let l: Value = serde_json::from_slice(
        &e.cmd()
            .current_dir(&link)
            .args(["list", "--json"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(l["tasks"][0]["title"], "here");
}

#[test]
fn task_folder_has_exactly_four_entries() {
    let e = CliEnv::new();
    e.json(&["create", "x"]);
    let mut names: Vec<String> = std::fs::read_dir(e.task_dir("t-0001"))
        .unwrap()
        .map(|d| d.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["attachments", "description.md", "details.md", "task.toml"]
    );
}

#[test]
fn broken_task_show_exits_3_list_reports_errors() {
    let e = CliEnv::new();
    e.json(&["create", "a"]);
    e.json(&["create", "b"]);
    let meta = e.task_dir("t-0002").join("task.toml");
    std::fs::write(&meta, "id = \"t-0002\"\nstatus = [\n").unwrap();
    let out = e.cmd().args(["show", "t-0002"]).output().unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains(&meta.display().to_string()));
    let l = e.json(&["list"]);
    assert_eq!(l["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(l["errors"][0]["id"], "t-0002");
    assert_eq!(l["errors"][0]["path"], meta.display().to_string());
}

#[test]
fn unknown_status_is_not_broken_and_move_fixes_it() {
    let e = CliEnv::new();
    e.json(&["create", "a"]);
    let meta = e.task_dir("t-0001").join("task.toml");
    let text = std::fs::read_to_string(&meta)
        .unwrap()
        .replace("status = \"todo\"", "status = \"blocked\"");
    std::fs::write(&meta, text).unwrap();
    let l = e.json(&["list"]);
    assert_eq!(l["tasks"][0]["status_valid"], false);
    assert!(l["errors"].as_array().unwrap().is_empty());
    assert_eq!(e.json(&["move", "t-0001", "done"])["status_valid"], true);
}

#[test]
fn exit_codes_not_found_and_usage() {
    let e = CliEnv::new();
    let out = e.cmd().args(["show", "t-0099", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(err_json(&out.stderr)["error"]["kind"], "not_found");
    let out = e.cmd().args(["create", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(64));
    assert_eq!(err_json(&out.stderr)["error"]["kind"], "usage");
    let out = e.cmd().args(["move", "1", "nope"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let out = e.cmd().args(["show", "../../etc"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let out = e
        .cmd()
        .args(["--project", "/does/not/exist", "list"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn details_from_stdin_and_list_filters() {
    let e = CliEnv::new();
    e.cmd()
        .args(["create", "a", "--details-file", "-", "--tag", "x"])
        .write_stdin("from stdin")
        .assert()
        .success();
    assert_eq!(e.read("t-0001", "details.md"), "from stdin");
    e.json(&["create", "b", "--priority", "P0", "--status", "testing"]);
    assert_eq!(
        e.json(&["list", "--tag", "x"])["tasks"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        e.json(&["list", "--priority", "P0"])["tasks"][0]["title"],
        "b"
    );
    assert_eq!(
        e.json(&["list", "--status", "testing"])["tasks"][0]["title"],
        "b"
    );
    let all = e.json(&["list", "--all-projects"]);
    assert_eq!(all["tasks"].as_array().unwrap().len(), 2);
    assert!(all["project"].is_null());
}

#[test]
fn attach_directory_and_bad_config_values() {
    let e = CliEnv::new();
    e.json(&["create", "a"]);
    let out = e
        .cmd()
        .args(["attach", "1"])
        .arg(e.dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("is a directory"));
    e.write_config("statuses = []\n");
    let out = e.cmd().args(["list"]).output().unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("config.toml: `statuses`"));
}

#[test]
fn missing_title_and_stale_tmp_on_load() {
    let e = CliEnv::new();
    e.json(&["create", "a"]);
    e.json(&["create", "b"]);
    let meta = e.task_dir("t-0002").join("task.toml");
    let text: String = std::fs::read_to_string(&meta)
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with("title"))
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(&meta, text).unwrap();
    let out = e.cmd().args(["show", "t-0002"]).output().unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains(&meta.display().to_string()));
    let l = e.json(&["list"]);
    assert_eq!(l["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(l["errors"][0]["id"], "t-0002");
    // 10d through the load path: a crash leftover older than 60 s is removed by `list`.
    let stale = e.task_dir("t-0001").join("task.toml.tmp-999-0");
    let fresh = e.task_dir("t-0001").join("task.toml.tmp-999-1");
    std::fs::write(&stale, "x").unwrap();
    std::fs::write(&fresh, "x").unwrap();
    let past = filetime::FileTime::from_system_time(
        std::time::SystemTime::now() - std::time::Duration::from_secs(120),
    );
    filetime::set_file_mtime(&stale, past).unwrap();
    e.json(&["list"]);
    assert!(!stale.exists());
    assert!(fresh.exists());
}
