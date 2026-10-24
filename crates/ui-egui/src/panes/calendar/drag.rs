//! Appointments dragged to another time, as Outlook's calendar has it: a chip's body moves it
//! (the day from where the pointer is, the time snapped to the quarter hour; in the month and
//! the all-day strip, to another day as long as it was), its bottom edge changes when it ends
//! and its top edge when it starts (the edges show a grip on the chip pointed at), a ghost
//! shows where it would land, Esc lets go, and letting go asks the engine to move it —
//! after asking the user what Outlook asks: one occurrence or the whole series, and whether
//! the guests of a meeting they organize hear of it ([`crate::dialogs::MoveForm`]).
//!
//! What can't be dragged starts no drag: an appointment in a read-only calendar, or a meeting
//! someone else organizes ([`AgendaItem::movable`]).

use super::*;

/// The grain a dragged time snaps to (Outlook's quarter hour).
const SNAP: i64 = 900;

/// A drag in progress.
#[derive(Clone, Debug)]
pub struct Drag {
    pub item: AgendaItem,
    /// An edge is dragged: the bottom one changes only the end, the top one (`top`) only the
    /// start.
    pub resize: bool,
    pub top: bool,
    /// Seconds from the item's start to where it was taken hold of, so it keeps its place under
    /// the pointer.
    pub grab: i64,
    /// Where it's going, in the item's own times, once the pointer has said.
    pub to: Option<(i64, i64)>,
}

/// Where chips are drawn, to work out where the pointer puts one.
pub(super) enum Area {
    /// The hours of the Day and Week views: the grid, where the chip's pane starts, a day's
    /// width, the first day shown and how many.
    Hours { grid: Rect, pane_x: f32, cw: f32, day0: i64, days: i64 },
    /// The all-day strip above them: by day only.
    Strip { rect: Rect, x0: f32, cw: f32, day0: i64, days: i64 },
    /// The month's cells: the heading's height, a cell's size, the first day shown.
    Month { area: Rect, head: f32, cw: f32, rh: f32, day0: i64 },
}

fn snap(secs: i64) -> i64 {
    (secs as f64 / SNAP as f64).round() as i64 * SNAP
}

impl Area {
    /// The day under `p` (none outside the days shown).
    fn day_at(&self, p: egui::Pos2) -> Option<i64> {
        let col = |x0: f32, cw: f32, days: i64| {
            let i = ((p.x - x0) / cw).floor();
            (i >= 0.0 && (i as i64) < days).then_some(i as i64)
        };
        match self {
            Area::Hours { grid, pane_x, cw, day0, days } => Some(day0 + col(grid.min.x + pane_x, *cw, *days)?),
            Area::Strip { x0, cw, day0, days, .. } => Some(day0 + col(*x0, *cw, *days)?),
            Area::Month { area, head, cw, rh, day0 } => {
                let i = col(area.min.x, *cw, 7)?;
                let w = ((p.y - area.min.y - head) / rh).floor();
                (0.0..6.0).contains(&w).then(|| day0 + (w as i64) * 7 + i)
            }
        }
    }

    /// Where the item lands with the pointer at `p`: its new start and end.
    fn target(&self, p: egui::Pos2, d: &Drag, s: &Session) -> Option<(i64, i64)> {
        let it = &d.item;
        let day = self.day_at(p)?;
        match self {
            Area::Hours { grid, .. } => {
                let secs = i64::from(((p.y - grid.min.y) / HOUR * 3600.0).round() as i32);
                // Bounded with max/min, not clamp: a chip near midnight has bounds the wrong way
                // round (one starting at 23:50 ends after the day), and clamp panics on those.
                let midnight = s.local_midnight(day_of(it.start, s));
                if d.resize && d.top {
                    let start = (midnight + snap(secs)).max(midnight).min(it.end - SNAP);
                    Some((start, it.end))
                } else if d.resize {
                    let end = (midnight + snap(secs)).min(midnight + 86_400).max(it.start + SNAP);
                    Some((it.start, end))
                } else {
                    let len = (it.end - it.start).clamp(SNAP, 86_400);
                    let start = s.local_midnight(day) + snap(secs - d.grab).clamp(0, 86_400 - len);
                    Some((start, start + len))
                }
            }
            Area::Strip { .. } | Area::Month { .. } => {
                let shift = (day - day_of(it.start, s)) * 86_400;
                Some((it.start + shift, it.end + shift))
            }
        }
    }

