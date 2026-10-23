//! Colour emoji, drawn from the system's emoji font.
//!
//! egui draws text from outlines in the text's colour, so an emoji from its own fonts is a line
//! drawing: black on a white page. The system has a colour emoji font (Noto Color Emoji on
//! Linux, Segoe UI Emoji on Windows, Apple Color Emoji on macOS), read once through
//! [`crate::Services::emoji_font`]. This hands egui a glyph rasterizer, asked about every
//! grapheme cluster before any font, which answers only for the clusters Unicode says are emoji
//! (UTS #51 emoji presentation: 😀, 🇫🇷, 👨‍👩‍👧, 1️⃣, and ☺️ with its selector; not ☺, ✔ or ↔,
//! which are text, nor anything with the text selector U+FE0E). It shapes the cluster with the
//! font (`harfrust`, the shaper egui's text uses: a flag, a skin tone, a family or a keycap is a
//! ligature in the font's own tables), then draws the glyphs from the font's bitmaps (`CBDT`,
//! `sbix`: a PNG per glyph, scaled down with a proper filter) or its layered outlines (`COLR`,
//! with `vello_cpu`, the rasterizer egui's text uses). egui keeps the result in its glyph atlas,
//! once per emoji and size, so wherever it draws text — the message list, the reading pane, the
//! editor, a calendar chip — the emoji is the picture, and it costs nothing more per frame. A
//! cluster the font can't draw (an emoji newer than it) is left to the fonts, as before; with
//! no colour font (the browser, a bare system) nothing is installed and nothing changes.

use std::sync::Arc;

use egui::epaint::text::{FontPriority, GlyphBitmap, GlyphRasterizer, GlyphRasterizerRequest, RasterizedGlyph, has_emoji_presentation};
use egui::{Color32, ColorImage, vec2};
use skrifa::instance::{LocationRef, Size};
use skrifa::raw::TableProvider as _;
use skrifa::{GlyphId, MetadataProvider as _};

/// The longest cluster drawn: a family of four with skin tones is 11 characters (43 bytes); a
/// run of joiners past this is hostile, or at least no emoji a font knows.
const MAX_CLUSTER_BYTES: usize = 64;
/// At most this many glyphs side by side, when the font hasn't the sequence as one.
const MAX_GLYPHS: usize = 4;
/// The largest emoji drawn, in pixels (a heading on a high-resolution screen is about 50).
const MAX_PX: f32 = 256.0;
/// The widest picture, in em (a sequence the font hasn't joined is drawn glyph by glyph).
const MAX_EM: f32 = 4.0;
/// The largest bitmap taken from a font (Noto's are 136 by 128).
const MAX_STRIKE: u32 = 1024;

/// Whether `cluster` (one grapheme cluster) is an emoji to draw from the colour font.
pub(crate) fn wants(cluster: &str) -> bool {
    !cluster.is_empty() && cluster.len() <= MAX_CLUSTER_BYTES && has_emoji_presentation(cluster)
}

/// A character that shapes to nothing: a joiner, a presentation selector, a tag (a flag's
/// letters). The font may map it to no glyph and still draw the sequence.
fn ignorable(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}' | '\u{FE0E}' | '\u{FE0F}' | '\u{E0020}'..='\u{E007F}')
}

/// The system's colour emoji font, parsed once.
pub struct Font {
    data: Arc<Vec<u8>>,
    index: u32,
    shaper: harfrust::ShaperData,
    units_per_em: f32,
    /// Has bitmaps (`CBDT`, `sbix`): drawn from their PNGs.
    bitmaps: bool,
    /// Has layered outlines (`COLR`): drawn with vello.
    outlines: bool,
}

/// One glyph of a shaped cluster: its id, and where its origin is, in pixels from the
/// cluster's origin (y down).
struct Placed {
    id: GlyphId,
    x: f32,
    y: f32,
}

/// A glyph's picture, placed: its pixels (premultiplied), and its top-left from the cluster's
/// origin, in pixels.
struct Part {
    image: image::RgbaImage,
    left: f32,
    top: f32,
}

