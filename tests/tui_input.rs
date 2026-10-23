//! Keys and mouse drive the same App methods (AC-2b/2c, 3b-3e, 6g).

mod support;

use herdr_tasks::cli::Surface;
use herdr_tasks::store::task::{self, Priority};
use herdr_tasks::tui::app::Chip;
use herdr_tasks::tui::app::{ImportReview, ReviewIssue};
use herdr_tasks::tui::keys::handle_key;
use herdr_tasks::tui::model::{GroupKey, Grouping, Row};
use herdr_tasks::tui::mouse::{handle_mouse, Zone};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use support::{draw, Env};

fn key(app: &mut herdr_tasks::tui::app::App, c: KeyCode) {
    handle_key(app, KeyEvent::new(c, KeyModifiers::NONE));
}

fn typ(app: &mut herdr_tasks::tui::app::App, s: &str) {
    for c in s.chars() {
        key(app, KeyCode::Char(c));
    }
}

/// Render, find the zone's rect, click its first cell.
fn click(app: &mut herdr_tasks::tui::app::App, z: &Zone, w: u16, h: u16) {
    draw(app, w, h);
    let r = app
        .hits
        .find(z)
        .unwrap_or_else(|| panic!("zone {z:?} not rendered"));
    handle_mouse(
        app,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: r.x,
            row: r.y,
            modifiers: KeyModifiers::NONE,
        },
    );
}

fn env() -> Env {
    let e = Env::new(&["alpha", "beta"]);
    e.create(0, "First", Priority::P1, "todo", false);
    e.create(0, "Second", Priority::P0, "todo", false);
    e
}

fn select_task(app: &mut herdr_tasks::tui::app::App, title: &str) {
    let i = app
        .rows
        .iter()
        .position(|r| match r {
            Row::Task(x) => app
                .entry(x)
                .and_then(|e| e.task())
                .is_some_and(|t| t.meta.title == title),
            _ => false,
        })
        .unwrap();
    app.select_row(i);
}

#[test]
fn tab_key_and_tab_click_cycle_grouping() {
    let e = env();
    let (mut app, _) = e.app(Surface::Dock);
    assert_eq!(app.grouping, Grouping::Priority);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.grouping, Grouping::Status);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.grouping, Grouping::Project);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.grouping, Grouping::Priority);
    click(&mut app, &Zone::GroupingTab(Grouping::Project), 48, 20);
    assert_eq!(app.grouping, Grouping::Project);
    click(&mut app, &Zone::GroupingTab(Grouping::Status), 48, 20);
    assert_eq!(app.grouping, Grouping::Status);
}

#[test]
fn collapse_by_keys_space_and_click() {
    let e = env();
    let (mut app, _) = e.app(Surface::Dock);
    let p0 = GroupKey::Priority(Priority::P0);
    select_task(&mut app, "Second");
    key(&mut app, KeyCode::Char('h'));
    assert!(app.collapsed.contains(&p0));
    key(&mut app, KeyCode::Char('l'));
    assert!(!app.collapsed.contains(&p0));
    app.goto_top();
    key(&mut app, KeyCode::Char(' '));
    assert!(app.collapsed.contains(&p0));
    key(&mut app, KeyCode::Char(' '));
    assert!(!app.collapsed.contains(&p0));
    click(&mut app, &Zone::GroupHeader(p0.clone()), 48, 20);
    assert!(app.collapsed.contains(&p0));
    assert!(!draw(&mut app, 48, 20).contains("Second"));
    click(&mut app, &Zone::GroupHeader(p0.clone()), 48, 20);
    assert!(!app.collapsed.contains(&p0));
    key(&mut app, KeyCode::Char('C'));
    assert_eq!(
        app.rows
            .iter()
            .filter(|r| matches!(r, Row::Task(_)))
            .count(),
        0
    );
    key(&mut app, KeyCode::Char('C'));
    assert!(app.collapsed.is_empty());
}

