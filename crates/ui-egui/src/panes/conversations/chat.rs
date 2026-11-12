// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The conversation pane (the module's right): the open chat's header (its name, the other
//! person's presence, its members), then its messages by day, one sender's in a row under one name
//! and picture, the user's own on the right; system lines; and the page before asked for when the
//! user scrolls to the top. A chat Graph won't let be read shows what the list gave of it and
//! "Open in Teams".
//!
//! What is hostile here: everything drawn came from senders (names, words, links, pictures,
//! files) or from Graph (the chat's link). "External" is drawn before the name a sender chose and
//! never cut; an application's message says "App". Names are on one line ([`one_line`]). A
//! picture kept with a message is asked for when the message is first drawn (at most
//! `HOSTED_PER_MESSAGE`); one from elsewhere only on a click (`parts.rs`). "Open in Teams" opens
//! the chat's link only when [`is_teams_chat_link`] takes it; otherwise it isn't drawn.

use egui::{Align, Color32, Layout, RichText, Sense, Ui, vec2};
use quatrelle_engine::Session;
use quatrelle_engine::model::dates;
use quatrelle_engine::model::ical::is_teams_chat_link;
use quatrelle_engine::model::pim::teams::{ChatKind, ChatMessage, ChatRow, MessageKind, Picture, Presence, Sender, is_internal, one_line};
use serde_json::json;

use super::ConversationsUi;
use super::pictures::{Shown, TeamsPictures};
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, UiState, widgets};

mod parts;
mod text;

/// How near the top (in points) the chat is scrolled for the page before to be asked for.
const TOP: f32 = 40.0;

/// What every part of the pane reads.
struct Pane<'a> {
    s: &'a Session,
    t: Tokens,
    pics: &'a TeamsPictures,
    /// The user's own id, and their organisation's.
    me: &'a str,
    tenant: &'a str,
    /// The chat's link, when it's one "Open in Teams" may open.
    teams_link: Option<&'a str>,
}

impl Pane<'_> {
    /// "Open in Teams", when the chat's link is one.
    fn open_in_teams(&self, ui: &mut Ui, acts: &mut Vec<Action>) {
        if let Some(url) = self.teams_link
            && ui.button(tl!("Open in Teams")).clicked()
        {
            acts.push(Action::OpenUrl(url.to_string()));
        }
    }

    /// A message's sender's name, as the chat's members have it when they're one.
    fn sender_name(&self, sender: &Sender) -> String {
        match sender {
            Sender::User { id, name, .. } => {
                let member = self.s.teams.member(id).map(|m| one_line(&m.name)).filter(|n| !n.is_empty());
                member.unwrap_or_else(|| one_line(name))
            }
            Sender::App { name, .. } => one_line(name),
            Sender::Nobody => String::new(),
        }
    }

    fn is_own(&self, m: &ChatMessage) -> bool {
        matches!(&m.sender, Sender::User { id, .. } if !self.me.is_empty() && id.eq_ignore_ascii_case(self.me))
    }
}

/// The open chat: its header, then its messages (or, Graph refusing them, what's known of it).
pub(crate) fn chat(s: &Session, ui_state: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let Some(open) = s.teams.open.as_deref() else { return };
    let Some(row) = s.teams.row(open) else { return };
    let UiState { conversations: c, teams_queue, tokens, .. } = ui_state;
    // Another chat: what was asked for the last one doesn't hold.
    if c.hosted_for.as_deref() != Some(open) {
        c.hosted_for = Some(open.to_string());
        c.hosted_asked.clear();
        c.older_asked = None;
        c.chat_view = None;
        c.chat_scroll_to = None;
        c.viewing = None;
    }
    // Pictures let go for room: their messages ask for them again once on screen (all of them,
    // when some let go were too many to remember).
    let (let_go, lost) = teams_queue.pictures.take_let_go();
    if lost {
        c.hosted_asked.clear();
    }
    for gone in let_go {
        if let Picture::Hosted { message, .. } = gone {
            c.hosted_asked.remove(&message);
        }
    }
    // The picture opened big, decoded whole; its file let go, it's asked for again.
    if teams_queue.pictures.view(c.viewing.as_ref())
        && let Some(picture) = &c.viewing
    {
        acts.push(Action::Run("teams.picture", picture_params(picture)));
    }
    let teams_link = s.teams.web_url.as_deref().filter(|u| is_teams_chat_link(u));
    let pane = Pane { s, t: *tokens, pics: &teams_queue.pictures, me: &s.prefs.teams.object_id, tenant: &s.prefs.teams.tenant_id, teams_link };
    egui::Frame::new().inner_margin(egui::Margin::symmetric(16, 10)).show(ui, |ui| header(&pane, ui, row));
    let line = ui.available_rect_before_wrap();
    ui.painter().hline(line.x_range(), line.top(), egui::Stroke::new(1.0, pane.t.border));
    if row.unreadable {
        unreadable(&pane, ui, row, acts);
    } else {
        messages(&pane, c, ui, open, acts);
    }
    viewer(&pane, c, ui.ctx());
}

