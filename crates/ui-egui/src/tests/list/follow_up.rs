// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a follow-up marked complete in the list and the reading pane (adapted from
//! upstream f1eb8a3). With `QUATRELLE_FRAME_PNG` set to a folder, the frames are also painted
//! there as pictures (a small painter of egui's triangles, for looking at them: no GPU here).

use std::collections::HashMap;

use egui::epaint::{ClippedPrimitive, ImageData, Primitive, TextureId};

use super::*;

/// The textures a frame's triangles are painted with, kept from frame to frame as a renderer
/// keeps them (the font's comes once, in the first frame).
#[derive(Default)]
pub(in crate::tests) struct Textures(HashMap<TextureId, ([usize; 2], Vec<egui::Color32>)>);

impl Textures {
    fn take(&mut self, delta: &egui::TexturesDelta) {
        for (id, sets) in &delta.set {
            for d in sets {
                let ImageData::Color(img) = &d.image;
                match d.pos {
                    None => {
                        self.0.insert(*id, (img.size, img.pixels.clone()));
                    }
                    Some([x0, y0]) => {
                        let Some((size, px)) = self.0.get_mut(id) else { continue };
                        for y in 0..img.size[1] {
                            for x in 0..img.size[0] {
                                if let (Some(dst), Some(src)) = (px.get_mut((y0 + y) * size[0] + x0 + x), img.pixels.get(y * img.size[0] + x)) {
                                    *dst = *src;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn sample(&self, id: TextureId, uv: egui::Pos2) -> [f32; 4] {
        let Some((size, px)) = self.0.get(&id) else { return [1.0; 4] };
        let x = ((uv.x * size[0] as f32) as usize).min(size[0].saturating_sub(1));
        let y = ((uv.y * size[1] as f32) as usize).min(size[1].saturating_sub(1));
        px.get(y * size[0] + x).map_or([1.0; 4], |c| c.to_array().map(|v| f32::from(v) / 255.0))
    }
}

/// A frame painted into a picture: each triangle filled, its texture taken at the nearest pixel,
/// blended as premultiplied colours are.
fn paint(prims: &[ClippedPrimitive], tex: &Textures, w: usize, h: usize) -> Vec<u8> {
    let mut buf = vec![[0.0f32; 4]; w * h];
    for p in prims {
        let Primitive::Mesh(mesh) = &p.primitive else { continue };
        let clip = p.clip_rect;
        for tri in mesh.indices.chunks_exact(3) {
            let v: Vec<&egui::epaint::Vertex> = tri.iter().filter_map(|&i| mesh.vertices.get(i as usize)).collect();
            let [a, b, c] = [v[0], v[1], v[2]];
            let area = (b.pos.x - a.pos.x) * (c.pos.y - a.pos.y) - (c.pos.x - a.pos.x) * (b.pos.y - a.pos.y);
            if area.abs() < 1e-6 {
                continue;
            }
            let x0 = a.pos.x.min(b.pos.x).min(c.pos.x).max(clip.min.x).max(0.0).floor() as usize;
            let x1 = (a.pos.x.max(b.pos.x).max(c.pos.x).min(clip.max.x).ceil().max(0.0) as usize).min(w);
            let y0 = a.pos.y.min(b.pos.y).min(c.pos.y).max(clip.min.y).max(0.0).floor() as usize;
            let y1 = (a.pos.y.max(b.pos.y).max(c.pos.y).min(clip.max.y).ceil().max(0.0) as usize).min(h);
            for y in y0..y1 {
                for x in x0..x1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let wa = ((b.pos.x - px) * (c.pos.y - py) - (c.pos.x - px) * (b.pos.y - py)) / area;
                    let wb = ((c.pos.x - px) * (a.pos.y - py) - (a.pos.x - px) * (c.pos.y - py)) / area;
                    let wc = 1.0 - wa - wb;
                    if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                        continue;
                    }
                    let uv = egui::pos2(a.uv.x * wa + b.uv.x * wb + c.uv.x * wc, a.uv.y * wa + b.uv.y * wb + c.uv.y * wc);
                    let t = tex.sample(mesh.texture_id, uv);
                    let col: [f32; 4] = std::array::from_fn(|i| {
                        let ch = |v: &egui::epaint::Vertex| f32::from(v.color.to_array()[i]) / 255.0;
                        (ch(a) * wa + ch(b) * wb + ch(c) * wc) * t[i]
                    });
                    if let Some(dst) = buf.get_mut(y * w + x) {
                        let keep = 1.0 - col[3];
                        *dst = std::array::from_fn(|i| col[i] + dst[i] * keep);
                    }
                }
            }
        }
    }
    buf.iter().flat_map(|p| [p[0], p[1], p[2]].map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8).into_iter().chain([255])).collect()
}

/// A frame, its textures kept; with `png`, painted into that picture.
pub(super) fn frame_kept(ctx: &egui::Context, a: &mut MailApp, tex: &mut Textures, png: Option<std::path::PathBuf>) -> Vec<String> {
    frame_with(ctx, a, tex, vec![], png).0
}

/// A frame with `events`, its textures kept; with `png`, painted into that picture. What it said
/// (its accessible labels and values), and the shapes it drew.
pub(in crate::tests) fn frame_with(
    ctx: &egui::Context,
    a: &mut MailApp,
    tex: &mut Textures,
    events: Vec<Event>,
    png: Option<std::path::PathBuf>,
) -> (Vec<String>, Vec<egui::Shape>) {
    frame_with_size(ctx, a, tex, events, png, (1280, 800))
}

/// [`frame_with`] in a window `(w, h)` pixels big (a long page drawn whole).
pub(in crate::tests) fn frame_with_size(
    ctx: &egui::Context,
    a: &mut MailApp,
    tex: &mut Textures,
    events: Vec<Event>,
    png: Option<std::path::PathBuf>,
    (w, h): (usize, usize),
) -> (Vec<String>, Vec<egui::Shape>) {
    let input =
        RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32, h as f32))), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    tex.take(&out.textures_delta);
    // Taken: freed ones are let go after painting.
    let freed: Vec<TextureId> = out.textures_delta.free.iter().copied().collect();
    out.textures_delta.clear();
    let mut shapes = Vec::new();
    for s in &out.shapes {
        flat(s.shape.clone(), &mut shapes);
    }
    if let Some(path) = png {
        let prims = ctx.tessellate(out.shapes, 1.0);
        let rgba = paint(&prims, tex, w, h);
        if let Some(img) = image::RgbaImage::from_raw(w as u32, h as u32, rgba) {
            let _ = img.save(path);
        }
    }
    for id in freed {
        tex.0.remove(&id);
    }
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    (nodes.iter().filter_map(|(_, n)| n.label().or(n.value()).map(str::to_string)).collect(), shapes)
}

/// A shape, its groups taken apart.
fn flat(s: egui::Shape, out: &mut Vec<egui::Shape>) {
    match s {
        egui::Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, out)),
        s => out.push(s),
    }
}

