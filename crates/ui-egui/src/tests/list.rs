//! UI tests: the message list, and the reading pane beside it.

use super::*;

/// The lines drawn along the bottom of what they're clipped to (a row, the column headings),
/// whose part outside it doesn't show: (where, the clip, its y).
fn bottom_lines(out: &egui::FullOutput) -> Vec<(egui::Rect, f32)> {
    fn walk(clip: egui::Rect, s: &egui::Shape, found: &mut Vec<(egui::Rect, f32)>) {
        match s {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(clip, s, found)),
            egui::Shape::LineSegment { points: [a, b], stroke } => {
                let along = (a.y - b.y).abs() < 0.01 && (b.x - a.x).abs() > clip.width() * 0.8;
                if along && stroke.width <= 1.0 && (a.y - clip.max.y).abs() <= 1.5 {
                    found.push((clip, a.y));
                }
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    for c in &out.shapes {
        walk(c.clip_rect, &c.shape, &mut found);
    }
    found
}

/// The line between two messages (Outlook draws one) lies inside the row it's clipped to, in the
/// table and in the cards: drawn on the row's bottom edge, egui put it on the pixel below, and
/// it never showed.
#[test]
fn the_line_between_two_messages_is_inside_its_row() {
    for layout in ["table", "cards"] {
        let (mut a, dir) = app(&format!("row-lines-{layout}"));
        let ctx = egui::Context::default();
        a.run("view.listLayout", json!({"value": layout}));
        frame(&ctx, &mut a, vec![]);
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        let lines = bottom_lines(&out);
        let rows = a.session.list.len().min(5);
        assert!(lines.len() >= rows, "{layout}: {} lines for {rows} rows", lines.len());
        for (clip, y) in lines {
            assert!(y + 0.5 <= clip.max.y + 0.01 && y - 0.5 >= clip.min.y - 0.01, "{layout}: a line at {y} below {clip:?}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// What can be done with a message stays whole under its header: in a reading pane at the
/// bottom, too short for the whole header, Reply, Reply all, Forward, Print and Save were cut in
/// half by the header's scrolling.
#[test]
fn the_reading_panes_buttons_stay_whole_at_the_bottom() {
    let (mut a, dir) = app("bottom-buttons");
    let ctx = egui::Context::default();
    a.run("view.readingPane", json!({"value": "bottom"}));
    let k = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let size = egui::vec2(1280.0, 800.0);
    assert!(settled_text_at(&ctx, &mut a, "Reply all", size).is_some());
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    fn texts(clip: egui::Rect, s: &egui::Shape, found: &mut Vec<(egui::Rect, egui::Rect)>) {
        match s {
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(clip, s, found)),
            egui::Shape::Text(t) if t.galley.text() == "Reply all" => found.push((clip, t.galley.rect.translate(t.pos.to_vec2()))),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for c in &out.shapes {
        texts(c.clip_rect, &c.shape, &mut found);
    }
    // The reading pane's (the ribbon has one too): each whole inside what it's clipped to.
    assert!(found.len() >= 2, "{found:?}");
    for (clip, text) in found {
        assert!(clip.contains_rect(text), "Reply all at {text:?}, cut by {clip:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}