impl Font {
    /// `None` unless `bytes` (face `index` of a collection) is a font with colour glyphs.
    pub fn parse(bytes: Vec<u8>, index: u32) -> Option<Font> {
        let face = skrifa::FontRef::from_index(&bytes, index).ok()?;
        let bitmaps = face.cbdt().is_ok() || face.sbix().is_ok();
        let outlines = face.colr().is_ok();
        if !bitmaps && !outlines {
            return None;
        }
        let units_per_em = f32::from(face.metrics(Size::unscaled(), LocationRef::default()).units_per_em);
        if !(16.0..=16384.0).contains(&units_per_em) {
            return None;
        }
        let shaper = harfrust::ShaperData::new(&face);
        Some(Font { data: Arc::new(bytes), index, shaper, units_per_em, bitmaps, outlines })
    }

    fn face(&self) -> Option<skrifa::FontRef<'_>> {
        skrifa::FontRef::from_index(&self.data, self.index).ok()
    }

    /// The cluster drawn `px` pixels to the em: its picture and its advance. `None` when the
    /// font hasn't a glyph for part of it (the fonts draw it then, as before).
    pub fn draw(&self, cluster: &str, px: f32) -> Option<RasterizedGlyph> {
        let face = self.face()?;
        let (glyphs, advance) = self.shape(&face, cluster, px)?;
        let bitmap = if self.bitmaps { self.draw_bitmaps(&face, &glyphs, px) } else { None };
        let bitmap = bitmap.or_else(|| if self.outlines { self.draw_outlines(&face, &glyphs, px) } else { None })?;
        Some(RasterizedGlyph { bitmap, advance_px: advance })
    }

    /// Shape `cluster`: its glyphs placed, and the advance in pixels.
    fn shape(&self, face: &skrifa::FontRef<'_>, cluster: &str, px: f32) -> Option<(Vec<Placed>, f32)> {
        let shaper = self.shaper.shaper(face).build();
        let mut buffer = harfrust::UnicodeBuffer::new();
        buffer.push_str(cluster);
        buffer.guess_segment_properties();
        let shaped = shaper.shape(buffer, harfrust::ShapeOptions::new());
        let scale = px / self.units_per_em;
        let mut pen = 0.0_f32;
        let mut glyphs: Vec<Placed> = Vec::new();
        for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
            let chr = cluster.get(info.cluster as usize..).and_then(|s| s.chars().next());
            if info.glyph_id == 0 {
                // No glyph for a character that draws: not this font's emoji.
                if !chr.is_some_and(ignorable) {
                    return None;
                }
                continue;
            }
            let advance = pos.x_advance as f32 * scale;
            // A joiner or selector the font keeps as a glyph of its own takes no room and
            // draws nothing.
            if advance <= 0.0 {
                continue;
            }
            glyphs.push(Placed { id: GlyphId::new(info.glyph_id), x: pen + pos.x_offset as f32 * scale, y: -(pos.y_offset as f32 * scale) });
            pen += advance;
            if glyphs.len() > MAX_GLYPHS || pen > MAX_EM * px {
                return None;
            }
        }
        (!glyphs.is_empty() && pen > 0.0).then_some((glyphs, pen))
    }

    /// The cluster from the font's bitmaps, one per glyph, each scaled to `px`.
    fn draw_bitmaps(&self, face: &skrifa::FontRef<'_>, glyphs: &[Placed], px: f32) -> Option<GlyphBitmap> {
        let strikes = face.bitmap_strikes();
        let unit = px / self.units_per_em;
        let mut parts: Vec<Part> = Vec::new();
        for g in glyphs {
            let Some(b) = strikes.glyph_for_size(Size::new(px), g.id) else { continue };
            if b.width == 0 || b.height == 0 || b.width > MAX_STRIKE || b.height > MAX_STRIKE || b.ppem_y.is_nan() || b.ppem_y < 1.0 {
                continue;
            }
            let strike = px / b.ppem_y;
            let Some(image) = decode(&b.data, b.width, b.height) else { continue };
            let w = (f32::from(u16::try_from(b.width).ok()?) * strike).round().max(1.0) as u32;
            let h = (f32::from(u16::try_from(b.height).ok()?) * strike).round().max(1.0) as u32;
            let image = image::imageops::resize(&image, w, h, image::imageops::FilterType::Triangle);
            // The picture's left, and how far its reference edge sits above the baseline.
            let left = g.x + b.bearing_x * unit + b.inner_bearing_x * strike;
            let up = b.bearing_y * unit + b.inner_bearing_y * strike;
            let top = match b.placement_origin {
                skrifa::bitmap::Origin::TopLeft => g.y - up,
                skrifa::bitmap::Origin::BottomLeft => g.y - up - h as f32,
            };
            parts.push(Part { image, left, top });
        }
        compose(&parts)
    }

    /// The cluster from the font's layered outlines, drawn with vello.
    fn draw_outlines(&self, face: &skrifa::FontRef<'_>, glyphs: &[Placed], px: f32) -> Option<GlyphBitmap> {
        use vello_cpu::{color, peniko};
        let coloured = face.color_glyphs();
        if !glyphs.iter().any(|g| coloured.get(g.id).is_some()) {
            return None;
        }
        // Room for one em on each side of the em box, and the whole run: cropped after.
        let right = glyphs.iter().map(|g| g.x).fold(0.0_f32, f32::max);
        let canvas_w = (3.0 * px + right).ceil().clamp(1.0, MAX_STRIKE as f32) as u16;
        let canvas_h = (3.0 * px).ceil().clamp(1.0, MAX_STRIKE as f32) as u16;
        let (origin_x, origin_y) = (px, 2.0 * px);
        let font = peniko::FontData::new(peniko::Blob::new(self.data.clone()), self.index);
        let mut ctx = vello_cpu::RenderContext::new(canvas_w, canvas_h);
        let mut resources = vello_cpu::Resources::new();
        ctx.set_paint(color::OpaqueColor::<color::Srgb>::WHITE);
        ctx.glyph_run(&mut resources, &font).font_size(px).hint(false).fill_glyphs(glyphs.iter().map(|g| vello_cpu::Glyph {
            id: g.id.to_u32(),
            x: origin_x + g.x,
            y: origin_y + g.y,
        }));
        let mut pixmap = vello_cpu::Pixmap::new(canvas_w, canvas_h);
        ctx.render(&mut pixmap, &mut resources);
        let (w, h) = (usize::from(canvas_w), usize::from(canvas_h));
        let pixels = pixmap.data_as_u8_slice();
        let at = |x: usize, y: usize| -> Option<[u8; 4]> {
            let i = 4 * (y * w + x);
            Some([*pixels.get(i)?, *pixels.get(i + 1)?, *pixels.get(i + 2)?, *pixels.get(i + 3)?])
        };
        // Cropped to what was painted.
        let (mut min, mut max) = ([w, h], [0, 0]);
        for y in 0..h {
            for x in 0..w {
                if at(x, y).is_some_and(|p| p[3] != 0) {
                    min = [min[0].min(x), min[1].min(y)];
                    max = [max[0].max(x + 1), max[1].max(y + 1)];
                }
            }
        }
        if max[0] <= min[0] || max[1] <= min[1] {
            return None;
        }
        let mut out: Vec<Color32> = Vec::with_capacity((max[0] - min[0]) * (max[1] - min[1]));
        for y in min[1]..max[1] {
            for x in min[0]..max[0] {
                // vello's pixmap is premultiplied RGBA.
                let p = at(x, y).unwrap_or([0; 4]);
                out.push(Color32::from_rgba_premultiplied(p[0], p[1], p[2], p[3]));
            }
        }
        Some(GlyphBitmap {
            image: ColorImage::new([max[0] - min[0], max[1] - min[1]], out),
            offset_px: vec2(min[0] as f32 - origin_x, min[1] as f32 - origin_y),
            is_color: true,
        })
    }
}