/// `teams.picture`'s params for a picture.
fn picture_params(p: &Picture) -> serde_json::Value {
    match p {
        Picture::Hosted { chat, message, hosted } => json!({"kind": "hosted", "chat": chat, "message": message, "hosted": hosted}),
        Picture::Photo { user } => json!({"kind": "photo", "user": user}),
        Picture::Outside { url } => json!({"kind": "outside", "url": url}),
    }
}

/// A framed tag ("External", "App") in the line: drawn whole.
fn tag(ui: &mut Ui, t: &Tokens, text: &str) -> egui::Response {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| ui.label(RichText::new(text).size(11.0).color(t.dim)))
        .response
}

/// A presence's word and colour; none when it isn't known.
fn presence(p: Presence, t: &Tokens) -> Option<(&'static str, Color32)> {
    let word = match p {
        Presence::Available => "Available",
        Presence::Busy => "Busy",
        Presence::Away => "Away",
        Presence::DoNotDisturb => "Do not disturb",
        Presence::Offline => "Offline",
        Presence::Unknown => return None,
    };
    let color = match p {
        Presence::Available => t.ok,
        Presence::Away => t.warn,
        Presence::Offline => t.faint,
        _ => t.danger,
    };
    Some((crate::i18n::tc("presence", word), color))
}

/// The header: "External" first when someone is from outside, the name, a one-to-one chat's
/// other person's presence, and the members (their list on a click); a chat the user is no
/// longer in says so under it.
fn header(p: &Pane, ui: &mut Ui, row: &ChatRow) {
    let t = &p.t;
    ui.horizontal(|ui| {
        if row.external {
            tag(ui, t, tl!("External")).on_hover_text(tl!("Someone in this conversation is outside your organisation"));
        }
        let room = (ui.available_width() - 140.0).max(60.0);
        let g = widgets::one_line(ui, &one_line(&row.name), theme::bold(17.0), t.text, room);
        ui.label(g);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| members(p, ui, row));
    });
    let shown =
        row.person.as_deref().filter(|_| row.kind == ChatKind::OneOnOne).and_then(|u| p.s.teams.presence.get(u)).and_then(|&pr| presence(pr, t));
    if let Some((word, color)) = shown {
        ui.horizontal(|ui| {
            let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().circle_filled(dot.center(), 4.5, color);
            ui.label(RichText::new(word).size(12.5).color(t.dim));
        });
    }
    if row.left {
        ui.label(RichText::new(tl!("You're no longer in this conversation")).size(12.5).color(t.dim));
    }
}

/// "Members: n", and on a click their list, each from outside marked "External".
fn members(p: &Pane, ui: &mut Ui, row: &ChatRow) {
    let t = &p.t;
    let list = p.s.teams.members().as_slice();
    let n = if list.is_empty() { row.members as usize } else { list.len() };
    ui.menu_button(tf!("Members: {n}", n = n), |ui| {
        ui.set_min_width(240.0);
        egui::ScrollArea::vertical().max_height(320.0).show_rows(ui, 24.0, list.len(), |ui, range| {
            for m in list.get(range).unwrap_or_default() {
                ui.horizontal(|ui| {
                    ui.set_height(22.0);
                    if !is_internal(m, p.tenant) {
                        tag(ui, t, tl!("External"));
                    }
                    let g = widgets::one_line(ui, &one_line(&m.name), theme::regular(13.0), t.text, ui.available_width());
                    ui.label(g);
                });
            }
        });
    });
}

