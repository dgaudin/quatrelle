// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The Calendar: a month to pick days from and the calendars in the side pane; a day, a week
//! or a month in the main one. A click selects an event or an empty half hour, a double click
//! opens it or adds an event there ([`pick`]); drag one to move it ([`drag`]).

use egui::{Align2, Color32, Key, Margin, Rect, RichText, Sense, Ui, pos2, vec2};
use quatrelle_engine::model::dates::{self, civil_from_days, days_from_civil};
use quatrelle_engine::model::ical::ShowAs;
use quatrelle_engine::{AgendaItem, CalMode, Session};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, Dialog, UiState, widgets};

mod days;
mod drag;
mod found;
mod menu;
mod pick;
mod side;
use days::days;
pub use drag::Drag;
#[cfg(test)]
pub(crate) use found::open_hit;
pub(crate) use found::{CalSearch, found};
pub use pick::Pick;
pub(crate) use pick::new_event;
pub use side::{nav, todo_bar};

/// Height of an hour in the day and week views.
pub(crate) const HOUR: f32 = 46.0;
pub(crate) const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
pub(crate) const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// A calendar's colour (`Session::calendar_color`): the first one is the app's; then Quatrelle's
/// eight categories' in turn; one chosen on this computer, the palette's.
pub fn color(i: u8, t: &Tokens) -> Color32 {
    use quatrelle_engine::{PALETTE_COLOR, PALETTE_COLORS};
    match i {
        0 => t.accent,
        _ if (PALETTE_COLOR..PALETTE_COLOR.saturating_add(PALETTE_COLORS)).contains(&i) => theme::category_color(i - PALETTE_COLOR, t.dark),
        _ => theme::category_color(i.saturating_sub(1) % 8, t.dark),
    }
}

fn today(s: &Session) -> i64 {
    dates::today(&s.clock)
}

/// `HH:MM`, local.
pub fn hm(t: i64, s: &Session) -> String {
    let c = dates::civil(t, &s.clock);
    format!("{:02}:{:02}", c.hour, c.minute)
}

/// The local day a time falls on.
fn day_of(t: i64, s: &Session) -> i64 {
    s.clock.local(t).div_euclid(86_400)
}

/// `October 2026`, `12 – 18 October 2026`, `Monday, 12 October 2026`.
pub fn heading(s: &Session) -> String {
    let (d0, d1) = s.cal_days();
    let month = |m: u32| crate::i18n::tc("month", MONTHS.get(m as usize - 1).copied().unwrap_or("")).to_string();
    match s.cal_mode {
        CalMode::Month => {
            let (y, m, _) = civil_from_days(s.cal_day());
            tf!("{month} {year}", month = month(m), year = y)
        }
        CalMode::Week | CalMode::WorkWeek => {
            let ((y0, m0, a), (y1, m1, b)) = (civil_from_days(d0), civil_from_days(d1 - 1));
            if (y0, m0) == (y1, m1) {
                tf!("{from} – {to} {month} {year}", from = a, to = b, month = month(m1), year = y1)
            } else {
                tf!("{from} {month_from} – {to} {month} {year}", from = a, month_from = month(m0), to = b, month = month(m1), year = y1)
            }
        }
        CalMode::Day => long_date(d0),
    }
}

/// A day written out: `Thursday, 22 October 2026`, in the interface's language.
pub fn long_date(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    let month = crate::i18n::tc("month", MONTHS.get(m as usize - 1).copied().unwrap_or("")).to_string();
    let wd = (day + 3).rem_euclid(7) as usize;
    let day_name = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"].get(wd).copied().unwrap_or("");
    tf!("{weekday}, {day} {month} {year}", weekday = tl!(day_name), day = d, month = month, year = y)
}

