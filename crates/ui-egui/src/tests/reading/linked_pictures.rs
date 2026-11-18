// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a picture inside a link (a newsletter's button) opens the link, shown or not; a
//! picture not shown says what it shows, and a spacer nothing.

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

/// Pictures on the web not shown, each with a description wider than the picture: a product's
/// photo of 280 by 280 shows its box with the description written in it, wrapped at its words, as
/// Outlook writes it in a picture it hasn't loaded (it was an empty frame); an icon of 24 in a link,
/// too small for its words, is a box that is said to a screen reader as the link and opens it.
#[test]
fn a_big_picture_not_shown_says_what_it_shows() {
    let shirt = "Blue cotton shirt with long sleeves, slim fit, from the new spring collection";
    let follow = "Follow Contoso on the network";
    let html = format!(
        "<p>Our new collection is here.</p>\
         <p><img src=\"https://img.contoso.example/shirt.jpg\" width=\"280\" height=\"280\" alt=\"{shirt}\"></p>\
         <p><a href=\"https://www.contoso.example/follow\"><img src=\"https://img.contoso.example/net.png\" width=\"24\" height=\"24\" alt=\"{follow}\"></a></p>"
    );
    let (mut a, dir) = app_with_html("big-picture-not-shown", "New collection", &html);
    let opened = Rc::new(RefCell::new(Vec::new()));
    a.services = Box::new(Opening(opened.clone()));
    use super::super::list::follow_up::{Textures, frame_with_size};
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    // Its textures kept from the first frame (the font's), for the picture painted below; at the
    // size `tap` draws at.
    let mut tex = Textures::default();
    let size = egui::vec2(1280.0, 800.0);
    frame_with_size(&ctx, &mut a, &mut tex, vec![], None, (1280, 800));
    let k = a.session.list.iter().find(|m| m.subject == "New collection").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let mut shapes = Vec::new();
    for _ in 0..6 {
        shapes = frame_with_size(&ctx, &mut a, &mut tex, vec![], None, (1280, 800)).1;
    }
    if let Some(d) = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from) {
        frame_with_size(&ctx, &mut a, &mut tex, vec![], Some(d.join("big-picture-not-shown.png")), (1280, 800));
    }
    let boxed = |w: f32| {
        shapes.iter().find_map(|s| match s {
            Shape::Rect(r) if r.brush.is_none() && r.stroke.width > 0.0 && (r.rect.width() - w).abs() < 1.5 && (r.rect.height() - w).abs() < 1.5 => {
                Some(r.rect)
            }
            _ => None,
        })
    };
    let photo = boxed(280.0).expect("the photo's box");
    let written = shapes.iter().find_map(|s| match s {
        Shape::Text(t) if t.galley.text() == shirt => Some(t),
        _ => None,
    });
    let written = written.unwrap_or_else(|| panic!("the description in the box: {:?}", texts(&shapes)));
    let at = written.galley.rect.translate(written.pos.to_vec2());
    assert!(photo.contains_rect(at), "inside the box: {at:?} in {photo:?}");
    assert!(written.galley.rows.len() > 1, "wrapped to the box");
    for pair in written.galley.rows.windows(2) {
        let end = pair[0].row.glyphs.last().map(|g| g.chr);
        let start = pair[1].row.glyphs.first().map(|g| g.chr);
        assert!(end.is_none_or(char::is_whitespace) || start.is_none_or(char::is_whitespace), "at its words: {shirt:?}");
    }
    // The icon: a box without words, said as the link, opening it.
    let icon = boxed(24.0).expect("the icon's box");
    let over: Vec<&str> = shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Text(t) if icon.shrink(1.0).intersects(t.galley.rect.translate(t.pos.to_vec2())) => Some(t.galley.text()),
            _ => None,
        })
        .collect();
    assert!(over.is_empty(), "no words in the icon: {over:?}");
    let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), ..Default::default() };
    let out = ctx.run_ui(input, |ui| a.ui(ui));
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    let role = nodes.iter().find(|(_, n)| n.label() == Some(follow)).map(|(_, n)| n.role());
    assert_eq!(role, Some(egui::accesskit::Role::Link), "said as the link");
    tap(&ctx, &mut a, icon.center());
    assert_eq!(*opened.borrow(), vec!["https://www.contoso.example/follow".to_string()], "the icon's box opens its link");
    let _ = std::fs::remove_dir_all(dir);
}

