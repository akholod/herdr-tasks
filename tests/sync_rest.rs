//! REST fallback against a local HTTP stub (AC-6j, 6k): no working `gh`,
//! token from the environment or `github.token_file`.

mod support;

use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use support::cli_env::{issue, CliEnv};
use support::http_stub::{self, Response, Stub};

const TOKEN: &str = "test-token-5f2a";

/// GitHub-like state: issues + a pull request; PATCH mutates and bumps time.
fn github(issues: Vec<Value>) -> (Stub, Arc<Mutex<Vec<Value>>>) {
    let state = Arc::new(Mutex::new(issues));
    let s = state.clone();
    let mut clock = 0;
    let stub = http_stub::start(Box::new(move |req| {
        let mut st = s.lock().unwrap();
        let (path, query) = req.path.split_once('?').unwrap_or((&req.path, ""));
        if req.method == "GET" && path == "/repos/o/r/issues" {
            let page: usize = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("page="))
                .unwrap()
                .parse()
                .unwrap();
            let per: usize = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("per_page="))
                .unwrap()
                .parse()
                .unwrap();
            let open: Vec<Value> = st
                .iter()
                .filter(|i| i["state"] == "open")
                .cloned()
                .collect();
            let chunk: Vec<Value> = open.into_iter().skip((page - 1) * per).take(per).collect();
            return Response::json(200, serde_json::to_string(&chunk).unwrap());
        }
        let n: Option<u64> = path
            .strip_prefix("/repos/o/r/issues/")
            .and_then(|n| n.parse().ok());
        let Some(i) = n.and_then(|n| st.iter_mut().find(|i| i["number"] == n)) else {
            return Response::json(404, r#"{"message":"Not Found"}"#);
        };
        if req.method == "PATCH" {
            let body: Value = serde_json::from_str(&req.body).unwrap();
            if let Some(t) = body.get("title") {
                i["title"] = t.clone();
            }
            if let Some(s) = body.get("state") {
                i["state"] = s.clone();
            }
            clock += 1;
            i["updated_at"] = json!(format!("2026-01-01T00:00:{clock:02}Z"));
        }
        Response::json(200, i.to_string())
    }));
    (stub, state)
}

/// gh-shaped fixture -> REST shape (lowercase state, snake_case time).
fn rest_issue(v: Value) -> Value {
    json!({
        "number": v["number"], "title": v["title"], "body": v["body"],
        "state": v["state"].as_str().unwrap().to_lowercase(),
        "labels": v["labels"], "updated_at": v["updatedAt"],
    })
}

fn fixture() -> Vec<Value> {
    vec![
        issue(
            1,
            "Crash on start",
            "Steps",
            &["bug"],
            "2025-12-31T10:00:00Z",
        ),
        issue(2, "Docs typo", "", &[], "2025-12-31T11:00:00Z"),
        issue(3, "Third", "x", &["a", "b"], "2025-12-31T12:00:00Z"),
    ]
}

fn rest_env(stub: &Stub, extra: &str) -> CliEnv {
    let e = CliEnv::new();
    e.write_config(&format!(
        "[github]\nmode = \"pull\"\nrepo = \"o/r\"\nimport_limit = 2\napi_url = \"{}\"\n{extra}",
        stub.url
    ));
    e
}

fn cmd(e: &CliEnv) -> assert_cmd::Command {
    let mut c = e.cmd();
    c.env("FAKE_GH_AUTH", "1").env("GITHUB_TOKEN", TOKEN);
    c
}

fn run_json(e: &CliEnv, args: &[&str]) -> Value {
    let out = cmd(e).args(args).arg("--json").output().unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

/// Task files minus timestamps, for comparing the gh and REST paths.
fn task_summary(e: &CliEnv) -> Vec<Value> {
    let l = e.json(&["list"]);
    l["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let id = t["id"].as_str().unwrap();
            json!([
                t["id"],
                t["title"],
                t["status"],
                t["tags"],
                t["description"],
                t["github"]["issue"],
                e.read(id, "details.md")
            ])
        })
        .collect()
}

