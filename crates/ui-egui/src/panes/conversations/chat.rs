// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The conversation pane (the module's right): the open chat's header (its name, the other
//! person's presence, its members, "Open in Teams", "Call": `header.rs`), then its messages by day, one sender's in a
//! row under one name and picture, the user's own on the right; system lines; the page before
//! asked for when the user scrolls to the top; opened at a search's hit, scrolled to it once and
//! the message tinted a moment, and, the newer ones left out, "Go to the newest messages" under
//! the last; after them, what the user sent that Teams doesn't have yet (`outgoing.rs`); and the
//! box to write in at the foot (`super::writing`). A message's reactions are chips under it
//! (`reactions.rs`); the pointer on it shows its bar (`hover.rs`: reactions, Copy, Edit and
//! Delete), a deletion is asked first; an edit going out draws its words dimmed in the bubble, a
//! deletion dims it, and either says where it stands under it. A chat Graph won't let be read shows
//! what the list gave of it and "Open in Teams".
//!
//! What is hostile here: everything drawn came from senders (names, words, links, pictures,
//! files) or from Graph (the chat's link). "External" is drawn before the name a sender chose and
//! never cut; an application's message says "App". Names are on one line ([`one_line`]). A
//! picture kept with a message is asked for when the message is first drawn (at most
//! `HOSTED_PER_MESSAGE`); one from elsewhere only on a click (`parts.rs`). "Open in Teams" is
//! drawn only when the chat's link is one `is_teams_chat_link` takes, and asks for it through
//! `teams.openInTeams`: the engine checks it again, and the window opens it in the meetings
//! browser's window, never as a link from a message.

use egui::{Align, Color32, Layout, RichText, Sense, Ui, vec2};
use quatrelle_engine::Session;
use quatrelle_engine::model::dates;
use quatrelle_engine::model::pim::teams::{ChatMessage, ChatRow, MessageKind, OutKind, Outgoing, Picture, Presence, Sender, one_line};
use serde_json::json;

use super::ConversationsUi;
use super::pictures::{Shown, TeamsPictures};
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, UiState, widgets};

mod files;
mod header;
mod hover;
mod outgoing;
mod parts;
mod reactions;
mod text;

/// How near the top (in points) the chat is scrolled for the page before to be asked for.
const TOP: f32 = 40.0;
/// How long the message a search found stays tinted once it's scrolled to, in seconds.
const FOUND_TINT: f64 = 2.0;
/// How much what goes out is dimmed (a message, an edit's words, a bubble being deleted).
const DIMMED: f32 = 0.6;

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
    /// The chat takes changes: the user is still in it (and Teams lets it be read).
    writable: bool,
}

impl Pane<'_> {
    /// "Open in Teams", when the chat's link is one: asked of the engine, which hands the window
    /// the link to open in the meetings browser's window.
    fn open_in_teams(&self, ui: &mut Ui, acts: &mut Vec<Action>) {
        if self.teams_link.is_some() && ui.button(tl!("Open in Teams")).clicked() {
            acts.push(Action::Run("teams.openInTeams", json!({})));
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
    // A new conversation is written in the open chat's place.
    if ui_state.conversations.new_chat.is_some() {
        super::new_chat::pane(s, &mut ui_state.conversations, ui, &ui_state.tokens, acts);
        return;
    }
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
        c.went_to = None;
        c.hovered = None;
        c.deleting = None;
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
    let teams_link = s.teams.teams_link();
    let (me, tenant, writable) = (&s.prefs.teams.object_id, &s.prefs.teams.tenant_id, !row.left && !row.unreadable);
    let pane = Pane { s, t: *tokens, pics: &teams_queue.pictures, me, tenant, teams_link, writable };
    egui::Frame::new().inner_margin(egui::Margin::symmetric(16, 10)).show(ui, |ui| header::header(&pane, ui, row, acts));
    let line = ui.available_rect_before_wrap();
    ui.painter().hline(line.x_range(), line.top(), egui::Stroke::new(1.0, pane.t.border));
    if row.unreadable {
        unreadable(&pane, ui, row, acts);
    } else {
        // The box first: it takes the foot of the pane, the messages the rest.
        super::writing::writing(s, c, ui, &pane.t, open, acts);
        messages(&pane, c, ui, open, acts);
    }
    viewer(&pane, c, ui.ctx());
    hover::delete_question(&pane, c, ui.ctx(), acts);
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
pub(super) fn tag(ui: &mut Ui, t: &Tokens, text: &str) -> egui::Response {
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

/// The messages, scrolled to the newest at first, or, opened at a search's hit, to its message
/// once the window holds it (the scroll is the user's after), that message tinted
/// [`FOUND_TINT`] seconds; scrolled to the top, the page before is asked for (once a page), and
/// when it comes in above the view stays where it was.
fn messages(p: &Pane, c: &mut ConversationsUi, ui: &mut Ui, chat: &str, acts: &mut Vec<Action>) {
    let window = &p.s.teams.window;
    let now = ui.input(|i| i.time);
    let (go_to, opening) = (p.s.teams.go_to.as_deref(), p.s.teams.go_to_opening);
    if go_to.is_none() {
        c.went_to = None;
    }
    // When this opening was gone to, if it was.
    let gone = c.went_to.filter(|(n, _)| *n == opening).map(|(_, at)| at);
    // The hit's message to scroll to now: once for each opening at it, as soon as the window that
    // opening loaded holds it (a click draws the window it leaves first).
    let scroll = go_to.filter(|g| gone.is_none() && window.iter().any(|m| m.id == *g));
    let tint = go_to.filter(|_| scroll.is_some() || gone.is_some_and(|at| now - at < FOUND_TINT));
    let front = window.front().map(|m| m.id.clone());
    // Older messages came in above what was on screen (the view isn't going to the hit).
    let above = scroll.is_none()
        && match (&c.chat_view, &front) {
            (Some((was_chat, was_front, ..)), Some(now)) => was_chat == chat && was_front != now && window.iter().any(|m| m.id == *was_front),
            _ => false,
        };
    let mut area = egui::ScrollArea::vertical().id_salt(("teams-chat", chat)).stick_to_bottom(true).auto_shrink([false, false]);
    if let Some(y) = c.chat_scroll_to.take().filter(|_| scroll.is_none()) {
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
                // The hit's message: its tint painted behind it once its place is known.
                let behind = (tint == Some(m.id.as_str())).then(|| (ui.painter().add(egui::Shape::Noop), ui.cursor().top()));
                if key.is_none() {
                    system(p, ui, m);
                    group = None;
                } else {
                    message(p, c, ui, m, first, acts);
                }
                if let Some((at, top)) = behind {
                    let r = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=ui.min_rect().bottom()).expand2(vec2(8.0, 3.0));
                    ui.painter().set(at, egui::Shape::rect_filled(r, 6.0, p.t.hover_row));
                    if scroll == Some(m.id.as_str()) {
                        ui.scroll_to_rect_animation(r, Some(Align::Center), egui::style::ScrollAnimation::none());
                    }
                }
            }
            if p.s.teams.newer_left_out {
                newest_link(ui, &p.t, chat, acts);
            }
            outgoing::outgoing(p, ui, chat, acts);
            ui.add_space(8.0);
        });
    });
    if scroll.is_some() {
        c.went_to = Some((opening, now));
    }
    // The tint goes once its time is up.
    if let Some((_, at)) = &c.went_to {
        let left = FOUND_TINT - (now - *at);
        if left > 0.0
            && let Ok(wait) = std::time::Duration::try_from_secs_f64(left)
        {
            ui.ctx().request_repaint_after(wait);
        }
    }
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

