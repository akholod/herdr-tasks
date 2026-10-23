//! AC-5a: a store write reaches the watcher within 1 s (native and poll).

use herdr_tasks::config::Config;
use herdr_tasks::store::task::{self, NewTask};
use herdr_tasks::store::Store;
use herdr_tasks::watch::{self, WatchEvent, WatchMode};
use std::path::Path;
use std::time::{Duration, Instant};

fn wait_for(w: &watch::StoreWatcher, want: &WatchEvent, budget: Duration) -> Option<Duration> {
    let start = Instant::now();
    while start.elapsed() < budget {
        if let Ok(e) = w.rx.recv_timeout(Duration::from_millis(20)) {
            if &e == want {
                return Some(start.elapsed());
            }
        }
    }
    None
}

fn drain(w: &watch::StoreWatcher) {
    std::thread::sleep(Duration::from_millis(400));
    while w.rx.try_recv().is_ok() {}
}

fn check(force_poll: bool) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("store");
    let w = watch::start(&root, force_poll).unwrap();
    assert!(root.join("projects").is_dir(), "projects/ created on start");
    if force_poll {
        assert_eq!(w.mode, WatchMode::Poll);
    }
    let store = Store::at(&root);
    let project = store.project_for_key(Path::new("/x/proj"));
    let cfg = Config::default();
    if force_poll {
        // Let the poll watcher take its initial scan first.
        std::thread::sleep(Duration::from_millis(600));
    }
    // First task of a new project: new folder + projects.toml.
    task::create_task(
        &store,
        &project,
        &cfg,
        NewTask {
            title: "a".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let t = wait_for(&w, &WatchEvent::Root, Duration::from_secs(3)).expect("RootChanged");
    assert!(t < Duration::from_millis(1000), "root event after {t:?}");
    drain(&w);
    let start = Instant::now();
    task::move_task(&project, &cfg, "t-0001", "done").unwrap();
    let t = wait_for(
        &w,
        &WatchEvent::Project(project.folder.clone()),
        Duration::from_secs(3),
    )
    .expect("ProjectChanged");
    assert!(
        start.elapsed() < Duration::from_millis(1000),
        "project event after {t:?}"
    );
}

#[test]
fn native_watcher_reports_within_one_second() {
    check(false);
}

#[test]
fn poll_watcher_reports_within_one_second() {
    check(true);
}