/// The main pane.
pub fn show(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    // Searching: the events found, in place of the days.
    if st.cal_search.active() {
        found(s, st, ui, acts);
        return;
    }
    let t = st.tokens;
    egui::Frame::new().inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui.button(tl!("Today")).clicked() {
                acts.push(Action::Run("calendar.goto", json!({"date": "today"})));
            }
            if widgets::tiny_icon_button(ui, Icon::ChevronLeft, "Previous", &t).clicked() {
                acts.push(Action::Run("calendar.goto", json!({"step": -1})));
            }
            if widgets::tiny_icon_button(ui, Icon::ChevronRight, "Next", &t).clicked() {
                acts.push(Action::Run("calendar.goto", json!({"step": 1})));
            }
            ui.add_space(6.0);
            ui.label(RichText::new(heading(s)).font(theme::bold(18.0)).color(t.text));
            // The view, as a drop-down at the right, as Outlook's.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let views = [
                    (CalMode::Day, "Day", "day"),
                    (CalMode::WorkWeek, "Work week", "workWeek"),
                    (CalMode::Week, "Week", "week"),
                    (CalMode::Month, "Month", "month"),
                ];
                let current = views.iter().find(|(m, _, _)| *m == s.cal_mode).map_or("Week", |(_, l, _)| *l);
                egui::ComboBox::from_id_salt("calendar-view").selected_text(tl!(current)).show_ui(ui, |ui| {
                    for (mode, label, code) in views {
                        if ui.selectable_label(s.cal_mode == mode, tl!(label)).clicked() {
                            acts.push(Action::Run("calendar.view", json!({"mode": code})));
                        }
                    }
                });
            });
        });
    });
    let (d0, d1) = s.cal_days();
    let began = web_time::Instant::now();
    let items = s.agenda(s.local_midnight(d0), s.local_midnight(d1));
    let found = began.elapsed();
    match s.cal_mode {
        CalMode::Month => month(s, st, ui, &items, d0, acts),
        CalMode::Day | CalMode::WorkWeek | CalMode::Week => days(s, st, ui, &items, d0, d1, acts),
    }
    pick::enter(s, st, ui, &items, acts);
    pick::delete(s, st, ui, &items, acts);
    slow_drawing(ui, began.elapsed(), found, items.len());
    // A drag whose chip is no longer drawn (the view changed under it) is let go of.
    if st.cal_drag.is_some() && !ui.input(|i| i.pointer.primary_down()) {
        st.cal_drag = None;
    }
}

/// A frame of the calendar this long or longer is said in the log.
const SLOW_DRAWING: web_time::Duration = web_time::Duration::from_millis(50);

/// The calendar drawn slowly (`took`, of which `found` finding what it shows): said in the log,
/// once every ten seconds at most, to tell where a slow calendar spends its time.
fn slow_drawing(ui: &Ui, took: web_time::Duration, found: web_time::Duration, shown: usize) {
    if took < SLOW_DRAWING {
        return;
    }
    let id = egui::Id::unique("calendar-slow-drawing");
    let now = ui.input(|i| i.time);
    let last = ui.ctx().data(|d| d.get_temp::<f64>(id));
    if last.is_some_and(|t| now - t < 10.0) {
        return;
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, now));
    log::info!("calendar: drawn in {} ms ({} ms finding the {shown} appointments shown)", took.as_millis(), found.as_millis());
}

/// Open an item (its event).
fn open(s: &Session, item: &AgendaItem) -> Option<Action> {
    Dialog::open_event(s, &item.uid, item.occurrence).map(Action::Dialog)
}

/// The id a chip is drawn under: the same meeting can be in two calendars (two accounts', or an
/// account's and a shared mailbox's) at the same time, so it's told apart by whose it is, or
/// the click goes to neither. (Fixed, not the pane's, so a test can find the chip.)
pub(crate) fn chip_id(item: &AgendaItem) -> egui::Id {
    egui::Id::unique(("calendar-chip", &item.account, &item.calendar, &item.uid, item.start))
}

/// The id of a day's background in the Month view (fixed, so a test can find it).
pub(crate) fn month_day_id(day: i64) -> egui::Id {
    egui::Id::unique(("calendar-month-day", day))
}

/// The id of an appointment's box on `day`: [`chip_id`] on the day it starts, another on each
/// other day it's drawn on (over two days, or past midnight), so each box is clicked, and opens
/// its menu, by itself (sharing one, a right click opened none, or one per day).
fn drawn_id(item: &AgendaItem, day: i64, s: &Session) -> egui::Id {
    if day_of(item.start, s) == day { chip_id(item) } else { chip_id(item).with(day) }
}

