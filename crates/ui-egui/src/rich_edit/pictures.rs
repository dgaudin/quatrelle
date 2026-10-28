//! Pictures in the message being written, at the size they're sent (from upstream b5f14ee and
//! 865a712): a picture shows at its width (its own, else the one it was given), its shape kept,
//! never wider than the text. Clicked, it's selected (Delete removes it); the handle at its
//! corner drags it bigger or smaller (a double-click on it gives it its own size back), and its
//! right-click menu sets its size, as Outlook's do. A wide one goes in at a page's width, on a
//! line of its own.

use std::path::Path;

use super::*;

/// The widest a picture goes in at by itself: a page's width in Outlook (6.5 inches). A wider
/// one is given this width as it's put in (a 1800-pixel screenshot was sent that wide, and
/// Outlook showed it so), and can be made bigger again.
pub(crate) const FIT_WIDTH: u32 = 624;
/// The narrowest a picture is dragged to.
const MIN_WIDTH: f32 = 16.0;

/// A picture's own size in pixels, from its header (nothing decoded); `None` when it isn't a
/// picture that can be read.
pub(crate) fn own_size(data: &[u8]) -> Option<(u32, u32)> {
    let (w, h) = image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format().ok()?.into_dimensions().ok()?;
    (w > 0 && h > 0).then_some((w, h))
}

/// The same, of a picture file.
pub(crate) fn own_size_of(path: &Path) -> Option<(u32, u32)> {
    let (w, h) = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.into_dimensions().ok()?;
    (w > 0 && h > 0).then_some((w, h))
}

/// How a picture of its own size `size` goes in: the width it's given ([`FIT_WIDTH`] when it's
/// wider, else its own), and whether on a line of its own (wider than a third of a page, as a
/// screenshot pasted mid-sentence goes in mail editors: two in a row went past the text's edge).
pub(crate) fn placement(size: Option<(u32, u32)>) -> (Option<u32>, bool) {
    (size.filter(|(w, _)| *w > FIT_WIDTH).map(|_| FIT_WIDTH), size.is_some_and(|(w, _)| w > FIT_WIDTH / 3))
}

/// The size a picture of its own size `own` (points) is shown and sent at: `width` wide, its
/// height following, else its own.
fn set_size(own: Option<Vec2>, width: Option<u16>) -> Option<Vec2> {
    match (own, width) {
        (Some(o), Some(w)) => Some(vec2(f32::from(w), f32::from(w) * o.y / o.x.max(1.0))),
        (own, _) => own,
    }
}

/// Each picture's size as the text sets it (by `cid:`): its own (`own`), or the width it was
/// given.
pub(super) fn sizes_as_set(own: HashMap<String, Vec2>, c: &Compose) -> HashMap<String, Vec2> {
    if c.inline.is_empty() {
        return own;
    }
    let mut out = own.clone();
    for (i, ch) in c.body.chars().enumerate() {
        if ch != PICTURE {
            continue;
        }
        let st = c.styles.style(i);
        if let (Some(cid), Some(w)) = (st.picture, st.picture_width)
            && let Some(s) = set_size(own.get(&cid).copied(), Some(w))
        {
            out.insert(cid, s);
        }
    }
    out
}

