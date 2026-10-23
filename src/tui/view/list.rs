//! Vertical grouped list (dock layout, and wide non-status groupings).

use super::super::app::App;
use super::super::model::{Grouping, Row};
use super::super::mouse::Zone;
use super::{line_at, priority_style, status_color, truncate, width};
use crate::store::task::TaskEntry;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::Frame;

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    app.hits.push(area, Zone::ListArea);
    let h = area.height as usize;
    app.list_height = h;
    if app.rows.is_empty() {
        let msg = if app.projects.is_empty() {
            "no projects — open a space in herdr"
        } else {
            "no tasks — press n to create one"
        };
        line_at(
            f,
            area,
            area.y,
            Line::from(Span::styled(
                truncate(msg, area.width as usize),
                Style::default().fg(Color::DarkGray),
            )),
            Style::default(),
        );
        return;
    }
    if app.cursor < app.list_offset {
        app.list_offset = app.cursor;
    } else if h > 0 && app.cursor >= app.list_offset + h {
        app.list_offset = app.cursor + 1 - h;
    }
    if app.list_offset + h > app.rows.len() {
        app.list_offset = app.rows.len().saturating_sub(h);
    }
    let statuses = app.statuses();
    let rows: Vec<(usize, Row)> = app
        .rows
        .iter()
        .cloned()
        .enumerate()
        .skip(app.list_offset)
        .take(h)
        .collect();
    for (n, (i, row)) in rows.into_iter().enumerate() {
        let y = area.y + n as u16;
        let rect = Rect {
            x: area.x,
            y,
            width: area.width,
            height: 1,
        };
        let selected = i == app.cursor;
        let (line, zone) = match &row {
            Row::Header {
                key,
                title,
                count,
                collapsed,
            } => {
                let arrow = if *collapsed { "▸" } else { "▾" };
                let text = truncate(&format!("{arrow} {title} ({count})"), area.width as usize);
                (
                    Line::from(Span::styled(
                        text,
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Zone::GroupHeader(key.clone()),
                )
            }
            Row::Task(r) => (
                task_line(app, r, &statuses, area.width as usize),
                Zone::TaskRow(r.clone()),
            ),
        };
        let style = if selected {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        line_at(f, area, y, line, style);
        app.hits.push(rect, zone);
    }
}

/// `P1 ● t-0042 title… · proj`; broken: `t-0003 ! invalid task.toml: …`.
pub fn task_line(
    app: &App,
    r: &super::super::model::ItemRef,
    statuses: &[String],
    w: usize,
) -> Line<'static> {
    let label = app
        .project(&r.folder)
        .map(|p| p.label.clone())
        .unwrap_or_default();
    match app.entry(r) {
        Some(TaskEntry::Ok(t)) => {
            let m = &t.meta;
            let suffix = if app.grouping == Grouping::Project {
                String::new()
            } else {
                format!(" · {label}")
            };
            let head = format!("{} ● {} ", m.priority.as_str(), m.id);
            let room = w.saturating_sub(width(&head) + width(&suffix));
            let mut title = truncate(&m.title, room);
            let mut suffix = suffix;
            if room < 4 {
                title = truncate(&m.title, w.saturating_sub(width(&head)));
                suffix.clear();
            }
            let dot = if t.status_valid {
                status_color(statuses, &m.status)
            } else {
                Color::Red
            };
            Line::from(vec![
                Span::styled(m.priority.as_str().to_string(), priority_style(m.priority)),
                Span::raw(" "),
                Span::styled("●", Style::default().fg(dot)),
                Span::raw(" "),
                Span::styled(m.id.clone(), Style::default().fg(Color::DarkGray)),
                Span::raw(" "),
                Span::raw(title),
                Span::styled(suffix, Style::default().fg(Color::DarkGray)),
            ])
        }
        Some(TaskEntry::Broken(b)) => Line::from(Span::styled(
            truncate(&format!("{} ! {}", b.id, b.error), w),
            Style::default().fg(Color::Red),
        )),
        None => Line::from(r.id.clone()),
    }
}