/// A chat Graph won't let be read: what the list gave of it, and "Open in Teams".
fn unreadable(p: &Pane, ui: &mut Ui, row: &ChatRow, acts: &mut Vec<Action>) {
    let t = &p.t;
    ui.add_space(48.0);
    ui.vertical_centered(|ui| {
        ui.set_max_width(520.0);
        let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
        icons::paint(ui.painter(), r, Icon::Info, t.dim);
        ui.add_space(8.0);
        ui.label(RichText::new(tl!("Teams doesn't let this conversation be read here")).size(15.0).color(t.text));
        let last = one_line(&row.last_line);
        if !last.is_empty() {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                // Centred: the tag's width (its text and frame) and the line's, measured first.
                let mark =
                    row.last_external.then(|| ui.painter().layout_no_wrap(tl!("External").to_string(), theme::regular(11.0), t.dim).size().x + 10.0);
                let room = ui.available_width() - mark.map_or(0.0, |w| w + ui.spacing().item_spacing.x);
                let g = widgets::one_line(ui, &last, theme::regular(13.0), t.dim, room);
                let wide = g.size().x + mark.map_or(0.0, |w| w + ui.spacing().item_spacing.x);
                ui.add_space(((ui.available_width() - wide) / 2.0).max(0.0));
                if row.last_external {
                    tag(ui, t, tl!("External")).on_hover_text(tl!("This sender is outside your organisation."));
                }
                ui.label(g);
            });
        }
        ui.add_space(14.0);
        p.open_in_teams(ui, acts);
    });
}

/// A day's heading: "Today", "Yesterday", else the date.
fn day_label(s: &Session, day: i64) -> String {
    match dates::today(&s.clock) - day {
        0 => tl!("Today").to_string(),
        1 => tl!("Yesterday").to_string(),
        _ => crate::i18n::date(&dates::day_label(day)),
    }
}

/// Seconds since 1970 from a message's milliseconds.
fn secs(ms: i64) -> i64 {
    ms.div_euclid(1000)
}

/// Who sent it, to group one sender's messages in a row: none for a system line.
fn sender_key(m: &ChatMessage) -> Option<String> {
    match &m.sender {
        _ if m.kind == MessageKind::System => None,
        Sender::User { id, .. } => Some(format!("u:{}", id.to_ascii_lowercase())),
        Sender::App { id, .. } => Some(format!("a:{id}")),
        Sender::Nobody => Some(String::new()),
    }
}

/// The messages, scrolled to the newest at first; scrolled to the top, the page before is asked
/// for (once a page), and when it comes in above the view stays where it was.
fn messages(p: &Pane, c: &mut ConversationsUi, ui: &mut Ui, chat: &str, acts: &mut Vec<Action>) {
    let window = &p.s.teams.window;
    let front = window.front().map(|m| m.id.clone());
    // Older messages came in above what was on screen.
    let above = match (&c.chat_view, &front) {
        (Some((was_chat, was_front, ..)), Some(now)) => was_chat == chat && was_front != now && window.iter().any(|m| m.id == *was_front),
        _ => false,
    };
    let mut area = egui::ScrollArea::vertical().id_salt(("teams-chat", chat)).stick_to_bottom(true).auto_shrink([false, false]);
    if let Some(y) = c.chat_scroll_to.take() {
        area = area.vertical_scroll_offset(y);
    }
    let out = area.show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin::symmetric(16, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let mut day = None;
            let mut group: Option<Option<String>> = None;
            for m in window {
                let d = p.s.clock.local_day(secs(m.created));
                if day != Some(d) {
                    day = Some(d);
                    group = None;
                    day_heading(p, ui, d);
                }
                let key = sender_key(m);
                let first = group.as_ref() != Some(&key);
                group = Some(key.clone());
                if key.is_none() {
                    system(p, ui, m);
                    group = None;
                } else {
                    message(p, c, ui, m, first, acts);
                }
            }
            ui.add_space(8.0);
        });
    });
    let (height, offset, view) = (out.content_size.y, out.state.clamped_offset().y, out.inner_rect.height());
    if above && let Some((_, _, was_height, was_offset)) = c.chat_view.clone() {
        c.chat_scroll_to = Some(was_offset + (height - was_height).max(0.0));
        ui.ctx().request_discard("older messages came in above");
    } else if c.chat_view.as_ref().is_some_and(|v| v.0 == chat)
        && height > view + 1.0
        && offset <= TOP
        && let Some(first) = window.front()
        && c.older_asked != Some(first.created)
    {
        c.older_asked = Some(first.created);
        acts.push(Action::Run("teams.older", json!({"chat": chat})));
    }
    c.chat_view = front.map(|f| (chat.to_string(), f, height, offset));
}