    /// The ghost: where the item would land, in its colour, with its new time (or its name).
    fn ghost(&self, ui: &Ui, t: &Tokens, s: &Session, d: &Drag, start: i64, end: i64) {
        let c = item_color(&d.item, t);
        let fill = c.gamma_multiply(if t.dark { 0.5 } else { 0.35 });
        let stroke = egui::Stroke::new(1.5, c);
        let painter = ui.painter();
        match self {
            Area::Hours { grid, pane_x, cw, day0, days } => {
                let day = day_of(start, s);
                if day < *day0 || day >= day0 + days {
                    return;
                }
                let midnight = s.local_midnight(day);
                let y = |time: i64| grid.min.y + ((time - midnight) as f32 / 3600.0) * HOUR;
                let x = grid.min.x + pane_x + cw * (day - day0) as f32;
                let r = Rect::from_min_max(pos2(x + 3.0, y(start) + 1.0), pos2(x + cw - 3.0, y(end.min(midnight + 86_400)) - 1.0));
                painter.rect(r, 4.0, fill, stroke, egui::StrokeKind::Inside);
                let label = format!("{} – {}", hm(start, s), hm(end, s));
                painter.with_clip_rect(r).text(r.min + vec2(6.0, 2.0), Align2::LEFT_TOP, label, theme::bold(12.0), t.text);
            }
            Area::Strip { rect, x0, cw, day0, days } => {
                let (a, b) = (day_of(start, s).max(*day0), day_of(end.max(start + 1) - 1, s).min(day0 + days - 1));
                if b < a {
                    return;
                }
                let r = Rect::from_min_size(pos2(x0 + cw * (a - day0) as f32 + 2.0, rect.min.y + 3.0), vec2(cw * (b - a + 1) as f32 - 4.0, 18.0));
                painter.rect(r, 4.0, fill, stroke, egui::StrokeKind::Inside);
                painter.with_clip_rect(r).text(r.min + vec2(6.0, 2.0), Align2::LEFT_TOP, d.item.summary.clone(), theme::regular(12.0), t.text);
            }
            Area::Month { area, head, cw, rh, day0 } => {
                let day = day_of(start, s);
                if day < *day0 || day >= day0 + 42 {
                    return;
                }
                let (w, i) = ((day - day0).div_euclid(7), (day - day0).rem_euclid(7));
                let cell = Rect::from_min_size(pos2(area.min.x + cw * i as f32, area.min.y + head + rh * w as f32), vec2(*cw, *rh));
                painter.rect_stroke(cell.shrink(1.0), 0.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
                let r = Rect::from_min_size(cell.min + vec2(3.0, 24.0), vec2(cw - 6.0, 17.0));
                painter.rect(r, 4.0, fill, stroke, egui::StrokeKind::Inside);
                let text = if d.item.all_day { d.item.summary.clone() } else { format!("{} {}", hm(start, s), d.item.summary) };
                painter.with_clip_rect(r).text(r.min + vec2(6.0, 2.0), Align2::LEFT_TOP, text, theme::regular(12.0), t.text);
            }
        }
    }
}

/// Where a chip's bottom edge is taken hold of to change when the appointment ends.
fn handle_rect(rect: Rect) -> Rect {
    Rect::from_min_max(pos2(rect.min.x, (rect.max.y - 7.0).max(rect.min.y + 4.0)), rect.max)
}

/// Where its top edge is taken hold of to change when it starts: none on a chip too short to
/// have both and still a body to move it by.
fn top_rect(rect: Rect) -> Option<Rect> {
    (rect.height() >= 26.0).then(|| Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + 6.0)))
}

