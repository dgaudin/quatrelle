// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What's selected in the calendar's grid, as Outlook's: a click selects an appointment or an
//! empty half hour, a drag selects a stretch of them, a double click opens it (or a new
//! appointment there), Enter opens the appointment selected or a meeting over the stretch,
//! Delete deletes the appointment (a series asks which first); in the Month view a click selects
//! a day. One thing at a time; one that's no longer shown just isn't drawn.

use super::*;

/// The appointment, the stretch of half hours or the day selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    /// An appointment, told apart as its chip is ([`chip_id`]): whose calendar, its uid, when
    /// this occurrence starts.
    Item { account: String, calendar: String, uid: String, start: i64 },
    /// One or more empty half hours of the Day and Week views: the day, the first half hour
    /// (0 to 47), the last one selected (the same for one half hour) and the calendar's pane
    /// (side by side; 0 otherwise).
    Slot { day: i64, half: i64, last: i64, pane: usize },
    /// A day of the Month view, clicked on its background.
    Day { day: i64 },
    /// Several days of the Month view, dragged out from the one pressed to the one under the
    /// pointer, in reading order (both included).
    Days { first: i64, last: i64 },
}

/// Days being dragged out in the Month view, from the one pressed (`anchor`) to the one under
/// the pointer.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DayDrag {
    pub(crate) anchor: i64,
}

/// A stretch of half hours being dragged out, from the half hour pressed (`anchor`) to the one
/// under the pointer: it stays in the day and pane where it began.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SlotDrag {
    pub(crate) day: i64,
    pub(crate) pane: usize,
    pub(crate) anchor: i64,
}

impl Pick {
    pub fn of(item: &AgendaItem) -> Pick {
        Pick::Item { account: item.account.clone(), calendar: item.calendar.clone(), uid: item.uid.clone(), start: item.start }
    }

    /// Whether this is `item`.
    pub fn is(&self, item: &AgendaItem) -> bool {
        matches!(self, Pick::Item { account, calendar, uid, start }
            if *account == item.account && *calendar == item.calendar && *uid == item.uid && *start == item.start)
    }
}

/// The border round the appointment selected: its colour darker (lighter on a dark window, where
/// darker would hardly show).
pub(super) fn outline(c: Color32, t: &Tokens) -> Color32 {
    let [r, g, b, _] = c.to_array();
    let shade = |x: u8| if t.dark { x.saturating_add(((255 - u16::from(x)) * 2 / 5) as u8) } else { (u16::from(x) * 3 / 5) as u8 };
    Color32::from_rgb(shade(r), shade(g), shade(b))
}

/// Enter opens the appointment selected, or a meeting over the stretch of half hours or days
/// selected, when it's shown (not while a dialog is open or text is typed).
pub(super) fn enter(s: &Session, st: &UiState, ui: &mut Ui, items: &[AgendaItem], acts: &mut Vec<Action>) {
    if let Some(item) = pressed(st, ui, items, Key::Enter) {
        acts.extend(open(s, item));
        return;
    }
    if st.dialog.is_some() || ui.ctx().text_edit_focused() {
        return;
    }
    let Some(meeting) = new_meeting(s, st.cal_pick.as_ref()) else { return };
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) {
        acts.push(meeting);
    }
}

/// The meeting Enter opens over the stretch of half hours or the days selected, when the view
/// still shows it: an all-day meeting over the days, in their calendar.
pub(crate) fn new_meeting(s: &Session, pick: Option<&Pick>) -> Option<Action> {
    match pick {
        Some(Pick::Slot { day, half, last, pane }) if slot_shown(s, *day, *pane) => {
            let cals = if s.prefs.calendars_side_by_side { s.shown_calendars() } else { Vec::new() };
            let cal = cals.get(*pane).filter(|_| cals.len() > 1);
            let (first, last) = ((*half).min(*last), (*half).max(*last));
            let midnight = s.local_midnight(*day);
            Some(days::new_event_in(s, midnight + first * 1800, midnight + (last + 1) * 1800, true, cal))
        }
        Some(Pick::Day { day }) if days_shown(s, *day, *day) => Some(days::new_all_day_in(s, *day, *day, true, None)),
        Some(Pick::Days { first, last }) if days_shown(s, *first, *last) => {
            let (first, last) = ((*first).min(*last), (*first).max(*last));
            Some(days::new_all_day_in(s, first, last, true, None))
        }
        _ => None,
    }
}

