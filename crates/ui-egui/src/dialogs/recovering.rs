// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Dialogs: Recover deleted items. What an Exchange server keeps of the items deleted for good,
//! listed from the server each time it opens: filtered, sorted by a heading, picked with Shift
//! and Ctrl, then brought back into Deleted Items or purged (after a question).

use std::collections::HashSet;

use quatrelle_engine::RecoverBlock;
use quatrelle_engine::model::{Recoverable, RecoverableKind, dates};

use super::*;

/// What the list is sorted by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecoverBy {
    Kind,
    Subject,
    From,
    /// When it was deleted: the most recent first, as it opens.
    #[default]
    Deleted,
    Size,
}

/// The window's own state: the list itself is the session's ([`Session::recovering`]).
#[derive(Clone, Debug, Default)]
pub struct RecoverForm {
    pub account: String,
    pub filter: String,
    pub by: RecoverBy,
    /// The reverse of the heading's natural order.
    pub flipped: bool,
    pub selected: HashSet<String>,
    /// The last item picked, where a Shift-click's range starts.
    pub anchor: Option<String>,
    /// The ids the question about purging is asked for.
    pub confirming: Option<Vec<String>>,
}

const ROW: f32 = 24.0;
const KIND_W: f32 = 28.0;
const FROM_W: f32 = 170.0;
const DATE_W: f32 = 150.0;
const SIZE_W: f32 = 80.0;

impl RecoverForm {
    /// The items shown: those the filter matches, in the order the heading gives.
    fn shown<'a>(&self, items: &'a [Recoverable]) -> Vec<&'a Recoverable> {
        let needle = self.filter.trim().to_lowercase();
        let mut rows: Vec<&Recoverable> = items
            .iter()
            .filter(|i| needle.is_empty() || i.subject.to_lowercase().contains(&needle) || i.from.to_lowercase().contains(&needle))
            .collect();
        let kind = |k: RecoverableKind| match k {
            RecoverableKind::Mail => 0,
            RecoverableKind::Appointment => 1,
            RecoverableKind::Contact => 2,
            RecoverableKind::Task => 3,
            RecoverableKind::Other => 4,
        };
        match self.by {
            RecoverBy::Kind => rows.sort_by_key(|i| (kind(i.kind), std::cmp::Reverse(i.deleted))),
            RecoverBy::Subject => rows.sort_by_cached_key(|i| i.subject.to_lowercase()),
            RecoverBy::From => rows.sort_by_cached_key(|i| i.from.to_lowercase()),
            RecoverBy::Deleted => rows.sort_by_key(|i| std::cmp::Reverse(i.deleted)),
            RecoverBy::Size => rows.sort_by_key(|i| std::cmp::Reverse(i.size)),
        }
        if self.flipped {
            rows.reverse();
        }
        rows
    }

    /// A click on a row: alone, it picks only that one; with Ctrl, it toggles it; with Shift, it
    /// picks the range from the last one picked.
    pub(crate) fn pick(&mut self, rows: &[&Recoverable], at: usize, ctrl: bool, shift: bool) {
        let Some(id) = rows.get(at).map(|r| r.id.clone()) else { return };
        let from = self.anchor.as_ref().and_then(|a| rows.iter().position(|r| r.id == *a));
        match (shift, from) {
            (true, Some(from)) => {
                if !ctrl {
                    self.selected.clear();
                }
                let (lo, hi) = (from.min(at), from.max(at));
                self.selected.extend(rows.iter().skip(lo).take(hi - lo + 1).map(|r| r.id.clone()));
            }
            _ if ctrl => {
                if !self.selected.remove(&id) {
                    self.selected.insert(id.clone());
                }
                self.anchor = Some(id);
            }
            _ => {
                self.selected.clear();
                self.selected.insert(id.clone());
                self.anchor = Some(id);
            }
        }
    }
}

impl Dialog {
    /// Recover deleted items for an account (its list is asked for by `folder.recoverDeleted`).
    pub fn recover(account: &str) -> Dialog {
        Dialog::Recover(Box::new(RecoverForm { account: account.to_string(), ..Default::default() }))
    }
}

