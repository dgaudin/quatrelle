// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The windows of their own: messages being written, and messages popped out.

use crate::dialogs::people_field::{MEETING_WINDOWS, places};
use crate::*;

mod file_page;
mod ribbon;

impl MailApp {
    /// Make message `id` the one acted on, with its editor (selection, undo) at hand.
    pub(crate) fn use_compose(&mut self, id: u64) -> bool {
        if !self.session.activate_compose(id) {
            return false;
        }
        use_editor(&mut self.ui, id);
        true
    }

    /// The message written in the main window, if any, made the one acted on (another's window
    /// may have taken over last frame); editors of messages gone are dropped.
    pub(crate) fn settle_composes(&mut self, ctx: &egui::Context) {
        let live: std::collections::HashSet<u64> = self.session.all_composes().map(|c| c.id).collect();
        self.ui.editors.retain(|id, _| live.contains(id));
        self.ui.across.keep(&live);
        self.ui.compose_shown.retain(|id| live.contains(id));
        if self.ui.dialog_for.is_some_and(|id| !live.contains(&id)) {
            self.ui.dialog_for = None;
        }
        // Their contacts fields go with them, as do a meeting window's (numbered apart) and a
        // dialog's (with an OK held for them), and a book's rows with its dialog: Select Names',
        // or a meeting's Select Attendees.
        let meetings = &self.ui.event_windows;
        let dialog = self.ui.dialog.iter().flat_map(places::dialog_windows);
        let windows: std::collections::HashSet<u64> =
            live.iter().copied().chain(meetings.iter().map(|(wid, _)| MEETING_WINDOWS | wid)).chain(dialog).collect();
        let mut books: std::collections::HashSet<u64> =
            meetings.iter().filter(|(_, f)| f.attendees_book.is_some()).map(|(wid, _)| MEETING_WINDOWS | wid).collect();
        if let Some(Dialog::SelectNames(f)) = &self.ui.dialog {
            books.insert(f.compose);
        }
        self.ui.people.settle(&mut self.session, &windows, &books);
        let inline = self.session.all_composes().find(|c| compose_inline(c, ctx)).map(|c| c.id);
        if let Some(id) = inline {
            self.use_compose(id);
        }
    }

    /// Each message being written in a window of its own (new mail, popped out, or set aside
    /// when another was started), as Outlook keeps every one open. Closing one keeps it in Drafts.
    pub(crate) fn compose_windows(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        let before = self.session.compose.as_ref().map(|c| c.id);
        let ids: Vec<u64> = self.session.all_composes().filter(|c| !compose_inline(c, ctx)).map(|c| c.id).collect();
        // A window opening goes a step down and right of those already open.
        let open = ids.len().saturating_sub(1);
        for id in ids {
            if self.use_compose(id) {
                let mut mine = Vec::new();
                self.compose_window(ctx, open, &mut mine);
                acts.extend(mine.into_iter().map(|a| Action::ForCompose(id, Box::new(a))));
            }
        }
        if let Some(id) = before {
            self.use_compose(id);
        }
    }

