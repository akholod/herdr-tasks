//! Rendered-screen snapshots (ratatui TestBackend + insta).

mod support;

use herdr_tasks::cli::Surface;
use herdr_tasks::store::task::Priority;
use herdr_tasks::tui::app::{ImportReview, ReviewIssue};
use herdr_tasks::tui::model::{GroupKey, Grouping};
use insta::assert_snapshot;
use support::{draw, Env};

fn sample() -> Env {
    let env = Env::new(&["alpha", "beta", "gamma"]);
    env.create(
        0,
        "Fix dock width on narrow tabs",
        Priority::P0,
        "in-progress",
        false,
    );
    env.create(0, "Write README", Priority::P2, "todo", false);
    env.create(1, "Release 0.1", Priority::P1, "testing", true);
    env.create(1, "Add GitHub sync", Priority::P1, "done", false);
    env
}

#[test]
fn dock_priority() {
    let env = sample();
    let (mut app, _) = env.app(Surface::Dock);
    assert_snapshot!(draw(&mut app, 48, 16));
}

#[test]
fn dock_status() {
    let env = sample();
    let (mut app, _) = env.app(Surface::Dock);
    app.set_grouping(Grouping::Status);
    assert_snapshot!(draw(&mut app, 48, 16));
}

#[test]
fn dock_project_shows_empty_space() {
    let env = sample();
    let (mut app, _) = env.app(Surface::Dock);
    app.set_grouping(Grouping::Project);
    let s = draw(&mut app, 48, 16);
    assert!(s.contains("▾ gamma (0)"), "{s}");
    assert_snapshot!(s);
}

#[test]
fn collapsed_group() {
    let env = sample();
    let (mut app, _) = env.app(Surface::Dock);
    app.set_collapsed(GroupKey::Priority(Priority::P0), true);
    assert_snapshot!(draw(&mut app, 48, 12));
}

#[test]
fn status_width_99_is_list_100_is_kanban() {
    let env = sample();
    let (mut app, _) = env.app(Surface::Dock);
    app.set_grouping(Grouping::Status);
    let narrow = draw(&mut app, 99, 12);
    assert!(!narrow.contains("─────"), "{narrow}");
    let wide = draw(&mut app, 100, 12);
    assert!(
        wide.contains("todo (1)") && wide.contains("─────"),
        "{wide}"
    );
    assert_snapshot!("status_99", narrow);
    assert_snapshot!("status_100", wide);
}

#[test]
fn board_surface_is_wide() {
    let env = sample();
    let (mut app, _) = env.app(Surface::Board);
    app.set_grouping(Grouping::Status);
    assert!(draw(&mut app, 60, 12).contains("─────"));
}

#[test]
fn detail_with_attachments() {
    let env = sample();
    let t = env.create(2, "Collect logs", Priority::P1, "todo", false);
    let src = env.store.root.join("log.txt");
    std::fs::write(&src, vec![b'x'; 12_595]).unwrap();
    let img = env.store.root.join("shot.png");
    std::fs::write(&img, vec![0u8; 300]).unwrap();
    let p = env.project(2);
    let cfg = herdr_tasks::config::Config::default();
    herdr_tasks::store::task::attach_files(&p, &cfg, &t.meta.id, &[src, img]).unwrap();
    let (mut app, _) = env.app(Surface::Dock);
    app.set_grouping(Grouping::Project);
    let i = app
        .rows
        .iter()
        .position(|r| matches!(r, herdr_tasks::tui::model::Row::Task(x) if x.folder == p.folder))
        .unwrap();
    app.select_row(i);
    app.open_detail();
    let s = draw(&mut app, 48, 30);
    assert!(s.contains("attachments (2)"), "{s}");
    assert!(s.contains("log.txt  12.3 KB"), "{s}");
    assert_snapshot!(s);
}

#[test]
fn broken_task_row() {
    let env = sample();
    let t = env.create(0, "Will break", Priority::P1, "todo", false);
    std::fs::write(t.dir.join("task.toml"), "id = \"t-0003\"\ntitle = \n").unwrap();
    let (mut app, _) = env.app(Surface::Dock);
    let s = draw(&mut app, 48, 16);
    assert!(s.contains("t-0003 ! invalid task.toml"), "{s}");
    assert!(s.contains("Write README"), "{s}");
    assert_snapshot!(s);
}

#[test]
fn broken_config_banner() {
    let env = sample();
    std::fs::write(env.store.root.join("config.toml"), "kanban_min_width = [\n").unwrap();
    let (mut app, _) = env.app(Surface::Dock);
    let s = draw(&mut app, 48, 10);
    assert!(s.contains("config.toml"), "{s}");
    assert!(s.contains("Fix dock"), "still renders tasks: {s}");
    let banner_line = s.lines().nth(1).unwrap().to_string();
    assert!(banner_line.starts_with("! "), "{s}");
}

#[test]
fn title_prompt() {
    let env = sample();
    let (mut app, _) = env.app(Surface::Dock);
    app.start_prompt();
    app.prompt.as_mut().unwrap().buffer = "Fix login".into();
    assert_snapshot!(draw(&mut app, 48, 10));
}

#[test]
fn import_review() {
    let env = sample();
    let (mut app, _) = env.app(Surface::Dock);
    app.open_review(ImportReview {
        label: "alpha".into(),
        candidates: vec![
            (
                ReviewIssue {
                    number: 12,
                    title: "Crash on start".into(),
                    labels: vec!["bug".into()],
                },
                true,
            ),
            (
                ReviewIssue {
                    number: 15,
                    title: "Docs typo".into(),
                    labels: vec![],
                },
                false,
            ),
        ],
        ignored: vec![ReviewIssue {
            number: 9,
            title: "Old idea".into(),
            labels: vec![],
        }],
        ignored_open: true,
        cursor: 0,
    });
    assert_snapshot!(draw(&mut app, 48, 14));
}