/// A font's bitmap as premultiplied pixels, `width` by `height` (from the font's metadata: the
/// PNG must agree). A mask (a one-colour bitmap) is no colour emoji.
fn decode(data: &skrifa::bitmap::BitmapData<'_>, width: u32, height: u32) -> Option<image::RgbaImage> {
    match data {
        skrifa::bitmap::BitmapData::Png(bytes) => {
            if bytes.len() > 4 << 20 {
                return None;
            }
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(MAX_STRIKE);
            limits.max_image_height = Some(MAX_STRIKE);
            limits.max_alloc = Some(16 << 20);
            let mut reader = image::ImageReader::new(std::io::Cursor::new(*bytes));
            reader.set_format(image::ImageFormat::Png);
            reader.limits(limits);
            let mut img = reader.decode().ok()?.to_rgba8();
            if img.width() != width || img.height() != height {
                return None;
            }
            for p in img.pixels_mut() {
                let a = u16::from(p.0[3]);
                for c in &mut p.0[..3] {
                    *c = ((u16::from(*c) * a + 127) / 255) as u8;
                }
            }
            Some(img)
        }
        skrifa::bitmap::BitmapData::Bgra(bytes) => {
            // Premultiplied already, BGRA.
            let n = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
            let bytes = bytes.get(..n)?;
            let rgba: Vec<u8> = bytes.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
            image::RgbaImage::from_raw(width, height, rgba)
        }
        skrifa::bitmap::BitmapData::Mask(_) => None,
    }
}

