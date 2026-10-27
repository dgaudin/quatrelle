//! The pictures of a message: drawn where its HTML puts them, and copied or saved from their
//! menu.

use super::*;

/// Draw one picture of an HTML message; `false` when it can't be shown (yet). Right-clicked, it
/// is copied — whole, not as shrunk to fit — or, one sent with the message, saved.
#[allow(clippy::too_many_arguments)]
pub(super) fn picture(
    s: &Session,
    st: &mut UiState,
    ui: &mut Ui,
    key: &MessageKey,
    p: &mime::Parsed,
    src: &str,
    allowed: bool,
    width: f32,
    acts: &mut Vec<Action>,
) -> bool {
    let (tex, own) = match picture_texture(s, st, ui.ctx(), key, p, src, allowed) {
        Pic::Ready(tex, own) => (tex, own),
        Pic::Loading => {
            ui.spinner();
            return true;
        }
        Pic::Missing => return false,
    };
    // Tracking pixels and spacers: nothing to see.
    if own.x <= 3.0 && own.y <= 3.0 {
        return true;
    }
    let r = ui.add(egui::Image::new(&tex).fit_to_exact_size(shown_size(own, width)).sense(Sense::click()));
    r.context_menu(|ui| {
        if ui.button(tl!("Copy picture")).clicked() {
            if let Some(img) = picture_bytes(s, key, p, src).and_then(|b| color_image(&b)) {
                ui.ctx().copy_image(img);
            }
            ui.close();
        }
        if let Some(index) = sent_with(p, src)
            && ui.button(tl!("Save picture as…")).clicked()
        {
            acts.push(Action::SaveAttachment(index));
            ui.close();
        }
    });
    true
}

/// The attachment a picture sent with the message is (named by `cid:`, or by the name or
/// address it was sent under); `None` for one on the web.
fn sent_with(p: &mime::Parsed, src: &str) -> Option<usize> {
    let lower = src.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return None;
    }
    mime::picture_attachment(p, src)
}

/// A picture's own bytes: as sent with the message, or as downloaded from the web.
fn picture_bytes(s: &Session, key: &MessageKey, p: &mime::Parsed, src: &str) -> Option<Vec<u8>> {
    if let Some(index) = sent_with(p, src) {
        let raw = s.store.as_ref()?.read_body(key)?;
        return mime::attachment_data(&raw, index);
    }
    match s.pictures.get(src) {
        Some(quatrelle_engine::Picture::Ready(bytes)) => Some(bytes.to_vec()),
        _ => None,
    }
}

/// The widest a picture in a message is shown, in points.
const WIDEST: f32 = 900.0;

/// The size a picture shows at in a message `width` wide: its own (a pixel a point, as Outlook
/// shows one), brought down to the width.
fn shown_size(own: Vec2, width: f32) -> Vec2 {
    own * (width.min(WIDEST) / own.x.max(1.0)).min(1.0)
}

/// The most pixels on a side a picture is decoded at to be shown: no more than it can be shown
/// at on this screen.
fn shown_pixels(ctx: &egui::Context) -> u32 {
    (WIDEST * ctx.pixels_per_point()).clamp(WIDEST, 4096.0) as u32
}

/// A message's picture, as far as it can be had now.
pub(super) enum Pic {
    /// Its texture, and its own size (the texture can be smaller).
    Ready(egui::TextureHandle, Vec2),
    /// Being downloaded.
    Loading,
    /// Not in the message, not allowed yet, or not a picture.
    Missing,
}

/// A picture of an HTML message: one inside it (`cid:`) always, one on the web once allowed.
pub(super) fn picture_texture(
    s: &Session,
    st: &mut UiState,
    ctx: &egui::Context,
    key: &MessageKey,
    p: &mime::Parsed,
    src: &str,
    allowed: bool,
) -> Pic {
    // One sent with the message: by `cid:`, or by the name or address it was sent under.
    // At the size it's shown at, not as an attachment's thumbnail (a screenshot was drawn 480
    // points wide).
    let lower = src.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        let Some(index) = sent_with(p, src) else { return Pic::Missing };
        let name = format!("att-{}-{index}-shown", key.to_id());
        return shown_picture(st, &name, || {
            let raw = s.store.as_ref()?.read_body(key)?;
            let data = mime::attachment_data(&raw, index)?;
            decode_sized(ctx, &name, &data, shown_pixels(ctx))
        });
    }
    if !allowed {
        return Pic::Missing;
    }
    match s.pictures.get(src) {
        Some(quatrelle_engine::Picture::Ready(bytes)) => {
            let name = format!("web-{src}");
            shown_picture(st, &name, || decode_sized(ctx, &name, bytes, shown_pixels(ctx)))
        }
        Some(quatrelle_engine::Picture::Loading) => Pic::Loading,
        _ => Pic::Missing,
    }
}

/// A picture decoded once (`decode`) and kept by `name` while messages are read.
fn shown_picture(st: &mut UiState, name: &str, decode: impl FnOnce() -> Option<(egui::TextureHandle, Vec2)>) -> Pic {
    if let Some(t) = st.shown_pictures.get(name) {
        return t.clone().map_or(Pic::Missing, |(tex, own)| Pic::Ready(tex, own));
    }
    // Each can take a few megabytes of the graphics card's memory.
    if st.shown_pictures.len() > 48 {
        st.shown_pictures.clear();
    }
    let t = decode();
    st.shown_pictures.insert(name.to_string(), t.clone());
    t.map_or(Pic::Missing, |(tex, own)| Pic::Ready(tex, own))
}

/// A picture covering `rect`, cropped to keep its shape (CSS's `background-size: cover`).
pub(super) fn cover(tex: &egui::TextureHandle, rect: egui::Rect) -> egui::Shape {
    let sz = tex.size_vec2();
    if sz.x < 1.0 || sz.y < 1.0 || rect.width() < 1.0 || rect.height() < 1.0 {
        return egui::Shape::Noop;
    }
    let scale = (rect.width() / sz.x).max(rect.height() / sz.y);
    let part = vec2((rect.width() / (sz.x * scale)).min(1.0), (rect.height() / (sz.y * scale)).min(1.0));
    let uv = egui::Rect::from_center_size(egui::pos2(0.5, 0.5), part);
    egui::Shape::image(tex.id(), rect, uv, Color32::WHITE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_shows_at_its_own_size_up_to_the_width() {
        // A screenshot wider than the reading pane fills it, its shape kept (it was drawn as its
        // 480-pixel thumbnail); a small one keeps its size.
        assert_eq!(shown_size(vec2(1920.0, 1080.0), 640.0), vec2(640.0, 360.0));
        assert_eq!(shown_size(vec2(1920.0, 1080.0), 2000.0), vec2(900.0, 506.25));
        assert_eq!(shown_size(vec2(300.0, 200.0), 640.0), vec2(300.0, 200.0));
        // Decoded smaller than it is, it keeps its own size.
        let ctx = egui::Context::default();
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(2000, 1000))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let (tex, own) = decode_sized(&ctx, "big", &png, shown_pixels(&ctx)).unwrap();
        assert_eq!(own, vec2(2000.0, 1000.0));
        assert_eq!(tex.size(), [900, 450]);
    }
}
