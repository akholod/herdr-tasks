//! AC-6g through the real TUI sync worker (fake `gh` on PATH). One test per
//! binary: it sets process-wide env vars.

mod support;

use herdr_tasks::cli::Surface;
use std::time::{Duration, Instant};
use support::cli_env::{issue, CliEnv};
use support::Env;

fn pump(app: &mut herdr_tasks::tui::app::App, until: impl Fn(&herdr_tasks::tui::app::App) -> bool) {
    let start = Instant::now();
    while !until(app) {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "timed out; status = {:?}",
            app.status
        );
        app.poll_sync();
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn review_overlay_imports_checked_and_remembers_unchecked() {
    let gh = CliEnv::new();
    gh.set_issues(&[
        issue(1, "Skip me", "a", &[], "2025-12-31T10:00:00Z"),
        issue(2, "Import me", "b", &["bug"], "2025-12-31T11:00:00Z"),
    ]);
    std::env::set_var("PATH", format!("{}:/usr/bin:/bin", gh.bin.display()));
    std::env::set_var("FAKE_GH_DIR", &gh.gh_dir);
    std::env::set_var("FAKE_GH_LOG", &gh.log);
    std::env::remove_var("GITHUB_TOKEN");
    std::env::remove_var("GH_TOKEN");

    let env = Env::new(&["alpha"]);
    let p = env.project(0);
    std::fs::create_dir_all(&p.dir).unwrap();
    std::fs::write(
        p.dir.join("config.toml"),
        "[github]\nmode = \"pull\"\nrepo = \"o/r\"\n",
    )
    .unwrap();
    let (mut app, _) = env.app(Surface::Dock);

    app.start_sync();
    assert!(app.syncing);
    pump(&mut app, |a| a.review.is_some());
    {
        let r = app.review.as_ref().unwrap();
        let nums: Vec<u64> = r.candidates.iter().map(|c| c.0.number).collect();
        assert_eq!(nums, vec![1, 2]);
    }
    app.review_toggle(Some(0)); // uncheck #1
    app.review_finish(true);
    pump(&mut app, |a| !a.syncing);
    assert!(!app.status.as_ref().unwrap().1, "{:?}", app.status);
    app.reload_all();
    let titles: Vec<String> = app.projects[0]
        .entries
        .iter()
        .filter_map(|e| e.task())
        .map(|t| t.meta.title.clone())
        .collect();
    assert_eq!(titles, vec!["Import me"]);
    assert_eq!(
        herdr_tasks::sync::state::load(&p).unwrap().ignored_issues,
        vec![1]
    );
    let log = gh.take_log();
    assert!(support::cli_env::write_calls(&log).is_empty(), "{log:?}");

    // Next R: #1 is ignored, #2 linked -> no overlay, nothing imported.
    app.start_sync();
    pump(&mut app, |a| !a.syncing || a.review.is_some());
    assert!(app.review.is_none());
    pump(&mut app, |a| !a.syncing);
    assert!(
        app.status.as_ref().unwrap().0.contains("imported 0"),
        "{:?}",
        app.status
    );

    // Esc on the overlay writes no ignores.
    let mut iss = gh.issues();
    iss.push(issue(3, "Third", "c", &[], "2025-12-31T12:00:00Z"));
    gh.set_issues(&iss);
    app.start_sync();
    pump(&mut app, |a| a.review.is_some());
    app.review_finish(false);
    pump(&mut app, |a| !a.syncing);
    assert_eq!(
        herdr_tasks::sync::state::load(&p).unwrap().ignored_issues,
        vec![1]
    );
    assert_eq!(app.projects[0].entries.len(), 1);
}
