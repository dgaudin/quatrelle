//! The Scheduling Assistant: a day's hours, a row each for the organizer, the guests and the
//! rooms, their busy hours as the server (or the calendars kept here) has them, and the
//! meeting drawn over them — clicked or dragged to another time, the start and end follow.

use egui::{Color32, Rect, Stroke, pos2};
use quatrelle_engine::Fetched;
use quatrelle_engine::model::addr;
use quatrelle_engine::model::scheduling::BusyKind;

use super::*;

/// A drag of the meeting's slot: how far into the meeting it was taken (seconds).
#[derive(Clone, Copy, Debug, Default)]
pub struct SchedDrag {
    pub grab: i64,
}

/// Width of the names column, height of a row and of the hours header.
const NAMES_W: f32 = 150.0;
const ROW_H: f32 = 26.0;
const HEAD_H: f32 = 18.0;
/// The hours shown at least (a working day), stretched to hold the meeting.
const DAY_FROM: i64 = 7;
const DAY_TO: i64 = 20;
/// A click or drag lands on these (seconds).
const SNAP: i64 = 15 * 60;
/// Rows drawn at most (the people and rooms past that are still in the meeting).
const MAX_ROWS: usize = 40;

/// The timeline's id in the window numbered `window` (0: the dialog): the tests find its
/// rectangle by it. One a window: two meetings can be open at once.
pub fn timeline_id(window: u64) -> egui::Id {
    egui::Id::unique(("event-scheduling-timeline", window))
}

/// The hours shown (from, to) for a meeting from `start` to `end` on a day starting at
/// `day_from`: the working day, widened to hold the meeting when it falls on that day.
pub fn hours_shown(start: i64, end: i64, day_from: i64) -> (i64, i64) {
    let (mut h0, mut h1) = (DAY_FROM, DAY_TO);
    if start < day_from + 86_400 && end > day_from {
        h0 = h0.min((start - day_from).div_euclid(3600).clamp(0, 23));
        h1 = h1.max((end - day_from + 3599).div_euclid(3600).clamp(1, 24));
    }
    (h0, h1.max(h0 + 1))
}

fn snap(t: i64) -> i64 {
    (t + SNAP / 2).div_euclid(SNAP) * SNAP
}

/// The colours a kind of busy time is drawn in: its fill and its edge.
fn colors(kind: BusyKind, t: &Tokens) -> (Color32, Color32) {
    match kind {
        BusyKind::Free => (Color32::TRANSPARENT, Color32::TRANSPARENT),
        BusyKind::Busy => (t.accent, t.accent),
        BusyKind::Tentative => (t.accent.gamma_multiply(0.35), t.accent),
        BusyKind::OutOfOffice => (Color32::from_rgb(0x8E, 0x5B, 0xC8), Color32::from_rgb(0x8E, 0x5B, 0xC8)),
        BusyKind::WorkingElsewhere => (Color32::TRANSPARENT, t.accent),
        BusyKind::NoData => (t.faint.gamma_multiply(0.35), t.faint),
    }
}

fn kind_label(kind: BusyKind) -> &'static str {
    match kind {
        BusyKind::Free => "Free",
        BusyKind::Busy => "Busy",
        BusyKind::Tentative => "Tentative",
        BusyKind::OutOfOffice => "Out of office",
        BusyKind::WorkingElsewhere => "Working elsewhere",
        BusyKind::NoData => "No information",
    }
}

/// A row of the view: who, their address, and what they are to the meeting.
pub(super) struct Row {
    pub name: String,
    pub email: String,
    pub kind: &'static str,
}