/// A day's heading, centred between two lines.
fn day_heading(p: &Pane, ui: &mut Ui, day: i64) {
    let t = &p.t;
    ui.add_space(12.0);
    let label = day_label(p.s, day);
    let g = ui.painter().layout_no_wrap(label, theme::bold(12.0), t.dim);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), g.size().y + 4.0), Sense::hover());
    let (mid, w) = (r.center(), g.size().x);
    let stroke = egui::Stroke::new(1.0, t.border);
    ui.painter().hline(r.left()..=(mid.x - w / 2.0 - 10.0), mid.y, stroke);
    ui.painter().hline((mid.x + w / 2.0 + 10.0)..=r.right(), mid.y, stroke);
    ui.painter().galley(mid - g.size() / 2.0, g, t.dim);
    ui.add_space(4.0);
}

/// A system line (someone added, the chat renamed): one centred dim line; none when it says
/// nothing.
fn system(p: &Pane, ui: &mut Ui, m: &ChatMessage) {
    let line = text::system_line(&m.body.plain());
    if line.is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.vertical_centered(|ui| {
        let g = widgets::one_line(ui, &line, theme::regular(12.0), p.t.dim, ui.available_width());
        ui.label(g);
    });
}

/// A message's time, and "(edited)" after it when it was.
fn time_line(p: &Pane, ui: &mut Ui, m: &ChatMessage) {
    let c = dates::civil(secs(m.created), &p.s.clock);
    if m.edited.is_some() && m.deleted.is_none() {
        ui.label(RichText::new(tl!("(edited)")).size(11.0).color(p.t.dim));
    }
    ui.label(RichText::new(format!("{:02}:{:02}", c.hour, c.minute)).size(11.0).color(p.t.dim));
}

/// One message: the user's own on the right on the accent's tint; another's on the left, the
/// first of a row under its sender's picture and name.
fn message(p: &Pane, c: &mut ConversationsUi, ui: &mut Ui, m: &ChatMessage, first: bool, acts: &mut Vec<Action>) {
    let t = &p.t;
    ui.add_space(if first { 10.0 } else { 3.0 });
    let width = (ui.available_width() * 0.7).clamp(180.0, 620.0);
    if p.is_own(m) {
        ui.with_layout(Layout::top_down(Align::Max), |ui| {
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                bubble(p, c, ui, m, t.accent_soft, width, acts);
                ui.with_layout(Layout::top_down(Align::Max), |ui| time_line(p, ui, m));
            });
            parts::reactions(p, ui, m);
        });
        return;
    }
    ui.horizontal_top(|ui| {
        let (pic, _) = ui.allocate_exact_size(vec2(32.0, if first { 32.0 } else { 1.0 }), Sense::hover());
        if first {
            avatar(p, ui, pic.center(), &m.sender);
        }
        ui.add_space(6.0);
        ui.vertical(|ui| {
            if first {
                name_line(p, ui, m);
            }
            ui.horizontal_top(|ui| {
                bubble(p, c, ui, m, if t.dark { t.chrome } else { Color32::from_rgb(0xF3, 0xF4, 0xF6) }, width, acts);
                ui.vertical(|ui| time_line(p, ui, m));
            });
            parts::reactions(p, ui, m);
        });
    });
}