    /// The window of the message acted on now, opening `n` steps down and right of the last
    /// place (as many as are open already).
    pub(crate) fn compose_window(&mut self, ctx: &egui::Context, n: usize, acts: &mut Vec<Action>) {
        let Some(c) = self.session.compose.as_ref() else { return };
        let mine = c.id;
        let t = self.ui.tokens;
        let subject = if c.subject.trim().is_empty() { tl!("Untitled").to_string() } else { c.subject.trim().to_string() };
        let id = egui::ViewportId::from_hash_of(("compose-window", c.id));
        let mut builder = crate::os_window().with_title(tf!("{subject} – Message", subject = subject)).with_min_inner_size([520.0, 420.0]);
        // Where the last one was, and its size: at its opening only (see `first_frame`); each
        // further one a step down and right.
        if first_frame(ctx, id) {
            let step = 28.0 * (n % 8) as f32;
            builder = match self.ui.places.get("compose") {
                Some(p) => WindowPlace { x: p.x + step, y: p.y + step, ..*p }.apply(builder),
                None => builder.with_inner_size([900.0, 720.0]),
            };
        }
        let (session, st) = (&mut self.session, &mut self.ui);
        let close = ctx.show_viewport_immediate(id, builder, |ui, _| {
            let mut place = st.places.get("compose").copied();
            after_open(ui, place);
            trace_window(ui, "compose");
            WindowPlace::note(&mut place, ui);
            if let Some(p) = place {
                st.places.insert("compose".into(), p);
            }
            let mut mine_acts: Vec<Action> = Vec::new();
            let mut closing = ui.input(|i| i.viewport().close_requested());
            // Something written: ask first, as Outlook does.
            if closing && session.compose_written() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
                st.dialog = Some(Dialog::SaveChanges);
                st.dialog_for = Some(mine);
                closing = false;
            }
            egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                reading::compose_window(session, st, ui, &mut mine_acts);
            });
            // A dialog for a message shows in that message's window only.
            let here = st.dialog_for.is_none_or(|f| f == mine);
            if let Some(mut d) = st.dialog.take() {
                if d.for_compose() && here {
                    let keep = dialogs::show(ui.ctx(), &mut d, session, &mut st.people, &mut st.pictures, &t, &mut mine_acts);
                    if keep && st.dialog.is_none() {
                        st.dialog = Some(d);
                    }
                } else {
                    st.dialog = Some(d);
                }
            }
            acts.extend(mine_acts);
            closing
        });
        if close {
            forget_open(ctx, id);
            acts.push(Action::Run("compose.close", json!({"save": false})));
        }
    }

    /// Each appointment or meeting in a window of its own, as Outlook opens them (several at once,
    /// the main window free meanwhile). One opened as a dialog (from the calendar, a reminder,
    /// Reply with Meeting) moves into a window of its own at once.
    /// While a data file is compacted in place (Compact Now's second step), a window over the rest
    /// says so: what reads or writes the account waits for it, so nothing is to be done meanwhile.
    /// Its first step, attachments made smaller, leaves the window free (File › Info shows it).
    pub(crate) fn compacting_window(&mut self, ctx: &egui::Context) {
        let Some(account) = self.session.compacting_in_place().map(str::to_string) else { return };
        let t = self.ui.tokens;
        let closing = self.ui.close_after_compact;
        let email = self.session.account(&account).map_or(account, |a| a.config.email.clone());
        egui::Modal::new(egui::Id::unique("compacting")).frame(crate::dialogs::dialog_frame(&t)).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(22.0).color(t.accent));
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(tl!("Compacting the data file…")).font(theme::bold(15.0)).color(t.text));
                    ui.label(RichText::new(email).color(t.dim));
                });
            });
            ui.add_space(8.0);
            ui.label(
                RichText::new(tl!(
                    "The space freed inside it goes back to the disk. It takes about a minute for 40 GB; Quatrelle carries on once it's done."
                ))
                .color(t.dim),
            );
            if closing {
                ui.add_space(6.0);
                ui.label(RichText::new(tl!("Quatrelle closes once it's done.")).color(t.text));
            }
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    pub(crate) fn event_windows(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        self.webex_answers();
        if matches!(self.ui.dialog, Some(Dialog::Event(_)))
            && let Some(Dialog::Event(mut f)) = self.ui.dialog.take()
        {
            self.ui.event_window_ids = self.ui.event_window_ids.wrapping_add(1);
            f.window = self.ui.event_window_ids;
            // Opened with guests (Reply with Meeting): a meeting, their lines shown.
            f.inviting |= !f.guests.trim().is_empty() || !f.optional.trim().is_empty();
            self.ui.event_windows.push((self.ui.event_window_ids, f));
        }
        let t = self.ui.tokens;
        let mut windows = std::mem::take(&mut self.ui.event_windows);
        let open = windows.len().saturating_sub(1);
        let mut closed: Vec<u64> = Vec::new();
        for (n, (wid, f)) in windows.iter_mut().enumerate() {
            // Its address book, listed from the session's (which the window only reads), and its
            // contacts fields made ready: its lines, and the address book's rows while it's open.
            if let Some(book) = f.attendees_book.as_mut() {
                book.search.refresh(&mut self.session);
            }
            pim_dialogs::prepare(f, &mut self.session, &mut self.ui.people);
            let title = if f.summary.trim().is_empty() { tl!("Untitled").to_string() } else { f.summary.trim().to_string() };
            let title = if f.inviting { tf!("{subject} – Meeting", subject = title) } else { tf!("{subject} – Appointment", subject = title) };
            let id = egui::ViewportId::from_hash_of(("event-window", *wid));
            let mut builder = crate::os_window().with_title(title).with_min_inner_size([640.0, 480.0]);
            if first_frame(ctx, id) {
                let step = 28.0 * ((n.min(open)) % 8) as f32;
                builder = match self.ui.places.get("event") {
                    Some(p) => WindowPlace { x: p.x + step, y: p.y + step, ..*p }.apply(builder),
                    None => builder.with_inner_size([920.0, 700.0]),
                };
            }
            let (session, places, people) = (&self.session, &mut self.ui.places, &mut self.ui.people);
            let wid = *wid;
            let mut mine: Vec<Action> = Vec::new();
            let (keep, close) = ctx.show_viewport_immediate(id, builder, |ui, _| {
                let mut place = places.get("event").copied();
                after_open(ui, place);
                trace_window(ui, "event");
                WindowPlace::note(&mut place, ui);
                if let Some(p) = place {
                    places.insert("event".into(), p);
                }
                let mut keep = true;
                egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface).inner_margin(egui::Margin::symmetric(14, 10))).show(
                    ui,
                    |ui| {
                        keep = pim_dialogs::meeting(ui, f, session, people, &t, &mut mine);
                    },
                );
                (keep, ui.input(|i| i.viewport().close_requested()))
            });
            // Its commands go to the engine as its own: done, the window closes.
            for a in mine {
                acts.push(match a {
                    Action::Submit(cmd, p) => Action::ForEvent(wid, cmd, p),
                    other => other,
                });
            }
            if !keep || close {
                forget_open(ctx, id);
                closed.push(wid);
                if let Some(p) = pim_dialogs::webex::leaving(f) {
                    acts.push(Action::Run("webex.deleteMeeting", p));
                }
            }
        }
        windows.retain(|(w, _)| !closed.contains(w));
        // A window opened meanwhile (from an action of this frame) is kept too.
        windows.append(&mut self.ui.event_windows);
        self.ui.event_windows = windows;
    }

    /// Show only one message, as its own window does (`key` from [`MessageKey::to_id`]).
    pub fn show_message_page(&mut self, key: &str) {
        let Some(k) = MessageKey::from_id(key) else { return };
        if self.session.run("message.popOut", &json!({"key": key})).is_ok() {
            self.ui.page_message = Some(k);
        }
    }

    /// A message's own window: its tabs, its ribbon and the message; a reply written here; closed
    /// when done.
    pub(crate) fn message_page(&mut self, ui: &mut Ui, key: &MessageKey, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        if self.session.compose.is_some() {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                reading::show(&mut self.session, &mut self.ui, ui, acts);
            });
            return;
        }
        let Some(w) = self.session.windows.iter().find(|w| &w.key == key).cloned() else {
            // Done with: the window goes (a browser may refuse; then this says so).
            self.services.close_page();
            egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                ui.centered_and_justified(|ui| ui.label(RichText::new(tl!("You can close this window.")).color(t.dim)));
            });
            return;
        };
        let subject =
            w.parsed.as_ref().map(|p| p.subject.clone()).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| tl!("(no subject)").to_string());
        let title = tf!("{subject} – Message", subject = subject);
        if title != self.ui.title_shown {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.ui.title_shown = title;
        }
        let mut mine: Vec<Action> = Vec::new();
        message_content(ui, &mut self.session, &mut self.ui, &w, &mut mine);
        acts.extend(mine.into_iter().map(|a| for_window(key, a)));
    }

    /// Messages opened in windows of their own (double-click), each a native window with its tabs
    /// and ribbon, as Outlook's; in the browser, each in a browser window of its own.
    pub(crate) fn message_windows(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        if self.ui.page_message.is_none() {
            for w in self.session.windows.clone() {
                if self.services.open_message_window(&w.key.to_id()) {
                    self.session.close_window(&w.key);
                }
            }
        }
        for w in self.session.windows.clone() {
            let key = w.key.clone();
            let subject =
                w.parsed.as_ref().map(|p| p.subject.clone()).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| tl!("(no subject)").to_string());
            let id = egui::ViewportId::from_hash_of(("message-window", key.to_id()));
            let mut builder = crate::os_window().with_title(tf!("{subject} – Message", subject = subject)).with_min_inner_size([480.0, 360.0]);
            // Where the last one was, and its size: at its opening only (see `first_frame`).
            if first_frame(ctx, id) {
                builder = match self.ui.places.get("message") {
                    Some(p) => p.apply(builder),
                    None => builder.with_inner_size([940.0, 740.0]),
                };
            }
            let (session, st) = (&mut self.session, &mut self.ui);
            let close = ctx.show_viewport_immediate(id, builder, |ui, _| {
                let mut place = st.places.get("message").copied();
                after_open(ui, place);
                trace_window(ui, "message");
                WindowPlace::note(&mut place, ui);
                if let Some(p) = place {
                    st.places.insert("message".into(), p);
                }
                let mut mine: Vec<Action> = Vec::new();
                // Escape closes the find bar first, while it's shown, and the window's dialog; on
                // the File page, it goes back to the message, as Outlook's.
                let filing = file_page::shown(ui.ctx(), &key);
                let bar = st.find.shown_in_window(ui.ctx(), &key) || st.window_dialog.as_ref().is_some_and(|(k, _)| *k == key) || filing;
                let closing = ui.input(|i| i.viewport().close_requested() || (i.key_pressed(Key::Escape) && !bar));
                if filing && st.window_dialog.is_none() && ui.input(|i| i.key_pressed(Key::Escape)) {
                    file_page::show(ui.ctx(), &key, false);
                }
                // Ctrl+P prints this window's message.
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::P)) {
                    mine.push(Action::Print);
                }
                // Delete, and Shift+Delete for good, delete this window's message. Only in a
                // window of its own: drawn inside the main one, its keys are the main window's.
                if ui.ctx().viewport_id() != egui::ViewportId::ROOT && !ui.ctx().text_edit_focused() {
                    let (delete, for_good) = ui.input_mut(|i| {
                        let cut = super::keys::cut_is_shift_delete(i.modifiers) && i.events.iter().any(|e| matches!(e, egui::Event::Cut));
                        (i.consume_key(egui::Modifiers::NONE, Key::Delete), i.consume_key(egui::Modifiers::SHIFT, Key::Delete) || cut)
                    });
                    if delete || for_good {
                        mine.extend(super::keys::delete_from_window(&key, for_good));
                    }
                    // Reply, Reply all and Forward by their keys, as in the main window.
                    let answer = ui.input_mut(|i| {
                        let shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
                        if i.consume_key(shift, Key::R) {
                            Some("mail.replyAll")
                        } else if i.consume_key(egui::Modifiers::COMMAND, Key::R) {
                            Some("mail.reply")
                        } else if i.consume_key(egui::Modifiers::COMMAND, Key::F) {
                            Some("mail.forward")
                        } else {
                            None
                        }
                    });
                    if let Some(id) = answer {
                        mine.push(Action::Run(id, json!({})));
                    }
                }
                message_content(ui, session, st, &w, &mut mine);
                let leave = mine.iter().any(|a| matches!(a, Action::Run("message.closeWindow", _)));
                // The File page (Junk Email options…) is the main window's: brought forward.
                if mine.iter().any(|a| matches!(a, Action::Backstage(_))) {
                    mine.push(Action::FocusMain);
                }
                // What it asks for is done to its message, whatever the main window has selected.
                acts.extend(mine.into_iter().map(|a| for_window(&key, a)));
                closing && !leave
            });
            if close {
                forget_open(ctx, id);
                acts.push(Action::Run("message.closeWindow", json!({"key": key.to_id()})));
            }
        }
        // A window gone takes its dialog and its ribbon's fit with it.
        if self.ui.window_dialog.as_ref().is_some_and(|(k, _)| !self.session.windows.iter().any(|w| w.key == *k)) {
            self.ui.window_dialog = None;
        }
        let open: std::collections::HashSet<String> = self.session.windows.iter().map(|w| ribbon::fit_key(&w.key)).collect();
        self.ui.ribbon_groups.retain(|k, _| !k.starts_with("message-window/") || open.contains(k));
        let keys: Vec<MessageKey> = self.session.windows.iter().map(|w| w.key.clone()).collect();
        file_page::keep(ctx, &keys);
    }

    /// An action from a message's own window, done to its message: that message stands in as the
    /// one selected and read while it runs, the window's dialog as the open one. A dialog it opens
    /// that acts on the message shows in that window; any other in the main window, brought
    /// forward (an appointment's opens in a window of its own).
    pub(crate) fn act_in_window(&mut self, key: MessageKey, a: Action) {
        let Some(scope) = self.session.enter_window_to_act(&key) else {
            // Its window gone (a file dialog's answer coming late): the reading pane's message
            // still takes it when it's the same one.
            if self.session.reading.as_ref().is_some_and(|r| r.key == key) {
                self.act(a);
            }
            return;
        };
        let theirs = self.ui.window_dialog.take_if(|(k, _)| *k == key).map(|(_, d)| d);
        let main = std::mem::replace(&mut self.ui.dialog, theirs);
        match a {
            Action::FindInMessage => self.ui.find.bar(true, &key).open(),
            a => self.act(a),
        }
        match std::mem::replace(&mut self.ui.dialog, main) {
            Some(d) if d.for_message_window() => self.ui.window_dialog = Some((key, d)),
            Some(d) => {
                self.ui.focus_main |= !matches!(d, Dialog::Event(_));
                self.ui.dialog = Some(d);
            }
            None => {}
        }
        self.session.leave_window(scope);
    }
}

