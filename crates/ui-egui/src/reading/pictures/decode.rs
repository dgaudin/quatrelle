// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Pictures read, within limits (they are hostile input), into pixels or textures: whole, shrunk
//! to fit, or cut to a circle (a person's picture).

use super::*;

/// A picture cut to a circle: its middle square is kept (a picture that isn't square would
/// otherwise fill a square box stretched, its circle an oval), then everything outside the
/// circle goes, with a pixel of smoothing at its edge.
fn round_pixels(img: image::RgbaImage) -> ([usize; 2], Vec<u8>) {
    let (w, h) = (img.width(), img.height());
    let side = w.min(h);
    let square = if side == 0 || (w == side && h == side) {
        img
    } else {
        image::imageops::crop_imm(&img, (w - side) / 2, (h - side) / 2, side, side).to_image()
    };
    let (w, h) = (square.width() as f32, square.height() as f32);
    let mut ci = egui::ColorImage::from_rgba_unmultiplied([square.width() as usize, square.height() as usize], square.as_raw());
    let (cx, cy, r) = (w / 2.0, h / 2.0, w.min(h) / 2.0);
    for (i, px) in ci.pixels.iter_mut().enumerate() {
        let (x, y) = ((i % square.width() as usize) as f32 + 0.5, (i / square.width() as usize) as f32 + 0.5);
        let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
        if d > r {
            px[3] = 0;
        } else if d > r - 1.0 {
            px[3] = (f32::from(px[3]) * (r - d)) as u8;
        }
    }
    let bytes = ci.pixels.iter().flat_map(|p| p.to_array()).collect();
    ([square.width() as usize, square.height() as usize], bytes)
}

/// A picture cut to a circle: what a person's picture looks like in a round avatar.
pub(crate) fn decode_round(ctx: &egui::Context, name: &str, data: &[u8], max: u32) -> Option<egui::TextureHandle> {
    decode_with(ctx, name, data, max, true)
}

/// A picture read, within limits: anything bigger than this is hostile, not a picture.
fn decoded(data: &[u8]) -> Option<image::DynamicImage> {
    if data.len() > 30 << 20 {
        return None;
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12_000);
    limits.max_image_height = Some(12_000);
    limits.max_alloc = Some(256 << 20);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format().ok()?;
    reader.limits(limits);
    reader.decode().ok()
}

/// A picture's pixels, whole (to copy it).
pub(crate) fn color_image(data: &[u8]) -> Option<egui::ColorImage> {
    let img = decoded(data)?.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
}

/// Pixels pasted (a screenshot) as a PNG, to go in a message.
pub(crate) fn png_of(img: &egui::ColorImage) -> Option<Vec<u8>> {
    let [w, h] = img.size;
    if w == 0 || h == 0 || w > 12_000 || h > 12_000 {
        return None;
    }
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|p| p.to_srgba_unmultiplied()).collect();
    let rgba = image::RgbaImage::from_raw(u32::try_from(w).ok()?, u32::try_from(h).ok()?, bytes)?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(rgba).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
}

pub(in crate::reading) fn decode_with(ctx: &egui::Context, name: &str, data: &[u8], max: u32, round: bool) -> Option<egui::TextureHandle> {
    decode_sized_with(ctx, name, data, max, round).map(|(tex, _)| tex)
}

/// [`attachments::decode`], with the picture's own size (in pixels, the texture's being smaller
/// when it was shrunk).
pub(crate) fn decode_sized(ctx: &egui::Context, name: &str, data: &[u8], max: u32) -> Option<(egui::TextureHandle, Vec2)> {
    decode_sized_with(ctx, name, data, max, false)
}

fn decode_sized_with(ctx: &egui::Context, name: &str, data: &[u8], max: u32, round: bool) -> Option<(egui::TextureHandle, Vec2)> {
    let limit = ctx.input(|i| i.max_texture_side).try_into().unwrap_or(u32::MAX);
    let (ci, filter, own) = pixels_of(data, max, limit, round)?;
    Some((ctx.load_texture(name, ci, filter), own))
}

/// A picture's pixels, at most `max` pixels on a side and never more than `limit` (the graphics
/// card's largest), how to draw them, and the picture's own size; pure, so it can run off the
/// window's thread (from upstream 321bef2). Pictures are hostile input: see [`decoded`].
pub(crate) fn decode_pixels(data: &[u8], max: u32, limit: u32) -> Option<(egui::ColorImage, egui::TextureOptions, Vec2)> {
    pixels_of(data, max, limit, false)
}

fn pixels_of(data: &[u8], max: u32, limit: u32, round: bool) -> Option<(egui::ColorImage, egui::TextureOptions, Vec2)> {
    let decoded = decoded(data)?;
    let own = vec2(decoded.width() as f32, decoded.height() as f32);
    // Never bigger than the graphics card takes (a software or browser one may take 2048 only).
    let max = max.min(limit).max(1);
    // Shrink large pictures; never enlarge small ones (the painter scales them crisply).
    let img = if decoded.width() > max || decoded.height() > max { decoded.thumbnail(max, max) } else { decoded }.to_rgba8();
    let (size, pixels) = if round { round_pixels(img) } else { ([img.width() as usize, img.height() as usize], img.into_raw()) };
    let ci = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
    // Pixel art and tiny swatches stay crisp when scaled up.
    let filter = if size[0].max(size[1]) < 96 { egui::TextureOptions::NEAREST } else { egui::TextureOptions::LINEAR };
    Some((ci, filter, own))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture that isn't square is cut to its middle square before the circle: drawn in a
    /// square box, a wide one was stretched and its "circle" came out an oval.
    #[test]
    fn a_round_picture_keeps_its_middle_square() {
        let wide = image::RgbaImage::from_pixel(8, 4, image::Rgba([200, 100, 50, 255]));
        let (size, pixels) = round_pixels(wide);
        assert_eq!(size, [4, 4], "cut to the middle square, not stretched");
        assert_eq!(pixels.len(), 4 * 4 * 4, "one RGBA for each pixel of the square");
        let alpha = |i: usize| pixels[i * 4 + 3];
        assert_eq!((alpha(0), alpha(3)), (0, 0), "the corners fall outside the circle");
        assert!(alpha(5) > 0 || alpha(6) > 0, "the middle is kept");
        // A square picture is left as it is.
        let square = image::RgbaImage::from_pixel(16, 16, image::Rgba([1, 2, 3, 255]));
        assert_eq!(round_pixels(square).0, [16, 16]);
    }
}
