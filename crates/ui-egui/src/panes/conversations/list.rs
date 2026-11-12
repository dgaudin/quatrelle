// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The conversations list (the module's left pane): the search box at its top, where the copy
//! stands, then one row per chat, newest activity first, drawn only for the rows on screen
//! (`ScrollArea::show_rows`: 20,000 chats cost a frame what 10 do). A row: the picture (a photo,
//! else coloured initials; a group's and a meeting's own icon), the presence dot of a one-to-one
//! chat, the name, the last line and its time.
//!
//! What is hostile here: every name and line came from Graph (through the worker): drawn on one
//! line ([`one_line`]), cut to the pane, never able to push "External" or "Can't be read here"
//! off the row, which are drawn first and whole. A group is always drawn as a group, whatever it
//! is named. What the list asks of the worker (presence, photos) is bounded by the rows on
//! screen, and each photo is asked once; when the list isn't drawn (another module, the pane
//! minimized, nothing listed), the worker is told no one is shown, so presence isn't asked for
//! from Mail.

use egui::{Color32, Rect, Sense, Ui, pos2, vec2};
use quatrelle_engine::Session;
use quatrelle_engine::model::dates;
use quatrelle_engine::model::pim::teams::{ChatKind, ChatRow, MAX_CHATS, Presence, Status, one_line};
use serde_json::json;

use super::pictures::TeamsPictures;
use super::{ConversationsUi, RowPicture};
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, UiState, widgets};

/// A row's height.
const ROW_H: f32 = 60.0;
/// The pause after the last key before the search goes, in seconds.
const SEARCH_PAUSE: f64 = 0.3;
/// The longest search, in characters (`teams.search` takes no more).
const QUERY_CHARS: usize = 200;
/// The most photos remembered as asked: past it, the set starts again (the engine still asks
/// each one once).
const PHOTOS_ASKED_MAX: usize = 5_000;

/// What a row's picture is (a group's is always the group's, whatever its name).
pub(crate) fn picture_of(r: &ChatRow) -> RowPicture {
    match &r.kind {
        ChatKind::Group => RowPicture::Group,
        ChatKind::Meeting => RowPicture::Meeting,
        ChatKind::OneOnOne => match &r.person {
            Some(user) => RowPicture::Person { user: user.clone(), initials: initials(&r.name) },
            None => RowPicture::Initials(initials(&r.name)),
        },
        ChatKind::Other { .. } => RowPicture::Initials(initials(&r.name)),
    }
}

/// The first letters of a name's first two words, in capitals.
fn initials(name: &str) -> String {
    one_line(name).split(' ').filter_map(|w| w.chars().find(|c| c.is_alphanumeric())).take(2).flat_map(char::to_uppercase).collect()
}

/// The list: the search box, where the copy stands, and the rows (or the search's hits).
pub(crate) fn list(s: &Session, ui_state: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = ui_state.tokens;
    let pics = &ui_state.teams_queue.pictures;
    let c = &mut ui_state.conversations;
    c.list_drawn = true;
    // Another account signed in: what was asked for the last one doesn't hold.
    if c.asked_for != s.prefs.teams.object_id {
        c.asked_for.clone_from(&s.prefs.teams.object_id);
        c.seen_users.clear();
        c.photos_asked.clear();
    }
    search_box(ui, &t, c, acts);
    states(s, ui, &t, acts);
    let searching = !c.query.trim().is_empty();
    let rows: Vec<&ChatRow> = if searching { hits(s, ui, &t) } else { s.teams.listed().collect() };
    if rows.is_empty() {
        if !searching && matches!(s.teams.status, Status::UpToDate | Status::NoKeychain | Status::Offline) {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| ui.label(egui::RichText::new(tl!("No conversations yet")).color(t.dim)));
        }
        no_one_shown(s, c, acts);
        return;
    }
    rows_pane(s, c, pics, ui, &t, &rows, acts);
}

/// The worker told no one is shown any more (it stops asking for their presence), once.
fn no_one_shown(s: &Session, c: &mut ConversationsUi, acts: &mut Vec<Action>) {
    if c.seen_users.is_empty() {
        return;
    }
    c.seen_users.clear();
    if s.prefs.teams.on && s.prefs.teams.signed_in {
        acts.push(Action::Run("teams.visible", json!({"users": []})));
    }
}