/// What a message's own window asks for: done to its message ([`Action::InWindow`]), but what
/// goes to the main window as it is (another message opened, a search, the File page, a link).
fn for_window(key: &MessageKey, a: Action) -> Action {
    let main = match &a {
        Action::Run(id, _) => *id == "message.closeWindow" || ["list.", "search.", "folder.", "view."].iter().any(|p| id.starts_with(p)),
        Action::Open(_)
        | Action::Search(_)
        | Action::Backstage(_)
        | Action::FocusMain
        | Action::Refuse(_)
        | Action::InWindow(..)
        | Action::OpenUrl(_)
        | Action::OpenMeeting(_)
        | Action::SetDark(_)
        | Action::ShowFormatted(..)
        | Action::ShowMarkdown(..) => true,
        _ => false,
    };
    if main { a } else { Action::InWindow(key.clone(), Box::new(a)) }
}

/// A message window's content, as Outlook's: its tabs and ribbon over the message, or its own File
/// page in their place; and its dialog. All but the message are drawn while its message stands in
/// as the one selected and read, so that what's offered is for it, wherever the main window is.
fn message_content(ui: &mut Ui, session: &mut Session, st: &mut UiState, w: &quatrelle_engine::Reading, mine: &mut Vec<Action>) {
    let t = st.tokens;
    let key = &w.key;
    let scope = session.enter_window(key);
    let filing = file_page::shown(ui.ctx(), key);
    if filing {
        file_page::page(ui, session, &t, w, mine);
    } else {
        egui::Panel::top(egui::Id::unique(("message-window-ribbon", key.to_id())))
            .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(8, 4)).stroke(egui::Stroke::new(1.0, t.border)))
            .show(ui, |ui| ribbon::message_ribbon(ui, session, st, w, mine));
    }
    if let Some((k, mut d)) = st.window_dialog.take() {
        if k == *key {
            let keep = dialogs::show(ui.ctx(), &mut d, session, &mut st.people, &mut st.pictures, &t, mine);
            if keep && st.window_dialog.is_none() {
                st.window_dialog = Some((k, d));
            }
        } else {
            st.window_dialog = Some((k, d));
        }
    }
    if let Some(scope) = scope {
        session.leave_window(scope);
    }
    if !filing {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
            reading::window(session, st, ui, w.clone(), mine);
        });
    }
}

