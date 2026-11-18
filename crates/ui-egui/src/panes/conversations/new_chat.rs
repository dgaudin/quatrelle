// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A new conversation (decision 14), written where the open chat is: Ctrl+N or the list's "New
//! conversation" opens it ([`open`]), Cancel closes it, what was typed let go; Esc sets it aside as
//! it is, for New conversation to bring back (it goes to a popup over the pane first, then to the
//! people listed under "To:": [`escape`]). The chat it hides isn't on screen (`MailApp::announce`
//! tells the engine so); a chat opened from behind it takes its place, the pane set aside the same
//! way ([`leave`], [`each_frame`]). "To:" holds the people picked as chips ("External" first for a
//! guest, a cross that takes one away, Backspace in the empty field the last one), then a field
//! whose text is looked up in the organisation's directory [`SEARCH_WAIT`] after the last key
//! (`teams.people`, once for each text); the people found are listed under it, those picked left
//! out: ↑ and ↓ choose, Enter or a click picks. At most [`NEW_CHAT_PEOPLE`]; from two on, a group
//! name. At the foot, a chat's box (`writing.rs`, its draft under [`NEW`]: bold, italic, emoji,
//! files and pictures; no "@" offer, no ↑): Enter there creates the conversation with its first
//! message ([`send`]), what was written staying in the pane until the chat is made. While it is
//! created, everything waits, Cancel too (the request is out: its answer settles it); a failure
//! the pane asked for says why, the people, the name, the text and the files still there to try
//! again; the chat made, or the one-to-one chat already there, takes the pane's place
//! ([`each_frame`]).
//!
//! What is hostile here: the directory's names and addresses (Graph's), drawn on one line through
//! [`one_line`] (no control or bidi character) and as text, never as markup; Teams' code for a
//! refusal, on one line. Only the people the directory gave are listed, and the engine takes only
//! those (H13). What the pane asks is bounded: [`PEOPLE_QUERY_CHARS`] characters, a look-up once
//! for each text and only after the pause; what it draws by [`PEOPLE_FOUND`] and
//! [`NEW_CHAT_PEOPLE`].

use egui::{Align, Color32, Key, Layout, Modifiers, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use quatrelle_engine::model::pim::teams::{NAME_CHARS, NEW_CHAT_PEOPLE, NotCreated, PEOPLE_FOUND, PEOPLE_QUERY_CHARS, Person, one_line};
use quatrelle_engine::{Session, TeamsCreating};
use serde_json::{Map, Value, json};

use super::ConversationsUi;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, widgets};

/// The new conversation's draft's key: no chat has an empty id.
pub(crate) const NEW: &str = "";
/// The pause after the last key in "To:" before the directory is asked, in seconds.
const SEARCH_WAIT: f64 = 0.3;
/// A person found's row, and the widest the rows are.
const ROW_H: f32 = 30.0;
const ROW_W: f32 = 520.0;
/// The narrowest "To:"'s field is beside the chips (else it starts a line of its own).
const FIELD_MIN: f32 = 180.0;
/// The widest a chip's name is drawn, and the group name's field.
const CHIP_NAME: f32 = 220.0;
const TOPIC_W: f32 = 360.0;

/// What the new conversation's pane holds between frames.
#[derive(Clone, Debug, Default)]
pub(crate) struct NewChat {
    /// What's typed in "To:".
    pub(crate) to: String,
    /// The people picked, in the order picked (at most [`NEW_CHAT_PEOPLE`]).
    pub(crate) picked: Vec<Person>,
    /// The group's name (sent from two people on).
    pub(crate) topic: String,
    /// When what's typed in "To:" is to be looked up (the window's clock, in seconds).
    pub(crate) search_due: Option<f64>,
    /// The person chosen among those listed (↑ and ↓).
    pub(crate) chosen: usize,
    /// "To:" is to get the keyboard.
    pub(crate) focus: bool,
    /// The pane asked for the conversation and the engine took it: a failure since is the pane's
    /// own, and said (one left from before it opened isn't).
    pub(super) asked: bool,
    /// A chat was opened from behind the pane: at the next frame it takes the pane's place.
    pub(super) leaving: bool,
}

/// What the directory gave for what's typed in "To:".
enum Found<'a> {
    /// Nothing typed (or no one more can be picked).
    Nothing,
    Searching,
    Failed,
    /// The people found, those picked left out.
    People(Vec<&'a Person>),
}