/// After a chip is drawn: a drag of it started, followed, or ended (`resp`: the chip's).
#[allow(clippy::too_many_arguments)]
pub(super) fn chip(
    ui: &mut Ui,
    st: &mut UiState,
    s: &Session,
    resp: &egui::Response,
    rect: Rect,
    item: &AgendaItem,
    area: &Area,
    acts: &mut Vec<Action>,
) {
    if !item.movable {
        return;
    }
    let same = |d: &Drag| d.item.uid == item.uid && d.item.occurrence == item.occurrence && d.item.start == item.start;
    // The edges, in the hours: the bottom one dragged, the end moves and the start stays; the top
    // one, the start moves and the end stays.
    let edge = |r: Rect, name: &str| ui.interact(r, resp.id.with(name), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeVertical);
    let (handle, top_handle) = match area {
        Area::Hours { .. } => (Some(edge(handle_rect(rect), "resize")), top_rect(rect).map(|r| edge(r, "resize-top"))),
        _ => (None, None),
    };
    // A grip on each edge of the chip pointed at, so it's seen that they can be taken hold of.
    if handle.is_some()
        && st.cal_drag.is_none()
        && (resp.hovered() || handle.as_ref().is_some_and(|h| h.hovered()) || top_handle.as_ref().is_some_and(|h| h.hovered()))
    {
        let grip = st.tokens.text.gamma_multiply(0.55);
        let bar = |y: f32| Rect::from_center_size(pos2(rect.center().x, y), vec2(18.0_f32.min(rect.width() - 8.0).max(4.0), 3.0));
        ui.painter().rect_filled(bar(rect.max.y - 3.0), 1.5, grip);
        if top_handle.is_some() {
            ui.painter().rect_filled(bar(rect.min.y + 3.0), 1.5, grip);
        }
    }
    let top = top_handle.as_ref().is_some_and(|h| h.drag_started());
    let resize = top || handle.as_ref().is_some_and(|h| h.drag_started());
    if (resp.drag_started() || resize) && st.cal_drag.is_none() {
        // Where it was taken hold of: the press, not where the pointer is once the drag is
        // decided (it has moved by then).
        let at = ui.input(|i| i.pointer.press_origin()).or_else(|| resp.interact_pointer_pos());
        let grab = match (area, at) {
            (Area::Hours { grid, .. }, Some(p)) => {
                i64::from(((p.y - grid.min.y) / HOUR * 3600.0).round() as i32) - (item.start - s.local_midnight(day_of(item.start, s)))
            }
            _ => 0,
        };
        st.cal_drag = Some(Drag { item: item.clone(), resize, top, grab, to: None });
    }
    if !st.cal_drag.as_ref().is_some_and(same) {
        return;
    }
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        st.cal_drag = None;
        return;
    }
    let pointer = ui.input(|i| i.pointer.latest_pos());
    if let (Some(d), Some(p)) = (st.cal_drag.as_mut(), pointer) {
        d.to = area.target(p, d, s);
        ui.ctx().set_cursor_icon(if d.resize { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::Grabbing });
    }
    // Let go: moved, when it went anywhere.
    let released = resp.drag_stopped()
        || handle.as_ref().is_some_and(|h| h.drag_stopped())
        || top_handle.as_ref().is_some_and(|h| h.drag_stopped())
        || !ui.input(|i| i.pointer.primary_down());
    if released
        && let Some(d) = st.cal_drag.take()
        && let Some((start, end)) = d.to
        && (start, end) != (d.item.start, d.item.end)
    {
        commit(s, &d.item, start, end, acts);
    }
}