#[test]
fn rest_matches_gh_path() {
    let mut issues = fixture();
    // A pull request on page 1 must be filtered out.
    let mut pr = rest_issue(issue(9, "A PR", "", &[], "2025-12-31T09:00:00Z"));
    pr["pull_request"] = json!({"url": "x"});
    let mut rest: Vec<Value> = vec![rest_issue(issues[0].clone()), pr];
    rest.extend(issues[1..].iter().cloned().map(rest_issue));
    let (stub, state) = github(rest);
    let r_env = rest_env(&stub, "");
    let r = run_json(&r_env, &["sync"]);
    assert_eq!(r["client"], "rest");
    assert_eq!(
        r["imported"].as_array().unwrap().len(),
        2,
        "limit 2 over 2 pages, PR skipped"
    );

    // Same fixture through fake gh (import_limit 2 -> first two open issues).
    let g_env = CliEnv::new();
    g_env.write_config("[github]\nmode = \"pull\"\nrepo = \"o/r\"\nimport_limit = 2\n");
    issues.truncate(3);
    g_env.set_issues(&issues);
    g_env.json(&["sync"]);
    assert_eq!(task_summary(&r_env), task_summary(&g_env));

    // Two-way: edit title, close, reopen over PATCH.
    r_env.write_config(&format!(
        "[github]\nmode = \"two-way\"\nrepo = \"o/r\"\nimport_limit = 2\napi_url = \"{}\"\n",
        stub.url
    ));
    r_env.json(&["update", "t-0001", "--title", "Renamed"]);
    r_env.json(&["move", "t-0001", "done"]);
    let r = run_json(&r_env, &["sync"]);
    assert_eq!(r["pushed"].as_array().unwrap().len(), 2);
    {
        let st = state.lock().unwrap();
        assert_eq!(st[0]["title"], "Renamed");
        assert_eq!(st[0]["state"], "closed");
    }
    r_env.json(&["move", "t-0001", "todo"]);
    run_json(&r_env, &["sync"]);
    assert_eq!(state.lock().unwrap()[0]["state"], "open");
    // Second run is a no-op: no PATCH.
    let n = stub.log.lock().unwrap().len();
    run_json(&r_env, &["sync"]);
    let log = stub.log.lock().unwrap();
    assert!(log[n..].iter().all(|r| r.method == "GET"));
    // Requests: methods, paths, headers.
    assert!(log.iter().any(|r| r.method == "GET"
        && r.path
            .starts_with("/repos/o/r/issues?state=open&per_page=2&page=2")));
    let patches: Vec<_> = log.iter().filter(|r| r.method == "PATCH").collect();
    assert_eq!(patches.len(), 3);
    assert!(patches.iter().all(|r| r.path == "/repos/o/r/issues/1"));
    let body = |i: usize| serde_json::from_str::<Value>(&patches[i].body).unwrap();
    assert_eq!(body(0), json!({"title": "Renamed"}));
    assert_eq!(body(1), json!({"state": "closed"}));
    assert_eq!(body(2), json!({"state": "open"}));
    for r in log.iter() {
        assert_eq!(
            r.header("authorization"),
            Some(format!("Bearer {TOKEN}").as_str())
        );
        assert_eq!(r.header("accept"), Some("application/vnd.github+json"));
        assert_eq!(r.header("x-github-api-version"), Some("2022-11-28"));
        assert_eq!(r.header("user-agent"), Some("herdr-tasks"));
    }
}

fn status_stub(status: u16, headers: Vec<(String, String)>) -> Stub {
    http_stub::start(Box::new(move |_| Response {
        status,
        headers: headers.clone(),
        body: r#"{"message":"nope"}"#.into(),
    }))
}

fn assert_no_token(out: &std::process::Output) {
    for s in [&out.stdout, &out.stderr] {
        assert!(
            !String::from_utf8_lossy(s).contains(TOKEN),
            "token leaked: {}",
            String::from_utf8_lossy(s)
        );
    }
}

#[test]
fn unauthorized_is_exit_4_without_token_in_output() {
    let stub = status_stub(401, vec![]);
    let e = rest_env(&stub, "");
    for json in [false, true] {
        let mut c = cmd(&e);
        c.arg("sync");
        if json {
            c.arg("--json");
        }
        let out = c.output().unwrap();
        assert_eq!(out.status.code(), Some(4));
        assert!(String::from_utf8_lossy(&out.stderr).contains("token rejected"));
        assert_no_token(&out);
    }
}

#[test]
fn rate_limited_is_exit_4_with_reset_time() {
    let stub = status_stub(
        403,
        vec![
            ("X-RateLimit-Remaining".into(), "0".into()),
            ("X-RateLimit-Reset".into(), "1893456000".into()),
        ],
    );
    let e = rest_env(&stub, "");
    let out = cmd(&e).args(["sync"]).output().unwrap();
    assert_eq!(out.status.code(), Some(4));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("rate limited") && err.contains("2030-01-01"),
        "{err}"
    );
    assert_no_token(&out);
}

#[test]
fn no_gh_and_no_token_names_both_options() {
    let e = CliEnv::new();
    e.set_mode("pull");
    let out = e
        .cmd()
        .env("FAKE_GH_AUTH", "1")
        .args(["sync", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(4));
    let v: Value = serde_json::from_slice(&out.stderr).unwrap();
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(
        msg.contains("gh auth login")
            && msg.contains("GITHUB_TOKEN")
            && msg.contains("GH_TOKEN")
            && msg.contains("github.token_file"),
        "{msg}"
    );
    assert!(out.stdout.is_empty());
}

#[test]
fn token_file_is_used() {
    let (stub, _) = github(fixture().into_iter().map(rest_issue).collect());
    let tf = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tf.path(), format!("{TOKEN}\n")).unwrap();
    let e = rest_env(
        &stub,
        &format!("token_file = \"{}\"\n", tf.path().display()),
    );
    let out = e
        .cmd()
        .env("FAKE_GH_AUTH", "1")
        .args(["sync", "--list-new", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_no_token(&out);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["client"], "rest");
    assert_eq!(v["candidates"].as_array().unwrap().len(), 2);
    let log = stub.log.lock().unwrap();
    assert_eq!(
        log[0].header("authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
}
