//! GitHub access behind one trait: `gh` CLI preferred, REST (`ureq`) with a
//! token as fallback, else exit 4 naming both options.

pub mod gh_cli;
pub mod rest;

use crate::config::GithubConfig;
use crate::error::{AppError, Result};
use crate::store::fsutil;
use chrono::{DateTime, Utc};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub closed: bool,
    pub labels: Vec<String>,
    pub updated_at: DateTime<Utc>,
}

pub trait GitHubApi: Send {
    /// `"gh"` or `"rest"`.
    fn kind(&self) -> &'static str;
    fn list_open_issues(&self, repo: &str, limit: usize) -> Result<Vec<Issue>>;
    fn view_issue(&self, repo: &str, number: u64) -> Result<Issue>;
    fn edit_title(&self, repo: &str, number: u64, title: &str) -> Result<()>;
    fn close(&self, repo: &str, number: u64) -> Result<()>;
    fn reopen(&self, repo: &str, number: u64) -> Result<()>;
}

pub struct Client {
    pub api: Box<dyn GitHubApi>,
    pub warnings: Vec<String>,
}

/// `gh` on PATH with `gh auth status` ok -> `GhCli`; else a token from
/// `GITHUB_TOKEN`, `GH_TOKEN`, `github.token_file` -> `Rest`; else exit 4.
pub fn select_client(cfg: &GithubConfig, env: &dyn Fn(&str) -> Option<String>) -> Result<Client> {
    if gh_cli::GhCli::available() {
        return Ok(Client {
            api: Box::new(gh_cli::GhCli),
            warnings: Vec::new(),
        });
    }
    let mut warnings = Vec::new();
    if let Some(token) = rest::find_token(cfg, env, &mut warnings)? {
        return Ok(Client {
            api: Box::new(rest::Rest::new(&cfg.api_url, token)),
            warnings,
        });
    }
    Err(AppError::External(
        "GitHub access unavailable: run `gh auth login` (gh CLI), or set GITHUB_TOKEN / GH_TOKEN, or github.token_file in config.toml".into(),
    ))
}

/// `owner/name` from an https or ssh GitHub remote URL.
pub fn parse_remote(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = if let Some(r) = url.strip_prefix("git@") {
        r.split_once(':')?.1
    } else if let Some(i) = url.find("://") {
        let after = &url[i + 3..];
        after.split_once('/')?.1
    } else {
        return None;
    };
    let rest = rest.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = rest.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    if owner.is_empty() || name.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("{owner}/{name}"))
}

/// `github.repo`, else the `origin` remote of the project key; else exit 3.
pub fn resolve_repo(cfg: &GithubConfig, key: &Path, config_path: &Path) -> Result<String> {
    let configured = cfg.repo.trim();
    if !configured.is_empty() {
        return Ok(configured.to_string());
    }
    let out = fsutil::command("git")
        .arg("-C")
        .arg(key)
        .args(["remote", "get-url", "origin"])
        .stderr(std::process::Stdio::null())
        .output();
    if let Ok(o) = out {
        if o.status.success() {
            if let Some(r) = parse_remote(&String::from_utf8_lossy(&o.stdout)) {
                return Ok(r);
            }
        }
    }
    Err(AppError::Config(format!(
        "no GitHub repo for {}: set github.repo = \"owner/name\" in {} or add an `origin` remote",
        key.display(),
        config_path.display()
    )))
}

pub(crate) fn parse_time(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| AppError::External(format!("bad GitHub timestamp `{s}`: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remotes() {
        for u in [
            "https://github.com/o/r.git",
            "https://github.com/o/r",
            "git@github.com:o/r.git",
            "ssh://git@github.com/o/r.git",
            "https://user@github.com/o/r/",
        ] {
            assert_eq!(parse_remote(u).as_deref(), Some("o/r"), "{u}");
        }
        assert_eq!(parse_remote("/local/path"), None);
    }
}
