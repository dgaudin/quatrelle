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
    let tex = match picture_texture(s, st, ui.ctx(), key, p, src, allowed) {
        Pic::Ready(tex) => tex,
        Pic::Loading => {
            ui.spinner();
            return true;
        }
        Pic::Missing => return false,
    };
    let sz = tex.size_vec2();
    // Tracking pixels and spacers: nothing to see.
    if sz.x <= 3.0 && sz.y <= 3.0 {
        return true;
    }
    let scale = (width.min(900.0) / sz.x).min(1.0);
    let r = ui.add(egui::Image::new(&tex).fit_to_exact_size(sz * scale).sense(Sense::click()));
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

/// A message's picture, as far as it can be had now.
pub(super) enum Pic {
    Ready(egui::TextureHandle),
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
    let lower = src.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return match sent_with(p, src) {
            Some(index) => preview(s, st, ctx, key, index).map_or(Pic::Missing, Pic::Ready),
            None => Pic::Missing,
        };
    }
    if !allowed {
        return Pic::Missing;
    }
    match s.pictures.get(src) {
        Some(quatrelle_engine::Picture::Ready(bytes)) => {
            if let Some(t) = st.web_pictures.get(src) {
                return t.clone().map_or(Pic::Missing, Pic::Ready);
            }
            if st.web_pictures.len() > 200 {
                st.web_pictures.clear();
            }
            let t = decode(ctx, &format!("web-{src}"), bytes, 1200);
            st.web_pictures.insert(src.to_string(), t.clone());
            t.map_or(Pic::Missing, Pic::Ready)
        }
        Some(quatrelle_engine::Picture::Loading) => Pic::Loading,
        _ => Pic::Missing,
    }
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
