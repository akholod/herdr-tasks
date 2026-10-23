//! Key dispatch (vim-style). Esc backs out and never quits.

use super::app::App;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn handle_key(app: &mut App, k: KeyEvent) {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && k.code == KeyCode::Char('c') {
        app.should_quit = true;
        return;
    }
    if app.help {
        app.help = false;
        return;
    }
    if app.prompt.is_some() {
        prompt_key(app, k);
        return;
    }
    if app.review.is_some() {
        review_key(app, k);
        return;
    }
    let g = std::mem::take(&mut app.pending_g);
    match k.code {
        KeyCode::Char('d') if ctrl => app.half_page(true),
        KeyCode::Char('u') if ctrl => app.half_page(false),
        KeyCode::Char('j') | KeyCode::Down => app.move_cursor(1),
        KeyCode::Char('k') | KeyCode::Up => app.move_cursor(-1),
        KeyCode::PageDown => app.half_page(true),
        KeyCode::PageUp => app.half_page(false),
        KeyCode::Char('g') if g => app.goto_top(),
        KeyCode::Char('g') => app.pending_g = true,
        KeyCode::Home => app.goto_top(),
        KeyCode::Char('G') | KeyCode::End => app.goto_bottom(),
        KeyCode::Tab => app.cycle_grouping(1),
        KeyCode::BackTab => app.cycle_grouping(-1),
        KeyCode::Char('h') | KeyCode::Left => app.collapse_current(true),
        KeyCode::Char('l') | KeyCode::Right => app.collapse_current(false),
        KeyCode::Char(' ') => {
            if let Some(super::model::Row::Header { key, .. }) = app.rows.get(app.cursor).cloned() {
                app.toggle_group(key);
            }
        }
        KeyCode::Char('C') => app.toggle_all(),
        KeyCode::Enter => app.activate(),
        KeyCode::Esc => {
            if app.detail_open {
                app.close_detail();
            } else {
                app.status = None;
            }
        }
        KeyCode::Char('s') => app.cycle_status(1),
        KeyCode::Char('S') => app.cycle_status(-1),
        KeyCode::Char('p') => app.cycle_priority(),
        KeyCode::Char('t') => app.toggle_pin(),
        KeyCode::Char('i') => app.toggle_details(),
        KeyCode::Char('J') => app.move_attachment(1),
        KeyCode::Char('K') => app.move_attachment(-1),
        KeyCode::Char('o') => app.open_attachment(None),
        KeyCode::Char('n') => app.start_prompt(),
        KeyCode::Char('R') => app.start_sync(),
        KeyCode::Char('r') => {
            app.reload_all();
            app.flash("reloaded", false);
        }
        KeyCode::Char('y') => app.yank(),
        KeyCode::Char('?') => app.help = true,
        KeyCode::Char('q') => app.should_quit = true,
        _ => {}
    }
}

fn prompt_key(app: &mut App, k: KeyEvent) {
    let Some(p) = app.prompt.as_mut() else { return };
    match k.code {
        KeyCode::Esc => app.prompt_cancel(),
        KeyCode::Enter => app.prompt_submit(),
        KeyCode::Backspace => {
            p.buffer.pop();
            p.error = None;
        }
        KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
            p.buffer.push(c);
            p.error = None;
        }
        _ => {}
    }
}

fn review_key(app: &mut App, k: KeyEvent) {
    match k.code {
        KeyCode::Char('j') | KeyCode::Down => app.review_move(1),
        KeyCode::Char('k') | KeyCode::Up => app.review_move(-1),
        KeyCode::Char(' ') => app.review_toggle(None),
        KeyCode::Char('a') => app.review_toggle_all(),
        KeyCode::Char('u') => app.review_unignore(),
        KeyCode::Enter => app.review_finish(true),
        KeyCode::Esc => app.review_finish(false),
        _ => {}
    }
}