/// An appointment in the grid, as drawn on `day`: its box (outlined when it's selected), its card
/// under the pointer, its menu on a right click. A click selects it, a right click too (then its
/// menu opens), a double click opens it, as Outlook's.
#[allow(clippy::too_many_arguments)]
fn chip(ui: &mut Ui, s: &Session, st: &mut UiState, rect: Rect, item: &AgendaItem, text: &str, day: i64, acts: &mut Vec<Action>) -> egui::Response {
    let t = st.tokens;
    let t = &t;
    // One that can move is taken hold of as well as clicked.
    let sense = if item.movable { Sense::click_and_drag() } else { Sense::click() };
    let resp = ui.interact(rect, drawn_id(item, day, s), sense);
    if resp.clicked() || resp.secondary_clicked() || resp.double_clicked() {
        st.cal_pick = Some(Pick::of(item));
    }
    if resp.double_clicked() {
        acts.extend(open(s, item));
    }
    let picked = st.cal_pick.as_ref().is_some_and(|p| p.is(item));
    let c = item_color(item, t);
    let tentative = item.needs_reply || item.status == "TENTATIVE";
    // Stronger under the pointer, with its colour round it.
    let hovered = resp.hovered();
    let fill = match (tentative, hovered) {
        (true, false) => t.surface,
        (true, true) => t.hover,
        (false, h) => c.gamma_multiply(match (t.dark, h) {
            (true, false) => 0.35,
            (true, true) => 0.5,
            (false, false) => 0.18,
            (false, true) => 0.3,
        }),
    };
    // Selected: a thicker, darker border round it (Outlook's).
    let stroke =
        if picked { egui::Stroke::new(2.0, pick::outline(c, t)) } else { egui::Stroke::new(if tentative || hovered { 1.0 } else { 0.0 }, c) };
    ui.painter().rect(rect, 4.0, fill, stroke, egui::StrokeKind::Inside);
    // How it shows the user's time, in the strip at its left, as Outlook draws it.
    strip(ui.painter(), t, Rect::from_min_size(rect.min, vec2(STRIP, rect.height())), c, strip_look(item.show_as));
    // A private one: a lock in its top right corner, the text stopping short of it.
    let locked = item.private && rect.width() >= 30.0;
    if locked {
        let lock = Rect::from_min_size(pos2(rect.max.x - 3.0 - 11.0, rect.min.y + 3.0), vec2(11.0, 11.0));
        icons::paint(ui.painter(), lock, Icon::Lock, t.dim);
    }
    // As many whole lines as the box holds, the last one ending in "…" when more is left (a line
    // cut in half by the box's edge read badly).
    let font = theme::regular(12.0);
    let row = ui.fonts_mut(|f| f.row_height(&font)).max(1.0);
    let width = (rect.width() - TEXT_X - 2.0 - if locked { 14.0 } else { 0.0 }).max(4.0);
    let rows = ((rect.height() - 4.0) / row).floor().max(1.0) as usize;
    let lay = |ui: &mut Ui, text: String, rows: usize| {
        let mut job = egui::text::LayoutJob::simple(text, font.clone(), t.text, width);
        job.wrap.max_rows = rows;
        job.wrap.overflow_character = Some('…');
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let painter = ui.painter().with_clip_rect(rect.shrink(1.0));
    let words: Vec<&str> = text.split_whitespace().collect();
    let narrow = words.iter().any(|w| ui.fonts_mut(|f| f.layout_no_wrap((*w).to_string(), font.clone(), t.text)).size().x > width);
    if narrow {
        // Too narrow for its words (appointments side by side): a word a line, cut with "…",
        // rather than broken letter by letter ("Dail / ie").
        for (i, w) in words.iter().take(rows).enumerate() {
            let more = i + 1 == rows && words.len() > rows;
            let galley = lay(ui, if more { format!("{w}…") } else { (*w).to_string() }, 1);
            painter.galley(rect.min + vec2(TEXT_X, 2.0 + row * i as f32), galley, t.text);
        }
    } else {
        painter.galley(rect.min + vec2(TEXT_X, 2.0), lay(ui, text.to_string(), rows), t.text);
    }
    // Outlook's card when the pointer rests on it: what it is, when, where, and its calendar
    // (not while something is dragged: the card would hide where it's going).
    if !resp.dragged() && !ui.input(|i| i.pointer.primary_down()) && !resp.context_menu_opened() {
        hover_card(&resp, s, t, item);
    }
    resp.context_menu(|ui| menu::menu(ui, s, item, acts));
    resp
}

/// The width of the strip at a chip's left, and where its text starts.
const STRIP: f32 = 5.0;
const TEXT_X: f32 = 8.0;

/// What an appointment is called in the grid: its subject, or "Private appointment" for a
/// private one given without it (a colleague's), rather than a blank box.
pub(crate) fn title(item: &AgendaItem) -> String {
    if item.private && item.summary.trim().is_empty() { tl!("Private appointment").to_string() } else { item.summary.clone() }
}

/// How the strip at a chip's left is drawn, by how the appointment shows the user's time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StripLook {
    /// Busy: filled in its colour.
    Solid,
    /// Tentative: hatched in its colour.
    Hatched,
    /// Free: white, outlined in its colour.
    Hollow,
    /// Out of office: purple.
    Purple,
    /// Working elsewhere: dotted in its colour.
    Dotted,
}

