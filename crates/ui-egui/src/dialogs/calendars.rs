// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Add calendar › From Address Book (Outlook's Open Calendar): the address book above, and its
//! "Calendar ->" row, a contacts field taking several people. Open waits for the names still being
//! checked (`app/sending.rs`, `dialog_ok`), then opens each person's calendar: one that opened
//! leaves the field, one that couldn't stays, red, with why.
//!
//! And a calendar's menu in the side pane asks here for a new calendar's name, another name for
//! one, or whether to delete one ([`CalendarForm`]).

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

/// What's asked of a calendar from its menu in the side pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CalendarAsk {
    /// A new calendar in the account.
    New,
    /// This calendar's new name.
    Rename { calendar: String },
    /// Whether to delete this calendar.
    Delete { calendar: String },
}

/// A calendar made, renamed or deleted: the account, what's asked, the name typed (the
/// calendar's own when it's deleted) and what the server or the engine said against it.
#[derive(Clone, Debug)]
pub struct CalendarForm {
    pub account: String,
    pub ask: CalendarAsk,
    pub name: String,
    pub error: Option<String>,
}

impl CalendarForm {
    /// A question to answer before going on (read as an alert).
    pub fn deleting(&self) -> bool {
        matches!(self.ask, CalendarAsk::Delete { .. })
    }
}

impl Dialog {
    pub fn new_calendar(account: &str) -> Dialog {
        Dialog::CalendarChange(Box::new(CalendarForm { account: account.to_string(), ask: CalendarAsk::New, name: String::new(), error: None }))
    }

    /// Rename `c`, its name as the server gave it made plain to start from.
    pub fn rename_calendar(account: &str, c: &quatrelle_engine::model::pim::CalendarInfo) -> Dialog {
        let mut name = quatrelle_engine::model::pim::plain_name(&c.name);
        if let Some((cut, _)) = name.char_indices().nth(quatrelle_engine::model::pim::MAX_CALENDAR_NAME) {
            name.truncate(cut);
        }
        let ask = CalendarAsk::Rename { calendar: c.id.clone() };
        Dialog::CalendarChange(Box::new(CalendarForm { account: account.to_string(), ask, name, error: None }))
    }

    /// Whether to delete `c`, named on one line as the side pane shows it.
    pub fn delete_calendar(account: &str, c: &quatrelle_engine::model::pim::CalendarInfo) -> Dialog {
        let name = quatrelle_engine::model::pim::teams::one_line(&c.name);
        let ask = CalendarAsk::Delete { calendar: c.id.clone() };
        Dialog::CalendarChange(Box::new(CalendarForm { account: account.to_string(), ask, name, error: None }))
    }
}

/// New calendar or Rename calendar (its name, one line), or Delete calendar (a question).
pub(super) fn draw_change(ui: &mut Ui, f: &mut CalendarForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    if let CalendarAsk::Delete { calendar } = &f.ask {
        ui.set_width(380.0);
        title(ui, t, "Delete calendar");
        let text = tf!("Delete the calendar {name} and its events? It goes to Deleted Items.", name = f.name);
        ui.add(egui::Label::new(RichText::new(text).color(t.dim)).wrap());
        if let Some(e) = &f.error {
            ui.label(RichText::new(e.as_str()).color(t.danger).small());
        }
        let (ok, cancel) = buttons(ui, t, "Delete", true);
        if ok {
            f.error = None;
            acts.push(Action::Submit("calendar.delete", json!({"account": f.account, "calendar": calendar})));
        }
        return !cancel;
    }
    let renaming = matches!(f.ask, CalendarAsk::Rename { .. });
    ui.set_width(360.0);
    title(ui, t, if renaming { "Rename calendar" } else { "New calendar" });
    if !renaming {
        let who = s.account(&f.account).map(|a| a.config.email.clone()).unwrap_or_default();
        ui.label(RichText::new(tf!("Made in {account}", account = who)).color(t.dim).small());
        ui.add_space(4.0);
    }
    let edit = egui::TextEdit::singleline(&mut f.name)
        .hint_text(tl!("Calendar name"))
        .char_limit(quatrelle_engine::model::pim::MAX_CALENDAR_NAME)
        .desired_width(f32::INFINITY);
    let r = ui.add(edit);
    if ui.memory(|m| m.focused().is_none()) {
        r.request_focus();
    }
    // One field: Enter means OK wherever the focus is.
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    if let Some(e) = &f.error {
        ui.label(RichText::new(e.as_str()).color(t.danger).small());
    }
    let ready = !f.name.trim().is_empty();
    let (ok, cancel) = buttons(ui, t, if renaming { "Rename" } else { "Create" }, ready);
    if (ok || enter) && ready {
        f.error = None;
        let name = f.name.trim();
        match &f.ask {
            CalendarAsk::Rename { calendar } => {
                acts.push(Action::Submit("calendar.rename", json!({"account": f.account, "calendar": calendar, "name": name})));
            }
            _ => acts.push(Action::Submit("calendar.create", json!({"account": f.account, "name": name}))),
        }
    }
    !cancel
}

impl Dialog {
    /// Add calendar › From Address Book: the address book, and the people whose calendars to open.
    pub fn open_calendars() -> Dialog {
        Dialog::OpenSharedCalendar(Box::new(CalendarsForm { search: AddressSearch::new(), ..Default::default() }))
    }
}
