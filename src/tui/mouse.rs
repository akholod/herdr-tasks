//! Mouse: one `Zone` enum, a last-pushed-wins `HitMap` filled during render,
//! and a dispatcher that calls the same `App` methods as the keys.

use super::app::{App, Chip};
use super::model::{GroupKey, Grouping, ItemRef, RowId};
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

#[derive(Debug, Clone, PartialEq)]
pub enum Zone {
    GroupingTab(Grouping),
    GroupHeader(GroupKey),
    TaskRow(ItemRef),
    KanbanHeader(String),
    Chip(Chip),
    AttachmentRow(usize),
    DetailClose,
    ListArea,
    DetailArea,
    ReviewRow(usize),
    ReviewImport,
    ReviewCancel,
    /// Swallows clicks on an overlay background.
    Overlay,
}

#[derive(Debug, Default, Clone)]
pub struct HitMap {
    pub zones: Vec<(Rect, Zone)>,
}

impl HitMap {
    pub fn clear(&mut self) {
        self.zones.clear();
    }

    pub fn push(&mut self, r: Rect, z: Zone) {
        if r.width > 0 && r.height > 0 {
            self.zones.push((r, z));
        }
    }

    /// The last-pushed zone containing (x, y): overlays shadow what is below.
    pub fn hit(&self, x: u16, y: u16) -> Option<&Zone> {
        self.zones
            .iter()
            .rev()
            .find(|(r, _)| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
            .map(|(_, z)| z)
    }

    pub fn find(&self, z: &Zone) -> Option<Rect> {
        self.zones
            .iter()
            .rev()
            .find(|(_, zz)| zz == z)
            .map(|(r, _)| *r)
    }
}

pub fn handle_mouse(app: &mut App, m: MouseEvent) {
    let zone = app.hits.hit(m.column, m.row).cloned();
    match m.kind {
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            let d: isize = if matches!(m.kind, MouseEventKind::ScrollDown) {
                1
            } else {
                -1
            };
            match zone {
                Some(Zone::DetailArea | Zone::AttachmentRow(_) | Zone::Chip(_)) => {
                    app.detail_scroll = (app.detail_scroll as isize + d).max(0) as u16;
                }
                Some(Zone::ReviewRow(_) | Zone::Overlay) if app.review.is_some() => {
                    app.review_move(d)
                }
                _ if app.review.is_none() => app.move_cursor(d),
                _ => {}
            }
        }
        MouseEventKind::Down(MouseButton::Left) => click(app, zone),
        _ => {}
    }
}

fn click(app: &mut App, zone: Option<Zone>) {
    if app.help {
        app.help = false;
        return;
    }
    let Some(z) = zone else { return };
    if app.review.is_some() {
        match z {
            Zone::ReviewRow(i) => app.review_toggle(Some(i)),
            Zone::ReviewImport => app.review_finish(true),
            Zone::ReviewCancel => app.review_finish(false),
            _ => {}
        }
        return;
    }
    if app.prompt.is_some() {
        return;
    }
    match z {
        Zone::GroupingTab(g) => app.set_grouping(g),
        Zone::GroupHeader(k) => {
            if let Some(i) = app
                .rows
                .iter()
                .position(|r| r.id() == RowId::Header(k.clone()))
            {
                app.select_row(i);
            }
            app.toggle_group(k);
        }
        Zone::KanbanHeader(s) => app.toggle_group(GroupKey::Status(s)),
        Zone::TaskRow(r) => {
            if let Some(i) = app
                .rows
                .iter()
                .position(|x| x.id() == RowId::Task(r.clone()))
            {
                app.select_row(i);
                app.open_detail();
            }
        }
        Zone::Chip(c) => app.chip(c),
        Zone::AttachmentRow(i) => app.open_attachment(Some(i)),
        Zone::DetailClose => app.close_detail(),
        _ => {}
    }
}
