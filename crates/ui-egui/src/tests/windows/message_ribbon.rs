// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a message's own window as Outlook's: its tabs (File, Message, Help), its Message
//! ribbon, and Reply, Reply all and Forward in its header, each acting on that window's message
//! and not on the one selected in the main window; who it went to named, not only addressed.
//! With `QUATRELLE_FRAME_PNG` set to a folder, the window is painted there in light and dark.

use super::super::list::follow_up::{Textures, frame_with, frame_with_size};
use super::*;

/// The sample with its first message in a window of its own; the window's message, and the one
/// after it (for the main window to select).
fn popped(name: &str) -> (MailApp, std::path::PathBuf, MessageKey, MessageKey) {
    let (mut a, dir) = app(name);
    let mine = a.session.list[0].key.clone();
    let other = a.session.list[1].key.clone();
    a.run("message.popOut", json!({"key": mine.to_id()}));
    (a, dir, mine, other)
}

/// How many times `text` was drawn.
fn drawn(shapes: &[egui::Shape], text: &str) -> usize {
    shapes.iter().filter(|s| matches!(s, egui::Shape::Text(t) if t.galley.text() == text)).count()
}

/// The window's tabs and its ribbon's groups, as Outlook's, each with Quatrelle's own buttons;
/// Reply, Reply all and Forward in its header too. In light and dark.
#[test]
fn a_message_window_has_tabs_a_message_ribbon_and_the_reply_group() {
    let (mut a, dir, _, _) = popped("window-ribbon");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut tex = Textures::default();
    let pictures = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    for (dark, theme) in [(Some(false), "light"), (Some(true), "dark")] {
        a.ui.dark = dark;
        for _ in 0..30 {
            frame_with(&ctx, &mut a, &mut tex, vec![], None);
        }
        let png = pictures.as_ref().map(|d| d.join(format!("message-window-{theme}.png")));
        let (said, shapes) = frame_with(&ctx, &mut a, &mut tex, vec![], png);
        // The window's own: the main window has none of these.
        for name in ["Help", "Ignore", "Junk", "Meeting", "Rules", "Actions", "Find related", "Zoom"] {
            assert!(said.iter().any(|l| l == name), "{name} in {said:?}");
        }
        for group in ["Delete", "Respond", "Quick steps", "Move", "Tags", "Find", "Zoom"] {
            assert!(drawn(&shapes, group) > 0, "the group {group}");
        }
        // Reply all: the main window's ribbon, the window's ribbon and the window's header (the
        // main window reads nothing).
        assert!(drawn(&shapes, "Reply all") >= 3, "{}", drawn(&shapes, "Reply all"));
    }
    let _ = std::fs::remove_dir_all(dir);
    // A wide window: every group with its large buttons.
    let (mut a, dir, _, _) = popped("window-ribbon-wide");
    a.ui.places.insert("message".into(), crate::WindowPlace { x: 10.0, y: 10.0, w: 1560.0, h: 700.0, maximized: false, monitor: None });
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut tex = Textures::default();
    for _ in 0..30 {
        frame_with_size(&ctx, &mut a, &mut tex, vec![], None, (1600, 900));
    }
    let png = pictures.as_ref().map(|d| d.join("message-window-wide.png"));
    let (said, _) = frame_with_size(&ctx, &mut a, &mut tex, vec![], png, (1600, 900));
    assert!(said.iter().any(|l| l == "Zoom"), "{said:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Reply in the window answers the window's message, while another is selected and read in the
/// main window, which stays on it.
#[test]
fn a_message_windows_reply_answers_its_message_not_the_one_selected() {
    let (mut a, dir, mine, other) = popped("window-reply-click");
    a.run("list.open", json!({"key": other.to_id()}));
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    // The window's (drawn over the main window's): its ribbon's or its header's, both its own.
    let at = settled_text_at(&ctx, &mut a, "Reply", size).expect("Reply");
    click_at(&ctx, &mut a, at, 1, size);
    let c = a.session.compose.as_ref().expect("a reply");
    assert_eq!(c.source.as_ref(), Some(&mine), "the window's message answered");
    assert_eq!(a.session.selection, vec![other.clone()], "the main window's selection as it was");
    assert_eq!(a.session.reading.as_ref().map(|r| r.key.clone()), Some(other));
    let _ = std::fs::remove_dir_all(dir);
}

/// Move to from the window: its dialog shows in the window (not the main one) and moves the
/// window's message, which then closes; the main window keeps its selection.
#[test]
fn a_message_windows_move_to_asks_in_it_and_moves_its_message() {
    let (mut a, dir, mine, other) = popped("window-move");
    a.run("list.open", json!({"key": other.to_id()}));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::InWindow(mine.clone(), Box::new(crate::Action::Dialog(Dialog::move_to()))));
    assert!(a.ui.dialog.is_none(), "not the main window's dialog");
    assert!(matches!(&a.ui.window_dialog, Some((k, Dialog::MoveTo { .. })) if *k == mine));
    let size = egui::vec2(1280.0, 800.0);
    // Its folders, the window's message's account's, drawn on top: Archive picked.
    let at = settled_text_at(&ctx, &mut a, "Archive", size).expect("the dialog's Archive");
    click_at(&ctx, &mut a, at, 1, size);
    assert!(a.session.summary(&mine).is_none(), "moved out of the Inbox");
    assert!(a.session.summary(&other).is_some(), "the main window's message stays");
    assert_eq!(a.session.selection, vec![other]);
    assert!(a.session.windows.is_empty(), "its window closes");
    assert!(a.ui.window_dialog.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// "To" names whom a message went to when its header gives only addresses, as Outlook does: a
/// contact by their card's name, oneself by one's account's; one unknown by the address.
#[test]
fn to_names_the_people_it_knows() {
    let dir = std::env::temp_dir().join(format!("quatrelle-ui-to-names-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    sample::seed_if_empty(&store, NOW).unwrap();
    let raw = "From: Clara Dias <clara@contoso.example>\r\nTo: bruno.reis@contoso.example, riley@studio.example\r\n\
               Cc: someone@contoso.example\r\nSubject: Names\r\nDate: Wed, 22 Oct 2026 10:00:00 +0200\r\n\
               Message-ID: <names@contoso.example>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello\r\n";
    let account = quatrelle_engine::sample::SAMPLE_ACCOUNT;
    let mut fs = store.load_folder(account, "INBOX").unwrap();
    let uid = fs.uid_next.max(1);
    let key = MessageKey { account: account.into(), folder: "INBOX".into(), validity: fs.validity.max(1), uid };
    store.write_body(&key, raw.as_bytes()).unwrap();
    let flags = quatrelle_engine::model::Flags { seen: true, ..Default::default() };
    fs.messages.push(quatrelle_engine::mime::summary_from_full(raw.as_bytes(), key.clone(), flags, raw.len() as u32, NOW));
    fs.uid_next = uid + 1;
    store.save_folder(account, "INBOX", &fs).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    let mut a = MailApp::new(s, Box::new(NullBackend), Box::new(NullServices(NOW)));
    a.session.run("contact.create", &json!({"name": "Bruno Reis", "emails": ["bruno.reis@contoso.example"]})).unwrap();
    a.run("list.open", json!({"key": key.to_id()}));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut tex = Textures::default();
    for _ in 0..4 {
        frame_with(&ctx, &mut a, &mut tex, vec![], None);
    }
    let (said, _) = frame_with(&ctx, &mut a, &mut tex, vec![], None);
    assert!(said.iter().any(|l| l == "Bruno Reis"), "the contact's name: {said:?}");
    assert!(said.iter().any(|l| l == "Riley Moreno"), "one's own account's name: {said:?}");
    assert!(!said.iter().any(|l| l == "bruno.reis@contoso.example" || l == "riley@studio.example"), "{said:?}");
    assert!(said.iter().any(|l| l == "someone@contoso.example"), "nobody known: the address");
    let _ = std::fs::remove_dir_all(dir);
}