/// The guests as typed, each piece read on its own, so a name still being typed doesn't hide
/// the others.
pub(super) fn guests(typed: &str) -> Vec<(String, String)> {
    typed
        .split([',', ';'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .filter_map(|p| addr::parse_list(p).ok().and_then(|l| l.into_iter().next()))
        .map(|a| (if a.name.is_empty() { a.email.clone() } else { a.name.clone() }, a.email))
        .collect()
}

/// The timeline of a room's day in the Room Finder (`window`: the meeting's window): the tests
/// find its rectangle by it.
pub fn room_timeline_id(window: u64) -> egui::Id {
    egui::Id::unique(("room-finder-timeline", window))
}

/// The view, under the form's fields.
pub(super) fn view(ui: &mut Ui, f: &mut EventForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) {
    let account = f.account_id();
    let Some(day) = day_bar(ui, f, s, t, true) else { return };
    ui.add_space(4.0);
    // Who: the organizer, the guests as typed (required, then optional), the rooms.
    let me = s.account(&account).map(|a| (a.config.name.clone(), a.config.email.clone())).unwrap_or_default();
    let mut rows = vec![Row { name: if me.0.trim().is_empty() { me.1.clone() } else { me.0.clone() }, email: me.1.clone(), kind: "Organizer" }];
    for (typed, kind) in [(&f.guests, "Required attendee"), (&f.optional, "Optional attendee")] {
        for (name, email) in guests(typed) {
            if !email.eq_ignore_ascii_case(&me.1) && !rows.iter().any(|r| r.email.eq_ignore_ascii_case(&email)) {
                rows.push(Row { name, email, kind });
            }
        }
    }
    rows.extend(f.rooms.iter().map(|(name, email)| Row { name: name.clone(), email: email.clone(), kind: "Room" }));
    rows.truncate(MAX_ROWS);
    grid(ui, f, s, t, acts, &rows, day, timeline_id(f.window));
    legend(ui, t);
    // Outlook's Add Attendees: the address book.
    if f.can_invite && !f.read_only {
        ui.add_space(4.0);
        if ui.button(tl!("Add Attendees…")).clicked() {
            f.select_attendees();
        }
    }
}

/// One room's day (the meeting's, or the one the Scheduling Assistant shows), the meeting over
/// it: when it's free, as its row in the Scheduling Assistant.
pub(super) fn room_day(ui: &mut Ui, f: &mut EventForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>, name: &str, email: &str) {
    let Some(day) = day_bar(ui, f, s, t, false) else { return };
    ui.add_space(2.0);
    let rows = [Row { name: name.to_string(), email: email.to_string(), kind: "Room" }];
    grid(ui, f, s, t, acts, &rows, day, room_timeline_id(f.window));
    legend(ui, t);
}

/// The day shown, and the way to the others (`today_too`: back to today as well). `None`, said:
/// the meeting has no times to show.
fn day_bar(ui: &mut Ui, f: &mut EventForm, s: &Session, t: &Tokens, today_too: bool) -> Option<i64> {
    let Some((start, _)) = f.times(s) else {
        ui.label(RichText::new(tl!("Give the meeting a start and an end to see when everyone is free.")).color(t.dim));
        return None;
    };
    let today = dates::today(&s.clock);
    let meeting_day = s.clock.local(start).div_euclid(86_400);
    let day = f.sched_day.unwrap_or(meeting_day);
    ui.horizontal(|ui| {
        if widgets::tiny_icon_button(ui, Icon::ChevronLeft, "Previous day", t).clicked() {
            f.sched_day = Some(day - 1);
        }
        if widgets::tiny_icon_button(ui, Icon::ChevronRight, "Next day", t).clicked() {
            f.sched_day = Some(day + 1);
        }
        ui.label(RichText::new(crate::calendar::long_date(day)).font(theme::bold(13.0)).color(t.text));
        if day != meeting_day && ui.small_button(tl!("Meeting day")).clicked() {
            f.sched_day = None;
        }
        if today_too && day != today && ui.small_button(tl!("Today")).clicked() {
            f.sched_day = Some(today);
        }
    });
    Some(day)
}

/// The rows' busy hours on a local day, the hours across, and the meeting over them: a click
/// puts it there, a drag carries it (`id`: the timeline's).
#[allow(clippy::too_many_arguments)]
fn grid(ui: &mut Ui, f: &mut EventForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>, rows: &[Row], day: i64, id: egui::Id) {
    let account = f.account_id();
    let Some((start, end)) = f.times(s) else { return };
    let day_from = s.clock.midnight(day);
    let day_to = s.clock.midnight(day.saturating_add(1));
    // What isn't known for the day yet is asked for (once: the engine marks it as asked).
    let mut missing: Vec<String> = Vec::new();
    for r in rows {
        if r.email.contains('@') && s.availability_now(&account, &r.email, day_from, day_to).is_none() && !missing.contains(&r.email) {
            missing.push(r.email.clone());
        }
    }
    if !missing.is_empty() {
        acts.push(Action::Run("calendar.availability", json!({"account": account, "emails": missing, "from": day_from, "to": day_to})));
    }
    let (h0, h1) = hours_shown(start, end, day_from);
    let hours = (h1 - h0) as f32;
    let (shown_from, shown_to) = (day_from + h0 * 3600, day_from + h1 * 3600);
    let width = ui.available_width();
    let timeline_w = (width - NAMES_W).max(120.0);
    let total_h = HEAD_H + ROW_H * rows.len() as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(width, total_h), Sense::hover());
    let p = ui.painter_at(rect);
    let x_of = |at: i64| rect.min.x + NAMES_W + ((at - day_from) as f32 / 3600.0 - h0 as f32) / hours * timeline_w;
    let time_of = |x: f32| day_from + (((x - rect.min.x - NAMES_W) / timeline_w * hours + h0 as f32) * 3600.0).round() as i64;
    // The hours, and their lines down the rows.
    for h in h0..=h1 {
        let x = x_of(day_from + h * 3600);
        p.line_segment([pos2(x, rect.min.y + HEAD_H), pos2(x, rect.max.y)], Stroke::new(1.0, t.border));
        if h < h1 {
            p.text(pos2(x + 3.0, rect.min.y + HEAD_H / 2.0), egui::Align2::LEFT_CENTER, format!("{h:02}"), theme::regular(11.0), t.faint);
        }
    }
    for (i, r) in rows.iter().enumerate() {
        let y0 = rect.min.y + HEAD_H + ROW_H * i as f32;
        let row = Rect::from_min_size(pos2(rect.min.x, y0), vec2(width, ROW_H));
        p.line_segment([row.left_bottom(), row.right_bottom()], Stroke::new(1.0, t.border));
        // Who, and what they are to the meeting on hover.
        let name_rect = Rect::from_min_size(row.min, vec2(NAMES_W - 8.0, ROW_H));
        let label = widgets::one_line(ui, &r.name, theme::regular(12.0), t.text, NAMES_W - 14.0);
        p.galley(pos2(name_rect.min.x + 4.0, row.center().y - label.size().y / 2.0), label, t.text);
        ui.interact(name_rect, id.with(("sched-who", i)), Sense::hover()).on_hover_text(format!("{} · {}", tl!(r.kind), r.email));
        // Their hours.
        let lane = Rect::from_min_max(pos2(x_of(shown_from), row.min.y + 4.0), pos2(x_of(shown_to), row.max.y - 4.0));
        let said = |text: &str, why: Option<&str>| {
            let (fill, _) = colors(BusyKind::NoData, t);
            p.rect_filled(lane, 2.0, fill);
            p.text(pos2(lane.min.x + 6.0, lane.center().y), egui::Align2::LEFT_CENTER, text, theme::regular(11.0), t.dim);
            if let Some(w) = why {
                ui.interact(lane, id.with(("sched-why", i)), Sense::hover()).on_hover_text(w);
            }
        };
        match s.availability_now(&account, &r.email, day_from, day_to) {
            None | Some(Fetched::Loading) => said(tl!("Loading availability…"), None),
            Some(Fetched::Failed(e)) => said(tl!("No information"), Some(e)),
            Some(Fetched::Ready(a)) if a.error.is_some() => said(tl!("No information"), a.error.as_deref()),
            Some(Fetched::Ready(a)) => {
                for (n, slot) in a.slots.iter().enumerate() {
                    let (a0, a1) = (slot.start.max(shown_from), slot.end.min(shown_to));
                    if a1 <= a0 {
                        continue;
                    }
                    let block = Rect::from_min_max(pos2(x_of(a0), lane.min.y), pos2(x_of(a1).max(x_of(a0) + 2.0), lane.max.y));
                    let (fill, edge) = colors(slot.kind, t);
                    p.rect_filled(block, 2.0, fill);
                    p.rect_stroke(block, 2.0, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
                    let when = format!("{} – {}", crate::calendar::hm(slot.start, s), crate::calendar::hm(slot.end, s));
                    let what = if slot.subject.is_empty() {
                        tl!(kind_label(slot.kind)).to_string()
                    } else {
                        format!("{} · {}", slot.subject, tl!(kind_label(slot.kind)))
                    };
                    ui.interact(block, id.with(("sched-slot", i, n)), Sense::hover()).on_hover_text(format!("{what}\n{when}"));
                }
            }
        }
    }
    // The meeting, over every row, when it falls on this day.
    if start < day_to && end > day_from && end > start {
        let x0 = x_of(start.max(shown_from));
        let x1 = x_of(end.min(shown_to)).max(x0 + 3.0);
        let slot = Rect::from_min_max(pos2(x0, rect.min.y + HEAD_H), pos2(x1, rect.max.y));
        p.rect_filled(slot, 2.0, t.accent.gamma_multiply(0.18));
        p.rect_stroke(slot, 2.0, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    }
    // A click puts the meeting there; a drag carries it along, as long as it was.
    let timeline = Rect::from_min_max(pos2(rect.min.x + NAMES_W, rect.min.y + HEAD_H), rect.max);
    let resp = ui.interact(timeline, id, Sense::click_and_drag());
    let len = (end - start).max(SNAP);
    let place = |f: &mut EventForm, at: i64| {
        let new_start = snap(at).clamp(day_from, day_to - len);
        f.set_times(s, new_start, new_start + len);
    };
    if resp.drag_started()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let at = time_of(pos.x);
        f.sched_drag = Some(SchedDrag { grab: if (start..end).contains(&at) { at - start } else { len / 2 } });
    }
    if resp.dragged()
        && let (Some(d), Some(pos)) = (f.sched_drag, resp.interact_pointer_pos())
    {
        place(f, time_of(pos.x) - d.grab);
    }
    if resp.drag_stopped() {
        f.sched_drag = None;
    }
    if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        place(f, time_of(pos.x));
    }
}

/// What the colours mean.
fn legend(ui: &mut Ui, t: &Tokens) {
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for kind in [BusyKind::Busy, BusyKind::Tentative, BusyKind::OutOfOffice, BusyKind::WorkingElsewhere, BusyKind::NoData] {
            let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
            let (fill, edge) = colors(kind, t);
            ui.painter().rect_filled(r, 2.0, fill);
            ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
            ui.label(RichText::new(tl!(kind_label(kind))).color(t.dim).small());
            ui.add_space(6.0);
        }
        ui.label(RichText::new(tl!("Click or drag the meeting to move it.")).color(t.faint).small());
    });
}