/// Why the folder menu's entry is greyed out, in words, or `None` when it can be used.
pub(crate) fn recover_blocked(s: &Session, account: &str) -> Option<&'static str> {
    match s.recover_block(account)? {
        RecoverBlock::NoServer => Some(tl!("This server doesn't keep deleted items for you to bring back.")),
        RecoverBlock::Offline => Some(tl!("Connect first: the server keeps these items.")),
    }
}

/// The discreet link above the list of Deleted Items, for an account the server keeps them of.
pub(crate) fn link(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    let quatrelle_engine::View::Folder { account, folder } = &s.view else { return };
    if s.folder(account, folder).map(|f| f.role) != Some(FolderRole::Trash) {
        return;
    }
    if s.account(account).is_none_or(|a| a.config.ews.is_none() && a.config.graph.is_none()) {
        return;
    }
    ui.horizontal(|ui| {
        let blocked = recover_blocked(s, account);
        let text = RichText::new(tl!("Recover deleted items…")).color(if blocked.is_some() { t.faint } else { t.accent }).small();
        let r = ui.add_enabled(blocked.is_none(), egui::Label::new(text).sense(Sense::click()));
        let r = match blocked {
            Some(why) => r.on_disabled_hover_text(why),
            None => r.on_hover_cursor(egui::CursorIcon::PointingHand),
        };
        if r.clicked() {
            open(account, acts);
        }
    });
}

/// Ask the server for the list and show the window.
pub(crate) fn open(account: &str, acts: &mut Vec<Action>) {
    acts.push(Action::Run("folder.recoverDeleted", json!({"account": account})));
    acts.push(Action::Dialog(Dialog::recover(account)));
}

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    let Dialog::Recover(f) = d else { return true };
    ui.set_width(720.0);
    title(ui, t, "Recover deleted items");
    ui.label(RichText::new(s.account(&f.account).map_or("", |a| a.config.email.as_str())).color(t.dim));
    ui.add_space(8.0);
    let Some(r) = s.recovering.as_ref().filter(|r| r.account == f.account) else {
        ui.label(RichText::new(tl!("The list isn't open any more.")).color(t.dim));
        return !close_row(ui);
    };
    // The question before a purge: Enter is Purge, Escape and Cancel keep everything (`keys.rs`).
    if let Some(ids) = f.confirming.clone() {
        let what = if ids.len() == 1 {
            tl!("This item will be deleted for good. Nobody can bring it back, and this can't be undone.").to_string()
        } else {
            tf!("These {n} items will be deleted for good. Nobody can bring them back, and this can't be undone.", n = ids.len())
        };
        ui.label(RichText::new(what).color(t.dim));
        let (ok, cancel) = buttons(ui, t, "Purge", !r.working);
        let enter = !cancel && !r.working && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ok || enter {
            acts.push(Action::Run("folder.recoverPurge", json!({"ids": ids})));
            f.confirming = None;
            f.selected.clear();
        } else if cancel {
            f.confirming = None;
        }
        return true;
    }
    ui.add(egui::TextEdit::singleline(&mut f.filter).hint_text(tl!("Search the deleted items")).desired_width(f32::INFINITY));
    ui.add_space(6.0);
    let rows = f.shown(&r.items);
    list(ui, t, s, f, &rows);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if r.loading || r.working {
            ui.spinner();
            ui.label(RichText::new(tl!("Asking the server…")).color(t.dim));
        } else if let Some(e) = &r.error {
            ui.add(egui::Label::new(RichText::new(tf!("The server said: {error}", error = e)).color(t.warn)).wrap());
        } else if let Some(last) = &r.last {
            let n = last.count;
            let said = if last.purged { tf!("Purged: {n}", n = n) } else { tf!("Restored into Deleted Items: {n}", n = n) };
            ui.label(RichText::new(said).color(t.dim));
        } else if r.items.is_empty() {
            ui.label(RichText::new(tl!("The server keeps nothing deleted for good for this account.")).color(t.dim));
        } else {
            ui.label(RichText::new(tf!("{n} items", n = widgets::count_text(rows.len() as u64))).color(t.dim));
        }
        if r.more && !r.loading && !r.working && ui.button(tl!("Show more")).clicked() {
            acts.push(Action::Run("folder.recoverMore", json!({})));
        }
    });
    ui.add_space(8.0);
    let picked: Vec<String> = rows.iter().filter(|i| f.selected.contains(&i.id)).map(|i| i.id.clone()).collect();
    let idle = !r.working && !picked.is_empty();
    button_row(ui, |ui| {
        let close = ui.button(tl!("Close")).clicked();
        if ui.add_enabled(idle, egui::Button::new(RichText::new(tl!("Purge")).color(t.danger))).clicked() {
            f.confirming = Some(picked.clone());
        }
        if widgets::primary_button(ui, "Restore", t, idle).clicked() {
            acts.push(Action::Run("folder.recoverRestore", json!({"ids": picked})));
            f.selected.clear();
        }
        if close {
            acts.push(Action::Run("folder.recoverClose", json!({})));
        }
        !close
    })
}

