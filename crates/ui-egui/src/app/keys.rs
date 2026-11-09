// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Keyboard shortcuts, Outlook's, dispatched to commands.

use crate::*;

impl MailApp {
    pub(crate) fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.ui.backstage.is_none() {
            self.ui.backstage_typing = false;
        }
        if self.ui.dialog.is_some() {
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                // Escape on the question about another domain is a no: the safe answer.
                if let Some(Dialog::TrustDomain { account, site }) = &self.ui.dialog {
                    let refusal = json!({"account": account, "domain": site});
                    self.run("account.refuseDomain", refusal);
                }
                // On the question about purging, Escape keeps everything and stays in the list
                // (the dialog's own pass answers it); from the list, it closes it.
                if matches!(&self.ui.dialog, Some(Dialog::Recover(f)) if f.confirming.is_some()) {
                    return;
                }
                if matches!(&self.ui.dialog, Some(Dialog::Recover(_))) {
                    self.run("folder.recoverClose", json!({}));
                }
                self.ui.dialog = None;
            }
            return;
        }
        if self.ui.backstage.is_some() {
            // Esc in one of its fields leaves the field; from outside one, the page.
            if ctx.input(|i| i.key_pressed(Key::Escape)) && !std::mem::take(&mut self.ui.backstage_typing) {
                self.ui.backstage = None;
            }
            return;
        }
        // Fold the ribbon away, or back (Outlook's Ctrl+F1).
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::F1)) {
            self.ui.ribbon_collapsed = !self.ui.ribbon_collapsed;
        }
        let typing = ctx.text_edit_focused();
        // A press anywhere but on an attachment hands the keyboard back from the attachments and
        // clears their selection; not one in a menu over them (Save Selected… is one).
        if ctx.input(|i| i.pointer.primary_pressed()) && !self.ui.attachment_hovered {
            let in_menu =
                ctx.input(|i| i.pointer.interact_pos()).and_then(|p| ctx.layer_id_at(p)).is_some_and(|l| l.order != egui::Order::Background);
            if !in_menu {
                self.ui.attachments_active = false;
                self.ui.attachment_selected = None;
            }
        }
        self.ui.attachment_hovered = false;
        // Text selected in a message (its body, headers): Ctrl+A and Ctrl+C are the text's.
        let text_selected = ctx.plugin_opt::<egui::text_selection::LabelSelectionState>().is_some_and(|p| p.lock().has_selection());
        let pressed: Vec<(Key, egui::Modifiers)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    // The digit row by where it is: with Shift, or on a layout whose top row types
                    // other characters (AZERTY), Ctrl+Shift+1 is still 1, not `!` or `&`.
                    egui::Event::Key { key, physical_key, pressed: true, modifiers, .. } => {
                        let digit = physical_key.filter(|p| {
                            matches!(
                                p,
                                Key::Num0 | Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 | Key::Num6 | Key::Num7 | Key::Num8 | Key::Num9
                            )
                        });
                        Some((digit.unwrap_or(*key), *modifiers))
                    }
                    egui::Event::Cut if cut_is_shift_delete(i.modifiers) => Some((Key::Delete, egui::Modifiers::SHIFT)),
                    _ => None,
                })
                .collect()
        });
        let reg = self.session.registry();
        // A key held down repeats faster than a frame is drawn: the presses of Down or Up (J,
        // K, with Shift too) queued in a row go as one step of so many messages, so only the
        // message landed on is read, and a frame costs one step whatever the repeat rate.
        let mut runs: Vec<(Key, egui::Modifiers, String, i64)> = Vec::new();
        for (key, m) in pressed {
            let combo = combo_of(key, m);
            let step = |c: &str| reg.by_shortcut(c).map(|spec| spec.id).filter(|id| steps(id));
            if let Some(last) = runs.last_mut()
                && let Some(id) = step(&combo)
                && step(&last.2) == Some(id)
            {
                last.3 = last.3.saturating_add(1);
                continue;
            }
            runs.push((key, m, combo, 1));
        }
        for (key, m, combo, n) in runs {
            let plain = !(m.command || m.ctrl || m.alt);
            // Single keys type text when a text box has focus.
            if typing && plain && !matches!(key, Key::F9 | Key::Escape) {
                continue;
            }
            // Writing a message: its own keys (formatting, Ctrl+R aligns right…), not the main
            // window's; Ctrl+Shift+B opens the address book from any of its boxes.
            if typing && let Some(c) = self.session.compose.as_ref().filter(|c| compose_inline(c, ctx)) {
                if combo == "Mod+Shift+B" {
                    self.ui.dialog = Some(Dialog::select_names(c, "to"));
                }
                if !matches!(combo.as_str(), "Mod+S" | "Mod+Enter") {
                    continue;
                }
            }
            // Ctrl+N: the shown module's new item, as Outlook's.
            if combo == "Mod+N" && self.session.module != Module::Mail {
                self.new_item();
                continue;
            }
            if combo == "Mod+E" {
                self.ui.focus_search = true;
                continue;
            }
            // Ctrl+P prints the message read in Mail, not while one is written here. Taken, so a
            // message's window drawn inside this one (the browser) doesn't print its own too.
            if combo == "Mod+P"
                && self.session.module == Module::Mail
                && self.session.reading.is_some()
                && !self.session.all_composes().any(|c| compose_inline(c, ctx))
            {
                ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::P));
                self.act(Action::Print);
                continue;
            }
            if combo == "Escape" {
                // The find bar shown under the message closes first (not from the search box).
                if self.ui.find.pane.shown(ctx) && !self.ui.search_focused {
                    self.ui.find.pane.close();
                    continue;
                }
                // Escape lets go of selected attachments first.
                if self.ui.attachment_selected.as_ref().is_some_and(|(_, v)| !v.is_empty()) {
                    self.ui.attachment_selected = None;
                    self.ui.attachments_active = false;
                    continue;
                }
                // In the Calendar, wherever the focus is (the box, a switch, the list), it empties
                // the calendar's search: the days come back.
                if self.session.module == Module::Calendar && !self.ui.cal_search.query.is_empty() {
                    self.ui.cal_search.query.clear();
                    continue;
                }
                if self.session.search.is_some() {
                    self.ui.search_text.clear();
                    self.run("search.clear", json!({}));
                }
                continue;
            }
            // In a text box these edit the text (select all, undo, redo).
            if typing && matches!(combo.as_str(), "Mod+A" | "Mod+Z" | "Mod+Y" | "Mod+Shift+Z") {
                continue;
            }
            // Ctrl+A in the attachments selects them all, as Outlook's does.
            if combo == "Mod+A" && self.ui.attachments_active {
                self.select_all_attachments();
                continue;
            }
            if text_selected && matches!(combo.as_str(), "Mod+A" | "Mod+C") {
                continue;
            }
            if let Some(spec) = reg.by_shortcut(&combo) {
                let id = spec.id;
                // Another module shown (Calendar, People, Tasks, Notes): the keys that act on the
                // messages still selected in Mail don't (Delete in the Calendar is the grid's).
                if self.session.module != Module::Mail && acts_on_mail(id) {
                    continue;
                }
                if id == "search.run" {
                    continue;
                }
                if matches!(id, "mail.delete" | "mail.deleteForGood") {
                    let for_good = id == "mail.deleteForGood";
                    // A message's page of its own (the browser): its message, not the list's
                    // selection.
                    if let Some(k) = self.ui.page_message.clone() {
                        for a in delete_from_window(&self.session, &k, for_good) {
                            self.act(a);
                        }
                        continue;
                    }
                    self.act(Action::DeleteMail { for_good });
                    continue;
                }
                if id == "mail.move" {
                    self.ui.dialog = Some(Dialog::move_to());
                    continue;
                }
                // Page Up and Page Down go as many rows as the list shows (with Shift too: the
                // plain ones are `list.pageUp`, a small p, which `ends_with("PageUp")` missed).
                let page = matches!(id, "list.pageUp" | "list.pageDown" | "list.extendPageUp" | "list.extendPageDown");
                let params = if page {
                    json!({"rows": self.ui.list_page})
                } else if steps(id) {
                    json!({"by": n})
                } else {
                    json!({})
                };
                // A ready-made Quick Step not set up yet asks for its setting first.
                if let Some(i) = id.strip_prefix("quickStep.").and_then(|n| n.parse::<usize>().ok()).and_then(|n| n.checked_sub(1)) {
                    let act = super::run_quick_step(&self.session, i);
                    if matches!(act, Action::Dialog(_)) && !self.session.selection.is_empty() {
                        self.act(act);
                        continue;
                    }
                }
                self.run(id, params);
                if moves_focus(id) {
                    self.ui.scroll_to_focus = true;
                }
            }
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(Key::Enter)) && self.session.compose.is_some() {
            self.run("compose.send", json!({}));
        }
    }

    /// The shown module's new item (Ctrl+N), as its ribbon's New button makes it: a message, an
    /// appointment (at the half hour or the day selected), a contact, a task, a note.
    fn new_item(&mut self) {
        if self.session.accounts.is_empty() {
            return;
        }
        match self.session.module {
            Module::Mail => self.run("mail.new", json!({})),
            Module::Calendar => {
                let new = calendar::new_event(&self.session, self.ui.cal_pick.as_ref());
                self.act(new);
            }
            Module::People => self.ui.dialog = Some(Dialog::new_contact()),
            Module::Tasks => self.ui.tasks.focus_new = true,
            Module::Notes => {
                let mut acts = Vec::new();
                notes::new_note(&mut self.ui, &mut acts);
                for a in acts {
                    self.act(a);
                }
            }
        }
    }

    /// Deleting from Deleted Items is permanent: ask first.
    pub fn needs_delete_confirm(&self) -> bool {
        question(&self.session, self.session.selected_iter().map(|m| &m.key), 0, false).is_some()
    }

    /// Delete the mail selected ([`Action::DeleteMail`]): the question first where it's for good
    /// and the option says so, else at once.
    pub(crate) fn delete_mail(&mut self, for_good: bool) {
        let n = self.session.selection.len();
        match question(&self.session, self.session.selected_iter().map(|m| &m.key), n, for_good) {
            Some(d) => self.ui.dialog = Some(d),
            None => self.run(if for_good { "mail.deleteForGood" } else { "mail.delete" }, json!({})),
        }
    }
}

