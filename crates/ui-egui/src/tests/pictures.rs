//! UI tests: pictures in the message being written, at their size and resized (from upstream
//! b5f14ee and 865a712).

use super::*;
use egui::{Pos2, Rect, Shape, vec2};

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