/// After the panes were drawn: the list wasn't (another module, the pane minimized), so no one
/// on it is shown.
pub(crate) fn list_not_drawn(s: &Session, c: &mut ConversationsUi, acts: &mut Vec<Action>) {
    if !std::mem::take(&mut c.list_drawn) {
        no_one_shown(s, c, acts);
    }
}

/// The search box at the top: the search goes [`SEARCH_PAUSE`] after the last key (Enter: at
/// once); emptied, the list comes back at once.
fn search_box(ui: &mut Ui, t: &Tokens, c: &mut ConversationsUi, acts: &mut Vec<Action>) {
    let frame = egui::Frame::new()
        .fill(if t.dark { t.surface } else { Color32::WHITE })
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(6, 3));
    let resp = frame
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icons::paint(ui.painter(), r, Icon::Search, t.dim);
                ui.add(
                    egui::TextEdit::singleline(&mut c.query)
                        .hint_text(tl!("Search conversations"))
                        .char_limit(QUERY_CHARS)
                        .frame(egui::Frame::NONE)
                        .desired_width(ui.available_width())
                        .text_color(t.text),
                )
            })
            .inner
        })
        .inner;
    if std::mem::take(&mut c.focus_search) {
        resp.request_focus();
    }
    let now = ui.input(|i| i.time);
    if resp.changed() {
        c.search_due = Some(now + SEARCH_PAUSE);
    }
    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    match c.search_due {
        Some(due) if enter || now >= due || c.query.trim().is_empty() => {
            c.search_due = None;
            acts.push(Action::Run("teams.search", json!({"query": c.query.trim()})));
        }
        Some(due) => ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64((due - now).max(0.01))),
        None => {}
    }
    ui.add_space(6.0);
}

/// Where the copy stands, above the rows: signing in, the first copy, offline, a sign-in or a
/// permission needed, or Teams stopped (and Sign in, for those three).
fn states(s: &Session, ui: &mut Ui, t: &Tokens, acts: &mut Vec<Action>) {
    let line = |ui: &mut Ui, text: &str, color: Color32| {
        ui.add(egui::Label::new(egui::RichText::new(text).color(color).size(12.5)).wrap());
    };
    match &s.teams.status {
        Status::SigningIn => line(ui, tl!("Signing in…"), t.dim),
        Status::Copying { done, of } => {
            line(ui, &tf!("Copying your conversations… {done} of {of} conversations", done = done, of = of), t.dim);
            if usize::try_from(*of).is_ok_and(|of| of >= MAX_CHATS) {
                line(ui, &tf!("Only the {n} most recent conversations are copied", n = widgets::count_text(MAX_CHATS as u64)), t.dim);
            }
        }
        Status::Offline => line(ui, tl!("Offline"), t.dim),
        Status::SignInAgain => line(ui, tl!("Teams: sign in again to go on"), t.danger),
        Status::PermissionMissing { scope } => line(
            ui,
            &tf!(
                "Quatrelle needs the permission {scope}: sign in again to accept it, or ask your administrator to grant admin consent in Microsoft Entra",
                scope = one_line(scope)
            ),
            t.danger,
        ),
        Status::Failed { why } => line(ui, &super::why_stopped(why), t.danger),
        Status::SignedOut | Status::UpToDate | Status::NoKeychain => return,
    }
    // The sign-in no longer holds, lacks a permission, or failed: it's tried again, the copy kept.
    if super::sign_in_again(&s.teams.status) && s.prefs.teams.on && ui.button(tl!("Sign in")).clicked() {
        acts.push(Action::Run("teams.signIn", json!({})));
    }
    ui.add_space(4.0);
}

/// The search's hits, one row per chat in the order found (a chat's messages found are one row).
/// While the worker hasn't answered the query typed, the last answer stays (the list neither
/// empties nor flickers at each key); before any, the list as it is.
fn hits<'a>(s: &'a Session, ui: &mut Ui, t: &Tokens) -> Vec<&'a ChatRow> {
    let note = |ui: &mut Ui, text: &str| {
        ui.label(egui::RichText::new(text).color(t.dim).size(12.5));
    };
    let Some(found) = s.teams.found.as_ref() else { return s.teams.listed().collect() };
    if found.indexing {
        note(ui, tl!("Indexing…"));
    }
    if found.partial {
        note(ui, tl!("Recent conversations only"));
    }
    let mut seen = std::collections::HashSet::new();
    let rows: Vec<&ChatRow> =
        found.hits.iter().filter(|(chat, _)| seen.insert(chat.as_str())).filter_map(|(chat, _)| s.teams.row(chat)).filter(|r| !r.hidden).collect();
    if rows.is_empty() {
        note(ui, tl!("No conversation found"));
    }
    rows
}