/// The headings and the rows.
fn list(ui: &mut Ui, t: &Tokens, s: &Session, f: &mut RecoverForm, rows: &[&Recoverable]) {
    let width = ui.available_width();
    let subject_w = (width - KIND_W - FROM_W - DATE_W - SIZE_W - 12.0).max(120.0);
    let font = theme::regular(13.0);
    let (head, _) = ui.allocate_exact_size(vec2(width, ROW), Sense::hover());
    ui.painter().line_segment([head.left_bottom(), head.right_bottom()], egui::Stroke::new(1.0, t.border));
    let mut x = head.left() + 6.0;
    let cols = [
        ("Type", KIND_W, RecoverBy::Kind, false),
        ("Subject", subject_w, RecoverBy::Subject, false),
        ("From", FROM_W, RecoverBy::From, false),
        ("Deleted", DATE_W, RecoverBy::Deleted, false),
        ("Size", SIZE_W, RecoverBy::Size, true),
    ];
    for (label, w, by, right) in cols {
        let r = egui::Rect::from_min_size(egui::pos2(x, head.top()), vec2(w, ROW));
        let marked = f.by == by;
        let name = if label == "Type" { "" } else { tl!(label) };
        let text = if marked { format!("{name} {}", if f.flipped { "▴" } else { "▾" }) } else { name.to_string() };
        let resp = ui.interact(r, egui::Id::unique(("recover-head", label)), Sense::click());
        let color = if marked || resp.hovered() { t.text } else { t.dim };
        let (pos, align) =
            if right { (r.right_center() - vec2(6.0, 0.0), egui::Align2::RIGHT_CENTER) } else { (r.left_center(), egui::Align2::LEFT_CENTER) };
        ui.painter().text(pos, align, text, theme::bold(12.5), color);
        if resp.clicked() {
            f.flipped = marked && !f.flipped;
            f.by = by;
        }
        x += w;
    }
    let mods = ui.input(|i| i.modifiers);
    let mut clicked = None;
    egui::ScrollArea::vertical().id_salt("recover-rows").max_height(360.0).auto_shrink([false, true]).show_rows(ui, ROW, rows.len(), |ui, range| {
        for at in range {
            let Some(item) = rows.get(at) else { continue };
            let (r, resp) = ui.allocate_exact_size(vec2(width, ROW), Sense::click());
            let on = f.selected.contains(&item.id);
            if on {
                ui.painter().rect_filled(r, 4.0, t.accent.gamma_multiply(0.25));
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 4.0, t.hover_row);
            }
            let icon = match item.kind {
                RecoverableKind::Mail => Icon::Inbox,
                RecoverableKind::Appointment => Icon::Calendar,
                RecoverableKind::Contact => Icon::Person,
                RecoverableKind::Task => Icon::Tasks,
                RecoverableKind::Other => Icon::Info,
            };
            crate::icons::paint(
                ui.painter(),
                crate::icons::centered(egui::Rect::from_min_size(r.left_top() + vec2(6.0, 0.0), vec2(KIND_W - 6.0, ROW)), 16.0),
                icon,
                t.dim,
            );
            let mut left = r.left() + 6.0 + KIND_W;
            let mut text = |ui: &Ui, s: &str, w: f32, color: egui::Color32| {
                let g = widgets::one_line(ui, s, font.clone(), color, w - 8.0);
                ui.painter().galley(egui::pos2(left, r.center().y - g.size().y / 2.0), g, color);
                left += w;
            };
            let subject = if item.subject.is_empty() { tl!("(no subject)") } else { item.subject.as_str() };
            text(ui, subject, subject_w, t.text);
            text(ui, &item.from, FROM_W, t.dim);
            let when = if item.deleted > 0 { crate::i18n::date(&dates::full_time(item.deleted, &s.clock)) } else { String::new() };
            text(ui, &when, DATE_W, t.dim);
            ui.painter().text(
                egui::pos2(r.right() - 6.0, r.center().y),
                egui::Align2::RIGHT_CENTER,
                widgets::size_text(item.size),
                font.clone(),
                t.dim,
            );
            if resp.clicked() {
                clicked = Some(at);
            }
        }
    });
    if let Some(at) = clicked {
        f.pick(rows, at, mods.command || mods.ctrl, mods.shift);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, subject: &str, from: &str, deleted: i64, size: u64) -> Recoverable {
        Recoverable { id: id.into(), subject: subject.into(), from: from.into(), deleted, size, ..Default::default() }
    }

    fn ids(rows: &[&Recoverable]) -> Vec<String> {
        rows.iter().map(|r| r.id.clone()).collect()
    }

    /// The list opens with the most recently deleted first; a heading sorts it, again reverses it,
    /// and the filter matches the subject or the sender.
    #[test]
    fn the_list_filters_and_sorts() {
        let items = vec![item("a", "Budget", "Zoe", 100, 5), item("b", "Plans", "Ana", 300, 9), item("c", "Notes", "Zoe", 200, 1)];
        let mut f = RecoverForm::default();
        assert_eq!(ids(&f.shown(&items)), ["b", "c", "a"], "newest first");
        f.by = RecoverBy::Subject;
        assert_eq!(ids(&f.shown(&items)), ["a", "c", "b"]);
        f.flipped = true;
        assert_eq!(ids(&f.shown(&items)), ["b", "c", "a"]);
        f.by = RecoverBy::Size;
        f.flipped = false;
        assert_eq!(ids(&f.shown(&items)), ["b", "a", "c"], "the biggest first");
        f.filter = " zoe ".into();
        assert_eq!(ids(&f.shown(&items)), ["a", "c"]);
        f.filter = "plan".into();
        assert_eq!(ids(&f.shown(&items)), ["b"]);
    }

    /// A click picks one; Ctrl toggles; Shift picks the range from the last one picked.
    #[test]
    fn clicks_pick_with_shift_and_ctrl() {
        let items: Vec<Recoverable> = (0..6).map(|n| item(&format!("i{n}"), "s", "f", 100 - n, 1)).collect();
        let mut f = RecoverForm::default();
        let rows = f.shown(&items);
        f.pick(&rows, 1, false, false);
        assert_eq!(f.selected, HashSet::from(["i1".to_string()]));
        f.pick(&rows, 4, false, true);
        assert_eq!(f.selected.len(), 4, "i1 to i4");
        f.pick(&rows, 0, true, false);
        assert_eq!(f.selected.len(), 5, "Ctrl adds");
        f.pick(&rows, 0, true, false);
        assert_eq!(f.selected.len(), 4, "and takes away");
        f.pick(&rows, 3, false, false);
        assert_eq!(f.selected, HashSet::from(["i3".to_string()]), "alone, it picks only that one");
        f.pick(&rows, 99, false, false);
        assert_eq!(f.selected.len(), 1, "a row that isn't there changes nothing");
    }
}
