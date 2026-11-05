// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Keyboard shortcuts, Outlook's, dispatched to commands.

use crate::*;

impl MailApp {
    pub(crate) fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.ui.dialog.is_some() {
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                // Escape on the question about another domain is a no: the safe answer.
                if let Some(Dialog::TrustDomain { account, site }) = &self.ui.dialog {
                    let refusal = json!({"account": account, "domain": site});
                    self.run("account.refuseDomain", refusal);
                }
                self.ui.dialog = None;
            }
            return;
        }
        if self.ui.backstage.is_some() {
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
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
                    _ => None,
                })
                .collect()
        });
        let reg = self.session.registry();
        for (key, m) in pressed {
            let name = match key {
                Key::ArrowDown => "Down".to_string(),
                Key::ArrowUp => "Up".to_string(),
                Key::Enter => "Enter".to_string(),
                k => k.name().to_string(),
            };
            let plain = !(m.command || m.ctrl || m.alt);
            // Single keys type text when a text box has focus.
            if typing && plain && !matches!(key, Key::F9 | Key::Escape) {
                continue;
            }
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
            combo.push_str(&name);
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
            if combo == "Escape" {
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
                if id == "mail.delete" && self.needs_delete_confirm() {
                    self.ui.dialog = Some(Dialog::ConfirmDelete { count: self.session.selection.len() });
                    continue;
                }
                if id == "mail.move" {
                    self.ui.dialog = Some(Dialog::move_to());
                    continue;
                }
                // Page Up and Page Down go as many rows as the list shows (with Shift too: the
                // plain ones are `list.pageUp`, a small p, which `ends_with("PageUp")` missed).
                let page = matches!(id, "list.pageUp" | "list.pageDown" | "list.extendPageUp" | "list.extendPageDown");
                let params = if page { json!({"rows": self.ui.list_page}) } else { json!({}) };
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
        self.session.prefs.confirm_permanent_delete
            && !self.session.selection.is_empty()
            && self.session.selected().iter().all(|m| {
                self.session
                    .account(&m.key.account)
                    .and_then(|a| a.folder(quatrelle_engine::model::FolderRole::Trash))
                    .is_some_and(|t| t.path == m.key.folder)
            })
    }
}

/// A command whose shortcut acts on the messages selected in Mail, or moves through its list:
/// delete, archive, reply, forward, move, read/unread, flag, Quick Steps, select all, next and
/// previous. New mail, Send / Receive, Undo, Search, the modules and the address book act anywhere.
fn acts_on_mail(id: &str) -> bool {
    (id.starts_with("mail.") && !matches!(id, "mail.new" | "mail.sync")) || id.starts_with("quickStep.") || id.starts_with("list.")
}

/// Whether a command moves the list's focus (which is then scrolled into view).
pub(crate) fn moves_focus(id: &str) -> bool {
    matches!(id, "list.next" | "list.prev" | "list.first" | "list.last" | "list.pageUp" | "list.pageDown") || id.starts_with("list.extend")
}
