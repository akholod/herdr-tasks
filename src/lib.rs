//! herdr-tasks: a cross-project task board for herdr.
//!
//! One binary: `tui` is the panel, every other subcommand is the CLI. All
//! three writers (CLI, TUI, sync) go through `store`.

pub mod config;
pub mod error;
pub mod store;
