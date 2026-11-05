// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: pictures in the message being written, at their size and resized (from upstream
//! b5f14ee and 865a712); and those of a message read (`pictures/`).

use super::*;
use egui::{Pos2, Rect, Shape, vec2};

mod bench;
mod bodies;
mod reading;
mod threads;
mod videos;

/// A picture of one colour, as a PNG.
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(w, h, image::Rgba([40, 90, 200, 255])))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// A PDF of `pages` pages (100 x 50 points), each with a box in it.
fn pdf(pages: usize) -> Vec<u8> {
    let content = "0 0 1 rg 10 10 40 30 re f";
    let kids: Vec<String> = (0..pages).map(|k| format!("{} 0 R", 3 + 2 * k)).collect();
    let mut objects = vec!["<< /Type /Catalog /Pages 2 0 R >>".to_string(), format!("<< /Type /Pages /Kids [{}] /Count {pages} >>", kids.join(" "))];
    for k in 0..pages {
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 50] /Contents {} 0 R >>", 4 + 2 * k));
        objects.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for off in offsets {
        out.extend(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

/// Pictures to attach: `n` small PNGs.
fn pictures(n: usize) -> Vec<(String, &'static str, Vec<u8>)> {
    (0..n).map(|i| (format!("photo {i}.png"), "image/png", png(8 + (i % 5) as u32, 8))).collect()
}

/// A message with these files (and, when given, this HTML showing these pictures by `cid:`) put
/// first in the Inbox, and the Inbox shown.
fn with_files_and_html(
    a: &mut MailApp,
    uid: u32,
    files: Vec<(String, &str, Vec<u8>)>,
    html: Option<String>,
    inline: Vec<quatrelle_engine::mime::InlinePicture>,
) -> MessageKey {
    use quatrelle_engine::mime::{self, Draft, OutAttachment};
    let d = Draft {
        subject: format!("Files {uid}"),
        body: "Here they are.".into(),
        attachments: files.into_iter().map(|(name, mime, data)| OutAttachment { name, mime: mime.into(), data }).collect(),
        html,
        inline,
        ..Default::default()
    };
    let raw = mime::build(&d, &quatrelle_engine::model::EmailAddress::new("Sam", "sam@studio.example"), NOW, &format!("<files{uid}@x>")).unwrap();
    with_raw(a, uid, &raw)
}

/// A message with these files put first in the Inbox, and the Inbox shown.
fn with_files(a: &mut MailApp, uid: u32, files: Vec<(String, &str, Vec<u8>)>) -> MessageKey {
    with_files_and_html(a, uid, files, None, Vec::new())
}

/// A message of this source put first in the Inbox, and the Inbox shown.
fn with_raw(a: &mut MailApp, uid: u32, raw: &[u8]) -> MessageKey {
    use quatrelle_engine::mime;
    let store = a.session.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid };
    inbox.messages.insert(0, mime::summary_from_full(raw, key.clone(), Default::default(), 0, NOW));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, raw).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    key
}

const WINDOW: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// Frames in a window this big until `done` (at most `max`); how many it took, and the most
/// pictures decoded in one of them.
fn pump(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2, max: usize, done: impl Fn(&MailApp) -> bool) -> (usize, usize) {
    let mut most = 0;
    for n in 0..max {
        if done(a) {
            return (n, most);
        }
        let before = a.ui.pictures.counts().1;
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        most = most.max(a.ui.pictures.counts().1 - before);
    }
    (max, most)
}

/// One frame in a window this big: how many pictures it made textures of (the fonts' aside).
fn textures_made(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2) -> usize {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    let n = out.textures_delta.set.iter().filter(|(id, _)| *id != &egui::TextureId::default()).count();
    out.textures_delta.clear();
    n
}

/// What a frame paints in the message: where its picture is (the one image not of the fonts),
/// and whether a handle shows (a small square in the accent colour).
fn paint(a: &mut MailApp, ctx: &egui::Context, events: Vec<Event>) -> (Option<Rect>, bool) {
    let accent = a.ui.tokens.accent;
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    fn walk(s: &Shape, picture: &mut Option<Rect>, handle: &mut bool, accent: egui::Color32) {
        match s {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, picture, handle, accent)),
            Shape::Mesh(m) if m.texture_id != egui::TextureId::default() => *picture = Some(m.calc_bounds()),
            Shape::Rect(r) if r.fill == accent && r.rect.width() < 12.0 && r.rect.height() < 12.0 => *handle = true,
            _ => {}
        }
    }
    let (mut picture, mut handle) = (None, false);
    out.shapes.iter().for_each(|c| walk(&c.shape, &mut picture, &mut handle, accent));
    (picture, handle)
}

