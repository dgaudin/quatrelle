//! Outlook's Select Attendees and Resources: the address book over a meeting's window, opened
//! from its Required, Optional and Rooms buttons, to add people to those lines and rooms to its
//! resources. Its rows are the contacts field (`att-required`, `att-optional`, `att-rooms`), each
//! refusing what it doesn't take.

use quatrelle_engine::model::{EmailAddress, addr};

use super::*;
use crate::dialogs::people_field::{ATTENDEE_FIELDS, MEETING_WINDOWS, PeopleFields, accepts_of, egui_id, meeting_field};
use crate::dialogs::{AddressSearch, names};
use crate::look::people_field::label_id;

/// A line of the meeting the dialog fills.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Line {
    #[default]
    Required,
    Optional,
    Resources,
}

impl Line {
    /// The line's field in the meeting's window.
    fn name(self) -> &'static str {
        match self {
            Line::Required => "required",
            Line::Optional => "optional",
            Line::Resources => "rooms",
        }
    }

    /// Its row in the dialog.
    fn row(self) -> &'static str {
        match self {
            Line::Required => "att-required",
            Line::Optional => "att-optional",
            Line::Resources => "att-rooms",
        }
    }
}

/// The button of a meeting's line, in the window numbered `window`: the label that names its
/// field (the tests find it by it).
pub fn line_button_id(window: u64, line: Line) -> egui::Id {
    label_id(egui_id(meeting_field(window, line.name())))
}

/// The dialog's state while it's open over the meeting's window.
#[derive(Clone, Debug, Default)]
pub struct AttendeesUi {
    pub search: AddressSearch,
    /// The line a double-click adds the person picked to: the one it was opened from.
    pub target: Line,
    /// The rows' texts (`Name <address>; …`), as their fields write them.
    pub required: String,
    pub optional: String,
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

    /// OK: the lines go back to the meeting, the rooms booked being those its Resources row
    /// books (`people`, the rows' fields; the place follows them as when they're booked in the
    /// Room Finder). `false`: text left in Resources isn't an address, and nothing changed.
    pub(crate) fn apply(&mut self, f: &mut EventForm, people: &PeopleFields) -> bool {
        if let Err(bad) = addr::parse_list(&self.resources) {
            self.error = Some(tf!("Not an address: {what}", what = bad));
            return false;
        }
        let tidy = |line: &str| line.trim().trim_end_matches([';', ',']).trim().to_string();
        f.guests = tidy(&self.required);
        f.optional = tidy(&self.optional);
        f.book_only(&people.addressees(meeting_field(f.window, Line::Resources.row())));
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

/// The rows' buttons, as wide as the longest label as it reads in the language shown.
fn button_width(ui: &Ui, labels: &[&str]) -> f32 {
    let font = theme::regular(13.0);
    let widest =
        labels.iter().map(|l| ui.painter().layout_no_wrap(tl!(l).to_string(), font.clone(), egui::Color32::PLACEHOLDER).size().x).fold(0.0, f32::max);
    (widest + 24.0).max(LABEL_W)
}

/// The dialog over the meeting's window (numbered `window`, its account `account`): the address
/// book, and the Required, Optional and Resources rows under it, each a contacts field with its
/// button adding the person picked, as Outlook has them. OK waits while a row holds what it
/// doesn't take. `Some(true)`: OK; `Some(false)`: cancelled.
#[allow(clippy::too_many_arguments)]
pub(super) fn dialog(
    ctx: &egui::Context,
    st: &mut AttendeesUi,
    s: &Session,
    people: &mut PeopleFields,
    t: &Tokens,
    window: u64,
    account: &str,
    acts: &mut Vec<Action>,
) -> Option<bool> {
    let id = egui::Id::unique(("select-attendees", window));
    let can_expand = s.directory_account(Some(account)).is_some();
    let mut closed = None;
    egui::Modal::new(id).frame(crate::dialogs::dialog_frame(t)).show(ctx, |ui| {
        ui.set_width(660.0);
        ui.label(RichText::new(tl!("Select Attendees and Resources")).font(theme::bold(18.0)).color(t.text));
        ui.add_space(6.0);
        let mut add = names::search(ui, id, &mut st.search, t, acts).then_some(st.target);
        ui.add_space(8.0);
        let rows = [(Line::Required, "Required ->"), (Line::Optional, "Optional ->"), (Line::Resources, "Resources ->")];
        let w = button_width(ui, &rows.map(|(_, label)| label));
        for (line, label) in rows {
            let field = meeting_field(window, line.row());
            let before = (line == Line::Resources).then(|| st.resources.clone());
            ui.horizontal_top(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(w, FIELD_H - 4.0), Sense::hover());
                if label_button(ui, t, r, label_id(egui_id(field)), label, "").clicked() {
                    add = Some(line);
                }
                ui.add_space(8.0);
                field_column(ui, |ui| {
                    people.draw(ui, t, field, Some(account), accepts_of(field.name), "", "", st.line(line), false, can_expand, false, acts);
                });
            });
            ui.add_space(4.0);
            if before.is_some_and(|b| b != st.resources) {
                st.error = None;
            }
        }
        if let (Some(line), Some(a)) = (add, st.search.picked().cloned()) {
            let field = meeting_field(window, line.row());
            people.add(field, s.known_addressee(Some(account), a));
            if line == Line::Resources {
                st.error = None;
            }
        }
        if let Some(e) = &st.error {
            ui.label(RichText::new(e).color(t.danger));
        }
        let blocked = ATTENDEE_FIELDS.iter().any(|&name| people.blocked(meeting_field(window, name)));
        let (ok, cancel) = crate::dialogs::buttons(ui, t, "OK", !blocked);
        if ok {
            closed = Some(true);
        } else if cancel {
            closed = Some(false);
        }
    });
    // A name several people fit, in a row: which one, over the dialog.
    people.choice_dialog(ctx, t, s, MEETING_WINDOWS | window, &ATTENDEE_FIELDS, acts);
    // Escape: as Cancel (a click beside it leaves it open, what's typed in it kept).
    if closed.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        closed = Some(false);
    }
    closed
}
