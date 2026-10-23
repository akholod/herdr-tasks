//! Top line: grouping tabs + project count.

use super::super::app::App;
use super::super::model::Grouping;
use super::super::mouse::Zone;
use super::span_w;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let mut spans = Vec::new();
    let mut x = area.x;
    for g in Grouping::ALL {
        let label = format!("[{}]", g.label());
        let style = if g == app.grouping {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan)
        };
        let s = Span::styled(label, style);
        let w = span_w(&s);
        if x + w > area.x + area.width {
            break;
        }
        app.hits.push(
            Rect {
                x,
                y: area.y,
                width: w,
                height: 1,
            },
            Zone::GroupingTab(g),
        );
        x += w + 1;
        spans.push(s);
        spans.push(Span::raw(" "));
    }
    let mut info = format!("{} proj", app.projects.len());
    if app.watch_mode == "poll" {
        info.push_str(" · watch: poll");
    }
    let used: u16 = spans.iter().map(span_w).sum();
    let iw = info.chars().count() as u16;
    if used + iw < area.width {
        spans.push(Span::raw(" ".repeat((area.width - used - iw) as usize)));
        spans.push(Span::styled(info, Style::default().fg(Color::DarkGray)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
