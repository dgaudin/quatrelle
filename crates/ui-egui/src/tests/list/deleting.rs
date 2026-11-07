// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: Shift+Delete deletes the mail selected for good, past Deleted Items. Asked first
//! while the option says so: Enter deletes, Escape keeps. In the Outbox, it does what Delete does;
//! in a message's page of its own, it deletes that message, not the list's selection. In Deleted
//! Items, every Delete (a row's button, its menu, the reading pane's) asks as the key does.

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
    matches!(a.ui.dialog, Some(Dialog::ConfirmDelete { count, .. }) if count == n)
}

#[test]
fn shift_delete_asks_then_enter_deletes_for_good() {
    let (mut a, dir, ctx) = opened("shift-delete-enter");
    press(&ctx, &mut a, Key::Delete, Modifiers::SHIFT);
    assert!(asking(&a, 1), "asked first");
    assert!(kept(&a, "INBOX"), "nothing deleted before the answer");
    press(&ctx, &mut a, Key::Enter, Modifiers::NONE);
    assert!(a.ui.dialog.is_none(), "answered");
    assert!(!kept(&a, "INBOX"), "gone from the Inbox");
    assert!(!kept(&a, "Trash"), "and not in Deleted Items: deleted for good");
    assert!(a.session.undo.is_empty(), "no Undo offered for it");
    let _ = std::fs::remove_dir_all(dir);
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

/// The question Delete asks in Deleted Items answers the same keys.
#[test]
fn delete_in_deleted_items_answers_enter_and_escape_too() {
    let (mut a, dir, ctx) = opened("trash-enter");
    press(&ctx, &mut a, Key::Delete, Modifiers::NONE);
    assert!(a.ui.dialog.is_none() && kept(&a, "Trash"), "moved to Deleted Items, unasked");
    a.run("folder.open", json!({"account": "sample", "folder": "Trash"}));
    a.run("list.select", json!({"key": key_of(&a, SUBJECT).to_id()}));
    press(&ctx, &mut a, Key::Delete, Modifiers::NONE);
    assert!(asking(&a, 1));
    press(&ctx, &mut a, Key::Escape, Modifiers::NONE);
    assert!(a.ui.dialog.is_none() && kept(&a, "Trash"));
    press(&ctx, &mut a, Key::Delete, Modifiers::NONE);
    press(&ctx, &mut a, Key::Enter, Modifiers::NONE);
    assert!(a.ui.dialog.is_none() && !kept(&a, "Trash"), "deleted for good");
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

/// A message's page of its own (the browser): Shift+Delete deletes that message, asked first,
/// not the one selected in the list; one no longer in the list isn't deleted (nor the list's).
#[test]
fn in_a_messages_own_page_shift_delete_deletes_its_message() {
    let (a, dir) = app("shift-delete-page");
    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut tab = MailApp::new(a.session, Box::new(NullBackend), Box::new(PageServices(opened.clone())));
    let ctx = egui::Context::default();
    frame(&ctx, &mut tab, vec![]);
    let other = "Character turnarounds";
    tab.run("list.select", json!({"key": key_of(&tab, other).to_id()}));
    tab.show_message_page(&key_of(&tab, SUBJECT).to_id());
    frame(&ctx, &mut tab, vec![]);
    press(&ctx, &mut tab, Key::Delete, Modifiers::SHIFT);
    assert!(asking(&tab, 1), "asked first");
    press(&ctx, &mut tab, Key::Enter, Modifiers::NONE);
    assert!(!kept(&tab, "INBOX") && !kept(&tab, "Trash"), "the page's message, for good");
    assert!(kept_as(&tab, "INBOX", other), "not the one selected in the list");

    // Another folder shown since: the page's message isn't in the list, the list's isn't taken.
    tab.show_message_page(&key_of(&tab, other).to_id());
    tab.run("folder.open", json!({"account": "sample", "folder": "Archive"}));
    tab.run("list.selectAll", json!({}));
    let archived = tab.session.selection.len();
    assert!(archived > 0, "something selected in the list");
    frame(&ctx, &mut tab, vec![]);
    let said = tab.session.notices.len();
    press(&ctx, &mut tab, Key::Delete, Modifiers::SHIFT);
    assert!(tab.ui.dialog.is_none() && kept_as(&tab, "INBOX", other), "refused");
    assert_eq!(tab.session.store.as_ref().unwrap().load_folder("sample", "Archive").unwrap().messages.len(), archived, "Archive untouched");
    assert!(tab.session.notices.len() > said, "said why");
    let _ = std::fs::remove_dir_all(dir);
}

/// Where the buttons named `name` are (as a screen reader is told of them), the pointer left
/// where it was.
fn buttons(ctx: &egui::Context, a: &mut MailApp, name: &str) -> Vec<egui::Rect> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let tree = out.platform_output.accesskit_update.expect("AccessKit is on");
    tree.nodes
        .iter()
        .filter(|(_, n)| n.label() == Some(name) && !n.is_disabled())
        .filter_map(|(_, n)| n.bounds())
        .map(|b| egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
        .collect()
}

/// The list's row for `subject`.
fn row(ctx: &egui::Context, a: &mut MailApp, subject: &str) -> egui::Rect {
    let rows = super::selecting::list_rows(ctx, a);
    rows.into_iter().find(|(l, _)| l.contains(subject)).map(|(_, r)| r).unwrap_or_else(|| panic!("no row for {subject}"))
}

/// The Delete button a row shows under the pointer, clicked.
fn hover_delete(ctx: &egui::Context, a: &mut MailApp, subject: &str) {
    let r = row(ctx, a, subject);
    frame(ctx, a, vec![Event::PointerMoved(r.center())]);
    let at = buttons(ctx, a, "Delete").into_iter().find(|b| r.contains(b.center())).expect("the row hovered shows Delete");
    super::selecting::click(ctx, a, at.center(), egui::PointerButton::Primary, Modifiers::NONE);
    frame(ctx, a, vec![]);
}

/// Delete in the menu a right-click on a row gives, clicked.
fn menu_delete(ctx: &egui::Context, a: &mut MailApp, subject: &str) {
    let r = row(ctx, a, subject);
    super::selecting::click(ctx, a, r.center(), egui::PointerButton::Secondary, Modifiers::NONE);
    let at = buttons(ctx, a, "Delete")
        .into_iter()
        .find(|b| !r.contains(b.center()) && (b.min.x - r.center().x).abs() < 40.0)
        .expect("the menu has Delete");
    super::selecting::click(ctx, a, at.center(), egui::PointerButton::Primary, Modifiers::NONE);
    frame(ctx, a, vec![]);
}

/// The sample with [`SUBJECT`] and another message in Deleted Items, shown there.
fn in_deleted_items(name: &str) -> (MailApp, std::path::PathBuf, egui::Context) {
    let (mut a, dir, ctx) = opened(name);
    ctx.enable_accesskit();
    a.run("list.select", json!({"key": key_of(&a, SUBJECT).to_id()}));
    a.run("list.select", json!({"key": key_of(&a, OTHER).to_id(), "add": true}));
    press(&ctx, &mut a, Key::Delete, Modifiers::NONE);
    assert!(a.ui.dialog.is_none() && kept(&a, "Trash") && kept_as(&a, "Trash", OTHER), "moved to Deleted Items, unasked");
    a.run("folder.open", json!({"account": "sample", "folder": "Trash"}));
    frame(&ctx, &mut a, vec![]);
    (a, dir, ctx)
}

const OTHER: &str = "Character turnarounds";

/// In Deleted Items, where deleting is for good, the button a row shows under the pointer asks
/// first, as the Delete key and the ribbon do; Escape keeps the message.
#[test]
fn in_deleted_items_the_rows_button_asks_first() {
    let (mut a, dir, ctx) = in_deleted_items("trash-row-button");
    hover_delete(&ctx, &mut a, SUBJECT);
    assert!(asking(&a, 1), "the row's button asks");
    assert!(kept(&a, "Trash"), "nothing deleted before the answer");
    press(&ctx, &mut a, Key::Escape, Modifiers::NONE);
    assert!(a.ui.dialog.is_none() && kept(&a, "Trash"));
    let _ = std::fs::remove_dir_all(dir);
}

/// Delete in a row's menu too; Enter deletes the message the menu was for, and only it.
#[test]
fn in_deleted_items_the_menus_delete_asks_first() {
    let (mut a, dir, ctx) = in_deleted_items("trash-row-menu");
    menu_delete(&ctx, &mut a, OTHER);
    assert!(asking(&a, 1), "the menu's Delete asks");
    assert!(kept_as(&a, "Trash", OTHER), "nothing deleted before the answer");
    press(&ctx, &mut a, Key::Enter, Modifiers::NONE);
    assert!(a.ui.dialog.is_none() && !kept_as(&a, "Trash", OTHER), "Enter deletes it for good");
    assert!(kept(&a, "Trash"), "only the one asked about");
    let _ = std::fs::remove_dir_all(dir);
}

/// A draft read in Deleted Items: the reading pane's Delete over it asks first too.
#[test]
fn in_deleted_items_a_drafts_delete_button_asks_first() {
    const DRAFT: &str = "Re: Sound mix, version 3";
    let (mut a, dir, ctx) = opened("trash-draft-button");
    ctx.enable_accesskit();
    a.run("mail.reply", json!({}));
    a.run("compose.saveDraft", json!({}));
    a.session.compose = None;
    a.run("folder.open", json!({"account": "sample", "folder": "Drafts"}));
    a.run("list.open", json!({"key": key_of(&a, DRAFT).to_id()}));
    press(&ctx, &mut a, Key::Delete, Modifiers::NONE);
    assert!(a.ui.dialog.is_none() && kept_as(&a, "Trash", DRAFT), "moved to Deleted Items, unasked");
    a.run("folder.open", json!({"account": "sample", "folder": "Trash"}));
    a.run("list.open", json!({"key": key_of(&a, DRAFT).to_id()}));
    frame(&ctx, &mut a, vec![]);
    let list = row(&ctx, &mut a, DRAFT);
    let at = buttons(&ctx, &mut a, "Delete").into_iter().find(|b| b.min.x > list.max.x).expect("the reading pane offers Delete");
    super::selecting::click(&ctx, &mut a, at.center(), egui::PointerButton::Primary, Modifiers::NONE);
    frame(&ctx, &mut a, vec![]);
    assert!(asking(&a, 1), "asked first");
    assert!(kept_as(&a, "Trash", DRAFT), "nothing deleted before the answer");
    let _ = std::fs::remove_dir_all(dir);
}

/// Anywhere else they move the mail to Deleted Items, unasked, as before.
#[test]
fn elsewhere_the_rows_button_and_menu_move_without_asking() {
    let (mut a, dir, ctx) = opened("inbox-row-delete");
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    hover_delete(&ctx, &mut a, SUBJECT);
    assert!(a.ui.dialog.is_none() && !kept(&a, "INBOX") && kept(&a, "Trash"), "moved, unasked");
    menu_delete(&ctx, &mut a, OTHER);
    assert!(a.ui.dialog.is_none() && !kept_as(&a, "INBOX", OTHER) && kept_as(&a, "Trash", OTHER), "moved, unasked");
    let _ = std::fs::remove_dir_all(dir);
}

/// Whether the window shows a text saying `what`.
fn shows(ctx: &egui::Context, a: &mut MailApp, what: &str) -> bool {
    frame_shapes(ctx, a, vec![], egui::vec2(1280.0, 800.0)).iter().any(|s| matches!(s, egui::Shape::Text(t) if t.galley.text().contains(what)))
}

fn sample_config(a: &mut MailApp, change: impl Fn(&mut quatrelle_engine::model::AccountConfig)) {
    change(&mut a.session.accounts.iter_mut().find(|x| x.config.id == "sample").expect("the sample account").config);
}

/// The question tells the truth about Exchange: mail deleted for good there is kept a while
/// among the server's recoverable items; anywhere else (IMAP here) it can't be undone.
#[test]
fn the_question_says_an_exchange_server_keeps_the_mail_a_while() {
    let (mut a, dir, ctx) = opened("for-good-exchange");
    for (ews, graph) in [(true, false), (false, true)] {
        sample_config(&mut a, |c| {
            c.ews = ews.then(|| "https://mail.contoso.example/EWS/Exchange.asmx".into());
            c.graph = graph.then(|| "https://graph.microsoft.com/v1.0".into());
        });
        press(&ctx, &mut a, Key::Delete, Modifiers::SHIFT);
        assert!(asking(&a, 1));
        assert!(shows(&ctx, &mut a, "the Exchange server keeps it for a while"), "EWS {ews}, Graph {graph}: kept a while");
        assert!(!shows(&ctx, &mut a, "This can't be undone."));
        press(&ctx, &mut a, Key::Escape, Modifiers::NONE);
    }
    sample_config(&mut a, |c| {
        c.ews = None;
        c.graph = None;
        c.imap.host = "imap.studio.example".into();
    });
    press(&ctx, &mut a, Key::Delete, Modifiers::SHIFT);
    assert!(asking(&a, 1));
    assert!(shows(&ctx, &mut a, "This message will be deleted for good. This can't be undone."), "IMAP: for good");
    assert!(!shows(&ctx, &mut a, "Exchange"));
    press(&ctx, &mut a, Key::Escape, Modifiers::NONE);
    let _ = std::fs::remove_dir_all(dir);
}

/// The question to empty Deleted Items says an Exchange server keeps the mail a while (and how to
/// bring it back); anywhere else (IMAP here) it can't be undone.
#[test]
fn emptying_says_an_exchange_server_keeps_the_mail_a_while() {
    let (mut a, dir, ctx) = opened("empty-exchange");
    let ask = |a: &mut MailApp| {
        let exchange = crate::dialogs::is_exchange(&a.session, "sample");
        a.ui.dialog =
            Some(Dialog::ConfirmEmpty { account: "sample".into(), folder: "Trash".into(), name: "Deleted Items".into(), for_good: true, exchange });
    };
    for (ews, graph) in [(true, false), (false, true)] {
        sample_config(&mut a, |c| {
            c.ews = ews.then(|| "https://mail.contoso.example/EWS/Exchange.asmx".into());
            c.graph = graph.then(|| "https://graph.microsoft.com/v1.0".into());
        });
        ask(&mut a);
        frame(&ctx, &mut a, vec![]);
        assert!(shows(&ctx, &mut a, "the Exchange server keeps it for a while"), "EWS {ews}, Graph {graph}: kept a while");
        assert!(shows(&ctx, &mut a, "Recover deleted items"));
        assert!(!shows(&ctx, &mut a, "This can't be undone."));
    }
    sample_config(&mut a, |c| {
        c.ews = None;
        c.graph = None;
        c.imap.host = "imap.studio.example".into();
    });
    ask(&mut a);
    frame(&ctx, &mut a, vec![]);
    assert!(shows(&ctx, &mut a, "Everything in it will be deleted for good"), "IMAP: for good");
    assert!(shows(&ctx, &mut a, "This can't be undone."));
    assert!(!shows(&ctx, &mut a, "Exchange"));
    let _ = std::fs::remove_dir_all(dir);
}