/// Where a window was and how big (in points), and whether it was maximized: its normal place is
/// kept while maximized, so Restore goes back to it, as Outlook does.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WindowPlace {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(default)]
    pub maximized: bool,
    /// The size of the monitor it was on, so a window whose monitor went (an external one
    /// unplugged) is brought back on screen ([`WindowPlace::rescue`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<[f32; 2]>,
}

/// Room a window leaves on each side of its monitor for the taskbar, the Dock or a desktop panel,
/// which the monitor size (all egui reports) includes.
pub(crate) const SYSTEM_BAR: egui::Vec2 = egui::vec2(48.0, 48.0);

/// The smallest a window is made to fit its monitor (the main window's minimum size).
pub(crate) const FIT_MIN: egui::Vec2 = egui::vec2(760.0, 480.0);

impl WindowPlace {
    /// Note where a window is now (nothing while it's minimized; while maximized, only that).
    pub fn note(place: &mut Option<WindowPlace>, ui: &Ui) {
        let v = ui.input(|i| i.viewport().clone());
        if v.minimized == Some(true) {
            return;
        }
        let maximized = v.maximized == Some(true);
        if maximized {
            if let Some(p) = place.as_mut() {
                p.maximized = true;
            }
            return;
        }
        if let (Some(outer), Some(inner)) = (v.outer_rect, v.inner_rect) {
            let monitor = v.monitor_size.filter(|m| m.is_finite() && m.x > 0.0 && m.y > 0.0).map(|m| [m.x, m.y]);
            *place = Some(WindowPlace { x: outer.min.x, y: outer.min.y, w: inner.width(), h: inner.height(), maximized: false, monitor });
        }
    }