/// Spacers on the web not shown, described as spacers (`spacer-FFFFFF`, a white picture laying a
/// table out): their room is kept — a column 32 wide, a gap 25 high — and nothing is drawn in it,
/// neither `[spacer-FFFFFF]` across the gap nor a frame down the column, nor a bubble under the
/// pointer. A picture described in words keeps its placeholder, and so does one whose
/// description only repeats its file's name when it fits in its place, across and down (`[4321]`,
/// a phone number in 89 by 22); in a rule 6 high it's not written, and the rule keeps its room.
#[test]
fn a_spacer_not_shown_keeps_its_room_and_draws_nothing() {
    let spacer = "https://cdn.example/img/spacer-FFFFFF.jpg";
    let html = format!(
        "<table cellspacing=\"0\" cellpadding=\"0\"><tr><td>Before</td>\
         <td width=\"32\"><img src=\"{spacer}\" width=\"32\" height=\"118\" alt=\"spacer-FFFFFF\"></td>\
         <td>After</td></tr></table>\
         <p>Above</p><p><img src=\"{spacer}\" width=\"522\" height=\"25\" alt=\"spacer-FFFFFF\"></p><p>Below</p>\
         <p><img src=\"https://cdn.example/img/hall.jpg\" width=\"180\" height=\"90\" alt=\"Grande salle\"></p>\
         <p><img src=\"https://cdn.example/4321.jpg\" width=\"89\" height=\"22\" alt=\"4321\"></p>\
         <p>Over</p><p><img src=\"https://cdn.example/img/rule-grey.gif\" width=\"672\" height=\"6\" alt=\"rule-grey\"></p><p>Under</p>"
    );
    let (mut a, dir) = app_with_html("spacer-not-shown", "Spacers", &html);
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject == "Spacers").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let mut shapes = Vec::new();
    for _ in 0..6 {
        shapes = frame_shapes(&ctx, &mut a, vec![], size);
    }
    let placed = |shapes: &[Shape], text: &str| {
        shapes.iter().rev().find_map(|s| match s {
            Shape::Text(t) if t.galley.text() == text => Some(t.galley.rect.translate(t.pos.to_vec2())),
            _ => None,
        })
    };
    let found = |shapes: &[Shape], text: &str| placed(shapes, text).unwrap_or_else(|| panic!("{text:?} is drawn: {:?}", texts(shapes)));
    let (before, after) = (found(&shapes, "Before"), found(&shapes, "After"));
    let (above, below) = (found(&shapes, "Above"), found(&shapes, "Below"));
    // The reading pane's texts (the list's preview, left of it, has its own words).
    let in_pane = |shapes: &[Shape]| -> Vec<String> {
        shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Text(t) if t.pos.x >= before.left() - 1.0 => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect()
    };
    let shown = in_pane(&shapes);
    assert!(shown.iter().any(|t| t.contains("Grande salle")), "a picture described in words keeps its placeholder: {shown:?}");
    assert!(shown.iter().any(|t| t == "[4321]"), "a file's name that fits stays written: {shown:?}");
    assert!(!shown.iter().any(|t| t.contains("rule-grey")), "a file's name taller than its place isn't written: {shown:?}");
    assert!(!shown.iter().any(|t| t.contains("spacer")), "a spacer's name isn't drawn: {shown:?}");
    // What shows: a fill, an outline or a picture (a table's cell is a frame with nothing to see).
    let framed: Vec<Rect> = shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Rect(r)
                if (r.fill.a() > 0 || (r.stroke.width > 0.0 && r.stroke.color.a() > 0) || r.brush.is_some())
                    && [(32.0, 118.0), (522.0, 25.0), (672.0, 6.0)]
                        .iter()
                        .any(|(w, h)| (r.rect.width() - w).abs() < 1.5 && (r.rect.height() - h).abs() < 1.5) =>
            {
                Some(r.rect)
            }
            _ => None,
        })
        .collect();
    assert!(framed.is_empty(), "nothing drawn in a spacer's room: {framed:?}");
    assert!(after.left() - before.right() >= 31.5, "the column keeps its width: {before:?} {after:?}");
    assert!(below.top() - above.bottom() >= 25.0, "the gap keeps its height: {above:?} {below:?}");
    let (over, under) = (found(&shapes, "Over"), found(&shapes, "Under"));
    assert!(under.top() - over.bottom() >= 6.0, "the rule keeps its room: {over:?} {under:?}");
    // The pointer on the column: no bubble names it.
    let at = egui::pos2((before.right() + after.left()) / 2.0, before.center().y);
    let mut shapes = frame_shapes(&ctx, &mut a, vec![Event::PointerMoved(at)], size);
    for _ in 0..60 {
        shapes = frame_shapes(&ctx, &mut a, vec![], size);
    }
    let shown = in_pane(&shapes);
    assert!(!shown.iter().any(|t| t.contains("spacer")), "no bubble over a spacer: {shown:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Every text drawn.
fn texts(shapes: &[Shape]) -> Vec<String> {
    shapes.iter().filter_map(|s| if let Shape::Text(t) = s { Some(t.galley.text().to_string()) } else { None }).collect()
}
