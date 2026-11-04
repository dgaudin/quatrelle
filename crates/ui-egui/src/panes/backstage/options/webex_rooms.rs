// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! File › Options › Calendar › Webex rooms: the rooms the Webex Meeting button's menu offers, a row
//! each (its name, its link, its lines and ×), then a row to add one. Each box is saved once it's
//! left, through `calendar.webexRoom`, what's typed cut to what a room keeps; a link that can't be
//! the room's says why under its row and stays in its box to be corrected, nothing sent.

use quatrelle_engine::{WEBEX_ROOM_LINE_MAX, WEBEX_ROOM_LINES_MAX, WEBEX_ROOM_NAME_MAX, WebexRoom};

use super::*;
use crate::pim_dialogs::webex_rooms::{link_refused, typed_room};

/// The name's box and the link's; the lines' box under them is as wide as the two.
const NAME_WIDTH: f32 = 180.0;
const LINK_WIDTH: f32 = 300.0;

/// A row's boxes.
const FIELDS: [&str; 3] = ["name", "link", "lines"];

/// Where box `field` of row `at` keeps what's typed in it until it's saved (or refused).
fn typed_id(at: usize, field: &str) -> egui::Id {
    egui::Id::unique(("options-webex-room", at, field))
}

/// The setting's title, a row per room, and the add row.
pub(super) fn webex_rooms(ui: &mut Ui, s: &Session, t: &Tokens, acts: &mut Vec<Action>) {
    let rooms = &s.prefs.webex_rooms;
    let about = tl!("Under Webex Meeting in an appointment's window, each room puts its link and its lines in the meeting.");
    setting_row(ui, t, false, name(t, tl!("Webex rooms")), Some(note(t, about)), |_| {});
    forget_moved_rows(ui, rooms);
    for (at, room) in rooms.iter().enumerate() {
        room_row(ui, t, at, room, rooms, acts);
        ui.add_space(10.0);
    }
    add_row(ui, t, rooms, acts);
    ui.add_space(8.0);
}

/// What's typed in a row is kept by its place: once the rooms change (one added, removed or given
/// another link, here or in an appointment's window), a row whose room isn't the one it had
/// forgets it, so one room's text never shows in another's row.
fn forget_moved_rows(ui: &Ui, rooms: &[WebexRoom]) {
    let id = egui::Id::unique("options-webex-room-links");
    let before: Vec<String> = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    if before.iter().map(String::as_str).eq(rooms.iter().map(|r| r.url.as_str())) {
        return;
    }
    for at in 0..before.len().max(rooms.len()) {
        if before.get(at).map(String::as_str) != rooms.get(at).map(|r| r.url.as_str()) {
            ui.data_mut(|d| FIELDS.iter().for_each(|f| d.remove::<String>(typed_id(at, f))));
        }
    }
    ui.data_mut(|d| d.insert_temp(id, rooms.iter().map(|r| r.url.clone()).collect::<Vec<String>>()));
}

/// What box `id` shows: what's typed in it, else what's kept.
fn shown(ui: &Ui, id: egui::Id, kept: &str) -> String {
    ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| kept.to_string())
}