/// The sender's picture: their photo once it came, else their initials.
fn avatar(p: &Pane, ui: &mut Ui, at: egui::Pos2, sender: &Sender) {
    let name = p.sender_name(sender);
    let key = match sender {
        Sender::User { id, .. } | Sender::App { id, .. } => id.as_str(),
        Sender::Nobody => "",
    };
    if let Sender::User { id, .. } = sender
        && let Some(tex) = p.pics.photo(id)
    {
        let r = egui::Rect::from_center_size(at, vec2(32.0, 32.0));
        egui::Image::new(tex).corner_radius(16.0).paint_at(ui, r);
        return;
    }
    let initials: String = name.split(' ').filter_map(|w| w.chars().find(|c| c.is_alphanumeric())).take(2).flat_map(char::to_uppercase).collect();
    widgets::avatar(ui, at, 16.0, &initials, theme::avatar_color(key, p.t.dark));
}

/// Above the first of a row: "External" first when the sender isn't proven of the
/// organisation, the name, "App" after an application's.
fn name_line(p: &Pane, ui: &mut Ui, m: &ChatMessage) {
    let t = &p.t;
    ui.horizontal(|ui| {
        let user = matches!(m.sender, Sender::User { .. });
        if user && !p.s.teams.members().sender_internal(&m.sender, p.tenant) {
            tag(ui, t, tl!("External")).on_hover_text(tl!("This sender is outside your organisation."));
        }
        let name = p.sender_name(&m.sender);
        let room = (ui.available_width() - 50.0).max(40.0);
        let g = widgets::one_line(ui, &name, theme::bold(12.5), t.text, room);
        ui.label(g);
        if matches!(m.sender, Sender::App { .. }) {
            tag(ui, t, tl!("App"));
        }
    });
}

/// The bubble: the text (or what became of it), its pictures, files and cards.
#[allow(clippy::too_many_arguments)]
fn bubble(p: &Pane, c: &mut ConversationsUi, ui: &mut Ui, m: &ChatMessage, fill: Color32, width: f32, acts: &mut Vec<Action>) {
    let t = &p.t;
    egui::Frame::new().fill(fill).corner_radius(8).inner_margin(egui::Margin::symmetric(10, 7)).show(ui, |ui| {
        ui.set_max_width(width);
        // The user's own hug the right, others' the left.
        let align = if p.is_own(m) { Align::Max } else { Align::Min };
        ui.with_layout(Layout::top_down(align), |ui| {
            let said = if m.deleted.is_some() {
                Some(tl!("This message was deleted"))
            } else if m.unshowable {
                Some(tl!("This message can't be shown"))
            } else {
                None
            };
            if let Some(said) = said {
                ui.label(RichText::new(said).italics().color(t.dim));
                return;
            }
            let cx = text::Context { me: p.me, members: p.s.teams.members(), color: t.text };
            text::body(ui, t, m, &cx, width - 20.0, acts);
            parts::pictures(p, c, ui, m, acts);
            parts::files(p, ui, m, width - 20.0, acts);
            parts::cards(p, ui, m, acts);
        });
    });
}

/// The picture clicked, big, over the window: Escape, a click outside or Close puts it away.
fn viewer(p: &Pane, c: &mut ConversationsUi, ctx: &egui::Context) {
    let Some(picture) = c.viewing.clone() else { return };
    let Some(Shown::Ready(thumb)) = p.pics.image(&picture) else {
        c.viewing = None;
        return;
    };
    // Whole once decoded; until then the thumbnail.
    let tex = p.pics.whole(&picture).unwrap_or(thumb);
    let t = &p.t;
    let modal = egui::Modal::new(egui::Id::unique("teams-picture")).frame(crate::dialogs::dialog_frame(t)).show(ctx, |ui| {
        let w = (ctx.content_rect().width() * 0.75).clamp(320.0, 1200.0);
        let h = (ctx.content_rect().height() * 0.7).max(240.0);
        let size = tex.size_vec2();
        let scale = (w / size.x.max(1.0)).min(h / size.y.max(1.0)).min(1.0);
        ui.vertical_centered(|ui| ui.add(egui::Image::new(tex).fit_to_exact_size(size * scale)));
        ui.add_space(8.0);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| ui.button(tl!("Close")).clicked()).inner
    });
    if modal.inner || modal.should_close() {
        c.viewing = None;
    }
}