/// A picture in a message shows as big as it is, and the handle at its corner, shown while it's
/// hovered, resizes it, its height following; its right-click menu sets a size, and one wider
/// than a page goes in at a page's width.
#[test]
fn pictures_in_a_message_show_their_size_and_resize_by_their_handle() {
    let (mut a, dir) = app("picture-size");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ann@x.example", "subject": "Poster", "body": ""}));
    let picture = |w: u32, h: u32| {
        let mut png = Vec::new();
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(w, h, image::Rgba([200, 90, 30, 255])));
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        png
    };
    a.run("compose.insertPicture", json!({"name": "poster.png", "data": picture(320, 200), "at": 0}));
    paint(&mut a, &ctx, vec![]);
    let (r, handle) = paint(&mut a, &ctx, vec![]);
    let r = r.expect("the picture is drawn");
    assert!((r.width() - 320.0).abs() < 1.0 && (r.height() - 200.0).abs() < 1.0, "its own size: {r:?}");
    assert!(!handle, "no handle while the pointer is elsewhere");
    let (_, handle) = paint(&mut a, &ctx, vec![Event::PointerMoved(r.center())]);
    assert!(handle, "hovered, the handle shows");
    // Dragged 120 points in from its corner: 200 wide, as tall as that makes it.
    let corner = r.right_bottom() - vec2(4.0, 4.0);
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    paint(&mut a, &ctx, vec![Event::PointerMoved(corner)]);
    paint(&mut a, &ctx, vec![button(corner, true)]);
    for k in 1..=6 {
        paint(&mut a, &ctx, vec![Event::PointerMoved(corner - vec2(20.0 * k as f32, 0.0))]);
    }
    paint(&mut a, &ctx, vec![button(corner - vec2(120.0, 0.0), false)]);
    let (r2, _) = paint(&mut a, &ctx, vec![Event::PointerMoved(Pos2::new(5.0, 5.0))]);
    let r2 = r2.expect("still drawn");
    assert!((r2.width() - 200.0).abs() < 1.5 && (r2.height() - 125.0).abs() < 1.5, "resized: {r2:?}");
    let c = a.session.compose.clone().unwrap();
    let at = c.body.chars().position(|ch| ch == quatrelle_engine::mime::rich::PICTURE).unwrap();
    assert!(c.styles.style(at).picture_width.is_some_and(|w| w.abs_diff(200) <= 1), "{:?}", c.styles.style(at));
    assert!(quatrelle_engine::mime::rich::to_html(&c.body, &c.styles).contains("width=\"200\""), "sent at that width");
    // Undone: its own size back.
    a.run("compose.resizePicture", json!({"cid": c.styles.style(at).picture.unwrap()}));
    let (r3, _) = paint(&mut a, &ctx, vec![]);
    assert!(r3.is_some_and(|r| (r.width() - 320.0).abs() < 1.0));
    // A screenshot wider than a page, pasted: at a page's width, on a line of its own.
    a.act(crate::Action::PasteImage(picture(1800, 900)));
    let c = a.session.compose.clone().unwrap();
    let cid = c.inline.last().map(|p| p.cid.clone()).unwrap();
    let wide = c
        .body
        .chars()
        .enumerate()
        .position(|(i, ch)| ch == quatrelle_engine::mime::rich::PICTURE && c.styles.style(i).picture.as_deref() == Some(cid.as_str()))
        .unwrap();
    assert_eq!(c.styles.style(wide).picture_width, Some(crate::rich_edit::FIT_WIDTH as u16));
    assert!(wide == 0 || c.body.chars().nth(wide - 1) == Some('\n'), "on a line of its own: {:?}", c.body);
    assert_eq!(c.body.chars().nth(wide + 1), Some('\n'), "on a line of its own: {:?}", c.body);
    let _ = std::fs::remove_dir_all(dir);
}
