//! `?` overlay: keys plus pane size, storage root and watcher mode.

use super::super::app::App;
use super::super::mouse::Zone;
use super::{centered, truncate};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

const KEYS: &[(&str, &str)] = &[
    ("j/k gg/G", "move, top/bottom"),
    ("Ctrl+d/u", "half page"),
    ("Tab S-Tab", "grouping"),
    ("h/l Space", "collapse/expand group"),
    ("C", "collapse/expand all"),
    ("Enter", "detail / open attachment"),
    ("Esc", "close detail"),
    ("s/S", "next/prev status"),
    ("p", "cycle priority"),
    ("t", "pin"),
    ("i", "details.md"),
    ("J/K o", "attachment cursor, open"),
    ("n", "new task"),
    ("R", "GitHub sync"),
    ("r", "reload"),
    ("y", "copy task id"),
    ("q", "quit"),
];

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let r = centered(area, 60, KEYS.len() as u16 + 7);
    f.render_widget(Clear, r);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" herdr-tasks help ");
    let inner = block.inner(r);
    f.render_widget(block, r);
    app.hits.push(r, Zone::Overlay);
    let w = inner.width as usize;
    let mut lines: Vec<Line> = KEYS
        .iter()
        .map(|(k, d)| Line::from(truncate(&format!("{k:<10} {d}"), w)))
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::styled(
        truncate(
            &format!(
                "pane: {}x{}  watch: {}",
                app.width, app.height, app.watch_mode
            ),
            w,
        ),
        Style::default().fg(Color::DarkGray),
    ));
    lines.push(Line::styled(
        truncate(&format!("root: {}", app.store.root.display()), w),
        Style::default().fg(Color::DarkGray),
    ));
    lines.push(Line::styled(
        truncate(
            &format!("root_source: {}", app.store.root_source.as_str()),
            w,
        ),
        Style::default().fg(Color::DarkGray),
    ));
    f.render_widget(Paragraph::new(lines), inner);
}
