//! A signature written in HTML in the message being written: the character it stands on
//! ([`rich::SIGNATURE`]) gets its room in the text, and it's drawn there as it's sent (its layout:
//! a logo beside the name), over the text but out of its way (clicks go to the text).

use quatrelle_engine::Session;

use super::*;
use crate::UiState;

/// The signature's room among the pictures' ([`layout::rooms`]).
pub(super) const ROOM: &str = "\u{E002}";

/// The signature as last drawn, and where it goes this frame.
#[derive(Default)]
pub(crate) struct Placed {
    /// Its size as drawn (its room in the text).
    size: Option<Vec2>,
    /// Its room this frame, the text's clip and the text's width.
    at: Option<(Rect, Rect, f32)>,
}

/// The pictures' sizes, with the signature's as last drawn.
pub(super) fn with_room(mut sizes: HashMap<String, Vec2>, placed: &Placed) -> HashMap<String, Vec2> {
    if let Some(size) = placed.size {
        sizes.insert(ROOM.to_string(), size);
    }
    sizes
}

/// The signature's room as drawn in text `wrap` wide: its whole height, as wide as fits.
pub(super) fn room(size: Vec2, wrap: f32) -> Vec2 {
    let widest = if wrap.is_finite() { (wrap - 4.0).max(24.0) } else { f32::INFINITY };
    vec2(size.x.min(widest), size.y.max(8.0))
}

/// Where the signature's character landed in the laid-out text `g` (at `origin`).
pub(super) fn place(ui: &Ui, ed: &mut Editor, c: &Compose, g: &Galley, origin: egui::Pos2) {
    ed.signature.at = None;
    if c.signature_html.is_empty() {
        return;
    }
    let Some(i) = c.body.chars().position(|ch| ch == rich::SIGNATURE) else { return };
    let a = g.pos_from_cursor(CCursor::new(i)).translate(origin.to_vec2());
    let wrap = g.job.wrap.max_width;
    let size = ed.signature.size.map_or(vec2(PICTURE_PX * 1.4, PICTURE_PX), |s| room(s, wrap));
    let at = Rect::from_min_size(egui::pos2(a.min.x + 2.0, a.max.y - size.y), size);
    ed.signature.at = Some((at, ui.clip_rect(), wrap));
}

/// Draw the signature of `c` where [`place`] put it, as a message shows it (its pictures from
/// the message), and keep its size for its room on the next frame.
pub(crate) fn draw(s: &Session, st: &mut UiState, ui: &mut Ui, ed: &mut Editor, c: &Compose) {
    let Some((at, clip, wrap)) = ed.signature.at.take() else { return };
    let width = if wrap.is_finite() { wrap } else { ui.available_width() };
    let bounds = Rect::from_min_size(at.min, vec2(width, 4000.0));
    let mut sig = ui.new_child(egui::UiBuilder::new().max_rect(bounds).layout(egui::Layout::top_down(egui::Align::Min)));
    sig.set_clip_rect(clip);
    // Out of the text's way: a click goes to the text under it (the signature is changed in
    // Signatures…), and it isn't greyed for it.
    sig.visuals_mut().disabled_alpha = 1.0;
    sig.disable();
    crate::reading::html_preview(s, st, &mut sig, &c.signature_html, &c.inline);
    let size = sig.min_rect().size();
    if ed.signature.size.is_none_or(|was| (was - size).length() > 0.5) {
        ed.signature.size = Some(size);
        ui.ctx().request_repaint();
    }
}
