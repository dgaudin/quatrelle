// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: attachments previewed in the reading pane, and opened.

use super::*;

#[test]
fn every_kind_of_attachment_previews_in_the_reading_pane() {
    let (mut a, dir) = app("attachment-preview");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject.starts_with("Your proof")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    let n = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map_or(0, |p| p.attachments.len());
    assert_eq!(n, 4, "notes, a PDF, a document and a workbook");
    for i in 0..n {
        a.ui.attachment_view = Some((k.clone(), i));
        // A PDF draws a page a frame.
        for _ in 0..3 {
            frame(&ctx, &mut a, vec![]);
        }
        assert_eq!(a.ui.attachment_view, Some((k.clone(), i)));
    }
    // Escape: back to the message.
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.attachment_view.is_none());
    // One that isn't there any more (another message's index) is just the message.
    a.ui.attachment_view = Some((k.clone(), 99));
    frame(&ctx, &mut a, vec![]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Back from a previewed attachment, its chip isn't left selected.
#[test]
fn leaving_an_attachment_preview_lets_go_of_its_chip() {
    let (mut a, dir) = app("attach-back");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject.contains("festival poster")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    let index = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.attachments[0].index).unwrap();
    a.ui.attachment_view = Some((k.clone(), index));
    a.ui.attachment_selected = Some((k.clone(), vec![index]));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.attachment_view.is_none() && a.ui.attachment_selected.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// Frames at a time of the test's own, so a double-click's two clicks can come closer together
/// than the double-click time, and the time can run out.
struct Clocked(f64);

impl Clocked {
    /// A frame `step` seconds after the last; each chip's ▾ (the chip's body is to its left), in
    /// reading order.
    fn frame(&mut self, ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>, step: f64) -> Vec<egui::Rect> {
        self.0 += step;
        let input = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
            time: Some(self.0),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
        let mut mores: Vec<egui::Rect> = nodes
            .iter()
            .filter(|(_, n)| n.label() == Some("More options"))
            .filter_map(|(_, n)| n.bounds())
            .map(|b| egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
            .collect();
        mores.sort_by(|p, q| p.top().total_cmp(&q.top()).then(p.left().total_cmp(&q.left())));
        mores
    }

    /// A double-click on the chip whose ▾ is `more`, then a click alone: the double-click opens
    /// the file (the test's services say it can't) and leaves no preview, then or after; the click
    /// previews it once the double-click time is out. What it previews, if anything.
    fn double_then_single(&mut self, ctx: &egui::Context, a: &mut MailApp, more: egui::Rect, what: &str) -> Option<(MessageKey, usize)> {
        let body = egui::pos2(more.left() - 60.0, more.center().y);
        let press = |pressed| Event::PointerButton { pos: body, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        let notices = a.session.notices.len();
        self.frame(ctx, a, vec![Event::PointerMoved(body)], 0.05);
        self.frame(ctx, a, vec![press(true)], 0.05);
        self.frame(ctx, a, vec![press(false)], 0.05);
        assert!(a.ui.attachment_view.is_none(), "{what}: a click waits out the double-click time");
        self.frame(ctx, a, vec![press(true)], 0.05);
        self.frame(ctx, a, vec![press(false)], 0.05);
        for _ in 0..3 {
            self.frame(ctx, a, vec![], 0.4);
        }
        assert!(a.ui.attachment_view.is_none(), "{what}: a double-click left a preview");
        assert!(a.session.notices.len() > notices, "{what}: a double-click opens the file");
        self.frame(ctx, a, vec![press(true)], 0.05);
        self.frame(ctx, a, vec![press(false)], 0.05);
        assert!(a.ui.attachment_view.is_none(), "{what}: a click waits out the double-click time");
        for _ in 0..2 {
            self.frame(ctx, a, vec![], 0.4);
        }
        a.ui.attachment_view.clone()
    }
}

/// A message with a page, a macro document and a PDF attached, put in the sample's Inbox.
fn files_to_check(a: &mut MailApp) -> MessageKey {
    let raw = b"From: Zoe Park <zoe@studio.example>\r\nTo: riley@studio.example\r\nSubject: Files to check\r\nMessage-ID: <risky@x>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nSee the files.\r\n--b\r\nContent-Type: text/html; name=\"page.html\"\r\nContent-Disposition: attachment; filename=\"page.html\"\r\nContent-Transfer-Encoding: base64\r\n\r\nPGh0bWw+PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0PjwvaHRtbD4=\r\n--b\r\nContent-Type: application/vnd.ms-word.document.macroEnabled.12; name=\"budget.docm\"\r\nContent-Disposition: attachment; filename=\"budget.docm\"\r\nContent-Transfer-Encoding: base64\r\n\r\nUEsDBAo=\r\n--b\r\nContent-Type: application/pdf; name=\"report.pdf\"\r\nContent-Disposition: attachment; filename=\"report.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0=\r\n--b--\r\n";
    let store = a.session.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9902 };
    inbox.messages.insert(0, quatrelle_engine::mime::summary_from_full(raw, key.clone(), Default::default(), raw.len() as u32, NOW));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, raw).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    key
}

/// A page whose scripts a browser would run, or a document whose macros would, asks first; a PDF
/// opens at once, and a page is only handed to its program once the question is answered.
#[test]
fn a_page_or_a_macro_document_asks_before_it_opens() {
    let (mut a, dir) = app("risky-attachment");
    let ctx = egui::Context::default();
    let key = files_to_check(&mut a);
    a.run("list.open", json!({"key": key.to_id()}));
    frame(&ctx, &mut a, vec![]);
    let at = |a: &MailApp, name: &str| {
        a.session
            .reading
            .as_ref()
            .and_then(|r| r.parsed.as_ref())
            .and_then(|p| p.attachments.iter().find(|x| x.name == name))
            .map(|x| x.index)
            .unwrap_or_else(|| panic!("{name} is attached"))
    };
    let (page, macro_doc, pdf) = (at(&a, "page.html"), at(&a, "budget.docm"), at(&a, "report.pdf"));
    // A page: asked first, and nothing handed to another program yet.
    a.act(crate::Action::OpenAttachment(page));
    match &a.ui.dialog {
        Some(Dialog::RiskyAttachment { name, risk: quatrelle_engine::mime::OpenRisk::WebPage, .. }) => assert_eq!(name, "page.html"),
        other => panic!("asked: {other:?}"),
    }
    assert!(a.session.notices.is_empty(), "nothing opened: {:?}", a.session.notices);
    // The question's Open: the file goes to its program (which the test's refuses, and is said).
    a.ui.dialog = None;
    a.act(crate::Action::InWindow(key.clone(), Box::new(crate::Action::OpenAttachmentNow(page))));
    assert!(a.session.notices.last().is_some_and(|n| n.text.contains("page.html could not be opened")), "{:?}", a.session.notices);
    a.session.notices.clear();
    // A macro document: its own warning.
    a.act(crate::Action::OpenAttachment(macro_doc));
    match &a.ui.dialog {
        Some(Dialog::RiskyAttachment { name, risk: quatrelle_engine::mime::OpenRisk::Macros, .. }) => assert_eq!(name, "budget.docm"),
        other => panic!("asked: {other:?}"),
    }
    a.ui.dialog = None;
    // A PDF is no risk: it goes straight to its program.
    a.act(crate::Action::OpenAttachment(pdf));
    assert!(a.ui.dialog.is_none(), "no question for a PDF");
    assert!(a.session.notices.last().is_some_and(|n| n.text.contains("report.pdf could not be opened")), "{:?}", a.session.notices);
    let _ = std::fs::remove_dir_all(dir);
}

/// Asked from a message's own window, the question shows in that window, not in the main one
/// (which reads another message); its Open hands that window's message's page to its program.
#[test]
fn a_page_asks_in_the_message_window_it_is_opened_from() {
    let (mut a, dir) = app("risky-attachment-window");
    let key = files_to_check(&mut a);
    let plain = a.session.list.iter().find(|m| !m.has_attachments).map(|m| m.key.clone()).unwrap();
    a.run("message.popOut", json!({"key": key.to_id()}));
    a.run("list.open", json!({"key": plain.to_id()}));
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    frame(&ctx, &mut a, vec![]);
    let page = a
        .session
        .windows
        .iter()
        .find(|w| w.key == key)
        .and_then(|w| w.parsed.as_ref())
        .and_then(|p| p.attachments.iter().find(|x| x.name == "page.html"))
        .map(|x| x.index)
        .expect("the window's page");
    a.act(crate::Action::InWindow(key.clone(), Box::new(crate::Action::OpenAttachment(page))));
    assert!(a.ui.dialog.is_none(), "not asked in the main window: {:?}", a.ui.dialog);
    match &a.ui.window_dialog {
        Some((k, Dialog::RiskyAttachment { key: of, name, .. })) => {
            assert_eq!((k, of), (&key, &key));
            assert_eq!(name, "page.html");
        }
        other => panic!("asked in the window: {other:?}"),
    }
    assert!(a.session.notices.is_empty(), "nothing opened yet: {:?}", a.session.notices);
    // Its Open, clicked in the window: the page goes to its program (which the test's refuses,
    // and that is said).
    let at = settled_text_at(&ctx, &mut a, "Open", size).expect("the question's Open");
    click_at(&ctx, &mut a, at, 1, size);
    assert!(a.ui.window_dialog.is_none(), "answered");
    assert!(a.session.notices.iter().any(|n| n.text.contains("page.html could not be opened")), "{:?}", a.session.notices);
    assert_eq!(a.session.reading.as_ref().map(|r| r.key.clone()), Some(plain), "the main window reads its own message");
    let _ = std::fs::remove_dir_all(dir);
}

/// A double-click on an attachment only opens it, as Outlook's: its first click showed the file
/// in the reading pane, left there behind the app it opened in. A click alone still previews it,
/// once the double-click time is out. Every kind: text, PDF, Office files, pictures, an attached
/// message and a file of no known kind.
#[test]
fn a_double_clicked_attachment_only_opens() {
    let (mut a, dir) = app("attach-double");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let raw = b"From: Sam <sam@studio.example>\r\nSubject: Forwarded with a file\r\nMIME-Version: 1.0\r\n\
        Content-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n--b1\r\nContent-Type: text/plain\r\n\r\nSee below.\r\n\
        --b1\r\nContent-Type: message/rfc822\r\nContent-Disposition: attachment; filename=\"earlier.eml\"\r\n\r\n\
        From: Zoe <zoe@studio.example>\r\nSubject: Earlier\r\n\r\nThe first one.\r\n\
        --b1\r\nContent-Type: application/x-unknown\r\nContent-Disposition: attachment; filename=\"data.qqq\"\r\n\r\nxyz\r\n--b1--\r\n";
    let store = a.session.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let own = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9901 };
    inbox.messages.insert(0, quatrelle_engine::mime::summary_from_full(raw, own.clone(), Default::default(), 0, NOW));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&own, raw).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    let mut clock = Clocked(1.0);
    for subject in ["Your proof", "festival poster", "Forwarded with a file"] {
        let k = a.session.list.iter().find(|m| m.subject.contains(subject)).map(|m| m.key.clone()).unwrap();
        a.run("list.open", json!({"key": k.to_id()}));
        for _ in 0..3 {
            clock.frame(&ctx, &mut a, vec![], 0.5);
        }
        let indices: Vec<usize> =
            a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.attachments.iter().map(|x| x.index).collect()).unwrap_or_default();
        assert!(indices.len() >= 2, "{subject}: {indices:?}");
        for (i, index) in indices.iter().enumerate() {
            let mut chips = Vec::new();
            for _ in 0..3 {
                chips = clock.frame(&ctx, &mut a, vec![], 0.5);
            }
            assert_eq!(chips.len(), indices.len(), "{subject}");
            let shown = clock.double_then_single(&ctx, &mut a, chips[i], &format!("{subject} #{i}"));
            assert_eq!(shown, Some((k.clone(), *index)), "{subject} #{i}: a click previews it");
            clock.frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)], 0.5);
            assert!(a.ui.attachment_view.is_none());
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// The same in a message's own window: a double-click only opens, a click previews.
#[test]
fn a_double_clicked_attachment_only_opens_in_a_message_window() {
    let (mut a, dir) = app("attach-double-window");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut clock = Clocked(1.0);
    clock.frame(&ctx, &mut a, vec![], 0.5);
    // The reading pane shows a message without attachments: every chip is the window's.
    let plain = a.session.list.iter().find(|m| !m.has_attachments).map(|m| m.key.clone()).unwrap();
    let with = a.session.list.iter().find(|m| m.subject.starts_with("Your proof")).map(|m| m.key.clone()).unwrap();
    a.run("message.popOut", json!({"key": with.to_id()}));
    a.run("list.open", json!({"key": plain.to_id()}));
    let mut chips = Vec::new();
    for _ in 0..4 {
        chips = clock.frame(&ctx, &mut a, vec![], 0.5);
    }
    assert_eq!(chips.len(), 4, "the window's four attachments");
    let w = a.session.windows.iter().find(|w| w.key == with).cloned().expect("the window");
    let index = w.parsed.as_ref().and_then(|p| p.attachments.get(1)).map(|x| x.index).unwrap();
    let shown = clock.double_then_single(&ctx, &mut a, chips[1], "the window's PDF");
    assert_eq!(shown, Some((with, index)), "a click previews it in the window");
    let _ = std::fs::remove_dir_all(dir);
}