pub(crate) fn strip_look(s: ShowAs) -> StripLook {
    match s {
        ShowAs::Busy => StripLook::Solid,
        ShowAs::Tentative => StripLook::Hatched,
        ShowAs::Free => StripLook::Hollow,
        ShowAs::OutOfOffice => StripLook::Purple,
        ShowAs::WorkingElsewhere => StripLook::Dotted,
    }
}

/// The strip `r`, drawn its `look` in colour `c` (hatches and dots kept inside it).
fn strip(painter: &egui::Painter, t: &Tokens, r: Rect, c: Color32, look: StripLook) {
    const STEP: f32 = 4.0;
    // A strip down a whole day takes 24 * HOUR / STEP (under 300) steps: room for a zoomed one.
    const MAX_STEPS: usize = 2048;
    let inside = painter.with_clip_rect(r);
    let line = egui::Stroke::new(1.0, c);
    match look {
        StripLook::Solid => {
            painter.rect_filled(r, 2.0, c);
        }
        StripLook::Purple => {
            painter.rect_filled(r, 2.0, theme::category_color(4, t.dark));
        }
        StripLook::Hollow => {
            painter.rect(r, 2.0, t.surface, line, egui::StrokeKind::Inside);
        }
        StripLook::Hatched => {
            painter.rect_filled(r, 2.0, t.surface);
            // Rising lines, one every STEP down the strip, starting above it so its top is hatched too.
            // (At most MAX_STEPS lines: a strip stretched out of all measure stops there.)
            let mut y = r.min.y;
            for _ in 0..MAX_STEPS {
                if y >= r.max.y + r.width() {
                    break;
                }
                inside.line_segment([pos2(r.min.x, y), pos2(r.max.x, y - r.width())], line);
                y += STEP;
            }
        }
        StripLook::Dotted => {
            painter.rect_filled(r, 2.0, t.surface);
            let mut y = r.min.y + STEP / 2.0;
            for _ in 0..MAX_STEPS {
                if y >= r.max.y {
                    break;
                }
                inside.circle_filled(pos2(r.center().x, y), 1.0, c);
                y += STEP;
            }
        }
    }
}

/// The colour of an appointment: its first category's, as Outlook draws a categorized one; else
/// its calendar's.
fn item_color(item: &AgendaItem, t: &Tokens) -> egui::Color32 {
    item.category.map_or_else(|| color(item.color, t), |i| theme::category_color(i, t.dark))
}

/// What the card says, one line each: when the appointment is, where, the calendar it's in, and
/// its files (nothing for what it doesn't have). As Outlook's card, which also shows the reminder (Quatrelle
/// doesn't read `VALARM` yet).
pub(crate) fn card_rows(s: &Session, item: &AgendaItem) -> Vec<(Icon, String)> {
    let mut out = vec![(Icon::Clock, when(s, item))];
    let location = item.location.trim();
    if !location.is_empty() {
        out.push((Icon::Location, location.to_string()));
    }
    out.push((Icon::Calendar, calendar_name(s, item)));
    if let Some(m) = item.reminder {
        out.push((Icon::Reminder, reminder_text(m)));
    }
    // The files kept with it, by name (the card can't be clicked: its window opens them).
    let files = s.event_files(&item.uid, Some(item.occurrence));
    if !files.is_empty() {
        let names: Vec<&str> = files.iter().map(|f| if f.name.trim().is_empty() { "attachment" } else { f.name.as_str() }).collect();
        out.push((Icon::Attach, crate::widgets::shorten(&names.join(", "), 120)));
    }
    out
}

