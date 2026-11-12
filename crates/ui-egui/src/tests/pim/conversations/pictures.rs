// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the conversations' pictures. Kept with a message (asked for as it comes on screen,
//! thumbnails that open big), people's photos on the list's rows, and the decoder's limits: a
//! picture past 40 megapixels or one that panics it is a placeholder, and it goes on.

use quatrelle_engine::model::pim::teams::{ChatMessage, Run};

use super::chat::*;
use super::*;
use crate::panes::conversations::{Decoder, IMAGES_BYTES, MAX_PIXELS, Shown, TeamsPictures};
/// A PNG's header saying `w` × `h`, and nothing else.
fn png_header(w: u32, h: u32) -> Vec<u8> {
    fn crc(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    let mut ihdr = b"IHDR".to_vec();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(13u32.to_be_bytes());
    png.extend(&ihdr);
    png.extend(crc(&ihdr).to_be_bytes());
    png
}

/// A small real PNG.
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([40, 120, 200, 255]));
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// Frames until the pictures are decoded (at most five seconds).
fn decoded(ctx: &egui::Context, a: &mut MailApp) {
    let started = std::time::Instant::now();
    loop {
        frame(ctx, a, vec![]);
        if !a.ui.teams_queue.pictures.busy() || started.elapsed() > std::time::Duration::from_secs(5) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    frame(ctx, a, vec![]);
}

/// A picture past 40 megapixels (its header says 10,000 × 5,000) is a placeholder: no texture,
/// and the decoder goes on with the next.
#[test]
fn a_picture_bigger_than_40_megapixels_is_a_placeholder() {
    assert!(u64::from(10_000u32 * 5_000) > MAX_PIXELS);
    let m = said(
        "m1",
        user("u-ana", "Ana Lima"),
        NOW,
        para(vec![
            words("Two"),
            Run { hosted: Some("big".into()), ..Default::default() },
            Run { hosted: Some("small".into()), ..Default::default() },
        ]),
    );
    let (mut a, dir, jobs) = chat_app("teams-chat-big", group_row(), TEAMS_LINK, vec![m]);
    let ctx = egui::Context::default();
    settled_at(&ctx, &mut a, (1280, 800));
    assert_eq!(picture_jobs(&jobs, |p| matches!(p, Picture::Hosted { .. })), 2, "both asked for");
    let hosted = |id: &str| Picture::Hosted { chat: CHAT.into(), message: "m1".into(), hosted: id.into() };
    let started = std::time::Instant::now();
    a.session.teams_apply(teams::Event::Picture { picture: hosted("big"), bytes: png_header(10_000, 5_000) });
    decoded(&ctx, &mut a);
    assert!(started.elapsed() < std::time::Duration::from_secs(2), "refused by its header, not decoded: {:?}", started.elapsed());
    let pics = &a.ui.teams_queue.pictures;
    assert!(matches!(pics.image(&hosted("big")), Some(Shown::Failed)), "a placeholder, no texture");
    assert!(pics.decoder.alive(), "the decoder goes on");
    let texts = settled_at(&ctx, &mut a, (1280, 800));
    assert!(has_text(&texts, "This picture can't be shown."), "{texts:?}");
    a.session.teams_apply(teams::Event::Picture { picture: hosted("small"), bytes: png(40, 30) });
    decoded(&ctx, &mut a);
    assert!(matches!(a.ui.teams_queue.pictures.image(&hosted("small")), Some(Shown::Ready(_))), "the next one is shown");
    let _ = std::fs::remove_dir_all(dir);
}

/// A picture that makes the decoder panic is a placeholder, and the thread decodes the next.
#[test]
fn a_picture_that_panics_the_decoder_is_a_placeholder() {
    fn boom(data: &[u8], side: u32) -> Option<egui::ColorImage> {
        assert!(data.first() != Some(&0), "a hostile picture");
        crate::panes::conversations::pixels(data, side)
    }
    let (mut a, dir, _, _) = teams_app("teams-chat-panic", true);
    let ctx = egui::Context::default();
    let mut pics = TeamsPictures::default();
    pics.decoder = Decoder::new(boom);
    let photo = |u: &str| Picture::Photo { user: u.into() };
    for (u, bytes) in [("u-bad", vec![0u8, 1, 2]), ("u-good", png(48, 48))] {
        a.run("teams.picture", json!({"kind": "photo", "user": u}));
        a.session.teams_apply(teams::Event::Picture { picture: photo(u), bytes });
    }
    pics.take(&mut a.session);
    let started = std::time::Instant::now();
    while pics.busy() && started.elapsed() < std::time::Duration::from_secs(5) {
        pics.made(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    pics.made(&ctx);
    assert!(pics.photo("u-bad").is_none(), "no photo: the initials stay");
    assert!(pics.photo("u-good").is_some(), "the thread went on");
    assert!(pics.decoder.alive());
    let _ = std::fs::remove_dir_all(dir);
}

/// A person's photo, once it came, is drawn on their row (until then their initials).
#[test]
fn a_photo_is_drawn_on_its_row() {
    let rows = vec![one_to_one("19:p@thread.v2", "Paul Durand", "u-paul", NOW)];
    let (mut a, dir, _) = module_app("teams-list-photo", rows);
    let ctx = egui::Context::default();
    let textured = |shapes: &[egui::Shape], id: egui::TextureId| {
        shapes.iter().any(|s| match s {
            egui::Shape::Mesh(m) => m.texture_id == id,
            egui::Shape::Rect(r) => r.fill_texture_id() == id,
            _ => false,
        })
    };
    let size = egui::vec2(1280.0, 800.0);
    for _ in 0..4 {
        frame_sized(&ctx, &mut a, vec![], size);
    }
    let before = frame_shapes(&ctx, &mut a, vec![], size);
    assert!(has_text(&texts_of(&before), "PD"), "the initials first");
    a.session.teams_apply(teams::Event::Picture { picture: Picture::Photo { user: "u-paul".into() }, bytes: png(48, 48) });
    decoded(&ctx, &mut a);
    let id = a.ui.teams_queue.pictures.photo("u-paul").map(egui::TextureHandle::id).expect("decoded");
    let after = frame_shapes(&ctx, &mut a, vec![], size);
    assert!(textured(&after, id), "the photo drawn");
    assert!(!has_text(&texts_of(&after), "PD"), "instead of the initials");
    let _ = std::fs::remove_dir_all(dir);
}

/// A picture kept with a message, once it came, is a thumbnail that opens big on a click; Close
/// puts it away.
#[test]
fn a_thumbnail_opens_the_picture_big() {
    let m = said("m1", user("u-ana", "Ana Lima"), NOW, para(vec![words("A picture"), Run { hosted: Some("h1".into()), ..Default::default() }]));
    let (mut a, dir, _) = chat_app("teams-chat-viewer", group_row(), TEAMS_LINK, vec![m]);
    let ctx = egui::Context::default();
    settled_at(&ctx, &mut a, (1280, 800));
    let picture = Picture::Hosted { chat: CHAT.into(), message: "m1".into(), hosted: "h1".into() };
    a.session.teams_apply(teams::Event::Picture { picture: picture.clone(), bytes: png(120, 80) });
    decoded(&ctx, &mut a);
    let Some(Shown::Ready(tex)) = a.ui.teams_queue.pictures.image(&picture) else { panic!("decoded") };
    let id = tex.id();
    let size = egui::vec2(1280.0, 800.0);
    let at = frame_shapes(&ctx, &mut a, vec![], size).iter().find_map(|s| match s {
        egui::Shape::Mesh(m) if m.texture_id == id => Some(m.calc_bounds()),
        egui::Shape::Rect(r) if r.fill_texture_id() == id => Some(r.rect),
        _ => None,
    });
    let at = at.expect("the thumbnail drawn");
    assert!(at.width() <= 240.0 && at.height() <= 180.0, "{at:?}");
    click_at(&ctx, &mut a, at.center(), 1, size);
    assert_eq!(a.ui.conversations.viewing.as_ref(), Some(&picture), "opened big");
    let texts = texts_of(&frame_shapes(&ctx, &mut a, vec![], size));
    let close = rect_of(&texts, |t| t == "Close").expect("the viewer's Close");
    click_at(&ctx, &mut a, close.center(), 1, size);
    assert!(a.ui.conversations.viewing.is_none(), "put away");
    let _ = std::fs::remove_dir_all(dir);
}

/// The pictures kept with messages are asked for as those messages come on screen: opening a
/// chat of many asks for the newest ones only, each once.
#[test]
fn pictures_are_asked_for_as_their_messages_come_on_screen() {
    let messages: Vec<ChatMessage> = (0..80)
        .map(|i| {
            said(
                &format!("m{i:02}"),
                user("u-ana", "Ana Lima"),
                NOW - 7200 + i * 30,
                para(vec![words("A picture"), Run { hosted: Some(format!("h{i}")), ..Default::default() }]),
            )
        })
        .collect();
    let (mut a, dir, jobs) = chat_app("teams-chat-on-screen", group_row(), TEAMS_LINK, messages);
    let ctx = egui::Context::default();
    settled_at(&ctx, &mut a, (1280, 800));
    settled_at(&ctx, &mut a, (1280, 800));
    let asked = |jobs: &Shared<Job>, id: &str| picture_jobs(jobs, |p| matches!(p, Picture::Hosted { hosted, .. } if hosted == id));
    let all = picture_jobs(&jobs, |p| matches!(p, Picture::Hosted { .. }));
    assert!(all > 0 && all < 20, "the ones on screen: {all}");
    assert_eq!(asked(&jobs, "h79"), 1, "the newest, once");
    assert_eq!(asked(&jobs, "h0"), 0, "not the oldest, scrolled away");
    let _ = std::fs::remove_dir_all(dir);
}

/// A stranger's message of 20,000 pictures (the parser's most runs) costs a frame what one of 20
/// does: at most 20 asked for and 20 placeholders drawn, the rest left out.
#[test]
fn a_message_of_20000_pictures_is_drawn_quickly() {
    let mut runs: Vec<Run> = (0..20_000).map(|i| Run { hosted: Some(format!("h{i}")), ..Default::default() }).collect();
    runs.extend((0..2_000).map(|i| Run { outside: Some(format!("https://images.example/{i}.gif")), ..Default::default() }));
    let m = said("m1", user("u-ana", "Ana Lima"), NOW, para(runs));
    let (mut a, dir, jobs) = chat_app("teams-pictures-many", group_row(), TEAMS_LINK, vec![m]);
    let ctx = egui::Context::default();
    settled_at(&ctx, &mut a, TALL);
    let started = std::time::Instant::now();
    let texts = settled_at(&ctx, &mut a, TALL);
    let took = started.elapsed();
    assert!(took < std::time::Duration::from_millis(1500), "seven frames took {took:?}");
    assert_eq!(picture_jobs(&jobs, |p| matches!(p, Picture::Hosted { .. })), teams::HOSTED_PER_MESSAGE, "20 asked for");
    let placeholders = texts.iter().filter(|(t, _)| t == "Picture").count();
    assert!(placeholders <= teams::HOSTED_PER_MESSAGE, "{placeholders} placeholders");
    assert!(texts.iter().filter(|(t, _)| t == "Show picture").count() <= 20);
    let _ = std::fs::remove_dir_all(dir);
}

/// How long the window may wait before its next frame, as the last one asked.
fn next_frame_in(ctx: &egui::Context, a: &mut MailApp) -> std::time::Duration {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    out.viewport_output.get(&egui::ViewportId::ROOT).map_or(std::time::Duration::MAX, |v| v.repaint_delay)
}

/// A picture not there yet is a placeholder that stays still: it doesn't keep the window
/// drawing.
#[test]
fn a_placeholder_on_screen_asks_no_repaint() {
    let m = said("m1", user("u-ana", "Ana Lima"), NOW, para(vec![words("A picture"), Run { hosted: Some("h1".into()), ..Default::default() }]));
    let (mut a, dir, _) = chat_app("teams-pictures-still", group_row(), TEAMS_LINK, vec![m]);
    let ctx = egui::Context::default();
    let texts = settled_at(&ctx, &mut a, (1280, 800));
    assert!(has_text(&texts, "Picture"), "{texts:?}");
    for _ in 0..3 {
        next_frame_in(&ctx, &mut a);
    }
    let wait = next_frame_in(&ctx, &mut a);
    assert!(wait >= std::time::Duration::from_secs(1), "the window keeps drawing: next frame in {wait:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The bytes of the conversations' picture textures.
fn texture_bytes(ctx: &egui::Context) -> usize {
    let tm = ctx.tex_manager();
    let tm = tm.read();
    tm.allocated().filter(|(_, m)| m.name == "teams-picture").map(|(_, m)| m.bytes_used()).sum()
}

/// More pictures than the cache holds: it stays under its bytes, and one let go while it was off
/// screen is asked for again when it's back on screen.
#[test]
fn a_picture_let_go_is_asked_again_and_the_cache_stays_bounded() {
    let m = said("m0", user("u-ana", "Ana Lima"), NOW, para(vec![words("The first"), Run { hosted: Some("h0".into()), ..Default::default() }]));
    let (mut a, dir, jobs) = chat_app("teams-pictures-evicted", group_row(), TEAMS_LINK, vec![m]);
    let ctx = egui::Context::default();
    settled_at(&ctx, &mut a, (1280, 800));
    let hosted = |message: &str, id: &str| Picture::Hosted { chat: CHAT.into(), message: message.into(), hosted: id.into() };
    let first = hosted("m0", "h0");
    let asked = |jobs: &Shared<Job>| picture_jobs(jobs, |p| *p == hosted("m0", "h0"));
    assert_eq!(asked(&jobs), 1);
    a.session.teams_apply(teams::Event::Picture { picture: first.clone(), bytes: png(600, 600) });
    decoded(&ctx, &mut a);
    assert!(matches!(a.ui.teams_queue.pictures.image(&first), Some(Shown::Ready(_))));
    // Off screen (in Mail), 80 others come.
    a.session.module = quatrelle_engine::Module::Mail;
    let big = png(600, 600);
    for i in 1..=80 {
        let p = hosted(&format!("x{i}"), "h");
        a.run("teams.picture", json!({"kind": "hosted", "chat": CHAT, "message": format!("x{i}"), "hosted": "h"}));
        a.session.teams_apply(teams::Event::Picture { picture: p, bytes: big.clone() });
        frame(&ctx, &mut a, vec![]);
    }
    decoded(&ctx, &mut a);
    let held = texture_bytes(&ctx);
    assert!(held <= IMAGES_BYTES, "the cache holds {} MB", held >> 20);
    assert!(a.ui.teams_queue.pictures.image(&first).is_none(), "the first was let go");
    // Back on screen: asked for again.
    a.session.module = quatrelle_engine::Module::Conversations;
    settled_at(&ctx, &mut a, (1280, 800));
    assert_eq!(asked(&jobs), 2, "asked for again");
    let _ = std::fs::remove_dir_all(dir);
}

/// A picture is kept at a thumbnail's size; it is decoded whole only once it's opened big, and
/// let go when the viewer closes.
#[test]
fn a_picture_is_decoded_whole_only_in_the_viewer() {
    let m = said("m1", user("u-ana", "Ana Lima"), NOW, para(vec![words("A picture"), Run { hosted: Some("h1".into()), ..Default::default() }]));
    let (mut a, dir, _) = chat_app("teams-pictures-whole", group_row(), TEAMS_LINK, vec![m]);
    let ctx = egui::Context::default();
    settled_at(&ctx, &mut a, (1280, 800));
    let picture = Picture::Hosted { chat: CHAT.into(), message: "m1".into(), hosted: "h1".into() };
    a.session.teams_apply(teams::Event::Picture { picture: picture.clone(), bytes: png(1200, 900) });
    decoded(&ctx, &mut a);
    let Some(Shown::Ready(tex)) = a.ui.teams_queue.pictures.image(&picture) else { panic!("decoded") };
    let thumb = tex.size();
    assert!(thumb[0] <= 480 && thumb[1] <= 480, "a thumbnail's size: {thumb:?}");
    a.ui.conversations.viewing = Some(picture.clone());
    // The viewer says what it shows, then the next frame asks for it whole.
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    decoded(&ctx, &mut a);
    settled_at(&ctx, &mut a, (1280, 800));
    let sizes: Vec<[usize; 2]> = ctx.tex_manager().read().allocated().filter(|(_, m)| m.name == "teams-picture").map(|(_, m)| m.size).collect();
    assert!(sizes.contains(&[1200, 900]), "whole in the viewer: {sizes:?}");
    a.ui.conversations.viewing = None;
    settled_at(&ctx, &mut a, (1280, 800));
    let sizes: Vec<[usize; 2]> = ctx.tex_manager().read().allocated().filter(|(_, m)| m.name == "teams-picture").map(|(_, m)| m.size).collect();
    assert!(!sizes.contains(&[1200, 900]), "let go once closed: {sizes:?}");
    let _ = std::fs::remove_dir_all(dir);
}
