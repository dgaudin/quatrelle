// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What's selected in the calendar's grid, as Outlook's: a click selects an appointment or an
//! empty half hour, a double click opens it (or a new appointment there), Enter opens the
//! appointment selected. One thing at a time; one that's no longer shown just isn't drawn.

use super::*;

/// The appointment or the half hour selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    /// An appointment, told apart as its chip is ([`chip_id`]): whose calendar, its uid, when
    /// this occurrence starts.
    Item { account: String, calendar: String, uid: String, start: i64 },
    /// An empty half hour of the Day and Week views: the day, the half hour (0 to 47) and the
    /// calendar's pane (side by side; 0 otherwise).
    Slot { day: i64, half: i64, pane: usize },
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
    let Some(pick) = st.cal_pick.as_ref().filter(|p| matches!(p, Pick::Item { .. })) else { return };
    if st.dialog.is_some() || ui.ctx().text_edit_focused() {
        return;
    }
    if !ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) {
        return;
    }
    if let Some(item) = items.iter().find(|i| pick.is(i)) {
        acts.extend(open(s, item));
    }
}
