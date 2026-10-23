//! "Новые issue (N)" overlay after a pull fetch: choose what to import.

use super::super::app::{App, ReviewLine};
use super::super::mouse::Zone;
use super::{centered, span_w, truncate};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(rv) = app.review.clone() else { return };
    let lines = rv.lines();
    let h = (lines.len() as u16 + 4).max(6);
    let r = centered(area, area.width.min(72), h);
    f.render_widget(Clear, r);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(format!(
            " Новые issue ({}) · {} ",
            rv.candidates.len(),
            rv.label
        ));
    let inner = block.inner(r);
    f.render_widget(block, r);
    app.hits.push(r, Zone::Overlay);
    let w = inner.width as usize;
    let list_h = inner.height.saturating_sub(2) as usize;
    let offset = (rv.cursor + 1).saturating_sub(list_h);
    for (n, (i, l)) in lines
        .iter()
        .enumerate()
        .skip(offset)
        .take(list_h)
        .enumerate()
    {
        let text = match l {
            ReviewLine::Candidate(c) => {
                let (iss, on) = &rv.candidates[*c];
                let labels = if iss.labels.is_empty() {
                    String::new()
                } else {
                    format!("  {}", iss.labels.join(", "))
                };
                format!(
                    "[{}] #{} {}{}",
                    if *on { "x" } else { " " },
                    iss.number,
                    iss.title,
                    labels
                )
            }
            ReviewLine::IgnoredHeader => format!(
                "{} Пропущенные ({})",
                if rv.ignored_open { "▾" } else { "▸" },
                rv.ignored.len()
            ),
            ReviewLine::Ignored(k) => format!(
                "    #{} {}  (u: вернуть)",
                rv.ignored[*k].number, rv.ignored[*k].title
            ),
        };
        let mut style = match l {
            ReviewLine::IgnoredHeader => Style::default().fg(Color::Magenta),
            ReviewLine::Ignored(_) => Style::default().fg(Color::DarkGray),
            ReviewLine::Candidate(_) => Style::default(),
        };
        if i == rv.cursor {
            style = style.add_modifier(Modifier::REVERSED);
        }
        let y = inner.y + n as u16;
        let row = Rect {
            x: inner.x,
            y,
            width: inner.width,
            height: 1,
        };
        f.render_widget(Paragraph::new(truncate(&text, w)).style(style), row);
        app.hits.push(row, Zone::ReviewRow(i));
    }
    let y = inner.y + inner.height.saturating_sub(1);
    let import = Span::styled(
        "[ Import ]",
        Style::default().fg(Color::Black).bg(Color::Green),
    );
    let cancel = Span::styled(
        "[ Cancel ]",
        Style::default().fg(Color::Black).bg(Color::Gray),
    );
    let hint = Span::styled("  Space a Enter Esc", Style::default().fg(Color::DarkGray));
    let (iw, cw) = (span_w(&import), span_w(&cancel));
    app.hits.push(
        Rect {
            x: inner.x,
            y,
            width: iw,
            height: 1,
        },
        Zone::ReviewImport,
    );
    app.hits.push(
        Rect {
            x: inner.x + iw + 1,
            y,
            width: cw,
            height: 1,
        },
        Zone::ReviewCancel,
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![import, Span::raw(" "), cancel, hint])),
        Rect {
            x: inner.x,
            y,
            width: inner.width,
            height: 1,
        },
    );
}