/// Draw picture `cid`, the one at character `at` (laid out at `a`), at its size in text `wrap`
/// wide; where it went is kept for [`interact`].
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(ui: &Ui, t: &Tokens, ed: &mut Editor, c: &Compose, at: usize, cid: &str, a: Rect, wrap: f32) {
    let shown = layout::shown_size(set_size(ed.picture_sizes.get(cid).copied(), c.styles.style(at).picture_width), wrap);
    let room = Rect::from_min_size(egui::pos2(a.min.x + 2.0, a.max.y - shown.y), shown);
    let painter = ui.painter();
    let tex = ed.pictures.entry(cid.to_string()).or_insert_with(|| {
        c.inline.iter().find(|p| p.cid == cid).and_then(|p| crate::reading::decode(ui.ctx(), &format!("compose-{cid}"), &p.data, PICTURE_DECODED))
    });
    let drawn = match tex {
        Some(tex) => {
            // Decoded smaller than it shows (a big one), it's drawn as big as its room.
            let size = tex.size_vec2();
            let scale = (room.width() / size.x.max(1.0)).min(room.height() / size.y.max(1.0));
            let r = Rect::from_min_size(room.min, size * scale);
            painter.image(tex.id(), r, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            r
        }
        None => {
            painter.rect_stroke(room, 4.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
            icons::paint(painter, icons::centered(room, 22.0), Icon::Image, t.dim);
            room
        }
    };
    ed.placed.push((at, drawn, cid.to_string()));
}

/// Picture at character `at` given `width` (`None`: its own).
fn set_width(c: &mut Compose, at: usize, width: Option<u16>) {
    let mut styles = std::mem::take(&mut c.styles);
    styles.restyle(&c.body, at..at + 1, |s| Style { picture_width: width, ..s.clone() });
    c.styles = styles;
}

/// Clicks, drags and right-clicks on the pictures drawn this frame, in text `wrap` wide.
pub(super) fn interact(ui: &mut Ui, t: &Tokens, ed: &mut Editor, c: &mut Compose, wrap: f32) {
    let placed = std::mem::take(&mut ed.placed);
    if c.sending {
        return;
    }
    let widest = if wrap.is_finite() { (wrap - 4.0).max(MIN_WIDTH) } else { f32::from(u16::MAX) };
    let painter = ui.painter().clone();
    for (at, rect, cid) in placed {
        let r = ui.interact(rect, egui::Id::unique(("compose-picture", c.id, at)), Sense::click());
        if r.clicked() || r.secondary_clicked() {
            ed.sel = at..at + 1;
            ed.refocus = Some(at..at + 1);
        }
        if ed.sel == (at..at + 1) {
            painter.rect_stroke(rect.expand(1.0), 0.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Outside);
        }
        // The corner handle, while the picture is selected or the pointer is over it (with the
        // button down on the handle the picture isn't hovered: a drag started without selecting
        // the picture first lost its handle, upstream's #14 test found it).
        if ed.sel == (at..at + 1) || r.hovered() || ui.rect_contains_pointer(rect) || ed.resizing == Some(at) {
            let handle = Rect::from_center_size(rect.right_bottom() - vec2(4.0, 4.0), vec2(10.0, 10.0));
            let h = ui
                .interact(handle, egui::Id::unique(("compose-picture-size", c.id, at)), Sense::click_and_drag())
                .on_hover_cursor(egui::CursorIcon::ResizeNwSe);
            painter.rect(handle, 1.0, t.accent, Stroke::new(1.0, t.surface), egui::StrokeKind::Inside);
            if h.double_clicked() && c.styles.style(at).picture_width.is_some() {
                ed.remember(c, at + 1);
                set_width(c, at, None);
                ed.refocus = Some(at..at + 1);
            }
            if h.drag_started() {
                ed.remember(c, at + 1);
                ed.resizing = Some(at);
            }
            if h.dragged()
                && let Some(p) = h.interact_pointer_pos()
            {
                let w = (p.x + 4.0 - rect.left()).clamp(MIN_WIDTH, widest);
                set_width(c, at, Some(w.round() as u16));
                ui.ctx().request_repaint();
            }
            if h.drag_stopped() {
                ed.resizing = None;
                ed.refocus = Some(at..at + 1);
            }
        }
        let own = ed.picture_sizes.get(&cid).map_or(rect.width(), |s| s.x);
        r.context_menu(|ui| {
            ui.set_min_width(160.0);
            let choices = [
                (tl!("Original size").to_string(), None),
                (tf!("{percent}%", percent = 75), Some(0.75)),
                (tf!("{percent}%", percent = 50), Some(0.5)),
                (tf!("{percent}%", percent = 25), Some(0.25)),
            ];
            for (label, part) in choices {
                if ui.button(label).clicked() {
                    ed.remember(c, at + 1);
                    set_width(c, at, part.map(|p| (own * p).round().clamp(MIN_WIDTH, f32::from(u16::MAX)) as u16));
                    ed.refocus = Some(at..at + 1);
                    ui.close();
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture shows at the width it's given (its height following), else its own; one wider
    /// than a page goes in at a page's width, and one wider than a third of it on a line of its
    /// own.
    #[test]
    fn pictures_go_in_and_show_at_their_width() {
        let own = Some(vec2(1200.0, 600.0));
        assert_eq!(set_size(own, Some(300)), Some(vec2(300.0, 150.0)));
        assert_eq!(set_size(own, None), own);
        assert_eq!(set_size(None, Some(300)), None);
        assert_eq!(placement(Some((2880, 1800))), (Some(FIT_WIDTH), true));
        assert_eq!(placement(Some((400, 300))), (None, true));
        assert_eq!(placement(Some((32, 32))), (None, false));
        assert_eq!(placement(None), (None, false));
        let png = quatrelle_engine::sample::sample_png();
        assert!(own_size(&png).is_some_and(|(w, h)| w > 0 && h > 0));
        assert_eq!(own_size(b"not a picture"), None);
        assert_eq!(own_size(png.get(..20).unwrap_or_default()), None, "cut short");
    }
}