/// When the reminder sounds, as Outlook's card says it: `15 minutes before`, `1 hour before`.
pub(crate) fn reminder_text(m: i64) -> String {
    let n = m.unsigned_abs();
    if n == 0 {
        return tl!("When it starts").to_string();
    }
    // Days and hours when they divide it, minutes otherwise.
    let (unit, count) = if n.is_multiple_of(1440) {
        (1440, n / 1440)
    } else if n.is_multiple_of(60) {
        (60, n / 60)
    } else {
        (1, n)
    };
    match (m > 0, count == 1, unit) {
        (true, true, 1440) => tl!("1 day before").to_string(),
        (true, false, 1440) => tf!("{n} days before", n = count),
        (true, true, 60) => tl!("1 hour before").to_string(),
        (true, false, 60) => tf!("{n} hours before", n = count),
        (true, true, _) => tl!("1 minute before").to_string(),
        (true, false, _) => tf!("{n} minutes before", n = count),
        (false, true, 1440) => tl!("1 day after").to_string(),
        (false, false, 1440) => tf!("{n} days after", n = count),
        (false, true, 60) => tl!("1 hour after").to_string(),
        (false, false, 60) => tf!("{n} hours after", n = count),
        (false, true, _) => tl!("1 minute after").to_string(),
        (false, false, _) => tf!("{n} minutes after", n = count),
    }
}

/// When an appointment is, as the Day view says it: `Friday, 23 October 2026 · 12:30 – 13:30`
/// (both days for something over several, `All day` for an all-day one).
fn when(s: &Session, item: &AgendaItem) -> String {
    let (d0, d1) = (day_of(item.start, s), day_of(item.end.max(item.start + 1) - 1, s));
    let date = if d1 > d0 { tf!("{from} – {to}", from = day_text(d0), to = day_text(d1)) } else { day_text(d0) };
    let time = if item.all_day { tl!("All day").to_string() } else { format!("{} – {}", hm(item.start, s), hm(item.end, s)) };
    format!("{date} · {time}")
}

/// A local day written out, as the Day view's heading has it.
fn day_text(d: i64) -> String {
    let (y, m, dd) = civil_from_days(d);
    let wd = (d + 3).rem_euclid(7) as usize;
    let name = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"].get(wd).copied().unwrap_or("");
    tf!(
        "{weekday}, {day} {month} {year}",
        weekday = tl!(name),
        day = dd,
        month = crate::i18n::tc("month", MONTHS.get(m as usize - 1).copied().unwrap_or("")),
        year = y
    )
}

/// The calendar an appointment is in, by the name the calendar's pane gives it.
fn calendar_name(s: &Session, item: &AgendaItem) -> String {
    if item.calendar == quatrelle_engine::model::pim::LOCAL {
        return tl!("Calendar").to_string();
    }
    s.shown_calendars()
        .into_iter()
        .find(|(a, c, _, _)| *a == item.account && *c == item.calendar)
        .map(|(_, _, n, _)| n)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| tl!("Calendar").to_string())
}

/// The card itself: the title on the appointment's colour, then its lines, as Outlook's.
fn hover_card(resp: &egui::Response, s: &Session, t: &Tokens, item: &AgendaItem) {
    let c = item_color(item, t);
    let name = Some(title(item)).filter(|x| !x.trim().is_empty()).unwrap_or_else(|| tl!("(no title)").to_string());
    let rows = card_rows(s, item);
    // The tooltip's own frame is replaced by the card's: the header's band on top, square
    // corners where the body meets it.
    let frame = egui::Frame::new().fill(t.surface).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(5.0);
    let mut tip = egui::Tooltip::for_enabled(resp);
    tip.popup = tip.popup.frame(frame).gap(6.0).width(340.0);
    tip.at_pointer().show(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
        egui::Frame::new()
            .fill(c.gamma_multiply(if t.dark { 0.45 } else { 0.22 }))
            .corner_radius(egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 })
            .inner_margin(egui::Margin::symmetric(10, 7))
            .show(ui, |ui| {
                ui.set_width(300.0);
                ui.label(RichText::new(name).font(theme::bold(13.5)).color(t.text));
            });
        egui::Frame::new().corner_radius(egui::CornerRadius { nw: 0, ne: 0, sw: 4, se: 4 }).inner_margin(egui::Margin::symmetric(10, 8)).show(
            ui,
            |ui| {
                ui.set_width(300.0);
                ui.spacing_mut().item_spacing.y = 5.0;
                for (icon, text) in &rows {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                        icons::paint(ui.painter(), r, *icon, t.dim);
                        ui.add(egui::Label::new(RichText::new(text).color(t.text)).wrap());
                    });
                }
            },
        );
    });
}