/// A follow-up marked complete shows as done: the list row says so (and draws its tick where the
/// flag was), the reading pane says on which day, the list arranged by the flag puts it under
/// Completed, and Clear Flag takes it away.
#[test]
fn a_completed_follow_up_shows_as_done() {
    let (mut a, dir) = app("follow-up-complete");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut tex = Textures::default();
    let pictures = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let picture = |name: &str| pictures.as_ref().map(|d| d.join(format!("{name}.png")));
    frame_kept(&ctx, &mut a, &mut tex, None);
    let m = (*a.session.list[1]).clone();
    a.run("list.open", json!({"key": m.key.to_id()}));
    a.run("mail.flag", json!({"due": "tomorrow"}));
    a.run("mail.flag", json!({"complete": true}));
    for (dark, name) in [(Some(false), "complete-light"), (Some(true), "complete-dark")] {
        a.ui.dark = dark;
        for _ in 0..3 {
            frame_kept(&ctx, &mut a, &mut tex, None);
        }
        frame_kept(&ctx, &mut a, &mut tex, picture(name));
    }
    let shown = frame_kept(&ctx, &mut a, &mut tex, None);
    let spoken = format!("{}, {}, ", m.from.display(), m.subject);
    let row = shown.iter().find(|l| l.starts_with(&spoken)).cloned().unwrap_or_default();
    assert!(row.contains("Completed") && !row.contains("Flagged"), "{row}");
    let today = quatrelle_engine::model::dates::day_label(quatrelle_engine::model::dates::today(&a.session.clock));
    assert!(shown.iter().any(|l| l.starts_with("Completed on ") && l.contains(&today)), "{shown:?}");
    // Arranged by the flag, under its own heading.
    a.run("view.arrange", json!({"by": "flag"}));
    assert!(a.session.rows().contains(&quatrelle_engine::Row::Header("Completed")));
    frame_kept(&ctx, &mut a, &mut tex, None);
    frame_kept(&ctx, &mut a, &mut tex, picture("complete-arranged"));
    a.run("view.arrange", json!({"by": "date"}));
    // In columns (the reading pane off): the tick in the flag column.
    a.run("view.readingPane", json!({"value": "off"}));
    frame_kept(&ctx, &mut a, &mut tex, None);
    frame_kept(&ctx, &mut a, &mut tex, picture("complete-columns"));
    a.run("view.readingPane", json!({"value": "right"}));
    // Cleared: neither.
    a.run("mail.flag", json!({"on": false}));
    let shown = frame_kept(&ctx, &mut a, &mut tex, None);
    assert!(!shown.iter().any(|l| l.contains("Completed")), "{shown:?}");
    let _ = std::fs::remove_dir_all(dir);
}