#[test]
fn attachment_opens_with_open_command_by_key_and_click() {
    let e = env();
    let p = e.project(0);
    std::fs::write(
        p.dir.join("config.toml"),
        "open_command = [\"/bin/echo\"]\n",
    )
    .unwrap();
    let src = e.store.root.join("a.txt");
    std::fs::write(&src, "hi").unwrap();
    task::attach_files(
        &p,
        &herdr_tasks::config::Config::default(),
        "t-0001",
        &[src.clone(), src],
    )
    .unwrap();
    let (mut app, opened) = e.app(Surface::Dock);
    select_task(&mut app, "First");
    key(&mut app, KeyCode::Enter);
    assert!(app.detail_open);
    key(&mut app, KeyCode::Enter);
    {
        let o = opened.lock().unwrap();
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].0, vec!["/bin/echo".to_string()]);
        assert!(o[0].1.is_absolute());
        assert!(
            o[0].1.ends_with("t-0001/attachments/a-1.txt")
                || o[0].1.ends_with("t-0001/attachments/a.txt")
        );
    }
    click(&mut app, &Zone::AttachmentRow(1), 48, 30);
    let o = opened.lock().unwrap();
    assert_eq!(o.len(), 2);
    assert!(o[1].1.ends_with("attachments/a.txt"), "{:?}", o[1].1);
}