/// Whether the Month view still shows the days `first` to `last`.
fn days_shown(s: &Session, first: i64, last: i64) -> bool {
    if s.cal_mode != CalMode::Month {
        return false;
    }
    let (d0, d1) = s.cal_days();
    first.min(last) >= d0 && first.max(last) < d1
}

/// Whether the Day, Work week or Week view still shows `day` in the calendar's `pane`.
fn slot_shown(s: &Session, day: i64, pane: usize) -> bool {
    if s.cal_mode == CalMode::Month {
        return false;
    }
    let (d0, d1) = s.cal_days();
    if day < d0 || day >= d1 {
        return false;
    }
    let cals = if s.prefs.calendars_side_by_side { s.shown_calendars() } else { Vec::new() };
    pane < if cals.len() > 1 { cals.len() } else { 1 }
}

/// Delete deletes the appointment selected, as the menu's Delete does, where the menu would
/// offer it ([`menu::changeable`]): a series asks first which, this occurrence or all of it.
/// (The selection goes once it's deleted: `event.delete` clears it.)
pub(super) fn delete(s: &Session, st: &UiState, ui: &mut Ui, items: &[AgendaItem], acts: &mut Vec<Action>) {
    let Some(item) = pressed(st, ui, items, Key::Delete).filter(|i| menu::changeable(s, i)) else { return };
    if item.recurring {
        let (uid, occurrence, name) = (item.uid.clone(), item.occurrence, title(item));
        acts.push(Action::Dialog(Dialog::DeleteOccurrence { uid, occurrence, name }));
    } else {
        acts.push(Action::Run("event.delete", json!({"uid": item.uid})));
    }
}

/// The appointment selected, shown, when `key` is pressed on it (not while a dialog is open or
/// text is typed): the key is taken.
fn pressed<'a>(st: &UiState, ui: &mut Ui, items: &'a [AgendaItem], key: Key) -> Option<&'a AgendaItem> {
    let pick = st.cal_pick.as_ref().filter(|p| matches!(p, Pick::Item { .. }))?;
    if st.dialog.is_some() || ui.ctx().text_edit_focused() {
        return None;
    }
    let item = items.iter().find(|i| pick.is(i))?;
    ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key)).then_some(item)
}

/// The new appointment Ctrl+N opens, as Outlook's: over the stretch of half hours selected (in
/// its calendar, side by side, as a double click there), on the day selected at 9:00 (as a
/// double click on it), else as the ribbon's New event.
pub(crate) fn new_event(s: &Session, pick: Option<&Pick>) -> Action {
    match pick {
        Some(Pick::Slot { day, half, last, pane }) if slot_shown(s, *day, *pane) => {
            let cals = if s.prefs.calendars_side_by_side { s.shown_calendars() } else { Vec::new() };
            let cal = cals.get(*pane).filter(|_| cals.len() > 1);
            let (first, last) = ((*half).min(*last), (*half).max(*last));
            let midnight = s.local_midnight(*day);
            days::new_event_in(s, midnight + first * 1800, midnight + (last + 1) * 1800, false, cal)
        }
        Some(Pick::Days { first, last }) if days_shown(s, *first, *last) => {
            let (first, last) = ((*first).min(*last), (*first).max(*last));
            if first == last {
                Action::Dialog(Dialog::new_event(s, Some(s.local_midnight(first) + 9 * 3600)))
            } else {
                days::new_all_day_in(s, first, last, false, None)
            }
        }
        Some(Pick::Day { day }) => Action::Dialog(Dialog::new_event(s, Some(s.local_midnight(*day) + 9 * 3600))),
        _ => Action::Dialog(Dialog::new_event(s, None)),
    }
}
