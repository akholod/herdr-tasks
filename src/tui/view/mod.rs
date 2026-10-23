//! Rendering. Every clickable element registers a `Zone` while drawing.

pub mod chrome;
pub mod detail;
pub mod help;
pub mod import_review;
pub mod kanban;
pub mod list;
pub mod prompt;

use super::app::App;
use crate::store::task::Priority;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn render(f: &mut Frame, app: &mut App) {
    let area = f.area();
    if app.width != area.width || app.height != area.height {
        let was = app.kanban();
        app.width = area.width;
        app.height = area.height;
        if was != app.kanban() {
            app.rebuild();
        }
    }
    app.hits.clear();
    let banners = app.banners();
    let nb = banners.len().min(3) as u16;
    let [top, ban, body, bottom] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(nb),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);
    chrome::render(f, app, top);
    for (i, b) in banners.iter().take(nb as usize).enumerate() {
        let r = Rect {
            y: ban.y + i as u16,
            height: 1,
            ..ban
        };
        f.render_widget(
            Paragraph::new(truncate(&format!("! {b}"), r.width as usize))
                .style(Style::default().fg(Color::White).bg(Color::Red)),
            r,
        );
    }
    let show_detail = app.detail_open && app.selected_task().is_some();
    let (list_area, detail_area) = if show_detail {
        if app.wide() {
            let [l, d] =
                Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
                    .areas(body);
            (l, Some(d))
        } else {
            let [l, d] = Layout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)])
                .areas(body);
            (l, Some(d))
        }
    } else {
        (body, None)
    };
    if app.kanban() {
        kanban::render(f, app, list_area);
    } else {
        list::render(f, app, list_area);
    }
    if let Some(d) = detail_area {
        detail::render(f, app, d);
    }
    if app.prompt.is_some() {
        prompt::render(f, app, bottom);
    } else {
        status_line(f, app, bottom);
    }
    if app.review.is_some() {
        import_review::render(f, app, body);
    }
    if app.help {
        help::render(f, app, area);
    }
}

fn status_line(f: &mut Frame, app: &App, area: Rect) {
    let (text, style) = match &app.status {
        Some((m, true)) => (m.clone(), Style::default().fg(Color::Red)),
        Some((m, false)) => (m.clone(), Style::default().fg(Color::Green)),
        None if app.syncing => ("syncing…".to_string(), Style::default().fg(Color::Yellow)),
        None => (
            "n new · s status · t pin · R sync · ? help".to_string(),
            Style::default().fg(Color::DarkGray),
        ),
    };
    f.render_widget(
        Paragraph::new(truncate(&text, area.width as usize)).style(style),
        area,
    );
}

pub fn priority_style(p: Priority) -> Style {
    match p {
        Priority::P0 => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        Priority::P1 => Style::default().fg(Color::Yellow),
        Priority::P2 => Style::default().fg(Color::Blue),
    }
}

/// First status gray, last green, others yellow/cyan; unknown red.
pub fn status_color(statuses: &[String], status: &str) -> Color {
    match statuses.iter().position(|s| s == status) {
        None => Color::Red,
        Some(0) => Color::Gray,
        Some(i) if i + 1 == statuses.len() => Color::Green,
        Some(i) if i % 2 == 1 => Color::Yellow,
        Some(_) => Color::Cyan,
    }
}

pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Cut to `max` columns, ending with `…` when shortened.
pub fn truncate(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw + 1 > max {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

/// Word wrap to `max` columns; keeps blank lines; hard-splits long words.
pub fn wrap(text: &str, max: usize) -> Vec<String> {
    let max = max.max(1);
    let mut out = Vec::new();
    for para in text.lines() {
        let mut line = String::new();
        let mut lw = 0;
        for word in para.split(' ') {
            let ww = width(word);
            if lw > 0 && lw + 1 + ww > max {
                out.push(std::mem::take(&mut line));
                lw = 0;
            }
            if ww > max {
                for c in word.chars() {
                    let cw = c.width().unwrap_or(0);
                    if lw + cw > max {
                        out.push(std::mem::take(&mut line));
                        lw = 0;
                    }
                    line.push(c);
                    lw += cw;
                }
                continue;
            }
            if lw > 0 {
                line.push(' ');
                lw += 1;
            }
            line.push_str(word);
            lw += ww;
        }
        out.push(line);
    }
    out
}

/// Render one line of spans into a 1-row rect.
pub fn line_at(f: &mut Frame, area: Rect, y: u16, line: Line, style: Style) {
    if y >= area.y + area.height {
        return;
    }
    let r = Rect {
        x: area.x,
        y,
        width: area.width,
        height: 1,
    };
    f.render_widget(Paragraph::new(line).style(style), r);
}

pub fn span_w(s: &Span) -> u16 {
    s.width() as u16
}

pub fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_and_wrap() {
        assert_eq!(truncate("hello world", 6), "hello…");
        assert_eq!(truncate("hi", 6), "hi");
        assert_eq!(wrap("aaa bbb ccc", 7), vec!["aaa bbb", "ccc"]);
        assert_eq!(wrap("a\n\nb", 5), vec!["a", "", "b"]);
        assert_eq!(wrap("abcdefgh", 3), vec!["abc", "def", "gh"]);
    }
}
