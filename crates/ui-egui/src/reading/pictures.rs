// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The pictures of a message: drawn where its HTML puts them, and copied or saved from their
//! menu. Those sent with it are taken out of it and decoded with its others ([`Pictures`]).

use super::*;

mod cache;
mod decode;

pub(crate) use cache::{Got, Pictures, Want};
pub(super) use decode::decode_with;
pub(crate) use decode::{color_image, decode_bounded, decode_pixels, decode_round, decode_sized, png_of};

/// Draw one picture of an HTML message at the size the message sets (`size`), in `width`, `zoom`
/// times (wider than that when zoomed in); `false` when it can't be shown (yet), else with the
/// picture's response when it is drawn (a link around it is followed from that). Right-clicked, it
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
    size: Option<mime::ImageSize>,
    zoom: f32,
    acts: &mut Vec<Action>,
) -> (bool, Option<egui::Response>) {
    let (tex, own) = match picture_texture(s, st, ui.ctx(), key, p, src, allowed, zoom) {
        Pic::Ready(tex, own) => (tex, own),
        Pic::Loading => {
            ui.spinner();
            return (true, None);
        }
        Pic::Missing => return (false, None),
    };
    // Tracking pixels and spacers that set no size: nothing to see.
    let Some(at) = shown_size(own, size, width) else { return (true, None) };
    let r = ui.add(egui::Image::new(&tex).fit_to_exact_size(at * zoom).maintain_aspect_ratio(false).sense(Sense::click()));
    if r.secondary_clicked() {
        super::body::text_selection::take_right_click(ui.ctx());
    }
    r.context_menu(|ui| {
        if ui.button(tl!("Copy picture")).clicked() {
            if let Some(img) = picture_bytes(s, &st.pictures, key, p, src).and_then(|b| color_image(&b)) {
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
    (true, Some(r))
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

/// A picture's own bytes: as sent with the message (as taken out for it while that's kept), or as
/// downloaded from the web.
fn picture_bytes(s: &Session, pics: &Pictures, key: &MessageKey, p: &mime::Parsed, src: &str) -> Option<Vec<u8>> {
    if let Some(index) = sent_with(p, src) {
        if let Some(data) = pics.source(key, index) {
            return Some(data.to_vec());
        }
        let raw = s.kept_or_held(key)?;
        return mime::attachment_data(&raw, index);
    }
    match s.pictures.get(src) {
        Some(quatrelle_engine::Picture::Ready(bytes)) => Some(bytes.to_vec()),
        _ => None,
    }
}

/// The widest a picture in a message is shown, in points.
const WIDEST: f32 = 900.0;

/// How big a picture of its own size `own` shows in room `width` wide: at the size the message
/// sets (`size`: both sides as set, as Outlook and browsers stretch it; one side, the other keeping
/// its shape), else its own (a pixel a point, as Outlook shows one, at most [`WIDEST`]); no wider
/// than `max-width` or the room, its shape kept. `None` for a tracking pixel or spacer that sets
/// no size: nothing to see. (Every picture showed at its own size: a logo made twice as sharp as it
/// says came out twice as big, and what was laid out around it moved: upstream 1721d72.)
pub(crate) fn shown_size(own: Vec2, size: Option<mime::ImageSize>, width: f32) -> Option<Vec2> {
    use mime::layout::Width;
    let set = size.filter(|z| z.width.is_some() || z.height.is_some());
    if set.is_none() && own.x <= 3.0 && own.y <= 3.0 {
        return None;
    }
    let shape = own.y.max(1.0) / own.x.max(1.0);
    // A percentage is of a box the message laid out, often narrower than this pane: a height set
    // beside it was meant for that width, so the shape is kept, no wider than a picture on its
    // own (a logo came out across the window, flat). A spacer has no shape to keep.
    let share = own.x > 3.0 && own.y > 3.0 && matches!(set.and_then(|z| z.width), Some(Width::Percent(_)));
    let w = set.and_then(|z| z.width).map(|w| match w {
        Width::Px(px) => px as f32,
        Width::Percent(pc) => width * f32::from(pc) / 100.0,
    });
    let h = set.and_then(|z| z.height).map(|h| h as f32);
    let (mut w, mut h) = match (w, h) {
        (Some(w), Some(_)) if share => (w, w * shape),
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w * shape),
        (None, Some(h)) => (h / shape, h),
        (None, None) => (own.x, own.y),
    };
    let room = if set.is_some() && !share { width } else { width.min(WIDEST) };
    let most = size.and_then(|z| z.max_width).map_or(f32::INFINITY, |m| m as f32).min(room);
    if w > most {
        h *= most / w;
        w = most;
    }
    (w >= 0.5 && h >= 0.5).then(|| egui::vec2(w, h.min(8000.0)))
}

/// The room a picture the message sizes keeps when it isn't shown: its width as set (no wider
/// than its `max-width` or `width`), and its height when that's set too; `None` when no width is
/// set, or one too thin to see (a spacer).
pub(crate) fn kept_room(size: Option<mime::ImageSize>, width: f32) -> Option<(f32, Option<f32>)> {
    use mime::layout::Width;
    let z = size?;
    let w = match z.width? {
        Width::Px(px) => px as f32,
        Width::Percent(pc) => width * f32::from(pc) / 100.0,
    };
    let w = w.min(z.max_width.map_or(f32::INFINITY, |m| m as f32)).min(width);
    (w >= 0.5).then(|| (w, z.height.map(|h| (h as f32).min(8000.0))))
}

/// A picture that can't be shown (on the web and not allowed, or not to be had), `zoom` times:
/// its description in brackets, in the link's colour in a link; or, when that is wider than the
/// picture would be ([`kept_room`]: a signature's icon 13 pixels wide), the picture's room — as
/// tall as it is wide, at most a line, unless the message sets its height — filled with what's
/// behind it when that shows (the box of colour around an icon), else outlined, its description
/// written in it when it holds its words ([`written_in`]: a product's photo), else on hover only.
/// A description wider than its place went down a narrow column a few letters a line, and pushed
/// the lines beside it out of line. A spacer ([`mime::spacer_described`]), or a picture whose
/// description only names its file ([`mime::names_itself`]) and isn't held by its place (across,
/// and down to a line of text when the message sets its height), keeps that room (none when the
/// message sets no width) and draws nothing in it, no bubble either: `None`. `[spacer-FFFFFF]` was
/// written across a white gap, a column a few pixels wide was framed, and a rule 6 high pushed the
/// layout down by a line; a file's name that fits stays written (`[4321]`, a phone number). A
/// tracking picture without a description (`never`: never loaded, whatever is shown) is left out
/// so too: a white spacer whose address a pixel 0 wide shares wrote `[picture]` over and under a
/// signature's frame and framed its gaps, pictures shown. A picture in a link is never left out
/// so: it stays the link, said and clicked as before.
pub(super) fn unseen(t: &Tokens, ui: &mut Ui, sp: &mime::Span, width: f32, page: [u8; 3], zoom: f32, never: bool) -> Option<egui::Response> {
    let alt = if sp.text.trim().is_empty() { "picture" } else { sp.text.trim() };
    let text = format!("[{alt}]");
    let zoom = zoom.max(0.01);
    let line = ui.text_style_height(&egui::TextStyle::Body);
    let kept = kept_room(sp.image_size, width / zoom);
    let room = kept.map(|(w, h)| vec2(w * zoom, h.map_or((w * zoom).min(line), |h| h * zoom)));
    let font = egui::TextStyle::Body.resolve(ui.style());
    let wide = ui.fonts_mut(|f| f.layout_no_wrap(text.clone(), font, Color32::WHITE).size().x);
    // Its height counts when the message sets it: a line of text must fit.
    let low = kept.and_then(|(_, h)| h).is_some_and(|h| h * zoom < line);
    let crowded = room.is_some_and(|room| wide > room.x) || low;
    // Never in a link: a signature's icon (`linkedin` for `…/linkedin.png`) stays the link.
    let spacer = mime::spacer_described(&sp.text)
        || (never && sp.text.trim().is_empty())
        || (crowded && mime::names_itself(sp.image.as_deref().unwrap_or_default(), &sp.text));
    if spacer && sp.link.is_none() {
        if let Some(room) = room {
            ui.allocate_space(room);
        }
        return None;
    }
    Some(match room {
        Some(room) if wide > room.x => {
            let sense = if sp.link.is_some() { Sense::click() } else { Sense::hover() };
            let (rect, r) = ui.allocate_exact_size(room, sense);
            // Too thin to see (a spacer): its room alone.
            if rect.width() >= 3.0 && rect.height() >= 3.0 {
                match theme::box_color(sp.background, page, t) {
                    Some(c) => ui.painter().rect_filled(rect, 0.0, Color32::from_rgb(c[0], c[1], c[2])),
                    None => ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside),
                };
            }
            let ink = theme::mail_colors_on(None, sp.background, page, t, t.faint).0;
            if let Some(g) = written_in(ui, alt, rect, line, ink) {
                ui.painter().with_clip_rect(rect.shrink(1.0)).galley(rect.min + vec2(WRITTEN_PAD, WRITTEN_PAD / 2.0), g, ink);
            }
            // In a link, said as the link it opens.
            let role = if sp.link.is_some() { egui::Role::Link } else { egui::Role::Image };
            r.widget_info(|| egui::WidgetInfo::labeled(role, true, alt));
            r.on_hover_text(alt)
        }
        // Inside a link, its place is the link's.
        _ => match &sp.link {
            Some(_) => ui.add(egui::Link::new(RichText::new(text).color(t.accent))),
            None => ui.add(egui::Label::new(RichText::new(text).color(t.faint)).selectable(false)),
        },
    })
}

/// Room left between a box and the description written in it, in points.
const WRITTEN_PAD: f32 = 4.0;
/// Characters of a description written in a box, at most.
const WRITTEN_MAX: usize = 300;

/// A picture's description written in its box (`rect`) when it isn't shown, as Outlook writes it
/// in a picture it hasn't loaded, wrapped at its words, as many lines as the box holds: when the
/// box is a line high (`line`) or more and its longest words fit across (a product's photo of
/// 280 pixels); `None` for a smaller box (a signature's icon, a social network's of 24), whose
/// description is only on hover, never cut into letters.
fn written_in(ui: &Ui, alt: &str, rect: Rect, line: f32, ink: Color32) -> Option<std::sync::Arc<egui::Galley>> {
    let across = rect.width() - 2.0 * WRITTEN_PAD;
    if rect.height() < line || across < line {
        return None;
    }
    let alt: String = alt.chars().take(WRITTEN_MAX).collect();
    let font = egui::TextStyle::Body.resolve(ui.style());
    // The longest few by letters, measured.
    let mut words: Vec<&str> = alt.split_whitespace().collect();
    words.sort_by_key(|w| std::cmp::Reverse(w.chars().count()));
    let fits = words.iter().take(3).all(|w| ui.fonts_mut(|f| f.layout_no_wrap((*w).to_string(), font.clone(), ink).size().x) <= across);
    if !fits {
        return None;
    }
    let mut job = egui::text::LayoutJob::simple(alt, font, ink, across);
    job.wrap.max_rows = ((rect.height() - WRITTEN_PAD) / line).floor().max(1.0) as usize;
    Some(ui.fonts_mut(|f| f.layout_job(job)))
}

/// The most pixels on a side a picture is decoded at to be shown: no more than it can be shown
/// at on this screen, `zoom` times its size.
fn shown_pixels(ctx: &egui::Context, zoom: f32) -> u32 {
    (WIDEST * ctx.pixels_per_point() * zoom.max(1.0)).clamp(WIDEST, 4096.0) as u32
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

/// A picture of an HTML message: one inside it (`cid:`) always, one on the web once allowed;
/// decoded for `zoom`.
#[allow(clippy::too_many_arguments)]
pub(super) fn picture_texture(
    s: &Session,
    st: &mut UiState,
    ctx: &egui::Context,
    key: &MessageKey,
    p: &mime::Parsed,
    src: &str,
    allowed: bool,
    zoom: f32,
) -> Pic {
    let most = shown_pixels(ctx, zoom);
    // One sent with the message: by `cid:`, or by the name or address it was sent under.
    // At the size it's shown at, not as an attachment's thumbnail (a screenshot was drawn 480
    // points wide).
    let lower = src.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        // Its message's text alone is here: the picture comes with the whole of it, or not at
        // all once that failed; nothing is kept meanwhile (its index can differ there).
        if s.text_first(key) {
            let failed = s.reading.iter().chain(&s.windows).any(|r| &r.key == key && r.error.is_some());
            return if failed { Pic::Missing } else { Pic::Loading };
        }
        let Some(index) = sent_with(p, src) else { return Pic::Missing };
        // Taken out with the message's others, in one read, and decoded a step at a time (each
        // read the whole message again, all in the frame that first showed it: upstream 321bef2).
        return match st.pictures.get(key, index, Want::Body(most)) {
            Got::Ready(tex, own) => Pic::Ready(tex, own),
            Got::Pending => Pic::Loading,
            Got::Failed => Pic::Missing,
        };
    }
    if !allowed {
        return Pic::Missing;
    }
    match s.pictures.get(src) {
        Some(quatrelle_engine::Picture::Ready(bytes)) => {
            let name = format!("web-{most}-{src}");
            shown_picture(st, &name, || decode_sized(ctx, &name, bytes, most))
        }
        Some(quatrelle_engine::Picture::Loading) => Pic::Loading,
        _ => Pic::Missing,
    }
}

/// A picture carried with what's drawn (a signature's, by `cid:`), as a message's: at the size
/// its HTML sets (a signature's logo often does), else its own, up to `width`, `zoom` times.
pub(super) fn carried_picture(
    st: &mut UiState,
    ui: &mut Ui,
    pic: &mime::InlinePicture,
    width: f32,
    size: Option<mime::ImageSize>,
    zoom: f32,
) -> Option<egui::Response> {
    let ctx = ui.ctx().clone();
    let most = shown_pixels(&ctx, zoom);
    let name = format!("carried-{}-{}-{most}", pic.cid, pic.data.len());
    match shown_picture(st, &name, || decode_sized(&ctx, &name, &pic.data, most)) {
        Pic::Ready(tex, own) => {
            let at = shown_size(own, size, width)?;
            Some(ui.add(egui::Image::new(&tex).fit_to_exact_size(at * zoom).maintain_aspect_ratio(false).sense(Sense::click())))
        }
        Pic::Loading | Pic::Missing => {
            ui.label(RichText::new(format!("[{}]", pic.name)).color(st.tokens.faint));
            None
        }
    }
}

/// Most web and carried pictures kept decoded.
const SHOWN_PICTURES: usize = 200;

/// A picture decoded once (`decode`) and kept by `name` while messages are read. Those shown
/// least recently go (never one shown in this frame: a message with more than were kept decoded
/// all of them again every frame); past that many shown at once it's kept anyway, and let go
/// once no longer shown (from upstream 321bef2).
fn shown_picture(st: &mut UiState, name: &str, decode: impl FnOnce() -> Option<(egui::TextureHandle, Vec2)>) -> Pic {
    let now = st.pictures.frame();
    if let Some((t, used)) = st.shown_pictures.get_mut(name) {
        *used = now;
        return t.clone().map_or(Pic::Missing, |(tex, own)| Pic::Ready(tex, own));
    }
    let t = decode();
    while st.shown_pictures.len() >= SHOWN_PICTURES {
        let oldest = st.shown_pictures.iter().filter(|(_, (_, used))| *used < now).min_by_key(|(_, (_, used))| *used).map(|(k, _)| k.clone());
        let Some(oldest) = oldest else { break };
        st.shown_pictures.remove(&oldest);
    }
    st.shown_pictures.insert(name.to_string(), (t.clone(), now));
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
        assert_eq!(shown_size(vec2(1920.0, 1080.0), None, 640.0), Some(vec2(640.0, 360.0)));
        assert_eq!(shown_size(vec2(1920.0, 1080.0), None, 2000.0), Some(vec2(900.0, 506.25)));
        assert_eq!(shown_size(vec2(300.0, 200.0), None, 640.0), Some(vec2(300.0, 200.0)));
        // Decoded smaller than it is, it keeps its own size.
        let ctx = egui::Context::default();
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(2000, 1000))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let (tex, own) = decode_sized(&ctx, "big", &png, shown_pixels(&ctx, 1.0)).unwrap();
        assert_eq!(own, vec2(2000.0, 1000.0));
        assert_eq!(tex.size(), [900, 450]);
    }

    #[test]
    fn a_picture_not_shown_keeps_the_width_it_sets() {
        use mime::layout::Width;
        let size = |width, height, max_width| Some(mime::ImageSize { width, height, max_width });
        // A signature's icon: 13 wide, its height left to the line.
        assert_eq!(kept_room(size(Some(Width::Px(13)), None, None), 640.0), Some((13.0, None)));
        assert_eq!(kept_room(size(Some(Width::Px(24)), Some(24), None), 640.0), Some((24.0, Some(24.0))));
        // No wider than its max-width or the room; a share of the room.
        assert_eq!(kept_room(size(Some(Width::Px(900)), Some(90), Some(600)), 640.0), Some((600.0, Some(90.0))));
        assert_eq!(kept_room(size(Some(Width::Px(900)), None, None), 300.0), Some((300.0, None)));
        assert_eq!(kept_room(size(Some(Width::Percent(50)), None, None), 300.0), Some((150.0, None)));
        // No width set (or none to see): nothing kept, its description shows.
        assert_eq!(kept_room(size(None, Some(20), None), 640.0), None);
        assert_eq!(kept_room(size(Some(Width::Px(0)), Some(1), None), 640.0), None);
        assert_eq!(kept_room(None, 640.0), None);
        // A height out of reason is capped.
        assert_eq!(kept_room(size(Some(Width::Px(10)), Some(u32::MAX), None), 640.0), Some((10.0, Some(8000.0))));
    }
}