#[test]
fn details_hidden_until_i_or_chip() {
    let e = env();
    let p = e.project(0);
    let cfg = herdr_tasks::config::Config::default();
    task::update_task(
        &p,
        &cfg,
        "1",
        task::TaskPatch {
            details: Some("SECRET CONTEXT".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let (mut app, _) = e.app(Surface::Dock);
    select_task(&mut app, "First");
    key(&mut app, KeyCode::Enter);
    assert!(!draw(&mut app, 48, 30).contains("SECRET CONTEXT"));
    key(&mut app, KeyCode::Char('i'));
    assert!(draw(&mut app, 48, 30).contains("SECRET CONTEXT"));
    key(&mut app, KeyCode::Char('i'));
    assert!(!draw(&mut app, 48, 30).contains("SECRET CONTEXT"));
    click(&mut app, &Zone::Chip(Chip::Details), 48, 30);
    assert!(draw(&mut app, 48, 30).contains("SECRET CONTEXT"));
}

fn meta(e: &Env, id: &str) -> task::TaskMeta {
    task::read_meta(&e.project(0).task_dir(id)).unwrap().0
}

#[test]
fn status_priority_pin_by_keys_and_chips() {
    let e = env();
    let (mut app, _) = e.app(Surface::Dock);
    select_task(&mut app, "First");
    let before = meta(&e, "t-0001");
    key(&mut app, KeyCode::Char('s'));
    let m = meta(&e, "t-0001");
    assert_eq!(m.status, "in-progress");
    assert!(m.updated > before.updated);
    let bumped = |e: &Env, prev: &mut chrono::DateTime<chrono::Utc>| {
        let u = meta(e, "t-0001").updated;
        assert!(u > *prev, "updated not bumped");
        *prev = u;
    };
    let mut last = m.updated;
    key(&mut app, KeyCode::Char('S'));
    assert_eq!(meta(&e, "t-0001").status, "todo");
    bumped(&e, &mut last);
    key(&mut app, KeyCode::Char('p'));
    assert_eq!(meta(&e, "t-0001").priority, Priority::P2);
    bumped(&e, &mut last);
    key(&mut app, KeyCode::Char('t'));
    assert!(meta(&e, "t-0001").pinned);
    bumped(&e, &mut last);
    // Selection follows the task into the Pinned group.
    assert_eq!(app.selected_task().unwrap().meta.title, "First");
    app.open_detail();
    let u0 = meta(&e, "t-0001").updated;
    click(&mut app, &Zone::Chip(Chip::Status), 48, 30);
    assert_eq!(meta(&e, "t-0001").status, "in-progress");
    bumped(&e, &mut last);
    click(&mut app, &Zone::Chip(Chip::Priority), 48, 30);
    assert_eq!(meta(&e, "t-0001").priority, Priority::P0);
    bumped(&e, &mut last);
    click(&mut app, &Zone::Chip(Chip::Pin), 48, 30);
    let m = meta(&e, "t-0001");
    assert!(!m.pinned);
    assert!(m.updated > u0);
}

fn entries(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn n_creates_task_in_selected_project() {
    let e = env();
    let (mut app, _) = e.app(Surface::Dock);
    select_task(&mut app, "First");
    key(&mut app, KeyCode::Char('n'));
    typ(&mut app, "Fix login");
    key(&mut app, KeyCode::Enter);
    assert!(app.prompt.is_none());
    let dir = e.project(0).task_dir("t-0003");
    assert_eq!(
        entries(&dir),
        vec!["attachments", "description.md", "details.md", "task.toml"]
    );
    let m = meta(&e, "t-0003");
    assert_eq!(
        (m.title.as_str(), m.status.as_str(), m.priority),
        ("Fix login", "todo", Priority::P1)
    );
    assert_eq!(app.selected_task().unwrap().meta.id, "t-0003");
}

#[test]
fn n_on_empty_space_group_creates_there() {
    let e = env();
    let (mut app, _) = e.app(Surface::Dock);
    app.set_grouping(Grouping::Project);
    let beta = e.project(1);
    assert!(!beta.dir.exists());
    let i = app
        .rows
        .iter()
        .position(|r| {
            r.id() == herdr_tasks::tui::model::RowId::Header(GroupKey::Project(beta.folder.clone()))
        })
        .unwrap();
    app.select_row(i);
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(app.prompt.as_ref().unwrap().label, "beta");
    typ(&mut app, "First in beta");
    key(&mut app, KeyCode::Enter);
    assert_eq!(entries(&beta.task_dir("t-0001")).len(), 4);
    assert!(e
        .store
        .registry()
        .unwrap()
        .values()
        .any(|f| f == &beta.folder));
}

#[test]
fn n_then_esc_creates_nothing_and_empty_title_rejected() {
    let e = env();
    let (mut app, _) = e.app(Surface::Dock);
    key(&mut app, KeyCode::Char('n'));
    key(&mut app, KeyCode::Enter);
    assert!(app.prompt.as_ref().unwrap().error.is_some());
    typ(&mut app, "abc");
    key(&mut app, KeyCode::Esc);
    assert!(app.prompt.is_none());
    assert_eq!(entries(&e.project(0).tasks_dir()).len(), 2);
}

#[test]
fn esc_closes_detail_and_never_quits() {
    let e = env();
    let (mut app, _) = e.app(Surface::Dock);
    select_task(&mut app, "First");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Esc);
    assert!(!app.detail_open);
    key(&mut app, KeyCode::Esc);
    assert!(!app.should_quit);
    key(&mut app, KeyCode::Char('q'));
    assert!(app.should_quit);
}

fn review() -> ImportReview {
    ImportReview {
        label: "alpha".into(),
        candidates: vec![
            (
                ReviewIssue {
                    number: 12,
                    title: "a".into(),
                    labels: vec![],
                },
                true,
            ),
            (
                ReviewIssue {
                    number: 15,
                    title: "b".into(),
                    labels: vec![],
                },
                true,
            ),
        ],
        ignored: vec![ReviewIssue {
            number: 9,
            title: "c".into(),
            labels: vec![],
        }],
        ignored_open: false,
        cursor: 0,
    }
}

fn ignored(e: &Env) -> Vec<u64> {
    herdr_tasks::sync::state::load(&e.project(0))
        .unwrap()
        .ignored_issues
}

#[test]
fn review_keys_space_a_u_enter() {
    let e = env();
    let (mut app, _) = e.app(Surface::Dock);
    select_task(&mut app, "First");
    app.open_review(review());
    key(&mut app, KeyCode::Char(' '));
    assert!(!app.review.as_ref().unwrap().candidates[0].1);
    key(&mut app, KeyCode::Char('a'));
    assert!(app.review.as_ref().unwrap().candidates.iter().all(|c| c.1));
    key(&mut app, KeyCode::Char('a'));
    assert!(app.review.as_ref().unwrap().candidates.iter().all(|c| !c.1));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char(' ')); // expand "Пропущенные"
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('u'));
    let r = app.review.as_ref().unwrap();
    assert!(r.ignored.is_empty());
    assert_eq!(r.candidates.last().unwrap().0.number, 9);
    key(&mut app, KeyCode::Enter);
    assert!(app.review.is_none());
    // #12 unchecked -> ignored; #15 and #9 checked -> not ignored.
    assert_eq!(ignored(&e), vec![12]);
}

#[test]
fn review_esc_writes_nothing() {
    let e = env();
    let (mut app, _) = e.app(Surface::Dock);
    select_task(&mut app, "First");
    app.open_review(review());
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Esc);
    assert!(app.review.is_none());
    assert!(!herdr_tasks::sync::state::path(&e.project(0)).exists());
}

