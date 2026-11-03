// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What's selected in the calendar's grid, as Outlook's: a click selects an appointment or an
//! empty half hour, a double click opens it (or a new appointment there), Enter opens the
//! appointment selected, Delete deletes it (a series asks which first); in the Month view a click
//! selects a day. One thing at a time; one that's no longer shown just isn't drawn.

use super::*;

/// The appointment, the half hour or the day selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    /// An appointment, told apart as its chip is ([`chip_id`]): whose calendar, its uid, when
    /// this occurrence starts.
    Item { account: String, calendar: String, uid: String, start: i64 },
    /// An empty half hour of the Day and Week views: the day, the half hour (0 to 47) and the
    /// calendar's pane (side by side; 0 otherwise).
    Slot { day: i64, half: i64, pane: usize },
    /// A day of the Month view, clicked on its background.
    Day { day: i64 },
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

/// Enter opens the appointment selected, when it's shown (not while a dialog is open or text is
/// typed).
pub(super) fn enter(s: &Session, st: &UiState, ui: &mut Ui, items: &[AgendaItem], acts: &mut Vec<Action>) {
    if let Some(item) = pressed(st, ui, items, Key::Enter) {
        acts.extend(open(s, item));
    }
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

/// The new appointment Ctrl+N opens, as Outlook's: at the half hour selected (in its calendar,
/// side by side, as a double click there), on the day selected at 9:00 (as a double click on
/// it), else as the ribbon's New event.
pub(crate) fn new_event(s: &Session, pick: Option<&Pick>) -> Action {
    match pick {
        Some(Pick::Slot { day, half, pane }) => {
            let cals = if s.prefs.calendars_side_by_side { s.shown_calendars() } else { Vec::new() };
            days::new_event_in(s, s.local_midnight(*day) + half * 1800, cals.get(*pane).filter(|_| cals.len() > 1))
        }
        Some(Pick::Day { day }) => Action::Dialog(Dialog::new_event(s, Some(s.local_midnight(*day) + 9 * 3600))),
        _ => Action::Dialog(Dialog::new_event(s, None)),
    }
}