/// "To:"'s field's id.
fn to_id() -> egui::Id {
    egui::Id::unique("teams-new-chat-to")
}

/// The pane opened, or "To:" given the keyboard again, while Teams is on and signed in: the one
/// set aside for a chat opened meanwhile comes back as it was. The open chat's delete question and
/// the picture shown big go.
pub(crate) fn open(s: &Session, c: &mut ConversationsUi) {
    if s.prefs.teams.on && s.prefs.teams.signed_in {
        (c.deleting, c.viewing) = (None, None);
        let n = c.new_chat.get_or_insert_with(|| c.new_chat_aside.take().unwrap_or_default());
        (n.focus, n.leaving) = (true, false);
    }
}

/// The pane closed (shown or set aside): what was written goes, the files of its pasted pictures
/// with it, unless a creation is waited for (its first message reads them).
fn close(s: &Session, c: &mut ConversationsUi) {
    let (shown, aside) = (c.new_chat.take(), c.new_chat_aside.take());
    if shown.is_some() || aside.is_some() {
        super::writing::drop_draft(c, NEW, matches!(s.teams.creating, Some(TeamsCreating::Waiting { .. })));
    }
}

/// A chat opened from behind the pane (a row of the list, Alt+↑ or ↓): it takes the pane's place
/// at the next frame ([`each_frame`]).
pub(crate) fn leave(c: &mut ConversationsUi) {
    if let Some(n) = c.new_chat.as_mut() {
        n.leaving = true;
    }
}

/// Esc in Conversations while the pane is open (`app/keys.rs`, once a dialog had it): a popup over
/// the pane (the emoji picker…) closes by itself, and nothing else goes; with people listed under
/// "To:", "To:" is emptied (the list goes, the keyboard stays there); else the pane is set aside as
/// it is (nothing typed is lost by a key: only Cancel lets it go), New conversation bringing it
/// back.
pub(crate) fn escape(ctx: &egui::Context, s: &Session, c: &mut ConversationsUi) {
    if c.deleting.is_some() || egui::Popup::is_any_open(ctx) {
        return;
    }
    let Some(n) = c.new_chat.as_mut() else { return };
    if matches!(found(s, n), Found::Nothing) {
        c.new_chat_aside = c.new_chat.take().map(|n| NewChat { leaving: false, focus: false, ..n });
    } else {
        n.to.clear();
        // The people found forgotten at the next frame (`ask`: `teams.people` of nothing).
        (n.chosen, n.search_due) = (0, Some(0.0));
    }
}

/// What the pane learns each frame, before the window tells the engine what's on screen
/// (`MailApp::announce`): Teams off or signed out, it goes; the chat a creation ended in, or the
/// one-to-one chat found there already (the engine shows it), closes it, its draft going, its files
/// the message's now; a chat opened from behind it takes its place, the pane set aside as it is.
pub(crate) fn each_frame(s: &mut Session, c: &mut ConversationsUi) {
    if !(s.prefs.teams.on && s.prefs.teams.signed_in) {
        close(s, c);
    }
    if s.take_teams_created().is_some() {
        (c.new_chat, c.new_chat_aside) = (None, None);
        super::writing::drop_draft(c, NEW, true);
    }
    if c.new_chat.as_ref().is_some_and(|n| n.leaving) {
        c.new_chat_aside = c.new_chat.take().map(|n| NewChat { leaving: false, ..n });
    }
}

/// Enter in the box with someone picked and something to send (`message`, the box's params):
/// the conversation asked for with them, the group's name from two people on, and that first
/// message.
pub(super) fn send(n: &mut NewChat, mut message: Map<String, Value>, acts: &mut Vec<Action>) {
    let people: Vec<&str> = n.picked.iter().map(|p| p.id.as_str()).collect();
    message.insert("people".into(), json!(people));
    let topic = n.topic.trim();
    if n.picked.len() >= 2 && !topic.is_empty() {
        message.insert("topic".into(), json!(topic));
    }
    n.asked = true;
    acts.push(Action::Run("teams.create", Value::Object(message)));
}

