//! Add calendar › From Address Book (Outlook's Open Calendar): the address book above, and its
//! "Calendar ->" row, a contacts field taking several people. Open waits for the names still being
//! checked (`app/sending.rs`, `dialog_ok`), then opens each person's calendar: one that opened
//! leaves the field, one that couldn't stays, red, with why.

use quatrelle_engine::model::Addressee;

use super::names::{self, AddressSearch};
use super::people_field::{DIALOG_WINDOWS, PeopleFields, accepts_of, dialog_field, egui_id};
use super::*;
use crate::look::people_field::input_id;

/// The address book, and the people whose calendars to open (the field's text).
#[derive(Clone, Debug, Default)]
pub struct CalendarsForm {
    pub search: AddressSearch,
    pub text: String,
    /// What the wait came to when it isn't about one entry (the directory not answering).
    pub error: Option<String>,
}

pub(super) fn draw(ui: &mut Ui, f: &mut CalendarsForm, s: &Session, people: &mut PeopleFields, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(620.0);
    title(ui, t, "Open a shared calendar");
    ui.add(
        egui::Label::new(
            RichText::new(tl!("Whose calendar? It opens read-only beside yours, when they've shared it with you in Exchange.")).color(t.dim),
        )
        .wrap(),
    );
    ui.add_space(8.0);
    let mut add = names::search(ui, egui::Id::unique("add-calendar"), &mut f.search, t, acts);
    ui.add_space(8.0);
    let field = dialog_field(0, "calendar");
    let account = s.directory_account(None);
    // Enter in the field's input goes for Open; one its list took (a suggestion) is gone by now.
    let had = ui.memory(|m| m.has_focus(input_id(egui_id(field))));
    let drawn =
        people.draw(ui, t, field, account.as_deref(), accepts_of(field.name), "Calendar ->", "", &mut f.text, false, account.is_some(), true, acts);
    let enter = had && ui.input(|i| i.key_pressed(egui::Key::Enter));
    // The person picked in the book goes in, as Select Names' `To ->` does.
    add |= drawn.book;
    if add && let Some(a) = f.search.picked() {
        people.add(field, Addressee::from(a.clone()));
    }
    people.choice_dialog(ui.ctx(), t, s, DIALOG_WINDOWS, &["calendar"], acts);
    if let Some(e) = &f.error {
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(e.as_str()).color(t.danger).small()).wrap());
    }
    let held = people.holding(DIALOG_WINDOWS);
    if held {
        ui.add_space(4.0);
        ui.label(RichText::new(tl!("Checking names…")).color(t.faint).small());
    }
    let something = people.pieces(field).iter().any(|p| !p.is_red()) || !people.typing(field).trim().is_empty();
    let ready = something && !held;
    let (ok, cancel) = buttons(ui, t, "Open", ready);
    if (ok || enter) && ready {
        acts.push(Action::DialogOk);
    }
    !cancel
}