/// The ghost of what's being dragged over an area, for the pane (`col`) the item is drawn in.
pub(super) fn paint(ui: &Ui, st: &UiState, s: &Session, t: &Tokens, area: &Area, col: usize, col_of: &dyn Fn(&AgendaItem) -> usize) {
    let Some(d) = st.cal_drag.as_ref() else { return };
    let Some((start, end)) = d.to else { return };
    if col_of(&d.item) != col {
        return;
    }
    // Only the area the item was taken from draws it: an all-day item never lands in the
    // hours, a timed one never in the strip.
    let fits = match area {
        Area::Hours { .. } => !d.item.all_day && d.item.end - d.item.start < 86_400,
        Area::Strip { .. } => d.item.all_day || d.item.end - d.item.start >= 86_400,
        Area::Month { .. } => true,
    };
    if fits {
        area.ghost(ui, t, s, d, start, end);
    }
}

/// Ask the engine to move it: at once when nothing needs asking; else after Outlook's
/// questions (one occurrence or the series; the guests told or not).
fn commit(s: &Session, item: &AgendaItem, start: i64, end: i64, acts: &mut Vec<Action>) {
    // All-day events keep their days as UTC midnights.
    let (start, end) = if item.all_day { (s.clock.local(start), s.clock.local(end)) } else { (start, end) };
    if !item.recurring && !item.organized {
        acts.push(Action::Run("event.move", json!({"uid": item.uid, "start": start, "end": end})));
        return;
    }
    let graph = s.account(&item.account).is_some_and(|a| a.config.graph.is_some());
    acts.push(Action::Dialog(Dialog::MoveEvent(Box::new(crate::dialogs::MoveForm {
        uid: item.uid.clone(),
        summary: item.summary.clone(),
        occurrence: item.occurrence,
        recurring: item.recurring,
        occurrence_allowed: s.occurrence_moves(&item.account),
        start,
        end,
        meeting: item.organized,
        quiet_allowed: !graph,
        this_occurrence: None,
        error: None,
    }))));
}

#[cfg(test)]
mod edge_tests {
    use super::*;

    /// An edge of an appointment near midnight dragged (one starting at 23:50 ends the next
    /// day): the bounds are the wrong way round, which `clamp` panicked on.
    #[test]
    fn edges_dragged_near_midnight_stay_in_bounds() {
        let noon = 1_800_000_000 - 1_800_000_000 % 86_400 + 12 * 3600;
        let s = Session::open(None, quatrelle_engine::model::dates::Clock::new(noon, 0));
        let midnight = s.local_midnight(day_of(noon, &s));
        let start = midnight + 23 * 3600 + 50 * 60;
        let item = AgendaItem { start, end: start + 1800, movable: true, ..Default::default() };
        let grid = Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, 24.0 * HOUR));
        let area = Area::Hours { grid, pane_x: 0.0, cw: 700.0, day0: day_of(start, &s), days: 1 };
        let drag = |top: bool| Drag { item: item.clone(), resize: true, top, grab: 0, to: None };
        // The bottom edge anywhere: it still ends a quarter hour after it starts, at least.
        for y in [0.0, 12.0 * HOUR, 24.0 * HOUR - 1.0] {
            let (a, b) = area.target(pos2(10.0, y), &drag(false), &s).unwrap();
            assert_eq!(a, start);
            assert!(b >= start + SNAP, "{y}: {b}");
        }
        // The top edge: starts at the day's first hour at the earliest, its last at the latest
        // (still before it ends).
        let (a, b) = area.target(pos2(10.0, 0.0), &drag(true), &s).unwrap();
        assert_eq!((a, b), (midnight, start + 1800));
        let (a, b) = area.target(pos2(10.0, 24.0 * HOUR - 1.0), &drag(true), &s).unwrap();
        assert_eq!((a, b), (midnight + 86_400, start + 1800));
    }
}