fn month(s: &Session, st: &mut UiState, ui: &mut Ui, items: &[AgendaItem], d0: i64, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let t = &t;
    let (_, shown_month, _) = civil_from_days(s.cal_day());
    let now = today(s);
    let area = ui.available_rect_before_wrap().shrink2(vec2(10.0, 4.0));
    let head = 22.0;
    let cw = area.width() / 7.0;
    let rh = ((area.height() - head) / 6.0).max(60.0);
    // Dragged to another day, as long as it was, at the same time.
    let drag_area = drag::Area::Month { area, head, cw, rh, day0: d0 };
    for (i, d) in DAYS.iter().enumerate() {
        ui.painter().text(pos2(area.min.x + cw * (i as f32 + 0.5), area.min.y + 10.0), Align2::CENTER_CENTER, tl!(d), theme::regular(12.0), t.dim);
    }
    for w in 0..6 {
        for i in 0..7 {
            let d = d0 + w * 7 + i;
            let r = Rect::from_min_size(pos2(area.min.x + cw * i as f32, area.min.y + head + rh * w as f32), vec2(cw, rh));
            let bg = ui.interact(r, month_day_id(d), Sense::click());
            if bg.clicked() || bg.double_clicked() {
                st.cal_pick = Some(Pick::Day { day: d });
            }
            if st.cal_pick == Some(Pick::Day { day: d }) {
                ui.painter().rect_filled(r, 0.0, t.accent_soft);
            } else if ui.rect_contains_pointer(r) {
                ui.painter().rect_filled(r, 0.0, t.hover_row);
            }
            ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
            let (_, m, dd) = civil_from_days(d);
            let num_pos = r.min + vec2(14.0, 12.0);
            if d == now {
                ui.painter().circle_filled(num_pos, 10.0, t.accent);
            }
            ui.painter().text(
                num_pos,
                Align2::CENTER_CENTER,
                dd.to_string(),
                theme::regular(12.0),
                if d == now {
                    t.on_accent
                } else if m == shown_month {
                    t.text
                } else {
                    t.faint
                },
            );
            let todays: Vec<&AgendaItem> =
                items.iter().filter(|it| day_of(it.start, s) <= d && day_of(it.end.max(it.start + 1) - 1, s) >= d).collect();
            let fit = ((rh - 26.0) / 19.0).floor().max(0.0) as usize;
            let shown = if todays.len() > fit { fit.saturating_sub(1) } else { todays.len() };
            for (k, it) in todays.iter().take(shown).enumerate() {
                let cr = Rect::from_min_size(r.min + vec2(3.0, 24.0 + 19.0 * k as f32), vec2(cw - 6.0, 17.0));
                let text = if it.all_day { title(it) } else { format!("{} {}", hm(it.start, s), title(it)) };
                let resp = chip(ui, s, st, cr, it, &text, d, acts);
                // Only on the day it starts: a chip drawn on each day it spans would start a
                // drag each.
                if day_of(it.start, s) == d {
                    drag::chip(ui, st, s, &resp, cr, it, &drag_area, acts);
                }
            }
            if todays.len() > shown {
                let more = ui.put(
                    Rect::from_min_size(r.min + vec2(6.0, 24.0 + 19.0 * shown as f32), vec2(cw - 12.0, 17.0)),
                    egui::Link::new(RichText::new(tf!("{n} more", n = todays.len() - shown)).small()),
                );
                if more.clicked() {
                    let (y, mm, ddd) = civil_from_days(d);
                    acts.push(Action::Run("calendar.view", json!({"mode": "day"})));
                    acts.push(Action::Run("calendar.goto", json!({"date": format!("{y:04}-{mm:02}-{ddd:02}")})));
                }
            }
            if bg.double_clicked() {
                acts.push(Action::Dialog(Dialog::new_event(s, Some(s.local_midnight(d) + 9 * 3600))));
            }
        }
    }
    drag::paint(ui, st, s, t, &drag_area, 0, &|_| 0);
    ui.allocate_rect(area, Sense::hover());
}