/// What the directory gave for what's typed in "To:", those picked left out.
fn found<'a>(s: &'a Session, n: &NewChat) -> Found<'a> {
    let query = n.to.trim();
    if query.is_empty() || n.picked.len() >= NEW_CHAT_PEOPLE {
        return Found::Nothing;
    }
    match s.teams.people.as_ref().filter(|p| p.query == query) {
        None => Found::Searching,
        Some(p) if p.waiting => Found::Searching,
        Some(p) if p.failed => Found::Failed,
        Some(p) => {
            let picked = |f: &Person| n.picked.iter().any(|q| q.id.eq_ignore_ascii_case(&f.id));
            Found::People(p.people.iter().take(PEOPLE_FOUND).filter(|f| !picked(f)).collect())
        }
    }
}

/// `p` picked: a chip, "To:" emptied with the keyboard kept there, and the people found forgotten
/// (`teams.people` of nothing). None past [`NEW_CHAT_PEOPLE`], nor one picked already.
fn pick(n: &mut NewChat, p: &Person, ui: &Ui, acts: &mut Vec<Action>) {
    if n.picked.len() >= NEW_CHAT_PEOPLE || n.picked.iter().any(|q| q.id.eq_ignore_ascii_case(&p.id)) {
        return;
    }
    n.picked.push(p.clone());
    n.to.clear();
    (n.chosen, n.search_due) = (0, None);
    acts.push(Action::Run("teams.people", json!({"query": ""})));
    ui.memory_mut(|m| m.request_focus(to_id()));
}

/// The pane: the heading and Cancel, where the creation stands, "To:" and the group's name; the
/// box at the foot; the people found between. While the conversation is created, everything waits,
/// Cancel too.
pub(crate) fn pane(s: &Session, c: &mut ConversationsUi, ui: &mut Ui, t: &Tokens, acts: &mut Vec<Action>) {
    // Teams switched off or signed out meanwhile: nothing can be created.
    if !(s.prefs.teams.on && s.prefs.teams.signed_in) {
        close(s, c);
        return;
    }
    let waiting = matches!(s.teams.creating, Some(TeamsCreating::Waiting { .. }));
    let mut cancel = false;
    egui::Frame::new().inner_margin(egui::Margin::symmetric(16, 10)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!("New conversation")).font(theme::bold(17.0)).color(t.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| cancel = ui.add_enabled(!waiting, egui::Link::new(tl!("Cancel"))).clicked());
        });
        // Cancel first: nothing typed or pressed in the same frame goes out.
        if cancel {
            return;
        }
        let Some(n) = c.new_chat.as_mut() else { return };
        state(s, n, ui, t);
        if waiting {
            ui.disable();
        }
        ui.add_space(6.0);
        to_line(s, n, ui, t, acts);
        if n.picked.len() >= 2 {
            ui.add_space(6.0);
            let name = egui::TextEdit::singleline(&mut n.topic).hint_text(tl!("Group name (optional)")).char_limit(NAME_CHARS);
            ui.add(name.desired_width(ui.available_width().min(TOPIC_W)));
        }
    });
    if cancel {
        close(s, c);
        return;
    }
    let line = ui.available_rect_before_wrap();
    ui.painter().hline(line.x_range(), line.top(), egui::Stroke::new(1.0, t.border));
    if waiting {
        ui.disable();
    }
    // The box first: it takes the foot of the pane, the people found the rest.
    super::writing::writing(s, c, ui, t, NEW, acts);
    if let Some(n) = c.new_chat.as_mut() {
        let area = egui::ScrollArea::vertical().id_salt("teams-new-chat-found").auto_shrink([false, false]);
        let clicked =
            area.show(ui, |ui| egui::Frame::new().inner_margin(egui::Margin::symmetric(16, 8)).show(ui, |ui| people_found(s, n, ui, t)).inner);
        if let Some(p) = clicked.inner {
            pick(n, &p, ui, acts);
        }
    }
}

/// Where the creation stands, under the heading: "Creating the conversation…" while it's waited
/// for; a failure the pane asked for, in the danger colour, and why.
fn state(s: &Session, n: &NewChat, ui: &mut Ui, t: &Tokens) {
    let line = |ui: &mut Ui, text: &str, color: Color32| {
        ui.add(egui::Label::new(RichText::new(text).size(12.5).color(color)).wrap());
    };
    match &s.teams.creating {
        Some(TeamsCreating::Waiting { .. }) => line(ui, tl!("Creating the conversation…"), t.dim),
        Some(TeamsCreating::Failed { why, code }) if n.asked => {
            line(ui, tl!("Teams didn't create the conversation"), t.danger);
            line(ui, &why_not(*why, code.as_deref()), t.dim);
        }
        _ => {}
    }
}

