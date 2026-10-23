//! Bottom-line prompt for `n`: `New task in <project>: <title>`.

use super::super::app::App;
use super::{truncate, width};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(p) = &app.prompt else { return };
    let w = area.width as usize;
    let head = truncate(&format!("New task in {}: ", p.label), w / 2);
    let room = w.saturating_sub(width(&head) + 1);
    // Show the tail of a long title so the cursor stays visible.
    let chars: Vec<char> = p.buffer.chars().collect();
    let mut start = 0;
    while start < chars.len() && width(&chars[start..].iter().collect::<String>()) > room {
        start += 1;
    }
    let shown: String = chars[start..].iter().collect();
    let mut spans = vec![
        Span::styled(
            head,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(shown),
        Span::styled("▏", Style::default().fg(Color::Cyan)),
    ];
    if let Some(e) = &p.error {
        spans = vec![Span::styled(
            truncate(&format!("{e} (Esc cancels)"), w),
            Style::default().fg(Color::Red),
        )];
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
