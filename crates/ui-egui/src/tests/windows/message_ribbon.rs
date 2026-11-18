// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a message's own window as Outlook's: its tabs (File, Message), its own File page, its
//! Message ribbon, and Reply, Reply all and Forward in its header, each acting on that window's
//! message and not on the one selected in the main window, wherever the main window is; its own
//! zoom; who it went to named, not only addressed.
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
        // The window's own: the main window has none of these. No Help: Quatrelle has none.
        for name in ["Message", "Ignore", "Junk", "Meeting", "Rules", "Actions", "Find related", "Zoom"] {
            assert!(said.iter().any(|l| l == name), "{name} in {said:?}");
        }
        assert!(!said.iter().any(|l| l == "Help"), "no Help tab");
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

/// The message windows' ribbon fits, by window.
fn fits(a: &MailApp) -> Vec<(String, usize, Vec<Vec<f32>>)> {
    let mut v: Vec<(String, usize, Vec<Vec<f32>>)> =
        a.ui.ribbon_groups
            .iter()
            .filter(|(k, _)| k.starts_with("message-window"))
            .map(|(k, g)| (k.clone(), g.compact_from, g.columns.clone()))
            .collect();
    v.sort_by(|x, y| x.0.cmp(&y.0));
    v
}

/// A frame in a window of `size`; how soon it asks for the next one.
fn frame_repaint(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2) -> std::time::Duration {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    out.viewport_output.get(&egui::ViewportId::ROOT).map_or(std::time::Duration::MAX, |v| v.repaint_delay)
}

/// Two message windows of different widths each keep their own fit of the ribbon (one fit for
/// both had each take the other's at every frame, ask for another frame, and never settle); once
/// fitted, no frame more is asked for. A window closed forgets its fit.
#[test]
fn two_message_windows_keep_their_own_ribbon_fit_and_settle() {
    let (mut a, dir) = app("window-fits");
    let wide = a.session.list[0].key.clone();
    let narrow = a.session.list[2].key.clone();
    let place = |w: f32| crate::WindowPlace { x: 10.0, y: 10.0, w, h: 560.0, maximized: false, monitor: None };
    let ctx = egui::Context::default();
    let size = egui::vec2(1700.0, 1000.0);
    a.ui.places.insert("message".into(), place(1560.0));
    a.run("message.popOut", json!({"key": wide.to_id()}));
    for _ in 0..3 {
        frame_repaint(&ctx, &mut a, size);
    }
    a.ui.places.insert("message".into(), place(640.0));
    a.run("message.popOut", json!({"key": narrow.to_id()}));
    for _ in 0..30 {
        frame_repaint(&ctx, &mut a, size);
    }
    let settled = fits(&a);
    assert_eq!(settled.len(), 2, "a fit for each window: {settled:?}");
    assert_ne!(settled[0].1, settled[1].1, "the wide one large, the narrow one compact: {settled:?}");
    for _ in 0..4 {
        let next = frame_repaint(&ctx, &mut a, size);
        assert_eq!(fits(&a), settled, "each keeps its own fit");
        assert!(next > std::time::Duration::from_millis(100), "no frame more asked for once fitted: {next:?}");
    }
    a.act(crate::Action::Run("message.closeWindow", json!({"key": narrow.to_id()})));
    frame_repaint(&ctx, &mut a, size);
    assert_eq!(fits(&a).len(), 1, "the closed window's fit forgotten");
    let _ = std::fs::remove_dir_all(dir);
}

/// What the window's accessible tree says of each button named `name`: its place, and whether it
/// can be used.
fn buttons_named(ctx: &egui::Context, a: &mut MailApp, name: &str) -> Vec<(egui::Rect, bool)> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let tree = out.platform_output.accesskit_update.expect("AccessKit is on");
    tree.nodes
        .iter()
        .filter(|(_, n)| n.label() == Some(name))
        .filter_map(|(_, n)| n.bounds().map(|b| (b, !n.is_disabled())))
        .map(|(b, on)| (egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)), on))
        .collect()
}

/// Its message no longer in the list (another folder shown in the main window, a message of it
/// selected and read): what files or marks the window's message is offered all the same, and
/// acts on it, the main window's list and selection left as they were.
#[test]
fn a_message_windows_buttons_act_on_it_wherever_the_main_window_is() {
    let (mut a, dir, mine, _) = popped("window-elsewhere");
    let subject = a.session.summary(&mine).unwrap().subject.clone();
    a.run("folder.open", json!({"account": "sample", "folder": "Sent"}));
    let first = a.session.list.first().map(|m| m.key.clone()).expect("Sent has mail");
    a.run("list.open", json!({"key": first.to_id()}));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    for _ in 0..10 {
        frame(&ctx, &mut a, vec![]);
    }
    for name in ["Ignore", "Junk", "Delete", "Archive", "Quick steps", "Move to", "Categorize", "Follow up", "Mark unread"] {
        let found = buttons_named(&ctx, &mut a, name);
        assert!(!found.is_empty() && found.iter().all(|(_, on)| *on), "{name} can be used: {found:?}");
    }
    let shown: Vec<MessageKey> = a.session.list.iter().map(|m| m.key.clone()).collect();
    let main = (a.session.selection.clone(), a.session.reading.as_ref().map(|r| r.key.clone()));
    a.act(crate::Action::InWindow(mine.clone(), Box::new(crate::Action::DeleteMail { for_good: false })));
    let trash = a.session.store.as_ref().unwrap().load_folder("sample", "Trash").unwrap();
    assert!(trash.messages.iter().any(|m| m.subject == subject), "the window's message in Deleted Items");
    assert_eq!(a.session.list.iter().map(|m| m.key.clone()).collect::<Vec<_>>(), shown, "the main window's list as it was");
    assert_eq!((a.session.selection.clone(), a.session.reading.as_ref().map(|r| r.key.clone())), main);
    assert!(a.session.windows.is_empty(), "its window closes");
    let _ = std::fs::remove_dir_all(dir);
}

