//! Status grouping in wide mode: pinned strip on top, one column per status.

use super::super::app::App;
use super::super::model::{Group, GroupKey, ItemRef, Row};
use super::super::mouse::Zone;
use super::{line_at, priority_style, truncate, width};
use crate::store::task::TaskEntry;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::Frame;

const STUB: u16 = 3;

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    app.hits.push(area, Zone::ListArea);
    app.list_height = area.height as usize;
    let groups = app.groups();
    let selected = app.selected_ref().cloned();
    let mut body = area;
    if let Some(p) = groups.iter().find(|g| g.key == GroupKey::Pinned) {
        let collapsed = app.collapsed.contains(&GroupKey::Pinned);
        let shown = if collapsed {
            0
        } else {
            p.items.len().min(3) as u16
        };
        let h = (1 + shown).min(area.height);
        let strip = Rect { height: h, ..area };
        header(
            f,
            app,
            strip,
            &format!(
                "{} ★ Pinned ({})",
                if collapsed { "▸" } else { "▾" },
                p.items.len()
            ),
            Zone::GroupHeader(GroupKey::Pinned),
        );
        for (i, item) in p.items.iter().take(shown as usize).enumerate() {
            let r = Rect {
                y: strip.y + 1 + i as u16,
                height: 1,
                ..strip
            };
            card(f, app, r, item, selected.as_ref() == Some(item), true);
        }
        body = Rect {
            y: area.y + h,
            height: area.height.saturating_sub(h),
            ..area
        };
    }
    let cols: Vec<&Group> = groups
        .iter()
        .filter(|g| g.key != GroupKey::Pinned)
        .collect();
    if cols.is_empty() || body.height == 0 {
        return;
    }
    let collapsed_set = app.collapsed.clone();
    let is_collapsed = |g: &Group| collapsed_set.contains(&g.key);
    let n_open = cols.iter().filter(|g| !is_collapsed(g)).count() as u16;
    let stubs = cols.len() as u16 - n_open;
    let free = body
        .width
        .saturating_sub(stubs * STUB + cols.len() as u16 - 1);
    let open_w = if n_open > 0 { free / n_open } else { 0 };
    let mut x = body.x;
    for (ci, g) in cols.iter().enumerate() {
        let collapsed = is_collapsed(g);
        let mut w = if collapsed { STUB } else { open_w };
        if ci + 1 == cols.len() {
            w = body.x + body.width - x;
        }
        let col = Rect {
            x,
            y: body.y,
            width: w,
            height: body.height,
        };
        let status = match &g.key {
            GroupKey::Status(s) => s.clone(),
            _ => "unknown".into(),
        };
        let zone = match &g.key {
            GroupKey::Status(s) => Zone::KanbanHeader(s.clone()),
            k => Zone::GroupHeader(k.clone()),
        };
        let title = if collapsed {
            format!("▸{}", g.items.len())
        } else {
            format!("{} ({})", status, g.items.len())
        };
        header(f, app, col, &title, zone);
        if !collapsed && col.height > 1 {
            let sep = Rect {
                y: col.y + 1,
                height: 1,
                ..col
            };
            line_at(
                f,
                sep,
                sep.y,
                Line::from("─".repeat(w as usize)),
                Style::default().fg(Color::DarkGray),
            );
            let h = col.height.saturating_sub(2) as usize;
            let sel_idx = selected
                .as_ref()
                .and_then(|s| g.items.iter().position(|i| i == s));
            let offset = sel_idx.map_or(0, |i| (i + 1).saturating_sub(h));
            for (i, item) in g.items.iter().skip(offset).take(h).enumerate() {
                let r = Rect {
                    x: col.x,
                    y: col.y + 2 + i as u16,
                    width: w,
                    height: 1,
                };
                card(f, app, r, item, selected.as_ref() == Some(item), false);
            }
        }
        x += w + 1;
        if x >= body.x + body.width {
            break;
        }
    }
    // Keep the cursor row in sync when a selected card is hidden.
    if let Some(s) = &selected {
        if !app.rows.iter().any(|r| matches!(r, Row::Task(i) if i == s)) {
            app.rebuild();
        }
    }
}

fn header(f: &mut Frame, app: &mut App, area: Rect, title: &str, zone: Zone) {
    let r = Rect { height: 1, ..area };
    line_at(
        f,
        r,
        r.y,
        Line::from(Span::styled(
            truncate(title, r.width as usize),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )),
        Style::default(),
    );
    app.hits.push(r, zone);
}

fn card(f: &mut Frame, app: &mut App, r: Rect, item: &ItemRef, selected: bool, with_project: bool) {
    let line = match app.entry(item) {
        Some(TaskEntry::Ok(t)) => {
            let m = &t.meta;
            let suffix = if with_project {
                app.project(&item.folder)
                    .map(|p| format!(" · {}", p.label))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let head = format!("{} ", m.priority.as_str());
            let room = (r.width as usize).saturating_sub(width(&head) + width(&suffix));
            Line::from(vec![
                Span::styled(m.priority.as_str().to_string(), priority_style(m.priority)),
                Span::raw(" "),
                Span::raw(truncate(&m.title, room)),
                Span::styled(suffix, Style::default().fg(Color::DarkGray)),
            ])
        }
        Some(TaskEntry::Broken(b)) => Line::from(Span::styled(
            truncate(&format!("{} ! {}", b.id, b.error), r.width as usize),
            Style::default().fg(Color::Red),
        )),
        None => Line::from(item.id.clone()),
    };
    let style = if selected {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    line_at(f, r, r.y, line, style);
    app.hits.push(r, Zone::TaskRow(item.clone()));
}