/// Why a conversation wasn't created, as the window says it (Teams' code on one line).
fn why_not(why: NotCreated, code: Option<&str>) -> String {
    match why {
        NotCreated::Offline => tl!("Teams can't be reached: try again").to_string(),
        NotCreated::Refused => match code {
            Some(code) => format!("{} ({})", tl!("Teams refused it"), one_line(code)),
            None => tl!("Teams refused it").to_string(),
        },
        NotCreated::TooMany => tl!("Another conversation is being created").to_string(),
    }
}

/// "To:": the people picked as chips, then the field (none once [`NEW_CHAT_PEOPLE`] are: it says
/// so), its own keys taken first while it has the keyboard; what's typed looked up after the pause.
fn to_line(s: &Session, n: &mut NewChat, ui: &mut Ui, t: &Tokens, acts: &mut Vec<Action>) {
    let id = to_id();
    if ui.is_enabled() && ui.memory(|m| m.has_focus(id)) {
        keys(s, n, ui, acts);
    }
    let focused = ui.memory(|m| m.has_focus(id));
    let frame = egui::Frame::new()
        .fill(if t.dark { t.surface } else { Color32::WHITE })
        .stroke(Stroke::new(1.0, if focused { t.accent } else { t.border }))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(8, 4));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tl!("To:")).size(13.5).color(t.dim));
            // Each drawn, the one whose cross was clicked taken away after.
            let mut gone = None;
            for (i, p) in n.picked.iter().enumerate() {
                if chip(ui, t, p) {
                    gone = Some(i);
                }
            }
            if let Some(i) = gone {
                n.picked.remove(i);
                n.chosen = 0;
            }
            if n.picked.len() >= NEW_CHAT_PEOPLE {
                ui.label(RichText::new(tl!("At most 20 people")).size(13.0).color(t.dim));
                return;
            }
            if ui.available_width() < FIELD_MIN {
                ui.end_row();
            }
            let listing = !matches!(found(s, n), Found::Nothing);
            let field = egui::TextEdit::singleline(&mut n.to)
                .id(id)
                .frame(egui::Frame::NONE)
                .hint_text(tl!("A name or an address"))
                .char_limit(PEOPLE_QUERY_CHARS)
                .return_key(None::<egui::KeyboardShortcut>)
                .desired_width(ui.available_width());
            let resp = ui.add(field);
            if std::mem::take(&mut n.focus) {
                resp.request_focus();
            }
            // While people are listed, Esc is the field's (it empties it: `escape`), not egui's
            // (which would take the keyboard away).
            if listing && resp.has_focus() {
                let filter = egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, escape: true, ..Default::default() };
                ui.memory_mut(|m| m.set_focus_lock_filter(id, filter));
            }
            if resp.changed() {
                n.search_due = Some(ui.input(|i| i.time) + SEARCH_WAIT);
                n.chosen = 0;
            }
        });
    });
    ask(s, n, ui, acts);
}

/// "To:"'s own keys, before it takes the frame's typing: ↑ and ↓ choose among the people found,
/// Enter picks the one chosen, Backspace in the empty field (nothing typed in the frame) takes
/// the last chip away.
fn keys(s: &Session, n: &mut NewChat, ui: &mut Ui, acts: &mut Vec<Action>) {
    let people: Vec<Person> = match found(s, n) {
        Found::People(p) => p.into_iter().cloned().collect(),
        _ => Vec::new(),
    };
    if !people.is_empty() {
        let (up, down) = ui.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::ArrowUp), i.consume_key(Modifiers::NONE, Key::ArrowDown)));
        let last = people.len().saturating_sub(1);
        n.chosen = n.chosen.min(last);
        if up {
            n.chosen = n.chosen.saturating_sub(1);
        }
        if down {
            n.chosen = n.chosen.saturating_add(1).min(last);
        }
    }
    if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
        && let Some(p) = people.get(n.chosen)
    {
        pick(n, p, ui, acts);
    }
    let typing = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Text(_) | egui::Event::Paste(_))));
    if n.to.is_empty() && !typing && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Backspace)) && n.picked.pop().is_some() {
        n.chosen = 0;
    }
}

