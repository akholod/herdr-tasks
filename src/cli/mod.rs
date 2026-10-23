//! Command line: clap definitions and the exit-code contract
//! (0 ok, 1 validation, 2 not found, 3 config/storage, 4 external, 64 usage).

pub mod commands;
pub mod output;

use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use crate::error::AppError;
use crate::store::task::Priority;

#[derive(Debug, Parser)]
#[command(
    name = "herdr-tasks",
    version,
    about = "Cross-project task board for herdr (CLI + TUI)",
    after_help = "Project = git root of the current directory (or of --project), else the directory itself."
)]
pub struct Cli {
    /// Print one JSON document on stdout (errors as JSON on stderr)
    #[arg(long, global = true)]
    pub json: bool,
    /// Act on the project of PATH instead of the current directory
    #[arg(long, global = true, value_name = "PATH")]
    pub project: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Command,
}

fn parse_priority(s: &str) -> Result<Priority, String> {
    s.parse::<Priority>().map_err(|e| e.to_string())
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a task
    Create(CreateArgs),
    /// List tasks of the project (or of all projects)
    List(ListArgs),
    /// Show one task
    Show {
        id: String,
        /// Include details.md
        #[arg(long)]
        details: bool,
    },
    /// Change title, description, details, priority, status or tags
    Update(UpdateArgs),
    /// Set the status of a task
    Move { id: String, status: String },
    /// Pin a task (or unpin with --off)
    Pin {
        id: String,
        #[arg(long)]
        off: bool,
    },
    /// Copy files into the task's attachments/
    Attach {
        id: String,
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Sync with GitHub issues (per github.mode)
    Sync(SyncArgs),
    /// Print the agent skill (SKILL.md)
    Skill,
    /// Show the resolved project, storage root and effective config
    Project,
    /// Run the panel
    Tui {
        #[arg(long, value_enum, default_value_t = Surface::Dock)]
        surface: Surface,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Surface {
    Dock,
    Board,
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    pub title: String,
    /// Short description (limited by description_limit)
    #[arg(short = 'd', long)]
    pub description: Option<String>,
    /// Detailed context for agents (details.md, hidden in the panel)
    #[arg(long, conflicts_with = "details_file")]
    pub details: Option<String>,
    /// Read details.md content from a file, or `-` for stdin
    #[arg(long, value_name = "PATH|-")]
    pub details_file: Option<String>,
    #[arg(long, value_parser = parse_priority)]
    pub priority: Option<Priority>,
    #[arg(long)]
    pub status: Option<String>,
    #[arg(long = "tag", value_name = "TAG")]
    pub tags: Vec<String>,
    #[arg(long)]
    pub pin: bool,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    #[arg(long)]
    pub status: Option<String>,
    #[arg(long, value_parser = parse_priority)]
    pub priority: Option<Priority>,
    #[arg(long)]
    pub tag: Option<String>,
    /// Every registered project
    #[arg(long)]
    pub all_projects: bool,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    pub id: String,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(short = 'd', long)]
    pub description: Option<String>,
    #[arg(long, conflicts_with_all = ["details_file", "append_details"])]
    pub details: Option<String>,
    #[arg(long, value_name = "PATH|-", conflicts_with = "append_details")]
    pub details_file: Option<String>,
    /// Append a paragraph to details.md
    #[arg(long, value_name = "TEXT")]
    pub append_details: Option<String>,
    #[arg(long, value_parser = parse_priority)]
    pub priority: Option<Priority>,
    #[arg(long)]
    pub status: Option<String>,
    #[arg(long = "tag", value_name = "TAG")]
    pub tags: Vec<String>,
    #[arg(long = "untag", value_name = "TAG")]
    pub untags: Vec<String>,
}

#[derive(Debug, Args)]
#[command(group(ArgGroup::new("op").multiple(false)))]
pub struct SyncArgs {
    /// Show the plan, write nothing
    #[arg(long, group = "op")]
    pub dry_run: bool,
    /// List new (unlinked, not ignored) open issues; no writes
    #[arg(long, group = "op")]
    pub list_new: bool,
    /// Import exactly these issues (removes them from ignored)
    #[arg(long, group = "op", num_args = 1.., value_name = "N")]
    pub import: Vec<u64>,
    /// Never offer these issues again
    #[arg(long, group = "op", num_args = 1.., value_name = "N")]
    pub ignore: Vec<u64>,
    /// Offer these issues again
    #[arg(long, group = "op", num_args = 1.., value_name = "N")]
    pub unignore: Vec<u64>,
}

/// Parse, run, print; returns the process exit code.
pub fn main() -> i32 {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let json = args.iter().any(|a| a == "--json");
    let cli = match Cli::try_parse_from(&args) {
        Ok(c) => c,
        Err(e) => {
            use clap::error::ErrorKind;
            if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
                let _ = e.print();
                return 0;
            }
            if json {
                let msg = e.kind().as_str().unwrap_or("invalid usage").to_string();
                let detail = e.to_string();
                let first = detail
                    .lines()
                    .next()
                    .unwrap_or(&msg)
                    .trim_start_matches("error: ");
                output::print_error(&AppError::Usage(first.to_string()), true);
            } else {
                let _ = e.print();
            }
            return 64;
        }
    };
    match commands::run(cli) {
        Ok(()) => 0,
        Err(e) => {
            output::print_error(&e, json);
            e.code()
        }
    }
}