/// The rows, only those on screen drawn; the people on them told to the worker (presence) and
/// their photos asked once.
fn rows_pane(s: &Session, c: &mut ConversationsUi, pics: &TeamsPictures, ui: &mut Ui, t: &Tokens, rows: &[&ChatRow], acts: &mut Vec<Action>) {
    let open_at = s.teams.open.as_deref().and_then(|o| rows.iter().position(|r| r.id == o));
    let mut area = egui::ScrollArea::vertical().id_salt("conversations-list").auto_shrink([false, false]);
    // Alt+↑/↓ opened one off screen: it's brought into view.
    if std::mem::take(&mut c.scroll_to_open)
        && let Some(i) = open_at
    {
        let (offset, height) = c.view;
        let top = i as f32 * ROW_H;
        if top < offset {
            area = area.vertical_scroll_offset(top);
        } else if top + ROW_H > offset + height {
            area = area.vertical_scroll_offset((top + ROW_H - height).max(0.0));
        }
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    let mut people: Vec<String> = Vec::new();
    let out = area.show_rows(ui, ROW_H, rows.len(), |ui, range| {
        for i in range {
            let Some(r) = rows.get(i) else { continue };
            if let Some(p) = r.person.as_ref().filter(|_| r.kind == ChatKind::OneOnOne) {
                people.push(p.clone());
            }
            if row(s, pics, ui, t, r, open_at == Some(i)).clicked() {
                acts.push(Action::Run("teams.open", json!({"chat": r.id})));
            }
        }
    });
    c.view = (out.state.clamped_offset().y, out.inner_rect.height());
    people.sort();
    people.dedup();
    for user in &people {
        if c.photos_asked.len() >= PHOTOS_ASKED_MAX {
            c.photos_asked.clear();
        }
        if c.photos_asked.insert(user.clone()) {
            acts.push(Action::Run("teams.picture", json!({"kind": "photo", "user": user})));
        }
    }
    if people != c.seen_users {
        acts.push(Action::Run("teams.visible", json!({"users": people})));
        c.seen_users = people;
    }
}

/// A framed tag ("External", "Can't be read here") from `left` on the line from `top`: its width.
fn tag(p: &egui::Painter, t: &Tokens, text: &str, left: f32, top: f32) -> (f32, Rect) {
    let g = p.layout_no_wrap(text.to_string(), theme::regular(11.0), t.dim);
    let w = g.size().x + 10.0;
    let r = Rect::from_min_size(pos2(left, top), vec2(w, 17.0));
    p.rect_stroke(r, 4.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    p.galley(r.center() - g.size() / 2.0, g, t.dim);
    (w, r)
}

/// A presence's colour; none when it isn't known.
fn presence_color(p: Presence, t: &Tokens) -> Option<Color32> {
    match p {
        Presence::Available => Some(t.ok),
        Presence::Busy | Presence::DoNotDisturb => Some(t.danger),
        Presence::Away => Some(t.warn),
        Presence::Offline => Some(t.faint),
        Presence::Unknown => None,
    }
}

/// One row.
fn row(s: &Session, pics: &TeamsPictures, ui: &mut Ui, t: &Tokens, r: &ChatRow, open: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let p = ui.painter_at(rect);
    let fill = if open {
        t.accent_soft
    } else if resp.hovered() {
        t.hover_row
    } else {
        Color32::TRANSPARENT
    };
    p.rect_filled(rect.shrink2(vec2(2.0, 1.0)), 6.0, fill);
    let unread = r.unread > 0;
    // A chat left is read only: dimmed.
    let (text, dim) = if r.left { (t.faint, t.faint) } else { (t.text, t.dim) };

    // The picture, and a one-to-one chat's presence.
    let c = pos2(rect.left() + 24.0, rect.center().y);
    match picture_of(r) {
        RowPicture::Person { user, initials } => {
            match pics.photo(&user) {
                Some(tex) => egui::Image::new(tex).corner_radius(18.0).paint_at(ui, Rect::from_center_size(c, vec2(36.0, 36.0))),
                None => widgets::avatar(ui, c, 18.0, &initials, theme::avatar_color(&user, t.dark)),
            }
            let presence = s.teams.presence.get(&user).copied().unwrap_or_default();
            if let Some(color) = presence_color(presence, t) {
                let dot = c + vec2(13.0, 13.0);
                p.circle_filled(dot, 6.0, t.surface);
                p.circle_filled(dot, 4.5, color);
                if presence == Presence::DoNotDisturb {
                    p.line_segment([dot - vec2(2.5, 0.0), dot + vec2(2.5, 0.0)], egui::Stroke::new(1.5, Color32::WHITE));
                }
            }
        }
        RowPicture::Initials(initials) => widgets::avatar(ui, c, 18.0, &initials, theme::avatar_color(&r.id, t.dark)),
        RowPicture::Group | RowPicture::Meeting => {
            p.circle_filled(c, 18.0, t.accent_soft);
            let icon = if r.kind == ChatKind::Meeting { Icon::Calendar } else { Icon::People };
            icons::paint(&p, icons::centered(Rect::from_center_size(c, vec2(20.0, 20.0)), 20.0), icon, t.accent);
        }
    }

    let left = rect.left() + 52.0;
    let right = rect.right() - 8.0;
    let line1 = rect.top() + 10.0;
    let line2 = rect.top() + 33.0;
    let mut said: Vec<String> = Vec::new();

    // First line: External, the name, a group's member count, the time.
    let mut end = right;
    if let Some(at) = r.at {
        // The row's time is in milliseconds.
        let time = crate::i18n::date(&dates::list_time(at.div_euclid(1000), &s.clock));
        let g = p.layout_no_wrap(time.clone(), if unread { theme::bold(12.0) } else { theme::regular(12.0) }, if unread { t.accent } else { dim });
        end -= g.size().x;
        p.galley(pos2(end, line1 + 1.0), g, dim);
        end -= 8.0;
        said.push(time);
    }
    if r.kind == ChatKind::Group {
        let g = p.layout_no_wrap(r.members.to_string(), theme::regular(12.0), dim);
        end -= g.size().x;
        p.galley(pos2(end, line1 + 1.0), g, dim);
        let ir = Rect::from_min_size(pos2(end - 15.0, line1 + 1.0), vec2(13.0, 13.0));
        icons::paint(&p, ir, Icon::People, dim);
        end -= 23.0;
        said.push(tf!("Members: {n}", n = r.members));
    }
    let mut x = left;
    if r.external {
        let (w, tr) = tag(&p, t, tl!("External"), x, line1);
        if ui.rect_contains_pointer(tr) {
            resp.clone().on_hover_text(tl!("Someone in this conversation is outside your organisation"));
        }
        x += w + 6.0;
        said.push(tl!("External").to_string());
    }
    let name = one_line(&r.name);
    let font = if unread { theme::bold(14.0) } else { theme::regular(14.0) };
    let g = widgets::one_line(ui, &name, font, text, end - x);
    p.galley(pos2(x, line1), g, text);
    said.insert(0, name);

    // Second line: External (the last sender), the last line, the mark of a chat Graph won't serve.
    let mut x = left;
    if r.last_external {
        let (w, tr) = tag(&p, t, tl!("External"), x, line2);
        if ui.rect_contains_pointer(tr) {
            resp.clone().on_hover_text(tl!("This sender is outside your organisation."));
        }
        x += w + 6.0;
    }
    let mut end = right;
    if r.unreadable {
        let g = p.layout_no_wrap(tl!("Can't be read here").to_string(), theme::regular(11.0), dim);
        end -= g.size().x + 10.0;
        tag(&p, t, tl!("Can't be read here"), end, line2);
        end -= 6.0;
    }
    let last = one_line(&r.last_line);
    if !last.is_empty() {
        let g = widgets::one_line(ui, &last, theme::regular(12.5), dim, end - x);
        p.galley(pos2(x, line2 + 1.0), g, dim);
        said.push(last);
    }
    if r.unreadable {
        said.push(tl!("Can't be read here").to_string());
    }
    if unread {
        said.push(tf!("Unread: {n}", n = r.unread));
    }
    widgets::named(ui, &resp, &said.join(", "), true, Some(open));
    if r.left { resp.on_hover_text(tl!("You're no longer in this conversation")) } else { resp }
}
