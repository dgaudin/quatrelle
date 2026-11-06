// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: Shift+Delete deletes the mail selected for good, past Deleted Items. Asked first
//! while the option says so: Escape keeps. In the Outbox, it does what Delete does.

use super::*;

const SUBJECT: &str = "Sound mix, version 3";

/// Whether the sample's `folder` still holds `subject` (on disk, not just in the list shown).
fn kept_as(a: &MailApp, folder: &str, subject: &str) -> bool {
    a.session.store.as_ref().and_then(|s| s.load_folder("sample", folder).ok()).is_some_and(|f| f.messages.iter().any(|m| m.subject == subject))
}

fn kept(a: &MailApp, folder: &str) -> bool {
    kept_as(a, folder, SUBJECT)
}

fn key_of(a: &MailApp, subject: &str) -> MessageKey {
    a.session.list.iter().find(|m| m.subject == subject).map(|m| m.key.clone()).unwrap_or_else(|| panic!("no {subject} in the list"))
}

/// The sample, its message [`SUBJECT`] open from the Inbox.
fn opened(name: &str) -> (MailApp, std::path::PathBuf, egui::Context) {
    let (mut a, dir) = app(name);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = key_of(&a, SUBJECT);
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    (a, dir, ctx)
}

fn press(ctx: &egui::Context, a: &mut MailApp, k: Key, m: Modifiers) {
    frame(ctx, a, vec![key(k, m)]);
    for _ in 0..2 {
        frame(ctx, a, vec![]);
    }
}

fn asking(a: &MailApp, n: usize) -> bool {
    matches!(a.ui.dialog, Some(Dialog::ConfirmDelete { count }) if count == n)
}

#[test]
fn escape_keeps_the_message() {
    let (mut a, dir, ctx) = opened("shift-delete-escape");
    press(&ctx, &mut a, Key::Delete, Modifiers::SHIFT);
    assert!(asking(&a, 1));
    press(&ctx, &mut a, Key::Escape, Modifiers::NONE);
    assert!(a.ui.dialog.is_none(), "Escape is Cancel");
    assert!(kept(&a, "INBOX") && a.session.list.iter().any(|m| m.subject == SUBJECT), "still there");
    assert!(!kept(&a, "Trash"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn with_the_option_off_shift_delete_deletes_at_once() {
    let (mut a, dir, ctx) = opened("shift-delete-unasked");
    a.run("view.confirmDelete", json!({"on": false}));
    press(&ctx, &mut a, Key::Delete, Modifiers::SHIFT);
    assert!(a.ui.dialog.is_none(), "not asked");
    assert!(!kept(&a, "INBOX") && !kept(&a, "Trash"));
    let _ = std::fs::remove_dir_all(dir);
}

/// In the Outbox, Shift+Delete is Delete: unasked, the message isn't sent.
#[test]
fn in_the_outbox_shift_delete_is_delete() {
    let (mut a, dir) = app("shift-delete-outbox");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let store = a.session.store.clone().unwrap();
    let raw = b"From: riley@studio.example\r\nTo: ann@example.org\r\nSubject: Plans\r\n\r\nSee you\r\n".to_vec();
    let item = quatrelle_engine::store::OutboxItem { id: 41, from: "riley@studio.example".into(), queued: NOW, ..Default::default() };
    store.outbox_add("sample", &item, &raw).unwrap();
    a.session.apply(MailEvent::Queued { account: "sample".into(), id: 41, error: "offline".into() });
    a.run("folder.outbox", json!({}));
    a.run("list.selectAll", json!({}));
    assert_eq!(a.session.selection.len(), 1);
    press(&ctx, &mut a, Key::Delete, Modifiers::SHIFT);
    assert!(a.ui.dialog.is_none(), "nothing to ask: it only isn't sent");
    assert!(store.outbox("sample").is_empty() && a.session.list.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}
