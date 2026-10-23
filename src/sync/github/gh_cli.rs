//! `gh` subprocess implementation of `GitHubApi`.

use super::{parse_time, GitHubApi, Issue};
use crate::error::{AppError, Result};
use crate::store::fsutil;
use serde::Deserialize;
use std::process::Stdio;

const FIELDS: &str = "number,title,body,state,labels,updatedAt";

pub struct GhCli;

#[derive(Deserialize)]
struct GhLabel {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhIssue {
    number: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    state: String,
    #[serde(default)]
    labels: Vec<GhLabel>,
    updated_at: String,
}

impl GhIssue {
    fn into_issue(self) -> Result<Issue> {
        Ok(Issue {
            number: self.number,
            title: self.title,
            body: self.body.unwrap_or_default(),
            closed: self.state.eq_ignore_ascii_case("closed"),
            labels: self.labels.into_iter().map(|l| l.name).collect(),
            updated_at: parse_time(&self.updated_at)?,
        })
    }
}

impl GhCli {
    /// `gh` runs and `gh auth status` exits 0.
    pub fn available() -> bool {
        fsutil::command("gh")
            .args(["auth", "status"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env("GH_PROMPT_DISABLED", "1")
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn run(args: &[&str]) -> Result<Vec<u8>> {
        let out = fsutil::command("gh")
            .args(args)
            .stdin(Stdio::null())
            .env("GH_PROMPT_DISABLED", "1")
            .output()
            .map_err(|e| AppError::External(format!("gh: {e}")))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let first = err
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("failed");
            return Err(AppError::External(format!(
                "gh {}: {}",
                args.iter().take(2).copied().collect::<Vec<_>>().join(" "),
                first.trim()
            )));
        }
        Ok(out.stdout)
    }
}

impl GitHubApi for GhCli {
    fn kind(&self) -> &'static str {
        "gh"
    }

    fn list_open_issues(&self, repo: &str, limit: usize) -> Result<Vec<Issue>> {
        let limit = limit.to_string();
        let out = Self::run(&[
            "issue", "list", "--repo", repo, "--state", "open", "--limit", &limit, "--json", FIELDS,
        ])?;
        let raw: Vec<GhIssue> = serde_json::from_slice(&out)
            .map_err(|e| AppError::External(format!("gh issue list: bad JSON: {e}")))?;
        raw.into_iter().map(GhIssue::into_issue).collect()
    }

    fn view_issue(&self, repo: &str, number: u64) -> Result<Issue> {
        let n = number.to_string();
        let out = Self::run(&["issue", "view", &n, "--repo", repo, "--json", FIELDS])?;
        let raw: GhIssue = serde_json::from_slice(&out)
            .map_err(|e| AppError::External(format!("gh issue view: bad JSON: {e}")))?;
        raw.into_issue()
    }

    fn edit_title(&self, repo: &str, number: u64, title: &str) -> Result<()> {
        let n = number.to_string();
        Self::run(&["issue", "edit", &n, "--repo", repo, "--title", title]).map(drop)
    }

    fn close(&self, repo: &str, number: u64) -> Result<()> {
        let n = number.to_string();
        Self::run(&["issue", "close", &n, "--repo", repo]).map(drop)
    }

    fn reopen(&self, repo: &str, number: u64) -> Result<()> {
        let n = number.to_string();
        Self::run(&["issue", "reopen", &n, "--repo", repo]).map(drop)
    }
}