#[test]
fn review_mouse_rows_and_chips() {
    let e = env();
    let (mut app, _) = e.app(Surface::Dock);
    select_task(&mut app, "First");
    app.open_review(review());
    click(&mut app, &Zone::ReviewRow(1), 48, 20);
    assert!(!app.review.as_ref().unwrap().candidates[1].1);
    click(&mut app, &Zone::ReviewCancel, 48, 20);
    assert!(app.review.is_none());
    app.open_review(review());
    click(&mut app, &Zone::ReviewRow(0), 48, 20);
    click(&mut app, &Zone::ReviewImport, 48, 20);
    assert_eq!(ignored(&e), vec![12]);
}

#[test]
fn kanban_header_click_collapses_column() {
    let e = env();
    let (mut app, _) = e.app(Surface::Board);
    app.set_grouping(Grouping::Status);
    click(&mut app, &Zone::KanbanHeader("todo".into()), 100, 20);
    assert!(app.collapsed.contains(&GroupKey::Status("todo".into())));
    assert!(draw(&mut app, 100, 20).contains("▸2"));
}

#[test]
fn task_row_click_selects_and_opens_detail() {
    let e = env();
    let (mut app, _) = e.app(Surface::Dock);
    draw(&mut app, 48, 20);
    let target = app
        .rows
        .iter()
        .find_map(|r| match r {
            Row::Task(x) if x.id == "t-0001" => Some(x.clone()),
            _ => None,
        })
        .unwrap();
    click(&mut app, &Zone::TaskRow(target), 48, 20);
    assert!(app.detail_open);
    assert_eq!(app.selected_task().unwrap().meta.title, "First");
    click(&mut app, &Zone::DetailClose, 48, 20);
    assert!(!app.detail_open);
}

#[test]
fn cli_change_shows_after_reload_of_project() {
    let e = env();
    let (mut app, _) = e.app(Surface::Dock);
    task::move_task(
        &e.project(0),
        &herdr_tasks::config::Config::default(),
        "t-0001",
        "done",
    )
    .unwrap();
    assert!(app.reload_project(&e.project(0).folder));
    app.set_grouping(Grouping::Status);
    let s = draw(&mut app, 48, 20);
    let done = s.find("▾ done").unwrap();
    assert!(s[done..].contains("First"), "{s}");
}

#[test]
fn reload_all_picks_up_global_config() {
    let e = env();
    let (mut app, _) = e.app(Surface::Dock);
    std::fs::write(
        e.store.root.join("config.toml"),
        "statuses = [\"open\", \"closed\"]\n",
    )
    .unwrap();
    app.reload_all();
    let pv = app.project(&e.project(0).folder).unwrap();
    assert_eq!(pv.cfg.statuses, vec!["open", "closed"]);
    assert_eq!(app.global_cfg.statuses, vec!["open", "closed"]);
}

#[test]
fn n_with_nothing_selected_targets_focused_space() {
    let e = Env::new(&["alpha", "beta"]);
    let mut d = e.discovery();
    d.focused_key = Some(e.spaces[1].0.clone());
    let mut app = herdr_tasks::tui::app::App::new(herdr_tasks::tui::app::Deps {
        store: e.store.clone(),
        surface: Surface::Dock,
        discover: Box::new(move || Some(d.clone())),
        opener: Box::new(herdr_tasks::tui::app::SystemOpener),
    });
    assert!(app.selected_ref().is_none());
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(app.prompt.as_ref().unwrap().label, "beta");
    typ(&mut app, "Focused");
    key(&mut app, KeyCode::Enter);
    assert!(e.project(1).task_dir("t-0001").join("task.toml").exists());
}
