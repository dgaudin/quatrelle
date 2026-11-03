// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a picture inside a link (a newsletter's button) opens the link, shown or not.

use super::*;
use egui::{Pos2, Rect, Shape};
use std::cell::RefCell;
use std::rc::Rc;

/// Services that remember the addresses opened.
struct Opening(Rc<RefCell<Vec<String>>>);

impl crate::Services for Opening {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
    fn open_url(&mut self, url: &str) -> Result<(), String> {
        self.0.borrow_mut().push(url.into());
        Ok(())
    }
}

const PIXEL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// The sample mailbox with a message whose HTML has a remote picture in a link, a picture sent
/// with it (`cid:`) in another, and one in none; open in the reading pane, and what is opened.
fn newsletter(name: &str) -> (MailApp, std::path::PathBuf, egui::Context, Rc<RefCell<Vec<String>>>) {
    let (mut a, dir) = app(name);
    let html = "<p><a href=\"https://example.com/rsvp\"><img src=\"https://example.com/b.jpg\" alt=\"Bouton Participera\"></a></p>\
                <p><a href=\"https://example.com/calendar\"><img src=\"cid:cal@x\" width=\"120\" height=\"40\"></a></p>\
                <p><img src=\"cid:plain@x\" width=\"90\" height=\"30\"></p>";
    let raw = format!(
        "From: Studio Shop <shop@studio.example>\r\nTo: riley@studio.example\r\nSubject: Newsletter buttons\r\n\
         Date: Wed, 22 Oct 2026 10:00:00 +0200\r\nMessage-ID: <buttons-{name}@x>\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/related; boundary=\"b1\"\r\n\r\n--b1\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n\
         --b1\r\nContent-Type: image/png\r\nContent-Transfer-Encoding: base64\r\nContent-ID: <cal@x>\r\n\
         Content-Disposition: inline; filename=\"cal.png\"\r\n\r\n{PIXEL}\r\n\
         --b1\r\nContent-Type: image/png\r\nContent-Transfer-Encoding: base64\r\nContent-ID: <plain@x>\r\n\
         Content-Disposition: inline; filename=\"plain.png\"\r\n\r\n{PIXEL}\r\n--b1--\r\n"
    );
    let store = a.session.store.clone().unwrap();
    let mut st = store.load_folder("sample", "INBOX").unwrap();
    let uid = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1;
    let key = quatrelle_engine::model::MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: st.validity.max(1), uid };
    let raw = raw.into_bytes();
    st.messages.insert(0, quatrelle_engine::mime::summary_from_full(&raw, key.clone(), Default::default(), raw.len() as u32, NOW));
    store.write_body(&key, &raw).unwrap();
    store.save_folder("sample", "INBOX", &st).unwrap();
    let opened = Rc::new(RefCell::new(Vec::new()));
    a.services = Box::new(Opening(opened.clone()));
    a.session.apply(quatrelle_engine::model::Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("list.open", json!({"key": key.to_id()}));
    for _ in 0..4 {
        frame(&ctx, &mut a, vec![]);
    }
    (a, dir, ctx, opened)
}

/// Clicks at `at` and lets a frame pass.
fn tap(ctx: &egui::Context, a: &mut MailApp, at: Pos2) {
    let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    for events in [vec![Event::PointerMoved(at)], vec![button(true)], vec![button(false)], vec![]] {
        frame(ctx, a, events);
    }
}

/// The picture drawn `w` by `h` points.
fn picture_of(shapes: &[Shape], w: f32, h: f32) -> Option<Rect> {
    shapes.iter().find_map(|s| match s {
        Shape::Rect(r) if r.brush.is_some() => {
            let r = r.rect;
            ((r.width() - w).abs() < 1.5 && (r.height() - h).abs() < 1.5).then_some(r)
        }
        _ => None,
    })
}

/// A remote picture not allowed shows as `[alt]` in the link's colour; a click on it opens the
/// link, as does one on a picture sent with the message; a picture in no link opens nothing.
#[test]
fn a_picture_in_a_link_opens_the_link() {
    let (mut a, dir, ctx, opened) = newsletter("linked-pictures");
    let size = egui::vec2(1280.0, 800.0);
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    let accent = a.ui.tokens.accent;
    let placeholder = shapes.iter().rev().find_map(|s| match s {
        Shape::Text(t) if t.galley.text() == "[Bouton Participera]" => {
            Some((t.pos + t.galley.rect.center().to_vec2(), t.galley.job.sections[0].format.color))
        }
        _ => None,
    });
    let (at, color) = placeholder.expect("the placeholder is drawn");
    assert_eq!(color, accent, "drawn as a link");
    tap(&ctx, &mut a, at);
    assert_eq!(*opened.borrow(), vec!["https://example.com/rsvp".to_string()], "the placeholder opens its link");
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    let linked = picture_of(&shapes, 120.0, 40.0).expect("the linked picture is drawn");
    tap(&ctx, &mut a, linked.center());
    assert_eq!(opened.borrow().last().map(String::as_str), Some("https://example.com/calendar"), "the picture opens its link");
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    let bare = picture_of(&shapes, 90.0, 30.0).expect("the other picture is drawn");
    let before = opened.borrow().len();
    tap(&ctx, &mut a, bare.center());
    assert_eq!(opened.borrow().len(), before, "a picture in no link opens nothing");
    let _ = std::fs::remove_dir_all(dir);
}
