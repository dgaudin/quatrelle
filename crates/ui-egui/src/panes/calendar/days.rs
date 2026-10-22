//! The Day, Work week and Week views: the days in columns over the hours (each calendar in a
//! pane of its own when side by side), the all-day events in a strip above, a click on an empty
//! half hour adding an event there, and appointments dragged to another time ([`super::drag`]).

use super::*;
use drag::Area;

/// A new event at `at`, in calendar `cal` (account, calendar id) when it can take one.
fn new_event_in(s: &Session, at: i64, cal: Option<&(String, String, String, u8)>) -> Action {
    let mut d = Dialog::new_event(s, Some(at));
    if let (Dialog::Event(f), Some((account, id, _, _))) = (&mut d, cal)
        && let Some(i) = f.calendars.iter().position(|(a, c, _)| a == account && c == id)
    {
        f.calendar = i;
    }
    Action::Dialog(d)
}

/// The space between calendars shown side by side.
const PANE_GAP: f32 = 10.0;

#[allow(clippy::too_many_arguments)]
pub(super) fn days(s: &Session, st: &mut UiState, ui: &mut Ui, items: &[AgendaItem], d0: i64, d1: i64, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let n = (d1 - d0).max(1);
    let gutter = 52.0;
    let now = today(s);
    let full = ui.available_rect_before_wrap();
    // Side by side (Outlook's): each calendar shown a pane of its own, its days in it, under a
    // heading in its colour.
    let cals: Vec<(String, String, String, u8)> = if s.prefs.calendars_side_by_side { s.shown_calendars() } else { Vec::new() };
    let k = if cals.len() > 1 { cals.len() } else { 1 };
    let gaps = PANE_GAP * (k - 1) as f32;
    let pane = ((full.width() - gutter - 16.0 - gaps) / k as f32).max(40.0 * n as f32);
    let cw = pane / n as f32;
    let col_of = |it: &AgendaItem| if k > 1 { cals.iter().position(|(a, c, _, _)| *a == it.account && *c == it.calendar).unwrap_or(0) } else { 0 };
    // Where calendar `j`'s pane starts, and day `i`'s column in it (from the left edge).
    let pane_x = |j: usize| gutter + (pane + PANE_GAP) * j as f32;
    let day_x = |i: i64, j: usize| pane_x(j) + cw * i as f32;
    let tint = |j: usize| cals.get(j).map(|(_, _, _, c)| color(*c, &t));
    // The calendars' headings: the name on its colour, × to hide it.
    if k > 1 {
        let (names, _) = ui.allocate_exact_size(vec2(full.width(), 24.0), Sense::hover());
        for (j, (account, id, name, c)) in cals.iter().enumerate() {
            let r = Rect::from_min_size(pos2(names.min.x + pane_x(j), names.min.y), vec2(pane, 24.0)).shrink2(vec2(0.0, 1.0));
            ui.painter().rect_filled(r, 3.0, color(*c, &t).gamma_multiply(if t.dark { 0.45 } else { 0.3 }));
            let label = if id == quatrelle_engine::model::pim::LOCAL { tl!("Calendar").to_string() } else { name.clone() };
            ui.painter().with_clip_rect(r.shrink2(vec2(6.0, 0.0)).with_max_x(r.max.x - 24.0)).text(
                pos2(r.min.x + 8.0, r.center().y),
                Align2::LEFT_CENTER,
                label.clone(),
                theme::bold(12.0),
                t.text,
            );
            let x = Rect::from_center_size(pos2(r.max.x - 12.0, r.center().y), vec2(18.0, 18.0));
            let resp = ui.interact(x, ui.scope_id().with(("hide-calendar", account, id)), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(x, 3.0, t.hover);
            }
            icons::paint(ui.painter(), x.shrink(4.0), Icon::Close, if resp.hovered() { t.text } else { t.dim });
            if resp.on_hover_text(tf!("Hide {calendar}", calendar = label)).clicked() {
                acts.push(Action::Run("calendar.show", json!({"account": account, "calendar": id, "shown": false})));
            }
        }
    }
    // Day headings, in every pane, as Outlook's: the weekday, the date under it, at the column's
    // left; today's in the app colour with a bar over it.
    let (head, _) = ui.allocate_exact_size(vec2(full.width(), 44.0), Sense::hover());
    const WEEKDAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    for j in 0..k {
        for i in 0..n {
            let d = d0 + i;
            let x = head.min.x + day_x(i, j);
            let (_, _, dd) = civil_from_days(d);
            let wd = WEEKDAYS.get(((d + 3).rem_euclid(7)) as usize).copied().unwrap_or("");
            let today_col = d == now;
            if today_col {
                ui.painter().rect_filled(Rect::from_min_size(pos2(x + 1.0, head.min.y), vec2(cw - 2.0, 3.0)), 0.0, t.accent);
            }
            if i > 0 {
                ui.painter().line_segment([pos2(x, head.min.y + 6.0), pos2(x, head.max.y)], egui::Stroke::new(1.0, t.border));
            }
            let clip = ui.painter().with_clip_rect(Rect::from_x_y_ranges(x..=x + cw - 2.0, head.y_range()));
            clip.text(pos2(x + 8.0, head.min.y + 13.0), Align2::LEFT_CENTER, tl!(wd), theme::regular(12.5), if today_col { t.accent } else { t.dim });
            let font = if today_col { theme::bold(17.0) } else { theme::regular(17.0) };
            clip.text(pos2(x + 8.0, head.min.y + 32.0), Align2::LEFT_CENTER, dd.to_string(), font, if today_col { t.accent } else { t.text });
        }
    }
    // All-day events, across their days (in their calendar's pane).
    let all_day: Vec<&AgendaItem> = items.iter().filter(|i| i.all_day || i.end - i.start >= 86_400).collect();
    if !all_day.is_empty() {
        let mut lanes: Vec<Vec<(usize, i64, i64)>> = Vec::new();
        let mut placed: Vec<(usize, &AgendaItem)> = Vec::new();
        for it in &all_day {
            let (a, b) = (day_of(it.start, s).max(d0), day_of(it.end.max(it.start + 1) - 1, s).min(d1 - 1));
            let j = col_of(it);
            let lane = lanes.iter().position(|l| l.iter().all(|(pj, x, y)| *pj != j || b < *x || a > *y)).unwrap_or(lanes.len());
            if lane == lanes.len() {
                lanes.push(Vec::new());
            }
            if let Some(l) = lanes.get_mut(lane) {
                l.push((j, a, b));
            }
            placed.push((lane, it));
        }
        let (strip, _) = ui.allocate_exact_size(vec2(full.width(), 20.0 * lanes.len().min(4) as f32 + 6.0), Sense::hover());
        ui.painter().text(pos2(strip.min.x + gutter - 6.0, strip.min.y + 10.0), Align2::RIGHT_CENTER, tl!("All day"), theme::regular(11.0), t.faint);
        for (lane, it) in placed.into_iter().filter(|(l, _)| *l < 4) {
            let (a, b) = (day_of(it.start, s).max(d0), day_of(it.end.max(it.start + 1) - 1, s).min(d1 - 1));
            let r = Rect::from_min_size(
                pos2(strip.min.x + day_x(a - d0, col_of(it)) + 2.0, strip.min.y + 3.0 + 20.0 * lane as f32),
                vec2(cw * (b - a + 1) as f32 - 4.0, 18.0),
            );
            let resp = chip(ui, s, &t, r, it, &it.summary);
            // Dragged along the strip: another day, as long as it was.
            let area = Area::Strip { rect: strip, x0: strip.min.x + pane_x(col_of(it)), cw, day0: d0, days: n };
            drag::chip(ui, st, s, &resp, r, it, &area, acts);
            if resp.clicked()
                && let Some(act) = open(s, it)
            {
                acts.push(act);
            }
        }
        for j in 0..k {
            drag::paint(ui, st, s, &t, &Area::Strip { rect: strip, x0: strip.min.x + pane_x(j), cw, day0: d0, days: n }, j, &col_of);
        }
    }
    let line = ui.available_rect_before_wrap();
    ui.painter().line_segment([line.left_top(), line.right_top()], egui::Stroke::new(1.0, t.border));
    // The hours.
    let mut scroll = egui::ScrollArea::vertical().auto_shrink(false).id_salt(("calendar-hours", s.cal_mode == CalMode::Day));
    // Each view (day, week) opens on the working day the first time it's shown.
    let view = usize::from(s.cal_mode == CalMode::Day);
    if !st.cal_scrolled[view] {
        scroll = scroll.vertical_scroll_offset(HOUR * 7.5);
        st.cal_scrolled[view] = true;
    }
    scroll.show(ui, |ui| {
        let (grid, _) = ui.allocate_exact_size(vec2(ui.available_width(), HOUR * 24.0), Sense::hover());
        // Each calendar's pane faintly in its colour.
        for j in 0..k {
            if let Some(c) = tint(j).filter(|_| k > 1) {
                let r = Rect::from_min_size(pos2(grid.min.x + pane_x(j), grid.min.y), vec2(pane, grid.height()));
                ui.painter().rect_filled(r, 0.0, c.gamma_multiply(if t.dark { 0.1 } else { 0.07 }));
            }
        }
        for h in 0..24 {
            let y = grid.min.y + HOUR * h as f32;
            for j in 0..k {
                let x0 = grid.min.x + pane_x(j);
                ui.painter().line_segment([pos2(x0, y), pos2(x0 + pane, y)], egui::Stroke::new(1.0, t.border));
            }
            if h > 0 {
                ui.painter().text(pos2(grid.min.x + gutter - 6.0, y), Align2::RIGHT_CENTER, format!("{h:02}:00"), theme::regular(11.0), t.faint);
            }
        }
        for j in 0..k {
            for i in 0..=n {
                let x = grid.min.x + day_x(i, j);
                ui.painter().line_segment([pos2(x, grid.min.y), pos2(x, grid.max.y)], egui::Stroke::new(1.0, t.border));
            }
        }
        // Click an empty half hour to add an event there (in that pane's calendar).
        for j in 0..k {
            for i in 0..n {
                let col = Rect::from_min_size(pos2(grid.min.x + day_x(i, j), grid.min.y), vec2(cw, grid.height()));
                let r = ui.interact(col, ui.scope_id().with(("hours", d0 + i, j)), Sense::click());
                // The half hour under the pointer lights up: a click adds an event there (not
                // while something is being dragged over it).
                if let Some(p) = r.hover_pos().filter(|_| st.cal_drag.is_none()) {
                    let half = ((p.y - grid.min.y) / (HOUR / 2.0)).floor().clamp(0.0, 47.0);
                    let slot = Rect::from_min_size(pos2(col.min.x, grid.min.y + half * HOUR / 2.0), vec2(cw, HOUR / 2.0)).shrink(1.0);
                    ui.painter().rect_filled(slot, 2.0, t.hover_row);
                }
                if r.clicked()
                    && let Some(p) = r.interact_pointer_pos()
                {
                    let half = ((p.y - grid.min.y) / (HOUR / 2.0)).floor().clamp(0.0, 47.0) as i64;
                    acts.push(new_event_in(s, s.local_midnight(d0 + i) + half * 1800, cals.get(j).filter(|_| k > 1)));
                }
            }
        }
        // Events, side by side when they overlap (each calendar in its own pane, side by side).
        for (i, j) in (0..n).flat_map(|i| (0..k).map(move |j| (i, j))) {
            let d = d0 + i;
            let (day0, day1) = (s.local_midnight(d), s.local_midnight(d + 1));
            let mut todays: Vec<&AgendaItem> = items
                .iter()
                .filter(|it| !it.all_day && it.end - it.start < 86_400 && it.start < day1 && it.end.max(it.start + 1) > day0 && col_of(it) == j)
                .collect();
            todays.sort_by_key(|it| (it.start, -(it.end - it.start)));
            let mut lane_end: Vec<i64> = Vec::new();
            let mut lanes: Vec<usize> = Vec::new();
            for it in &todays {
                let lane = lane_end.iter().position(|e| *e <= it.start).unwrap_or(lane_end.len());
                if lane == lane_end.len() {
                    lane_end.push(it.end);
                } else if let Some(e) = lane_end.get_mut(lane) {
                    *e = it.end;
                }
                lanes.push(lane);
            }
            let width = (cw - 6.0) / lane_end.len().max(1) as f32;
            for (it, lane) in todays.iter().zip(lanes) {
                let top = ((it.start.max(day0) - day0) as f32 / 3600.0) * HOUR;
                let bottom = ((it.end.min(day1) - day0) as f32 / 3600.0) * HOUR;
                let r = Rect::from_min_size(
                    pos2(grid.min.x + day_x(i, j) + 3.0 + width * lane as f32, grid.min.y + top + 1.0),
                    vec2(width - 2.0, (bottom - top - 2.0).max(18.0)),
                );
                // The place beside the time when both fit on one line (a short event still shows it),
                // else on a line of its own.
                let mut text = format!("{}\n{} – {}", it.summary, hm(it.start, s), hm(it.end, s));
                if !it.location.is_empty() {
                    let both = format!("{} – {} · {}", hm(it.start, s), hm(it.end, s), it.location);
                    let w = ui.fonts_mut(|f| f.layout_no_wrap(both, theme::regular(12.0), t.text).size().x);
                    text.push_str(if w <= r.width() - 8.0 { " · " } else { "\n" });
                    text.push_str(&it.location);
                }
                let resp = chip(ui, s, &t, r, it, &text);
                // Dragged by its body to another hour or day; by its bottom edge, made to end
                // at another time.
                let area = Area::Hours { grid, pane_x: pane_x(j), cw, day0: d0, days: n };
                drag::chip(ui, st, s, &resp, r, it, &area, acts);
                if resp.clicked()
                    && let Some(a) = open(s, it)
                {
                    acts.push(a);
                }
            }
        }
        // Where what's dragged would land, over everything else.
        for j in 0..k {
            drag::paint(ui, st, s, &t, &Area::Hours { grid, pane_x: pane_x(j), cw, day0: d0, days: n }, j, &col_of);
        }
        // Now, in every pane.
        let n_day = today(s);
        if n_day >= d0 && n_day < d1 {
            let y = grid.min.y + ((s.clock.now - s.local_midnight(n_day)) as f32 / 3600.0) * HOUR;
            for j in 0..k {
                let x0 = grid.min.x + day_x(n_day - d0, j);
                ui.painter().line_segment([pos2(x0, y), pos2(x0 + cw, y)], egui::Stroke::new(2.0, t.danger));
                ui.painter().circle_filled(pos2(x0, y), 4.0, t.danger);
            }
        }
    });
}
