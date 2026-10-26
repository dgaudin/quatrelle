//! Outlook's Select Attendees and Resources: the address book over a meeting's window, opened
//! from its Required and Optional buttons, to add people to those lines and rooms to its
//! resources.

use quatrelle_engine::model::{EmailAddress, addr};

use super::*;
use crate::dialogs::{AddressSearch, names};

/// A line of the meeting the dialog fills.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Line {
    #[default]
    Required,
    Optional,
    Resources,
}

/// The button of a meeting's line, in the window numbered `window`: the tests find it by it.
pub fn line_button_id(window: u64, line: Line) -> egui::Id {
    egui::Id::unique(("meeting-line-button", window, line as u8))
}

/// The dialog's state while it's open over the meeting's window.
#[derive(Clone, Debug, Default)]
pub struct AttendeesUi {
    pub search: AddressSearch,
    /// The line a double-click adds the person picked to: the one it was opened from.
    pub target: Line,
    pub required: String,
    pub optional: String,
    /// The rooms, as typed (`Name <address>; …`).
    pub resources: String,
    /// Resources that aren't addresses, when OK was pressed.
    pub error: Option<String>,
}

impl AttendeesUi {
    /// Opened from a line, on the meeting's lines as they are.
    pub fn open(f: &EventForm, target: Line) -> AttendeesUi {
        // A room known by its address alone is written so.
        let rooms: Vec<EmailAddress> =
            f.rooms.iter().map(|(name, email)| EmailAddress::new(if name == email { String::new() } else { name.clone() }, email.clone())).collect();
        AttendeesUi {
            search: AddressSearch::new(),
            target,
            required: f.guests.clone(),
            optional: f.optional.clone(),
            resources: addr::format_list(&rooms),
            error: None,
        }
    }

    fn line(&mut self, line: Line) -> &mut String {
        match line {
            Line::Required => &mut self.required,
            Line::Optional => &mut self.optional,
            Line::Resources => &mut self.resources,
        }
    }

    /// OK: the lines go back to the meeting, the rooms booked being those in Resources (the
    /// place follows them as when they're booked in the Room Finder). `false`: a resource isn't
    /// an address, and nothing changed.
    pub fn apply(&mut self, f: &mut EventForm) -> bool {
        let rooms = match addr::parse_list(&self.resources) {
            Ok(rooms) => rooms,
            Err(bad) => {
                self.error = Some(tf!("Not an address: {what}", what = bad));
                return false;
            }
        };
        let tidy = |line: &str| line.trim().trim_end_matches([';', ',']).trim().to_string();
        f.guests = tidy(&self.required);
        f.optional = tidy(&self.optional);
        for (_, email) in f.rooms.clone() {
            if !rooms.iter().any(|r| r.email.eq_ignore_ascii_case(&email)) {
                f.remove_room(&email);
            }
        }
        for r in rooms {
            f.add_room(&r.name, &r.email);
        }
        true
    }
}

impl EventForm {
    /// Outlook's Add Attendees: the address book over the window, a double-click adding to
    /// Required.
    pub fn select_attendees(&mut self) {
        self.attendees_book = Some(Box::new(AttendeesUi::open(self, Line::Required)));
    }
}

/// The dialog over the meeting's window (numbered `window`): the address book, and the
/// Required, Optional and Resources lines under it, each with its button adding the person
/// picked, as Outlook has them. `Some(true)`: OK; `Some(false)`: cancelled.
pub(super) fn dialog(ctx: &egui::Context, st: &mut AttendeesUi, t: &Tokens, window: u64, acts: &mut Vec<Action>) -> Option<bool> {
    let id = egui::Id::unique(("select-attendees", window));
    let mut closed = None;
    egui::Modal::new(id).frame(crate::dialogs::dialog_frame(t)).show(ctx, |ui| {
        ui.set_width(660.0);
        ui.label(RichText::new(tl!("Select Attendees and Resources")).font(theme::bold(18.0)).color(t.text));
        ui.add_space(6.0);
        let mut add = names::search(ui, id, &mut st.search, t, acts).then_some(st.target);
        ui.add_space(8.0);
        for (line, label) in [(Line::Required, "Required ->"), (Line::Optional, "Optional ->"), (Line::Resources, "Resources ->")] {
            if names::field_row(ui, label, st.line(line)) {
                add = Some(line);
            }
        }
        if let (Some(line), Some(a)) = (add, st.search.picked().cloned()) {
            names::add_to(st.line(line), &a);
            if line == Line::Resources {
                st.error = None;
            }
        }
        if let Some(e) = &st.error {
            ui.label(RichText::new(e).color(t.danger));
        }
        let (ok, cancel) = crate::dialogs::buttons(ui, t, "OK", true);
        if ok {
            closed = Some(true);
        } else if cancel {
            closed = Some(false);
        }
    });
    // Escape: as Cancel (a click beside it leaves it open, what's typed in it kept).
    if closed.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        closed = Some(false);
    }
    closed
}