/// The parts laid over one another in one picture, placed by their top-left.
fn compose(parts: &[Part]) -> Option<GlyphBitmap> {
    let first = parts.first()?;
    let (mut left, mut top, mut right, mut bottom) = (first.left, first.top, first.left, first.top);
    for p in parts {
        left = left.min(p.left);
        top = top.min(p.top);
        right = right.max(p.left + p.image.width() as f32);
        bottom = bottom.max(p.top + p.image.height() as f32);
    }
    let (left, top) = (left.floor(), top.floor());
    let w = (right - left).ceil().max(1.0) as usize;
    let h = (bottom - top).ceil().max(1.0) as usize;
    if w > MAX_STRIKE as usize || h > MAX_STRIKE as usize {
        return None;
    }
    let mut out = vec![Color32::TRANSPARENT; w * h];
    for p in parts {
        let (dx, dy) = ((p.left - left).round() as usize, (p.top - top).round() as usize);
        for (x, y, px) in p.image.enumerate_pixels() {
            let Some(slot) = out.get_mut((y as usize + dy) * w + x as usize + dx) else { continue };
            let [r, g, b, a] = px.0;
            // Over, premultiplied: what's under shows through the picture's clear parts.
            let keep = u16::from(255 - a);
            let mix = |s: u8, d: u8| (u16::from(s) + (u16::from(d) * keep + 127) / 255).min(255) as u8;
            *slot = Color32::from_rgba_premultiplied(mix(r, slot.r()), mix(g, slot.g()), mix(b, slot.b()), mix(a, slot.a()));
        }
    }
    Some(GlyphBitmap { image: ColorImage::new([w, h], out), offset_px: vec2(left, top), is_color: true })
}

/// The rasterizer egui asks about every cluster, before its fonts (installed with
/// `Context::add_glyph_rasterizer`).
pub(crate) fn rasterizer(font: Font) -> GlyphRasterizer {
    let font = Arc::new(font);
    GlyphRasterizer::new("quatrelle::emoji", move |r: &GlyphRasterizerRequest<'_>| {
        if !wants(r.cluster) || !(1.0..=MAX_PX).contains(&r.font_size_px) {
            return None;
        }
        font.draw(r.cluster, r.font_size_px)
    })
    .with_priority(FontPriority::Highest)
}

