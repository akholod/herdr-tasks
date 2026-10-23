//! The panel: terminal setup, event loop, watcher and discovery refresh.

pub mod app;
pub mod keys;
pub mod model;
pub mod mouse;
pub mod view;

use crate::cli::Surface;
use crate::error::{AppError, Result};
use crate::herdr;
use crate::store::{project, Store};
use crate::watch::{self, WatchEvent};
use app::{App, Deps, SystemOpener};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, SetTitle,
};
use ratatui::Terminal;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const DISCOVERY_EVERY: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_millis(100);

/// Real discovery: herdr snapshot + cached cwd -> key resolution.
pub fn herdr_discovery() -> app::DiscoverFn {
    let own = std::env::var("HERDR_PANE_ID")
        .ok()
        .filter(|s| !s.is_empty());
    let plugin = std::env::current_exe()
        .ok()
        .and_then(|e| herdr::plugin_dir_of(&e));
    let mut cache: HashMap<String, Option<PathBuf>> = HashMap::new();
    Box::new(move || {
        let snap = herdr::snapshot()?;
        let mut resolve = |cwd: &str| {
            cache
                .entry(cwd.to_string())
                .or_insert_with(|| project::resolve_key(std::path::Path::new(cwd)).ok())
                .clone()
        };
        Some(herdr::discover_projects(
            &snap,
            own.as_deref(),
            plugin.as_deref(),
            &mut resolve,
        ))
    })
}

pub fn run(surface: Surface) -> Result<()> {
    let store = Store::resolve();
    store.ensure()?;
    let title = match surface {
        Surface::Dock => "herdr-tasks-dock",
        Surface::Board => "herdr-tasks-board",
    };
    let io = |e: std::io::Error| AppError::Config(format!("terminal: {e}"));
    let app = App::new(Deps {
        store,
        surface,
        discover: herdr_discovery(),
        opener: Box::new(SystemOpener),
    });
    enable_raw_mode().map_err(io)?;
    let mut out = std::io::stdout();
    execute!(
        out,
        EnterAlternateScreen,
        EnableMouseCapture,
        SetTitle(title)
    )
    .map_err(io)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(out)).map_err(io)?;
    let res = event_loop(&mut terminal, app);
    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    );
    let _ = terminal.show_cursor();
    res
}

fn event_loop<B: Backend + Write>(terminal: &mut Terminal<B>, mut app: App) -> Result<()> {
    let io = |e: std::io::Error| AppError::Config(format!("terminal: {e}"));
    let root = app.store.root.clone();
    let mut watcher = watch::start(&root, false).ok();
    app.watch_mode = watcher.as_ref().map_or("off", |w| w.mode.as_str());
    let mut last_discovery = Instant::now();
    loop {
        terminal.draw(|f| view::render(f, &mut app)).map_err(io)?;
        if let Some(text) = app.clipboard.take() {
            let _ = write!(
                terminal.backend_mut(),
                "\x1b]52;c;{}\x07",
                base64(text.as_bytes())
            );
            let _ = Write::flush(terminal.backend_mut());
        }
        if app.should_quit {
            return Ok(());
        }
        if event::poll(TICK).map_err(io)? {
            match event::read().map_err(io)? {
                Event::Key(k) if k.kind == KeyEventKind::Press => keys::handle_key(&mut app, k),
                Event::Mouse(m) => mouse::handle_mouse(&mut app, m),
                _ => {}
            }
        }
        let mut rediscover = false;
        let mut root_changed = false;
        if let Some(w) = &watcher {
            if w.native_failed() {
                watcher = watch::start(&root, true).ok();
                app.watch_mode = watcher.as_ref().map_or("off", |w| w.mode.as_str());
            }
        }
        if let Some(w) = &watcher {
            for ev in w.drain() {
                match ev {
                    WatchEvent::Project(folder) => {
                        if !app.reload_project(&folder) {
                            rediscover = true;
                        }
                    }
                    WatchEvent::Root => root_changed = true,
                }
            }
        }
        if root_changed {
            // Global config feeds every project's merged config.
            app.reload_all();
            last_discovery = Instant::now();
        } else if rediscover || last_discovery.elapsed() >= DISCOVERY_EVERY {
            app.refresh_projects();
            last_discovery = Instant::now();
        }
        app.poll_sync();
    }
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_encodes() {
        assert_eq!(super::base64(b"t-0001"), "dC0wMDAx");
        assert_eq!(super::base64(b"ab"), "YWI=");
    }
}
