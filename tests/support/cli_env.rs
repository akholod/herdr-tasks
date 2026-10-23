//! A temp store + project dir + fake `gh` for CLI/sync tests.
#![allow(dead_code)]

use assert_cmd::Command;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub struct CliEnv {
    pub dir: tempfile::TempDir,
    pub root: PathBuf,
    pub proj: PathBuf,
    pub bin: PathBuf,
    pub gh_dir: PathBuf,
    pub log: PathBuf,
}

impl CliEnv {
    pub fn new() -> CliEnv {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("store");
        let proj = dir.path().join("proj");
        let bin = dir.path().join("bin");
        let gh_dir = dir.path().join("gh");
        for d in [&proj, &bin, &gh_dir] {
            std::fs::create_dir_all(d).unwrap();
        }
        let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-gh.sh");
        std::os::unix::fs::symlink(fake, bin.join("gh")).unwrap();
        let log = dir.path().join("gh.log");
        std::fs::write(&log, "").unwrap();
        let proj = std::fs::canonicalize(proj).unwrap();
        let e = CliEnv {
            dir,
            root,
            proj,
            bin,
            gh_dir,
            log,
        };
        e.set_issues(&[]);
        e
    }

    pub fn cmd(&self) -> Command {
        let mut c = Command::cargo_bin("herdr-tasks").unwrap();
        c.current_dir(&self.proj)
            .env("HERDR_TASKS_HOME", &self.root)
            .env("PATH", format!("{}:/usr/bin:/bin", self.bin.display()))
            .env("FAKE_GH_DIR", &self.gh_dir)
            .env("FAKE_GH_LOG", &self.log)
            .env_remove("GITHUB_TOKEN")
            .env_remove("GH_TOKEN")
            .env_remove("HERDR_PLUGIN_CONFIG_DIR");
        c
    }

    /// Run with args, expect success, parse stdout JSON.
    pub fn json(&self, args: &[&str]) -> Value {
        let out = self.cmd().args(args).arg("--json").output().unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{args:?}: {e}: {}", String::from_utf8_lossy(&out.stdout)))
    }

    pub fn project_dir(&self) -> PathBuf {
        PathBuf::from(self.json(&["project"])["path"].as_str().unwrap())
    }

    pub fn write_config(&self, text: &str) {
        let d = self.project_dir();
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("config.toml"), text).unwrap();
    }

    pub fn set_mode(&self, mode: &str) {
        self.write_config(&format!("[github]\nmode = \"{mode}\"\nrepo = \"o/r\"\n"));
    }

    pub fn set_issues(&self, issues: &[Value]) {
        std::fs::write(
            self.gh_dir.join("issues.json"),
            serde_json::to_string(issues).unwrap(),
        )
        .unwrap();
    }

    pub fn issues(&self) -> Vec<Value> {
        serde_json::from_str(&std::fs::read_to_string(self.gh_dir.join("issues.json")).unwrap())
            .unwrap()
    }

    pub fn take_log(&self) -> Vec<String> {
        let s = std::fs::read_to_string(&self.log).unwrap();
        std::fs::write(&self.log, "").unwrap();
        s.lines().map(str::to_string).collect()
    }

    pub fn task_dir(&self, id: &str) -> PathBuf {
        self.project_dir().join("tasks").join(id)
    }

    pub fn read(&self, id: &str, file: &str) -> String {
        std::fs::read_to_string(self.task_dir(id).join(file)).unwrap()
    }

    /// (path, mtime) of every file under the project's tasks/.
    pub fn mtimes(&self) -> Vec<(PathBuf, std::time::SystemTime)> {
        let mut v = Vec::new();
        let tasks = self.project_dir().join("tasks");
        for t in std::fs::read_dir(&tasks).unwrap().flatten() {
            for f in std::fs::read_dir(t.path()).unwrap().flatten() {
                v.push((f.path(), f.metadata().unwrap().modified().unwrap()));
            }
        }
        v.sort();
        v
    }
}

pub fn issue(n: u64, title: &str, body: &str, labels: &[&str], updated: &str) -> Value {
    json!({
        "number": n, "title": title, "body": body, "state": "OPEN",
        "labels": labels.iter().map(|l| json!({"name": l})).collect::<Vec<_>>(),
        "updatedAt": updated,
    })
}

pub fn write_calls(log: &[String]) -> Vec<String> {
    log.iter()
        .filter(|l| {
            l.starts_with("issue edit")
                || l.starts_with("issue close")
                || l.starts_with("issue reopen")
                || l.starts_with("issue create")
        })
        .cloned()
        .collect()
}