/// Box `id`'s text, kept for the next frame while `held` (typed in, or refused); else the box
/// shows what's kept again.
fn hold(ui: &Ui, id: egui::Id, text: String, held: bool) {
    if held {
        ui.data_mut(|d| d.insert_temp(id, text));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
}

/// The lines typed as a room keeps them: no control character, no space at either end, no blank
/// line, at most [`WEBEX_ROOM_LINES_MAX`] of at most [`WEBEX_ROOM_LINE_MAX`] characters (the rest
/// cut, so the engine never refuses them for their size).
fn kept_lines(text: &str) -> Vec<String> {
    text.split('\n')
        .map(|l| {
            let l: String = l.chars().filter(|c| !c.is_control()).collect();
            l.trim().chars().take(WEBEX_ROOM_LINE_MAX).collect::<String>().trim_end().to_string()
        })
        .filter(|l| !l.is_empty())
        .take(WEBEX_ROOM_LINES_MAX)
        .collect()
}

/// Room `at`'s row: its name, its link and ×, its lines under them, then why the link typed can't
/// be its link. The room is found by its link as kept, or as it's changed in this frame: a box is
/// left on the click that lands on × or on another box, and what it saves goes first, the room
/// taken out last.
fn room_row(ui: &mut Ui, t: &Tokens, at: usize, room: &WebexRoom, rooms: &[WebexRoom], acts: &mut Vec<Action>) {
    let url = room.url.as_str();
    let mut link = url.to_string();
    let mut refused = None;
    let mut remove = false;
    ui.horizontal(|ui| {
        // The room carried over from 1.0 has no name until it's given one; a name can't be taken
        // off again (the box shows it back).
        let id = typed_id(at, "name");
        let mut text = shown(ui, id, &room.name);
        let hint = if room.name.is_empty() { tl!("My personal room") } else { tl!("Room name") };
        let r = ui.add(egui::TextEdit::singleline(&mut text).hint_text(hint).char_limit(WEBEX_ROOM_NAME_MAX).desired_width(NAME_WIDTH));
        let typed = text.trim();
        let taken_off = typed.is_empty() && !room.name.is_empty();
        if r.lost_focus() && typed != room.name && !taken_off {
            acts.push(Action::Run("calendar.webexRoom", json!({"url": url, "name": typed})));
        }
        hold(ui, id, text, r.has_focus());

        // Its link, checked first: one that can't be the room's stays in the box, and so does one
        // that became right since (the room that had it gone, or given another), to be saved
        // when the box is left again.
        let id = typed_id(at, "link");
        let mut text = shown(ui, id, url);
        let r = ui.add(egui::TextEdit::singleline(&mut text).hint_text("https://site.webex.com/meet/name").desired_width(LINK_WIDTH));
        let typed = text.trim();
        let changed = typed != url;
        if changed {
            refused = link_refused(typed, rooms.iter().enumerate().filter(|(i, _)| *i != at).map(|(_, r)| r));
        }
        let save = r.lost_focus() && changed && refused.is_none();
        if save {
            acts.push(Action::Run("calendar.webexRoom", json!({"url": typed, "was": url})));
            link = typed.to_string();
        }
        hold(ui, id, text, r.has_focus() || (changed && !save));

        remove = widgets::tiny_icon_button(ui, Icon::Close, "Remove this Webex room", t).clicked();
    });
    // Its lines, as many as it keeps.
    let id = typed_id(at, "lines");
    let mut text = shown(ui, id, &room.details.join("\n"));
    let hint = tl!("Meeting number, video address, dial-in numbers… copied into the notes");
    let width = NAME_WIDTH + ui.spacing().item_spacing.x + LINK_WIDTH;
    let r = ui.add(egui::TextEdit::multiline(&mut text).hint_text(hint).desired_rows(2).desired_width(width));
    if r.lost_focus() {
        let lines = kept_lines(&text);
        if lines != room.details {
            acts.push(Action::Run("calendar.webexRoom", json!({"url": link, "details": lines.join("\n")})));
        }
    }
    hold(ui, id, text, r.has_focus());
    if remove {
        acts.push(Action::Run("calendar.webexRoom", json!({"url": link, "remove": true})));
    }
    if let Some(why) = refused {
        ui.label(RichText::new(why).color(t.danger).small());
    }
}

/// The row that adds a room: its name and its link, then Add a room once both are right and there
/// are fewer rooms than are kept ([`typed_room`], as the menu's), or why not under them.
fn add_row(ui: &mut Ui, t: &Tokens, rooms: &[WebexRoom], acts: &mut Vec<Action>) {
    let id = egui::Id::unique("options-webex-room-new");
    let (mut new_name, mut link): (String, String) = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    let mut add = false;
    let room = ui
        .horizontal(|ui| {
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            let r = ui
                .add(egui::TextEdit::singleline(&mut new_name).hint_text(tl!("Room name")).char_limit(WEBEX_ROOM_NAME_MAX).desired_width(NAME_WIDTH));
            add |= enter && r.lost_focus();
            let r = ui.add(egui::TextEdit::singleline(&mut link).hint_text("https://site.webex.com/meet/name").desired_width(LINK_WIDTH));
            add |= enter && r.lost_focus();
            let room = typed_room(&new_name, &link, rooms);
            add |= ui.add_enabled(room.is_ok(), egui::Button::new(tl!("Add a room"))).clicked();
            room
        })
        .inner;
    match room {
        Ok(room) if add => {
            acts.push(Action::Run("calendar.webexRoom", json!({"url": room.url, "name": room.name})));
            new_name.clear();
            link.clear();
        }
        Err(Some(why)) => {
            ui.label(RichText::new(why).color(t.danger).small());
        }
        _ => {}
    }
    ui.data_mut(|d| d.insert_temp(id, (new_name, link)));
}
