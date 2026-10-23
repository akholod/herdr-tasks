//! Task card: title, chips, description, attachments, hidden details.

use super::super::app::{App, Chip};
use super::super::mouse::Zone;
use super::{priority_style, span_w, status_color, truncate, wrap};
use crate::store::fsutil::human_size;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

type Hit = (u16, u16, Zone);

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(t) = app.selected_task().cloned() else {
        return;
    };
    let folder = app
        .selected_ref()
        .map(|r| r.folder.clone())
        .unwrap_or_default();
    let (label, statuses) = app
        .project(&folder)
        .map(|p| (p.label.clone(), p.cfg.statuses.clone()))
        .unwrap_or_default();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            format!(" {} · {} ", t.meta.id, label),
            Style::default().fg(Color::Cyan),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.hits.push(area, Zone::DetailArea);
    let close = Rect {
        x: area.x + area.width.saturating_sub(4),
        y: area.y,
        width: 3,
        height: 1,
    };
    f.render_widget(
        Paragraph::new("[x]").style(Style::default().fg(Color::DarkGray)),
        close,
    );
    app.hits.push(close, Zone::DetailClose);
    let w = inner.width as usize;
    let mut lines: Vec<(Line<'static>, Vec<Hit>)> = Vec::new();
    for l in wrap(&t.meta.title, w) {
        lines.push((
            Line::from(Span::styled(
                l,
                Style::default().add_modifier(Modifier::BOLD),
            )),
            vec![],
        ));
    }
    // Chips: [status] [P1] [pin] [details]
    let m = &t.meta;
    let st_style = Style::default().fg(if t.status_valid {
        status_color(&statuses, &m.status)
    } else {
        Color::Red
    });
    let pin_style = if m.pinned {
        Style::default().fg(Color::Black).bg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let det = if app.details_expanded {
        "[details ▾]"
    } else {
        "[details]"
    };
    let chips = [
        (
            Span::styled(format!("[{}]", m.status), st_style),
            Chip::Status,
        ),
        (
            Span::styled(
                format!("[{}]", m.priority.as_str()),
                priority_style(m.priority),
            ),
            Chip::Priority,
        ),
        (
            Span::styled(
                if m.pinned { "[pinned]" } else { "[pin]" }.to_string(),
                pin_style,
            ),
            Chip::Pin,
        ),
        (
            Span::styled(det.to_string(), Style::default().fg(Color::Cyan)),
            Chip::Details,
        ),
    ];
    let mut spans = Vec::new();
    let mut hits = Vec::new();
    let mut x = 0u16;
    for (s, c) in chips {
        let sw = span_w(&s);
        if x > 0 && (x + 1 + sw) as usize > w {
            lines.push((
                Line::from(std::mem::take(&mut spans)),
                std::mem::take(&mut hits),
            ));
            x = 0;
        }
        if x > 0 {
            spans.push(Span::raw(" "));
            x += 1;
        }
        hits.push((x, sw, Zone::Chip(c)));
        spans.push(s);
        x += sw;
    }
    lines.push((Line::from(spans), hits));
    let mut meta = Vec::new();
    if !m.tags.is_empty() {
        meta.push(format!("#{}", m.tags.join(" #")));
    }
    if let Some(g) = &m.github {
        meta.push(format!("github #{}", g.issue));
    }
    if !meta.is_empty() {
        lines.push((
            Line::from(Span::styled(
                truncate(&meta.join("  "), w),
                Style::default().fg(Color::DarkGray),
            )),
            vec![],
        ));
    }
    lines.push((Line::from(""), vec![]));
    if t.description.trim().is_empty() {
        lines.push((
            Line::from(Span::styled(
                "(no description)",
                Style::default().fg(Color::DarkGray),
            )),
            vec![],
        ));
    } else {
        for l in wrap(t.description.trim_end(), w) {
            lines.push((Line::from(l), vec![]));
        }
    }
    if !t.attachments.is_empty() {
        lines.push((Line::from(""), vec![]));
        lines.push((
            Line::from(Span::styled(
                format!("attachments ({})", t.attachments.len()),
                Style::default().add_modifier(Modifier::BOLD),
            )),
            vec![],
        ));
        for (i, a) in t.attachments.iter().enumerate() {
            let cur = if i == app.attachment_cursor {
                "›"
            } else {
                " "
            };
            let size = human_size(a.size);
            let name = truncate(&a.name, w.saturating_sub(size.len() + 4));
            let style = if i == app.attachment_cursor {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            };
            lines.push((
                Line::from(vec![
                    Span::styled(format!("{cur} {name}  "), style),
                    Span::styled(size, Style::default().fg(Color::DarkGray)),
                ]),
                vec![(0, inner.width, Zone::AttachmentRow(i))],
            ));
        }
    }
    for warn in &t.warnings {
        lines.push((
            Line::from(Span::styled(
                truncate(&format!("warning: {warn}"), w),
                Style::default().fg(Color::Yellow),
            )),
            vec![],
        ));
    }
    if app.details_expanded {
        lines.push((Line::from(""), vec![]));
        lines.push((
            Line::from(Span::styled(
                "── details.md ──",
                Style::default().fg(Color::DarkGray),
            )),
            vec![],
        ));
        let d = t.details();
        if d.trim().is_empty() {
            lines.push((
                Line::from(Span::styled(
                    "(empty)",
                    Style::default().fg(Color::DarkGray),
                )),
                vec![],
            ));
        }
        for l in wrap(d.trim_end(), w) {
            lines.push((Line::from(l), vec![]));
        }
    }
    let max_scroll = lines.len().saturating_sub(inner.height as usize) as u16;
    if app.detail_scroll > max_scroll {
        app.detail_scroll = max_scroll;
    }
    for (i, (line, hits)) in lines
        .into_iter()
        .skip(app.detail_scroll as usize)
        .take(inner.height as usize)
        .enumerate()
    {
        let y = inner.y + i as u16;
        f.render_widget(
            Paragraph::new(line),
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            },
        );
        for (hx, hw, z) in hits {
            app.hits.push(
                Rect {
                    x: inner.x + hx,
                    y,
                    width: hw.min(inner.width.saturating_sub(hx)),
                    height: 1,
                },
                z,
            );
        }
    }
}