/// What deleting these `count` messages asks first, as the option says (on unless turned off):
/// Delete for good (Shift+Delete) does, and Delete when all of them are in Deleted Items, where
/// it's for good too. Not in the Outbox: deleting there only keeps a message from being sent.
/// None: they go at once. The question says when Exchange keeps them all a while.
fn question<'a>(s: &Session, keys: impl Iterator<Item = &'a MessageKey>, count: usize, for_good: bool) -> Option<Dialog> {
    let in_trash =
        |k: &MessageKey| s.account(&k.account).and_then(|a| a.folder(quatrelle_engine::model::FolderRole::Trash)).is_some_and(|t| t.path == k.folder);
    let exchange = |k: &MessageKey| s.account(&k.account).is_some_and(|a| a.config.ews.is_some() || a.config.graph.is_some());
    let mut any = false;
    let mut all_in_trash = true;
    let mut all_outbox = true;
    let mut all_exchange = true;
    for k in keys {
        any = true;
        all_in_trash &= in_trash(k);
        let outbox = s.outbox_entry(k).is_some();
        all_outbox &= outbox;
        // An Outbox message was never on the server: nothing keeps it.
        all_exchange &= !outbox && exchange(k);
    }
    let asks = s.prefs.confirm_permanent_delete && any && if for_good { !all_outbox } else { all_in_trash };
    asks.then_some(Dialog::ConfirmDelete { count, exchange: all_exchange })
}