/// What's typed in "To:" looked up once the pause after the last key is over (emptied, the
/// people found forgotten at once): once for each text, not again while the directory has it.
fn ask(s: &Session, n: &mut NewChat, ui: &Ui, acts: &mut Vec<Action>) {
    let Some(due) = n.search_due else { return };
    let now = ui.input(|i| i.time);
    let query = n.to.trim();
    if now < due && !query.is_empty() {
        if let Ok(wait) = std::time::Duration::try_from_secs_f64((due - now).max(0.01)) {
            ui.ctx().request_repaint_after(wait);
        }
        return;
    }
    n.search_due = None;
    let known = s.teams.people.as_ref();
    if known.is_some_and(|p| p.query == query && !p.failed) || (query.is_empty() && known.is_none()) {
        return;
    }
    acts.push(Action::Run("teams.people", json!({"query": query})));
}

/// A person picked, as a chip: "External" first for a guest, the name on one line, and a cross
/// that takes them away (whether it was clicked).
fn chip(ui: &mut Ui, t: &Tokens, p: &Person) -> bool {
    let name = one_line(&p.name);
    let frame = egui::Frame::new().fill(t.accent_soft).corner_radius(11).inner_margin(egui::Margin { left: 8, right: 3, top: 2, bottom: 2 });
    frame
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if p.guest {
                    super::chat::tag(ui, t, tl!("External"));
                }
                let g = widgets::one_line(ui, &name, theme::regular(13.0), t.text, CHIP_NAME);
                ui.label(g);
                let (r, x) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                widgets::named(ui, &x, &format!("{}: {name}", tl!("Remove")), true, None);
                if x.hovered() {
                    ui.painter().rect_filled(r, 9.0, t.hover);
                }
                icons::paint(ui.painter(), icons::centered(r, 9.0), Icon::Close, if x.hovered() { t.text } else { t.dim });
                x.on_hover_text(tl!("Remove")).clicked()
            })
            .inner
        })
        .inner
}

/// The people found for what's typed in "To:": "Searching…" while the directory is asked, each
/// person on a row ("External" first for a guest, the name, the address dim; the one chosen on
/// the accent's soft tint), "No one found", or the directory not searched. The one clicked.
fn people_found(s: &Session, n: &NewChat, ui: &mut Ui, t: &Tokens) -> Option<Person> {
    let note = |ui: &mut Ui, text: &str, color: Color32| {
        ui.label(RichText::new(text).size(12.5).color(color));
    };
    let people = match found(s, n) {
        Found::Nothing => return None,
        Found::Searching => {
            note(ui, tl!("Searching…"), t.dim);
            return None;
        }
        Found::Failed => {
            note(ui, tl!("The directory couldn't be searched"), t.danger);
            return None;
        }
        Found::People(p) if p.is_empty() => {
            note(ui, tl!("No one found"), t.dim);
            return None;
        }
        Found::People(p) => p,
    };
    let chosen = n.chosen.min(people.len().saturating_sub(1));
    let mut clicked = None;
    ui.spacing_mut().item_spacing.y = 0.0;
    let w = ui.available_width().min(ROW_W);
    for (k, p) in people.into_iter().enumerate() {
        let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::click());
        let name = one_line(&p.name);
        widgets::named(ui, &resp, &name, true, None);
        let painter = ui.painter();
        if k == chosen || resp.hovered() {
            painter.rect_filled(rect, 4.0, if k == chosen { t.accent_soft } else { t.hover_row });
        }
        let (mut x, mid) = (rect.left() + 8.0, rect.center().y);
        if p.guest {
            // "External", framed as a tag, drawn whole before the name.
            let g = painter.layout_no_wrap(tl!("External").to_string(), theme::regular(11.0), t.dim);
            let tag = Rect::from_min_size(pos2(x, mid - g.size().y / 2.0 - 1.0), g.size() + vec2(10.0, 2.0));
            painter.rect_stroke(tag, 4.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
            painter.galley(tag.min + vec2(5.0, 1.0), g, t.dim);
            x = tag.right() + 6.0;
        }
        let g = widgets::one_line(ui, &name, theme::regular(13.5), t.text, (rect.right() - 8.0 - x).max(40.0));
        let after = x + g.size().x + 10.0;
        painter.galley(pos2(x, mid - g.size().y / 2.0), g, t.text);
        let address = one_line(&p.address);
        let room = rect.right() - 8.0 - after;
        if !address.is_empty() && room > 24.0 {
            let g = widgets::one_line(ui, &address, theme::regular(12.0), t.dim, room);
            painter.galley(pos2(after, mid - g.size().y / 2.0), g, t.dim);
        }
        if resp.clicked() {
            clicked = Some(p.clone());
        }
    }
    clicked
}