/// Ask the system for its colour emoji font (once: the caller remembers) and hand egui the
/// rasterizer. `false` when there's none, or it can't be read: emoji stay as they were.
pub(crate) fn install(ctx: &egui::Context, services: &mut dyn crate::Services) -> bool {
    let Some((bytes, index)) = services.emoji_font() else {
        log::info!("no colour emoji font: emoji are drawn as line drawings");
        return false;
    };
    match Font::parse(bytes, index) {
        Some(font) => {
            ctx.add_glyph_rasterizer(rasterizer(font));
            true
        }
        None => {
            log::warn!("the colour emoji font can't be read: emoji are drawn as line drawings");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::FontFamily;

    /// Where the Linux and macOS packages put their colour emoji font (none on a build
    /// machine without one: those tests say so and pass).
    fn system_font() -> Option<Font> {
        let paths = [
            "/usr/share/fonts/google-noto-color-emoji-fonts/NotoColorEmoji.ttf",
            "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
            "/usr/share/fonts/noto-emoji/NotoColorEmoji.ttf",
            "/usr/share/fonts/noto/NotoColorEmoji.ttf",
            "/System/Library/Fonts/Apple Color Emoji.ttc",
        ];
        let bytes = std::env::var_os("QUATRELLE_EMOJI_FONT")
            .and_then(|p| std::fs::read(p).ok())
            .or_else(|| paths.iter().find_map(|p| std::fs::read(p).ok()))?;
        let font = Font::parse(bytes, 0);
        assert!(font.is_some(), "the system's emoji font parses");
        font
    }

    fn request(cluster: &str, px: f32) -> Option<RasterizedGlyph> {
        let font = system_font()?;
        let r = rasterizer(font);
        (r.rasterize)(&GlyphRasterizerRequest { cluster, family: &FontFamily::Proportional, font_size_px: px, subpixel_offset_px: 0.0 })
    }

    /// Unicode's emoji presentation decides: pictures, sequences and selectors are drawn;
    /// text symbols, digits and plain text are not; a hostile cluster is left alone.
    #[test]
    fn clusters_that_are_emoji_are_wanted() {
        for c in ["😀", "🦀", "🇫🇷", "👨\u{200D}👩\u{200D}👧", "👋🏽", "1\u{FE0F}\u{20E3}", "☺\u{FE0F}", "❤\u{FE0F}", "🛠\u{FE0F}", "🏳\u{FE0F}\u{200D}🌈"]
        {
            assert!(wants(c), "{c:?}");
        }
        for c in ["", "a", "1", "#", "☺", "✔", "↔", "©", "→", "😀\u{FE0E}", "é", "日本", "مرحبا"] {
            assert!(!wants(c), "{c:?}");
        }
        let chain = format!("👨{}", "\u{200D}👩".repeat(200));
        assert!(!wants(&chain), "a cluster of joiners without end is capped");
        assert!(!wants(&"a".repeat(1000)));
    }

    /// With the system's font: an emoji is a colour picture about an em wide, a flag and a
    /// family are one picture each (the font's ligatures), a keycap too; text isn't drawn.
    #[test]
    fn the_system_font_draws_emoji_as_pictures() {
        let Some(g) = request("😀", 28.0) else {
            eprintln!("no colour emoji font here: skipped");
            return;
        };
        let [w, h] = g.bitmap.image.size;
        assert!(g.bitmap.is_color);
        assert!((20..=44).contains(&w) && (20..=44).contains(&h), "{w}x{h}");
        assert!((24.0..=44.0).contains(&g.advance_px), "{}", g.advance_px);
        assert!(g.bitmap.image.pixels.iter().any(|p| p.a() > 200 && (p.r() != p.g() || p.g() != p.b())), "in colour");
        // Sits on the baseline: its top is above it, its bottom a little under.
        assert!(g.bitmap.offset_px.y < -10.0 && g.bitmap.offset_px.y + h as f32 > -4.0, "{:?}", g.bitmap.offset_px);
        for seq in ["🇫🇷", "👨\u{200D}👩\u{200D}👧", "1\u{FE0F}\u{20E3}", "👋🏽", "☺\u{FE0F}"] {
            let g = request(seq, 28.0).unwrap();
            assert!(g.advance_px <= 1.6 * 28.0, "{seq:?} is one picture, not several: {}", g.advance_px);
            assert!(g.bitmap.image.size[0] <= 48, "{seq:?}");
        }
        for text in ["a", "1", "☺", "😀\u{FE0E}", "日"] {
            assert!(request(text, 28.0).is_none(), "{text:?} is text");
        }
        assert!(request("😀", 2000.0).is_none(), "too big to draw");
        assert!(request("😀", 0.0).is_none());
    }

    /// With the rasterizer installed, egui's own text lays the emoji out as a colour glyph in
    /// its atlas (one glyph per character, the flag's second letter zero-width), the letters
    /// around it as before.
    #[test]
    fn egui_draws_them_in_colour_in_its_text() {
        let Some(font) = system_font() else {
            eprintln!("no colour emoji font here: skipped");
            return;
        };
        let ctx = egui::Context::default();
        ctx.add_glyph_rasterizer(rasterizer(font));
        let mut galley = None;
        let mut out = ctx.run_ui(Default::default(), |ui| {
            galley = Some(ui.fonts_mut(|f| f.layout_no_wrap("Hi 😀🇫🇷!".into(), egui::FontId::proportional(14.0), Color32::WHITE)));
        });
        let uploaded = !out.textures_delta.set.is_empty();
        out.textures_delta.clear();
        let galley = galley.unwrap();
        let glyphs: Vec<(char, bool, f32)> = galley.rows.iter().flat_map(|r| r.glyphs.iter().map(|g| (g.chr, g.is_color, g.advance_width))).collect();
        assert_eq!(glyphs.len(), "Hi 😀🇫🇷!".chars().count(), "{glyphs:?}");
        assert!(glyphs.iter().any(|(c, color, w)| *c == '😀' && *color && *w > 12.0), "{glyphs:?}");
        assert!(glyphs.iter().any(|(c, color, w)| *c == '🇫' && *color && *w > 12.0), "{glyphs:?}");
        assert!(glyphs.iter().any(|(c, _, w)| *c == '🇷' && *w == 0.0), "the flag's second letter: {glyphs:?}");
        assert!(glyphs.iter().all(|(c, color, _)| *color == matches!(c, '😀' | '🇫')), "{glyphs:?}");
        assert!(uploaded, "the atlas took the pictures");
    }

    /// No colour font, or a font without colour glyphs: nothing is installed, and an emoji is
    /// drawn as before, from egui's monochrome fonts.
    #[test]
    fn without_a_colour_font_emoji_are_drawn_as_before() {
        assert!(Font::parse(b"not a font".to_vec(), 0).is_none());
        let plain = egui::FontDefinitions::default().font_data.get("Ubuntu-Light").map(|d| d.font.as_ref().as_ref().to_vec()).unwrap();
        assert!(Font::parse(plain.clone(), 0).is_none(), "a text font has no colour glyphs");
        assert!(Font::parse(plain, 7).is_none(), "an index past the end");
        let ctx = egui::Context::default();
        assert!(!install(&ctx, &mut crate::NullServices(0)));
        let mut galley = None;
        let mut out = ctx.run_ui(Default::default(), |ui| {
            galley = Some(ui.fonts_mut(|f| f.layout_no_wrap("😀".into(), egui::FontId::proportional(14.0), Color32::WHITE)));
        });
        let uploaded = !out.textures_delta.set.is_empty();
        out.textures_delta.clear();
        let g = galley.unwrap();
        let glyph = g.rows[0].glyphs[0];
        assert!(!glyph.is_color && glyph.advance_width > 0.0 && !glyph.uv_rect.is_nothing(), "{glyph:?}");
        assert!(uploaded);
    }
}
