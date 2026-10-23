//! Configuration: built-in defaults < global `<root>/config.toml` <
//! project `<root>/projects/<folder>/config.toml`.

use crate::error::{AppError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SyncMode {
    #[default]
    Off,
    Pull,
    Push,
    TwoWay,
}

impl SyncMode {
    pub fn as_str(self) -> &'static str {
        match self {
            SyncMode::Off => "off",
            SyncMode::Pull => "pull",
            SyncMode::Push => "push",
            SyncMode::TwoWay => "two-way",
        }
    }

    pub fn pulls(self) -> bool {
        matches!(self, SyncMode::Pull | SyncMode::TwoWay)
    }

    pub fn pushes(self) -> bool {
        matches!(self, SyncMode::Push | SyncMode::TwoWay)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GithubConfig {
    pub mode: SyncMode,
    pub repo: String,
    pub import_limit: usize,
    pub token_file: String,
    pub api_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Config {
    pub description_limit: usize,
    pub statuses: Vec<String>,
    pub kanban_min_width: u16,
    pub open_command: Vec<String>,
    pub github: GithubConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            description_limit: 500,
            statuses: ["todo", "in-progress", "testing", "done"]
                .map(String::from)
                .to_vec(),
            kanban_min_width: 100,
            open_command: Vec::new(),
            github: GithubConfig {
                mode: SyncMode::Off,
                repo: String::new(),
                import_limit: 200,
                token_file: String::new(),
                api_url: "https://api.github.com".into(),
            },
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct PartialGithub {
    pub mode: Option<SyncMode>,
    pub repo: Option<String>,
    pub import_limit: Option<usize>,
    pub token_file: Option<String>,
    pub api_url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct PartialConfig {
    pub description_limit: Option<usize>,
    pub statuses: Option<Vec<String>>,
    pub kanban_min_width: Option<u16>,
    pub open_command: Option<Vec<String>>,
    pub github: Option<PartialGithub>,
}

impl Config {
    /// First status: default for new tasks and for reopened issues.
    pub fn first_status(&self) -> &str {
        &self.statuses[0]
    }

    /// Last status: "done", maps to a closed GitHub issue.
    pub fn done_status(&self) -> &str {
        self.statuses.last().map(String::as_str).unwrap_or("done")
    }

    pub fn merge(mut self, p: PartialConfig) -> Config {
        if let Some(v) = p.description_limit {
            self.description_limit = v;
        }
        if let Some(v) = p.statuses {
            self.statuses = v;
        }
        if let Some(v) = p.kanban_min_width {
            self.kanban_min_width = v;
        }
        if let Some(v) = p.open_command {
            self.open_command = v;
        }
        if let Some(g) = p.github {
            let gh = &mut self.github;
            if let Some(v) = g.mode {
                gh.mode = v;
            }
            if let Some(v) = g.repo {
                gh.repo = v;
            }
            if let Some(v) = g.import_limit {
                gh.import_limit = v;
            }
            if let Some(v) = g.token_file {
                gh.token_file = v;
            }
            if let Some(v) = g.api_url {
                gh.api_url = v;
            }
        }
        self
    }

    /// Strict load for the CLI: any broken layer is an error (exit 3).
    pub fn load(root: &Path, folder: Option<&str>) -> Result<Config> {
        let mut cfg = Config::default();
        for path in layer_paths(root, folder) {
            if let Some(p) = read_partial(&path)? {
                cfg = cfg.merge(p);
                cfg.validate(&path)?;
            }
        }
        Ok(cfg)
    }

    /// Soft load for the TUI: a broken layer is skipped and reported, the
    /// remaining layers still apply.
    pub fn load_soft(root: &Path, folder: Option<&str>) -> (Config, Vec<String>) {
        let mut cfg = Config::default();
        let mut errors = Vec::new();
        for path in layer_paths(root, folder) {
            match read_partial(&path) {
                Ok(Some(p)) => {
                    let merged = cfg.clone().merge(p);
                    match merged.validate(&path) {
                        Ok(()) => cfg = merged,
                        Err(e) => errors.push(e.to_string()),
                    }
                }
                Ok(None) => {}
                Err(e) => errors.push(e.to_string()),
            }
        }
        (cfg, errors)
    }

    fn validate(&self, path: &Path) -> Result<()> {
        if self.statuses.is_empty() {
            return Err(AppError::Config(format!(
                "{}: `statuses` must not be empty",
                path.display()
            )));
        }
        if self.description_limit == 0 {
            return Err(AppError::Config(format!(
                "{}: `description_limit` must be > 0",
                path.display()
            )));
        }
        Ok(())
    }
}

pub fn global_path(root: &Path) -> PathBuf {
    root.join("config.toml")
}

pub fn project_path(root: &Path, folder: &str) -> PathBuf {
    root.join("projects").join(folder).join("config.toml")
}

fn layer_paths(root: &Path, folder: Option<&str>) -> Vec<PathBuf> {
    let mut v = vec![global_path(root)];
    if let Some(f) = folder {
        v.push(project_path(root, f));
    }
    v
}

fn read_partial(path: &Path) -> Result<Option<PartialConfig>> {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(AppError::io(path, e)),
    };
    toml::from_str::<PartialConfig>(&src)
        .map(Some)
        .map_err(|e| AppError::toml(path, &src, &e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, s: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, s).unwrap();
    }

    #[test]
    fn project_overrides_global() {
        let d = tempfile::tempdir().unwrap();
        write(&global_path(d.path()), "description_limit = 300\n");
        write(&project_path(d.path(), "p"), "description_limit = 400\n");
        assert_eq!(
            Config::load(d.path(), Some("p")).unwrap().description_limit,
            400
        );
        assert_eq!(
            Config::load(d.path(), Some("q")).unwrap().description_limit,
            300
        );
        let e = tempfile::tempdir().unwrap();
        assert_eq!(
            Config::load(e.path(), Some("p")).unwrap().description_limit,
            500
        );
    }

    #[test]
    fn github_table_merges_field_by_field() {
        let d = tempfile::tempdir().unwrap();
        write(&global_path(d.path()), "[github]\nrepo = \"o/r\"\n");
        write(
            &project_path(d.path(), "p"),
            "[github]\nmode = \"two-way\"\n",
        );
        let c = Config::load(d.path(), Some("p")).unwrap();
        assert_eq!(c.github.mode, SyncMode::TwoWay);
        assert_eq!(c.github.repo, "o/r");
        assert_eq!(c.github.import_limit, 200);
    }

    #[test]
    fn invalid_toml_names_file_line_and_column() {
        let d = tempfile::tempdir().unwrap();
        let p = project_path(d.path(), "p");
        write(&p, "description_limit = 10\nstatuses = [\n");
        let err = Config::load(d.path(), Some("p")).unwrap_err();
        assert_eq!(err.code(), 3);
        let msg = err.to_string();
        assert!(msg.contains(&p.display().to_string()), "{msg}");
        assert!(msg.contains("line 3, column 1"), "{msg}");
    }

    #[test]
    fn soft_load_skips_broken_layer() {
        let d = tempfile::tempdir().unwrap();
        write(&global_path(d.path()), "kanban_min_width = 80\n");
        write(&project_path(d.path(), "p"), "kanban_min_width = \n");
        let (c, errs) = Config::load_soft(d.path(), Some("p"));
        assert_eq!(c.kanban_min_width, 80);
        assert_eq!(errs.len(), 1);
    }
}