/// The File tab opens the window's own File page, as Outlook's message window does (not the main
/// window's): Save As, Save Attachments, Print and the message's headers, each for the window's
/// message; Back returns to the message.
#[test]
fn a_message_windows_file_tab_opens_its_own_file_page() {
    let (mut a, dir, mine, other) = popped("window-file-page");
    a.run("list.open", json!({"key": other.to_id()}));
    let subject = a.session.summary(&mine).unwrap().subject.clone();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let size = egui::vec2(1280.0, 800.0);
    // The window's File (drawn over the main window's).
    let at = settled_text_at(&ctx, &mut a, "File", size).expect("File");
    click_at(&ctx, &mut a, at, 1, size);
    assert!(a.ui.backstage.is_none(), "not the main window's File page");
    let said = |a: &mut MailApp| {
        let mut tex = Textures::default();
        frame_with(&ctx, a, &mut tex, vec![], None).0
    };
    let page = said(&mut a);
    for name in ["Back", "Info", "Save As", "Print", "Message headers", "Close"] {
        assert!(page.iter().any(|l| l == name), "{name} in {page:?}");
    }
    assert!(page.iter().any(|l| l == &subject), "the window's message's details: {page:?}");
    let at = settled_text_at(&ctx, &mut a, "Message headers", size).expect("Message headers");
    click_at(&ctx, &mut a, at, 1, size);
    match &a.ui.window_dialog {
        Some((k, Dialog::Preview { text, .. })) => {
            assert_eq!(k, &mine);
            assert!(text.contains(&subject), "the window's message's headers: {text}");
        }
        other => panic!("its headers in the window: {other:?}"),
    }
    assert!(a.ui.dialog.is_none());
    let back = said(&mut a);
    assert!(!back.iter().any(|l| l == "Save Attachments"), "back on the message: {back:?}");
    let _ = std::fs::remove_dir_all(dir);
    // The page, painted (its textures kept from the first frame on).
    if let Some(pictures) = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from) {
        let (mut a, dir, _, other) = popped("window-file-page-picture");
        a.run("list.open", json!({"key": other.to_id()}));
        let ctx = egui::Context::default();
        let mut tex = Textures::default();
        let mut at = None;
        for _ in 0..30 {
            at = text_at(&frame_with(&ctx, &mut a, &mut tex, vec![], None).1, "File").or(at);
        }
        let at = at.expect("File");
        let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        for events in [vec![Event::PointerMoved(at)], vec![press(true)], vec![press(false)], vec![Event::PointerMoved(egui::pos2(600.0, 600.0))]] {
            frame_with(&ctx, &mut a, &mut tex, events, None);
        }
        for _ in 0..10 {
            frame_with(&ctx, &mut a, &mut tex, vec![], None);
        }
        frame_with(&ctx, &mut a, &mut tex, vec![], Some(pictures.join("message-window-file-page.png")));
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Each window keeps its own zoom: the window's text drawn at it, the main window's zoom left as
/// it is.
#[test]
fn a_message_windows_zoom_is_its_own() {
    let (mut a, dir) = app_with_html("window-zoom", "Zoomed", "<p>Read me larger</p>");
    let mine = a.session.list.iter().find(|m| m.subject == "Zoomed").map(|m| m.key.clone()).unwrap();
    let other = a.session.list.iter().find(|m| m.key != mine).map(|m| m.key.clone()).unwrap();
    a.run("message.popOut", json!({"key": mine.to_id()}));
    a.run("list.open", json!({"key": other.to_id()}));
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    let height = |a: &mut MailApp| {
        let mut h = None;
        for _ in 0..6 {
            let shapes = frame_shapes(&ctx, a, vec![], size);
            h = shapes.iter().rev().find_map(|s| match s {
                egui::Shape::Text(t) if t.galley.text().starts_with("Read me larger") => Some(t.galley.rect.height()),
                _ => None,
            });
        }
        h.expect("the window's text")
    };
    let before = height(&mut a);
    a.act(crate::Action::Run("view.zoom", json!({"percent": 200, "key": mine.to_id()})));
    let after = height(&mut a);
    assert!(after > before * 1.6, "drawn at its own zoom: {before} then {after}");
    assert_eq!(a.session.prefs.reading_zoom, 100, "the main window's zoom as it was");
    assert_eq!(a.session.window_zoom(&mine), 200);
    let _ = std::fs::remove_dir_all(dir);
}