    /// Where to move a restored window, once its monitor is known (the other Craft apps' fix): a
    /// window whose monitor differs from the one it was saved on (an external monitor unplugged)
    /// may sit where no monitor is any more, so it's centred on the primary one, whose top-left
    /// corner is the origin. A maximized window fills whichever monitor the system picks.
    pub fn rescue(self, info: &egui::ViewportInfo) -> Option<egui::Pos2> {
        if self.maximized {
            return None;
        }
        let saved = egui::Vec2::from(self.monitor?);
        let now = info.monitor_size.filter(|s| s.is_finite() && s.x > 0.0 && s.y > 0.0)?;
        if (now - saved).length() <= 1.0 {
            return None;
        }
        let free = now - egui::vec2(self.w, self.h);
        Some(egui::pos2((free.x / 2.0).max(0.0), (free.y / 2.0).max(0.0)))
    }

    /// Once the monitor is known, a smaller size and a centred position for a window that doesn't
    /// fit on it, frame included: one saved on a bigger screen, or the default size on a laptop,
    /// reached under the taskbar or past the screen's edge (WordCraft found). `restored`: it was
    /// put where a saved place says (perhaps another monitor), so only its size counts; one the
    /// system placed is on the primary monitor, so its position counts too.
    pub fn fit(info: &egui::ViewportInfo, restored: bool) -> Option<(egui::Pos2, egui::Vec2)> {
        if info.maximized == Some(true) || info.fullscreen == Some(true) || info.minimized == Some(true) {
            return None;
        }
        let monitor = info.monitor_size.filter(|s| s.is_finite() && s.x > 0.0 && s.y > 0.0)?;
        let inner = info.inner_rect.filter(|r| r.is_finite())?;
        let outer = info.outer_rect.filter(|r| r.is_finite()).unwrap_or(inner);
        let frame = (outer.size() - inner.size()).max(egui::Vec2::ZERO);
        let room = monitor - 2.0 * SYSTEM_BAR;
        let too_big = outer.width() > room.x || outer.height() > room.y;
        let edge = monitor - SYSTEM_BAR;
        let past_edge = !restored && (outer.max.x > edge.x || outer.max.y > edge.y);
        if !too_big && !past_edge {
            return None;
        }
        let size = (room - frame).min(inner.size()).max(FIT_MIN);
        let free = monitor - (size + frame);
        Some((egui::pos2((free.x / 2.0).max(0.0), (free.y / 2.0).max(0.0)), size))
    }