/// Delete (or, `for_good`, Shift+Delete) on the message of a window of its own, or of the
/// browser's page for one: that message, asked first as in the list; the window closes. One no
/// longer in the list (another folder opened since) isn't deleted: the list's selection would be,
/// in its place.
pub(crate) fn delete_from_window(s: &Session, key: &MessageKey, for_good: bool) -> Vec<Action> {
    if s.summary(key).is_none() {
        return vec![Action::Refuse(tl!("This message is no longer in the list: open its folder to delete it.").to_string())];
    }
    let close = Action::Run("message.closeWindow", json!({"key": key.to_id()}));
    if let Some(d) = question(s, std::iter::once(key), 1, for_good) {
        return vec![Action::Open(key.clone()), Action::Dialog(d), Action::FocusMain, close];
    }
    vec![Action::Open(key.clone()), Action::Run(if for_good { "mail.deleteForGood" } else { "mail.delete" }, json!({})), close]
}

/// A command whose shortcut acts on the messages selected in Mail, or moves through its list:
/// delete, archive, reply, forward, move, read/unread, flag, Quick Steps, select all, next and
/// previous. New mail, Send / Receive, Undo, Search, the modules and the address book act anywhere.
fn acts_on_mail(id: &str) -> bool {
    (id.starts_with("mail.") && !matches!(id, "mail.new" | "mail.sync")) || id.starts_with("quickStep.") || id.starts_with("list.")
}

/// Shift+Delete on Windows, which egui hands over as Cut (a clipboard shortcut there), not as
/// the key: a Cut with Shift down and neither Ctrl nor Alt (Ctrl+X is a Cut too).
pub(crate) fn cut_is_shift_delete(m: egui::Modifiers) -> bool {
    cfg!(target_os = "windows") && m.shift && !(m.command || m.ctrl || m.alt)
}

/// A shortcut as the registry names it: `Mod+Shift+Down`.
fn combo_of(key: Key, m: egui::Modifiers) -> String {
    let mut combo = String::new();
    if m.command || m.ctrl {
        combo.push_str("Mod+");
    }
    if m.alt {
        combo.push_str("Alt+");
    }
    if m.shift {
        combo.push_str("Shift+");
    }
    combo.push_str(match key {
        Key::ArrowDown => "Down",
        Key::ArrowUp => "Up",
        Key::Enter => "Enter",
        k => k.name(),
    });
    combo
}

/// The commands that go a message at a time (`{"by": n}`: so many at once).
fn steps(id: &str) -> bool {
    matches!(id, "list.next" | "list.prev" | "list.extendNext" | "list.extendPrev")
}

/// Whether a command moves the list's focus (which is then scrolled into view).
pub(crate) fn moves_focus(id: &str) -> bool {
    matches!(id, "list.next" | "list.prev" | "list.first" | "list.last" | "list.pageUp" | "list.pageDown") || id.starts_with("list.extend")
}
