//! GitHub REST fallback (`ureq`, blocking). The token only ever travels in the
//! `Authorization` header; errors name the URL path, never the token.

use super::{parse_time, GitHubApi, Issue};
use crate::config::GithubConfig;
use crate::error::{AppError, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

pub struct Rest {
    agent: ureq::Agent,
    base: String,
    token: String,
}

/// `GITHUB_TOKEN`, then `GH_TOKEN`, then the trimmed content of
/// `github.token_file` (warning if group/world-readable).
pub fn find_token(
    cfg: &GithubConfig,
    env: &dyn Fn(&str) -> Option<String>,
    warnings: &mut Vec<String>,
) -> Result<Option<String>> {
    for k in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if let Some(v) = env(k)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
        {
            return Ok(Some(v));
        }
    }
    let file = cfg.token_file.trim();
    if file.is_empty() {
        return Ok(None);
    }
    let path = match file.strip_prefix("~/") {
        Some(rest) => PathBuf::from(env("HOME").unwrap_or_default()).join(rest),
        None => PathBuf::from(file),
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|e| AppError::Config(format!("github.token_file {}: {e}", path.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(m) = std::fs::metadata(&path) {
            if m.permissions().mode() & 0o077 != 0 {
                warnings.push(format!(
                    "github.token_file {} is readable by group/others (chmod 600 it)",
                    path.display()
                ));
            }
        }
    }
    let token = text.trim().to_string();
    Ok((!token.is_empty()).then_some(token))
}

#[derive(Deserialize)]
struct RestLabel {
    name: String,
}

#[derive(Deserialize)]
struct RestIssue {
    number: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    state: String,
    #[serde(default)]
    labels: Vec<RestLabel>,
    updated_at: String,
    #[serde(default)]
    pull_request: Option<Value>,
}

impl RestIssue {
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

impl Rest {
    pub fn new(base: &str, token: String) -> Rest {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent("herdr-tasks")
            .build()
            .into();
        Rest {
            agent,
            base: base.trim_end_matches('/').to_string(),
            token,
        }
    }

    fn check(
        &self,
        method: &str,
        path: &str,
        mut resp: ureq::http::Response<ureq::Body>,
    ) -> Result<String> {
        let status = resp.status().as_u16();
        let header = |k: &str| {
            resp.headers()
                .get(k)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let remaining = header("x-ratelimit-remaining");
        let reset = header("x-ratelimit-reset");
        let body = resp.body_mut().read_to_string().unwrap_or_default();
        match status {
            200..=299 => Ok(body),
            401 => Err(AppError::External(format!(
                "GitHub REST {method} {path}: token rejected (401)"
            ))),
            403 | 429 if remaining.as_deref() == Some("0") || status == 429 => {
                let when = reset
                    .and_then(|r| r.parse::<i64>().ok())
                    .and_then(|r| chrono::DateTime::from_timestamp(r, 0))
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_else(|| "unknown".into());
                Err(AppError::External(format!(
                    "GitHub REST {method} {path}: rate limited ({status}), resets at {when}"
                )))
            }
            _ => {
                let msg = serde_json::from_str::<Value>(&body)
                    .ok()
                    .and_then(|v| {
                        v.get("message")
                            .and_then(|m| m.as_str())
                            .map(str::to_string)
                    })
                    .unwrap_or_default();
                Err(AppError::External(format!(
                    "GitHub REST {method} {path}: HTTP {status} {msg}"
                )))
            }
        }
    }

    fn get(&self, path: &str) -> Result<String> {
        let resp = self
            .agent
            .get(format!("{}{path}", self.base))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .call()
            .map_err(|e| {
                AppError::External(format!(
                    "GitHub REST GET {path}: {}",
                    redact(&e.to_string(), &self.token)
                ))
            })?;
        self.check("GET", path, resp)
    }

    fn patch(&self, path: &str, body: Value) -> Result<()> {
        let resp = self
            .agent
            .patch(format!("{}{path}", self.base))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send_json(&body)
            .map_err(|e| {
                AppError::External(format!(
                    "GitHub REST PATCH {path}: {}",
                    redact(&e.to_string(), &self.token)
                ))
            })?;
        self.check("PATCH", path, resp).map(drop)
    }
}

fn redact(s: &str, token: &str) -> String {
    if token.is_empty() {
        s.to_string()
    } else {
        s.replace(token, "***")
    }
}

impl GitHubApi for Rest {
    fn kind(&self) -> &'static str {
        "rest"
    }

    fn list_open_issues(&self, repo: &str, limit: usize) -> Result<Vec<Issue>> {
        let per_page = limit.clamp(1, 100);
        let mut out = Vec::new();
        for page in 1.. {
            let path = format!("/repos/{repo}/issues?state=open&per_page={per_page}&page={page}");
            let body = self.get(&path)?;
            let items: Vec<RestIssue> = serde_json::from_str(&body).map_err(|e| {
                AppError::External(format!("GitHub REST GET {path}: bad JSON: {e}"))
            })?;
            if items.is_empty() {
                break;
            }
            for it in items {
                if it.pull_request.is_none() && out.len() < limit {
                    out.push(it.into_issue()?);
                }
            }
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    fn view_issue(&self, repo: &str, number: u64) -> Result<Issue> {
        let path = format!("/repos/{repo}/issues/{number}");
        let body = self.get(&path)?;
        let it: RestIssue = serde_json::from_str(&body)
            .map_err(|e| AppError::External(format!("GitHub REST GET {path}: bad JSON: {e}")))?;
        it.into_issue()
    }

    fn edit_title(&self, repo: &str, number: u64, title: &str) -> Result<()> {
        self.patch(
            &format!("/repos/{repo}/issues/{number}"),
            json!({ "title": title }),
        )
    }

    fn close(&self, repo: &str, number: u64) -> Result<()> {
        self.patch(
            &format!("/repos/{repo}/issues/{number}"),
            json!({ "state": "closed" }),
        )
    }

    fn reopen(&self, repo: &str, number: u64) -> Result<()> {
        self.patch(
            &format!("/repos/{repo}/issues/{number}"),
            json!({ "state": "open" }),
        )
    }
}