    /// A place that's sensible to open at: a real size, and on a screen as far as can be told.
    pub fn usable(self) -> Option<WindowPlace> {
        let fine = |v: f32| v.is_finite();
        (fine(self.x) && fine(self.y) && self.w >= 200.0 && self.h >= 150.0 && self.w < 20_000.0 && self.h < 20_000.0)
            .then_some(self)
            .filter(|p| p.x > -2_000.0 && p.y > -200.0 && p.x < 20_000.0 && p.y < 20_000.0)
    }

    /// Open a window where this place says.
    pub fn apply(self, b: egui::ViewportBuilder) -> egui::ViewportBuilder {
        match self.usable() {
            // On Windows, winit 0.30.13 showed a window created maximized at once, though eframe
            // keeps it hidden until its first frame: it appeared, vanished and appeared again. The
            // patched winit (storytold/winit) maximizes a hidden window only as it's shown.
            Some(p) => b.with_position([p.x, p.y]).with_inner_size([p.w, p.h]).with_maximized(p.maximized),
            None => b,
        }
    }
}

/// What a window does once it has opened, from its place as it opened (`place`), called at the
/// top of each of its frames before the place is noted again: on the first frame that knows its
/// monitor, one restored where no monitor is any more is brought back ([`WindowPlace::rescue`]),
/// and one too big for its screen is made to fit ([`WindowPlace::fit`]).
///
/// [`forget_open`] when it closes, so it does the same when it opens again.
pub fn after_open(ui: &Ui, place: Option<WindowPlace>) {
    let ctx = ui.ctx();
    let id = egui::Id::unique(("after-open", ctx.viewport_id()));
    let place = place.and_then(WindowPlace::usable);
    // Absent: its first frame; Some(pending): still to check once the monitor is known.
    let pending: Option<Option<WindowPlace>> = ctx.data(|d| d.get_temp(id));
    // A window with no place (the system put it) is checked too.
    let pending = pending.unwrap_or_else(|| Some(place.unwrap_or_default()));
    let info = ui.input(|i| i.viewport().clone());
    let known = info.monitor_size.is_some_and(|m| m.is_finite() && m.x > 0.0 && m.y > 0.0);
    let pending = match pending {
        Some(p) if known => {
            let restored = place.is_some();
            if let Some(pos) = p.rescue(&info) {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
            } else if !p.maximized
                && let Some((pos, size)) = WindowPlace::fit(&info, restored)
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
            }
            None
        }
        other => other,
    };
    ctx.data_mut(|d| d.insert_temp(id, pending));
}

/// A closed window opens afresh next time ([`after_open`]).
pub(crate) fn forget_open(ctx: &egui::Context, viewport: egui::ViewportId) {
    ctx.data_mut(|d| d.remove::<Option<WindowPlace>>(egui::Id::unique(("after-open", viewport))));
}