/// Newer messages left out (opened at an older one): the way to them, under the last message
/// received, which opens the chat again at its newest.
fn newest_link(ui: &mut Ui, t: &Tokens, chat: &str, acts: &mut Vec<Action>) {
    ui.add_space(12.0);
    ui.vertical_centered(|ui| {
        // In the accent: the window's text colour would make it read as a line of the chat.
        if ui.link(RichText::new(tl!("Go to the newest messages")).size(13.0).color(t.accent)).clicked() {
            acts.push(Action::Run("teams.open", json!({"chat": chat})));
        }
    });
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
/// first of a row under its sender's picture and name. Its bubble and time are the row its bar
/// goes over; under them its chips, and its edit or deletion going out.
fn message(p: &Pane, c: &mut ConversationsUi, ui: &mut Ui, m: &ChatMessage, first: bool, acts: &mut Vec<Action>) {
    let t = &p.t;
    ui.add_space(if first { 10.0 } else { 3.0 });
    let width = (ui.available_width() * 0.7).clamp(180.0, 620.0);
    let change = p.s.teams.change_of(&m.id);
    // Chips take a click as the engine takes a reaction: not while the message is being deleted.
    let writable = p.writable && !change.is_some_and(|o| matches!(o.what, OutKind::Delete { .. }));
    let under = |c: &mut ConversationsUi, ui: &mut Ui, row: egui::Rect, acts: &mut Vec<Action>| {
        hover::bar(p, c, ui, m, row, acts);
        reactions::reactions(p, ui, m, writable, acts);
        if let Some(o) = change {
            outgoing::change_state(p, ui, o, acts);
        }
    };
    if p.is_own(m) {
        ui.with_layout(Layout::top_down(Align::Max), |ui| {
            let row = ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                bubble(p, c, ui, m, change, t.accent_soft, width, acts);
                ui.with_layout(Layout::top_down(Align::Max), |ui| time_line(p, ui, m));
            });
            under(c, ui, row.response.rect, acts);
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
            let row = ui.horizontal_top(|ui| {
                bubble(p, c, ui, m, change, if t.dark { t.chrome } else { Color32::from_rgb(0xF3, 0xF4, 0xF6) }, width, acts);
                ui.vertical(|ui| time_line(p, ui, m));
            });
            under(c, ui, row.response.rect, acts);
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

/// The bubble: the text (or what became of it), its pictures, files and cards; `change` its edit
/// going out (its words drawn dimmed instead) or its deletion (the bubble dimmed).
#[allow(clippy::too_many_arguments)]
fn bubble(
    p: &Pane,
    c: &mut ConversationsUi,
    ui: &mut Ui,
    m: &ChatMessage,
    change: Option<&Outgoing>,
    fill: Color32,
    width: f32,
    acts: &mut Vec<Action>,
) {
    let t = &p.t;
    let edit = change.filter(|o| matches!(o.what, OutKind::Edit { .. }));
    let deleting = change.is_some_and(|o| matches!(o.what, OutKind::Delete { .. }));
    ui.scope(|ui| {
        if deleting {
            ui.multiply_opacity(DIMMED);
        }
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
                let (body, mentions) = edit.map_or((&m.body, &m.mentions), |o| (&o.body, &o.mentions));
                ui.scope(|ui| {
                    if edit.is_some() {
                        ui.multiply_opacity(DIMMED);
                    }
                    text::body(ui, t, body, mentions, &cx, width - 20.0, acts);
                });
                parts::pictures(p, c, ui, m, acts);
                files::files(p, ui, m, width - 20.0, acts);
                parts::cards(p, ui, m, acts);
            });
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
