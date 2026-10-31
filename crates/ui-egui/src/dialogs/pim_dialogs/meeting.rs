//! An appointment or a meeting in a window of its own, as Outlook opens one: a ribbon (the
//! Appointment or Meeting tab, and the Scheduling Assistant), then its title, guests, times and
//! place, and its notes taking the rest of the window. The form is the dialog's ([`EventForm`]);
//! only where it's shown differs. Its Required, Optional and Rooms lines are the contacts field
//! (`required`, `optional`, `rooms` of the window, made ready by [`prepare`] before it's drawn).

use quatrelle_engine::model::ical::ShowAs;

use super::event::{reminder_choice, repeat_options};
use super::*;
use crate::dialogs::people_field::{MEETING_FIELDS, MEETING_WINDOWS, PeopleFields, accepts_of, meeting_field};
use crate::widgets::Groups;

/// The window's contacts fields made ready before it's drawn (where the session can be changed:
/// the window only reads it). Its account's rooms are asked once, so that a room is known as one
/// in every line. Required, Optional and Rooms are read from the form's texts; a room entered
/// among the guests is booked, with a word under its line. While Select Attendees and Resources
/// is open, its rows too, a room among their guests going to its Resources.
pub(crate) fn prepare(f: &mut EventForm, s: &mut Session, people: &mut PeopleFields) {
    if !f.can_invite {
        return;
    }
    let account = f.account_id();
    if s.rooms_now(&account).is_none() {
        let _ = s.request_rooms(&account, false);
    }
    let a = Some(account.as_str());
    let (required, optional) = (meeting_field(f.window, "required"), meeting_field(f.window, "optional"));
    people.prepare(s, required, a, accepts_of("required"), &f.guests);
    people.prepare(s, optional, a, accepts_of("optional"), &f.optional);
    for field in [required, optional] {
        for who in people.take_moved(field) {
            f.add_room(&who.name, &who.address);
            people.say(field, moved_note(&who));
        }
    }
    people.prepare(s, meeting_field(f.window, "rooms"), a, accepts_of("rooms"), &f.rooms_line);
    let Some(st) = f.attendees_book.as_deref() else { return };
    let rows = [("att-required", &st.required), ("att-optional", &st.optional), ("att-rooms", &st.resources)];
    for (name, text) in rows {
        people.prepare(s, meeting_field(f.window, name), a, accepts_of(name), text);
    }
    let resources = meeting_field(f.window, "att-rooms");
    for field in [meeting_field(f.window, "att-required"), meeting_field(f.window, "att-optional")] {
        for who in people.take_moved(field) {
            people.say(field, moved_note(&who));
            people.add(resources, who);
        }
    }
}

/// What a line says of a room taken out of it.
fn moved_note(who: &quatrelle_engine::model::Addressee) -> String {
    tf!("{room} was moved to Rooms", room = who.display())
}

/// The window's content. `false` once it's done with from its ribbon (Close, for one that can't
/// be changed); saving, sending and deleting close it once the engine has done them. `people`
/// draws its contacts fields, made ready by [`prepare`].
pub fn meeting(ui: &mut Ui, f: &mut EventForm, s: &Session, people: &mut PeopleFields, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    tabs(ui, t, f);
    ui.add_space(2.0);
    let mut keep = true;
    egui::ScrollArea::horizontal().id_salt("meeting-ribbon").show(ui, |ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            keep = ribbon(ui, t, f, &s.prefs.webex_room, acts);
        });
    });
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.border);
    ui.add_space(8.0);
    // Outlook's Appointment Recurrence, over the window while it's open.
    if let Some(st) = f.recurrence.as_mut() {
        match super::recurrence::dialog(ui.ctx(), st, t, f.window, dates::today(&s.clock)) {
            Some(super::recurrence::Closed::Rule(r)) => {
                // Outlook moves the appointment to the first day the repeat falls on.
                if let Some(day) = super::pickers::day_of(&f.date) {
                    let shift = r.first_from(day) - day;
                    if shift != 0 {
                        f.date = super::pickers::text_of(day + shift);
                        if let Some(end) = super::pickers::day_of(&f.end_date) {
                            f.end_date = super::pickers::text_of(end + shift);
                        }
                    }
                }
                f.repeat = r.rule();
                f.recurrence = None;
            }
            Some(super::recurrence::Closed::Removed) => {
                f.repeat.clear();
                f.recurrence = None;
            }
            Some(super::recurrence::Closed::Cancelled) => f.recurrence = None,
            None => {}
        }
    }
    // Outlook's Select Attendees and Resources, over the window while it's open.
    if let Some(mut st) = f.attendees_book.take() {
        let account = f.account_id();
        match super::attendees::dialog(ui.ctx(), &mut st, s, people, t, f.window, &account, acts) {
            // People added make it a meeting (from the Scheduling Assistant, an appointment).
            Some(true) if st.apply(f, people) => f.inviting |= !f.guests.is_empty() || !f.optional.is_empty(),
            Some(false) => {}
            _ => f.attendees_book = Some(st),
        }
    }
    if f.scheduling && !f.all_day {
        super::scheduling::view(ui, f, s, t, acts);
        ui.add_space(8.0);
        // Outlook's assistant keeps the times under it.
        ui.add_enabled_ui(!f.read_only, |ui| times(ui, t, f, s));
    } else {
        header(ui, t, f, s, people, acts);
        body(ui, t, f, acts);
    }
    // A name several people fit in one of its lines: which one, over the window.
    if f.attendees_book.is_none() {
        people.choice_dialog(ui.ctx(), t, s, MEETING_WINDOWS | f.window, &MEETING_FIELDS, acts);
    }
    keep
}

/// The two tabs: the appointment (a meeting once guests are invited) and the Scheduling
/// Assistant (for one with times).
fn tabs(ui: &mut Ui, t: &Tokens, f: &mut EventForm) {
    ui.horizontal(|ui| {
        let first = if f.inviting { "Meeting" } else { "Appointment" };
        for (assistant, label) in [(false, first), (true, "Scheduling Assistant")] {
            if assistant && f.all_day {
                continue;
            }
            let on = f.scheduling == assistant;
            let text =
                RichText::new(tl!(label)).color(if on { t.accent } else { t.dim }).font(if on { theme::bold(13.5) } else { theme::regular(13.5) });
            let r = ui.add(egui::Button::new(text).frame_when_inactive(false));
            if on {
                let rect = r.rect;
                ui.painter().line_segment(
                    [egui::pos2(rect.left() + 6.0, rect.bottom() + 1.0), egui::pos2(rect.right() - 6.0, rect.bottom() + 1.0)],
                    egui::Stroke::new(2.0, t.accent),
                );
            }
            if r.clicked() {
                f.scheduling = assistant;
                f.sched_day = None;
            }
        }
    });
}

/// Ask the engine to keep it (a meeting's guests are sent the invitation or the update).
fn submit(f: &mut EventForm, acts: &mut Vec<Action>) {
    f.error = None;
    acts.push(Action::Submit(if f.uid.is_some() { "event.update" } else { "event.create" }, f.params()));
}

/// The Webex personal room in the appointment, as Webex's own add-in puts it: as the place (one
/// typed is kept) and at the end of the notes, for the guests to join by.
fn add_webex(f: &mut EventForm, url: &str) {
    if f.location.trim().is_empty() {
        f.location = url.to_string();
    }
    if !f.description.contains(url) {
        let notes = f.description.trim_end();
        let gap = if notes.is_empty() { "" } else { "\n\n" };
        f.description = format!("{notes}{gap}{}\n{url}\n", tl!("Join the Webex meeting:"));
    }
    f.join = Some(url.to_string());
}

/// The Webex personal room asked for under Webex Meeting, the first time (the window is
/// `window`): its link, once one is given that is a personal room's.
fn webex_room(ui: &mut Ui, t: &Tokens, window: u64) -> Option<String> {
    let id = egui::Id::unique(("webex-room-typed", window));
    let mut typed: String = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    ui.set_width(400.0);
    ui.label(RichText::new(tl!("Your Webex personal room")).font(theme::bold(13.0)).color(t.text));
    ui.label(
        RichText::new(tl!("Its link is on your Webex profile, under Personal Room. It's kept in File › Options › Calendar for the next meetings."))
            .color(t.dim)
            .small(),
    );
    ui.add_space(4.0);
    let link = typed.trim().to_string();
    let good = quatrelle_engine::model::ical::is_webex_link(&link);
    let mut taken = None;
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut typed).hint_text("https://site.webex.com/meet/name").desired_width(270.0));
        if ui.memory(|m| m.focused().is_none()) {
            r.request_focus();
        }
        // In a menu the box keeps the focus on Enter: Enter while typing in it.
        let enter = (r.has_focus() || r.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.add_enabled(good, egui::Button::new(tl!("Add to the meeting"))).clicked() || enter) && good {
            taken = Some(link.clone());
        }
    });
    if !link.is_empty() && !good {
        ui.label(RichText::new(tl!("Not a Webex personal room: https://<site>.webex.com/meet/<name>")).color(t.danger).small());
    }
    if taken.is_some() {
        typed.clear();
    }
    ui.data_mut(|d| d.insert_temp(id, typed));
    taken
}

/// The Appointment (or Meeting) tab's groups. `false`: closed from it.
fn ribbon(ui: &mut Ui, t: &Tokens, f: &mut EventForm, webex: &str, acts: &mut Vec<Action>) -> bool {
    let t = *t;
    let mut keep = true;
    let mut groups = Groups::new(ui);
    let writable = !f.read_only;
    let meeting = f.inviting && f.can_invite;
    if writable {
        let (icon, label) = match (meeting, f.uid.is_some()) {
            (true, false) => (Icon::Send, "Send"),
            (true, true) => (Icon::Send, "Send Update"),
            _ => (Icon::Calendar, "Save & Close"),
        };
        if widgets::big_button(ui, icon, label, &t, !f.summary.trim().is_empty()).clicked() {
            submit(f, acts);
        }
    }
    // A series asks which: this occurrence or all of it.
    if let Some(uid) = f.uid.clone().filter(|_| writable) {
        let r = widgets::big_button(ui, Icon::Delete, "Delete", &t, true);
        if f.recurring {
            egui::Popup::menu(&r).show(|ui| {
                if ui.button(tl!("Delete this occurrence")).clicked() {
                    acts.push(Action::Submit("event.delete", json!({"uid": uid, "occurrence": f.occurrence})));
                    ui.close();
                }
                if ui.button(tl!("Delete the series")).clicked() {
                    acts.push(Action::Submit("event.delete", json!({"uid": uid})));
                    ui.close();
                }
            });
        } else if r.clicked() {
            acts.push(Action::Submit("event.delete", json!({"uid": uid})));
        }
    }
    if !writable && widgets::big_button(ui, Icon::Close, "Close", &t, true).clicked() {
        keep = false;
    }
    groups.end(ui, &t, "Actions");
    // An online meeting (Teams, Webex, Zoom…): Join, as Outlook has it; none yet, Webex
    // Meeting puts the personal room in, asked for under the button the first time (kept in
    // Options from then on).
    if let Some(url) = f.join.clone() {
        if widgets::big_button(ui, Icon::Video, "Join meeting", &t, true).on_hover_text(&url).clicked() {
            acts.push(Action::OpenMeeting(url));
        }
        groups.end(ui, &t, "Online meeting");
    } else if writable {
        let r = widgets::big_button(ui, Icon::Video, "Webex Meeting", &t, true);
        if webex.is_empty() {
            let r = r.on_hover_text(tl!("Puts your Webex personal room in the meeting"));
            egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                if let Some(url) = webex_room(ui, &t, f.window) {
                    acts.push(Action::Run("calendar.webexRoom", json!({"url": url})));
                    add_webex(f, &url);
                    ui.close();
                }
            });
        } else if r.on_hover_text(webex).clicked() {
            add_webex(f, webex);
        }
        groups.end(ui, &t, "Online meeting");
    }
    // Someone else's invitation, not answered yet.
    if f.needs_reply
        && let Some(uid) = f.uid.clone()
    {
        for (icon, label, code) in [(Icon::Read, "Accept", "accepted"), (Icon::Clock, "Tentative", "tentative"), (Icon::Close, "Decline", "declined")]
        {
            if widgets::big_button(ui, icon, label, &t, true).clicked() {
                acts.push(Action::Submit("event.respond", json!({"uid": uid, "answer": code})));
            }
        }
        groups.end(ui, &t, "Respond");
    }
    if !writable {
        return keep;
    }
    if f.can_invite {
        if widgets::big_toggle(ui, Icon::People, "Invite Attendees", &t, f.inviting).clicked() {
            f.inviting = !f.inviting;
        }
        if f.inviting {
            let r = widgets::big_button(ui, Icon::CheckNames, "Response Options", &t, true);
            egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                ui.checkbox(&mut f.responses, tl!("Request Responses"));
                ui.checkbox(&mut f.new_times, tl!("Allow New Time Proposals"));
            });
        }
        groups.end(ui, &t, "Attendees");
    }
    // Show As and the reminder stacked, as Outlook has them; how it repeats a button of its own.
    widgets::stack(ui, |ui| {
        let label_w = 110.0;
        for (name, show_as) in [("Show As", true), ("Reminder", false)] {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(label_w, 20.0), Sense::hover());
                ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, tl!(name), theme::regular(13.0), t.dim);
                if show_as {
                    show_as_choice(ui, f);
                } else {
                    reminder_choice(ui, f, 130.0);
                }
            });
        }
    });
    let r = widgets::big_toggle(ui, Icon::Sync, "Repeat", &t, !f.repeat.is_empty());
    egui::Popup::menu(&r).show(|ui| {
        repeat_options(ui, f);
        ui.separator();
        if ui.button(tl!("Recurrence…")).clicked() {
            f.open_recurrence();
            ui.close();
        }
    });
    if f.can_invite && !f.all_day && widgets::big_toggle(ui, Icon::Location, "Room Finder", &t, f.finder.is_some()).clicked() {
        f.finder = if f.finder.is_some() { None } else { Some(RoomFinderUi::default()) };
    }
    groups.end(ui, &t, "Options");
    let r = widgets::big_button(ui, Icon::Tag, "Categorize", &t, !f.all_categories.is_empty());
    egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| categories(ui, t, f));
    if widgets::big_toggle(ui, Icon::Lock, "Private", &t, f.private)
        .on_hover_text(tl!("Others who see your calendar see only that the time is taken"))
        .clicked()
    {
        f.private = !f.private;
    }
    groups.last(ui, &t, "Tags");
    keep
}

/// Outlook's Show As: how its time shows to others.
fn show_as_choice(ui: &mut Ui, f: &mut EventForm) {
    let label = |x: ShowAs| -> &'static str {
        match x {
            ShowAs::Free => "Free",
            ShowAs::WorkingElsewhere => "Working Elsewhere",
            ShowAs::Tentative => "Tentative",
            ShowAs::Busy => "Busy",
            ShowAs::OutOfOffice => "Out of Office",
        }
    };
    egui::ComboBox::from_id_salt("event-show-as").width(130.0).selected_text(tl!(label(f.show_as))).show_ui(ui, |ui| {
        for x in ShowAs::ALL {
            ui.selectable_value(&mut f.show_as, x, tl!(label(x)));
        }
    });
}

/// The categories to tick, each in its colour (Outlook's Categorize).
fn categories(ui: &mut Ui, t: Tokens, f: &mut EventForm) {
    for (name, color) in f.all_categories.clone() {
        let mut on = f.categories.iter().any(|c| c.eq_ignore_ascii_case(&name));
        let c = theme::category_color(color, t.dark);
        if ui.checkbox(&mut on, RichText::new(format!("● {name}")).color(c)).changed() {
            if on {
                f.categories.push(name.clone());
            } else {
                f.categories.retain(|x| !x.eq_ignore_ascii_case(&name));
            }
        }
    }
}

/// A meeting's line: the contacts field in the rest of its form row, its name the row's button,
/// a word under it when it has one.
#[allow(clippy::too_many_arguments)]
fn people_line(
    ui: &mut Ui,
    t: &Tokens,
    people: &mut PeopleFields,
    field: quatrelle_engine::FieldId,
    account: &str,
    hint: &str,
    text: &mut String,
    can_expand: bool,
    acts: &mut Vec<Action>,
) {
    field_column(ui, |ui| {
        people.draw(ui, t, field, Some(account), accepts_of(field.name), "", hint, text, false, can_expand, false, acts);
    });
}

/// The title, the guests' lines, the times, the place and the rooms, and the calendar a new one
/// goes in; then what's said of it (organizer, answers).
fn header(ui: &mut Ui, t: &Tokens, f: &mut EventForm, s: &Session, people: &mut PeopleFields, acts: &mut Vec<Action>) {
    let account = f.account_id();
    let can_expand = s.directory_account(Some(&account)).is_some();
    let tip = tl!("Choose from the address book");
    let id = |line| super::line_button_id(f.window, line);
    let (required, optional, rooms) = (id(super::Line::Required), id(super::Line::Optional), id(super::Line::Resources));
    ui.add_enabled_ui(!f.read_only, |ui| {
        form_row(ui, t, false, None, "Title", |ui| {
            field(ui, &mut f.summary, tl!("Add a title"), f32::INFINITY);
        });
        // Required and Optional open the address book, as in Outlook; each is the contacts field.
        let mut open = None;
        if f.inviting && f.can_invite {
            let hint = "Names <addresses>, separated by ; or ,";
            for (line, button, name, label) in
                [(super::Line::Required, required, "required", "Required"), (super::Line::Optional, optional, "optional", "Optional")]
            {
                let text = if line == super::Line::Required { &mut f.guests } else { &mut f.optional };
                let field = meeting_field(f.window, name);
                if button_row(ui, t, button, label, tip, |ui| people_line(ui, t, people, field, &account, hint, text, can_expand, acts)) {
                    open = Some(line);
                }
            }
        }
        times(ui, t, f, s);
        form_row(ui, t, false, None, "Location", |ui| {
            field(ui, &mut f.location, tl!("Add a place"), f32::INFINITY);
        });
        // The rooms booked are its Rooms line (the address book opens on its Resources), the
        // Room Finder's toggle after it.
        if f.can_invite {
            let field = meeting_field(f.window, "rooms");
            let mut line = f.rooms_line.clone();
            if button_row(ui, t, rooms, "Rooms", tip, |ui| {
                super::rooms::with_finder_toggle(ui, f, |ui| people_line(ui, t, people, field, &account, "", &mut line, can_expand, acts));
            }) {
                open = Some(super::Line::Resources);
            }
            // What the line books, as it changed (an entry typed, removed, or settled).
            let booked = people.addressees(field);
            if line != f.rooms_line || !f.books(&booked) {
                f.rooms_from_field(&booked, line);
            }
            if f.finder.is_some() {
                super::rooms::finder(ui, f, s, t, acts);
                ui.add_space(8.0);
            }
        }
        if let Some(line) = open {
            f.attendees_book = Some(Box::new(super::AttendeesUi::open(f, line)));
        }
        if f.uid.is_none() && f.calendars.len() > 1 {
            let name = f.calendars.get(f.calendar).map(|c| c.2.clone()).unwrap_or_default();
            form_row(ui, t, false, None, "Calendar", |ui| {
                egui::ComboBox::from_id_salt("meeting-calendar").width(254.0).selected_text(name).show_ui(ui, |ui| {
                    for (i, c) in f.calendars.iter().enumerate() {
                        ui.selectable_value(&mut f.calendar, i, &c.2);
                    }
                });
            });
        }
    });
    if f.recurring {
        ui.label(RichText::new(tl!("Changes apply to every occurrence.")).color(t.faint).small());
    }
    if let Some(o) = &f.organizer {
        ui.label(RichText::new(tf!("Organizer: {name}", name = o)).color(t.dim).small());
    }
    if !f.attendees.is_empty() {
        let list: Vec<String> = f.attendees.iter().map(|(who, p)| format!("{who} ({})", tl!(partstat_label(p)))).collect();
        ui.label(RichText::new(tf!("Attendees: {names}", names = list.join(", "))).color(t.dim).small());
    }
    if !f.can_invite && !f.rooms.is_empty() {
        ui.label(RichText::new(tf!("Rooms: {names}", names = f.rooms_text())).color(t.dim).small());
    }
    if let Some(e) = &f.error {
        ui.label(RichText::new(e).color(t.danger));
    }
}

/// Start and end: a date and a time each (the time gone for an all-day event). Moving the start
/// moves the end with it, the meeting keeping its length, as Outlook does.
fn times(ui: &mut Ui, t: &Tokens, f: &mut EventForm, s: &Session) {
    let today = dates::today(&s.clock);
    let before = f.times(s);
    let mut moved = false;
    form_row(ui, t, false, None, "Start time", |ui| {
        moved |= super::pickers::date(ui, t, "meeting-start", &mut f.date, today);
        if !f.all_day {
            moved |= super::pickers::time(ui, t, &mut f.start);
        }
        ui.add_space(8.0);
        ui.allocate_ui_with_layout(vec2(ui.available_width(), FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.checkbox(&mut f.all_day, tl!("All day event"));
        });
    });
    if moved && let (Some((a, b)), Some((start, _))) = (before, f.times(s)) {
        f.set_times(s, start, start.saturating_add(b - a));
    }
    form_row(ui, t, false, None, "End time", |ui| {
        super::pickers::date(ui, t, "meeting-end", &mut f.end_date, today);
        if !f.all_day {
            super::pickers::time(ui, t, &mut f.end);
        }
    });
    // How it repeats, in words, as Outlook says it under the times.
    if !f.repeat.is_empty() {
        let rule = match f.repeat.as_str() {
            r @ ("daily" | "weekly" | "monthly" | "yearly") => format!("FREQ={}", r.to_ascii_uppercase()),
            r => r.to_string(),
        };
        let day = super::pickers::day_of(&f.date).unwrap_or(0);
        let said = super::recurrence::Recurrence::from_rule(&rule, day).map_or_else(|| tl!("Custom").to_string(), |r| r.describe());
        ui.label(RichText::new(tf!("Occurs: {what}", what = said)).color(t.dim).small());
        ui.add_space(6.0);
    }
}

/// The notes, the rest of the window tall: to read with their links (an invitation's), or to
/// write in.
fn body(ui: &mut Ui, t: &Tokens, f: &mut EventForm, acts: &mut Vec<Action>) {
    ui.add_space(6.0);
    if f.reading_notes {
        notes(ui, t, &f.description, acts);
        if !f.read_only && ui.small_button(tl!("Edit notes")).clicked() {
            f.reading_notes = false;
        }
        return;
    }
    let links_h = if f.links.is_empty() { 0.0 } else { 28.0 };
    let h = (ui.available_height() - links_h).max(120.0);
    let rows = ((h - 16.0) / 18.0).max(4.0) as usize;
    egui::ScrollArea::vertical().id_salt("meeting-notes").max_height(h).show(ui, |ui| {
        let w = ui.available_width();
        ui.add_enabled(
            !f.read_only,
            egui::TextEdit::multiline(&mut f.description).hint_text(tl!("Notes")).desired_width(w).desired_rows(rows).margin(Margin::symmetric(8, 6)),
        );
    });
    // Its links, to open (the box only edits them): a Teams invitation's "Join the meeting now".
    if !f.links.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tl!("Links:")).color(t.dim).small());
            for url in &f.links {
                let shown = url.split_once("://").map_or(url.as_str(), |(_, r)| r);
                if ui.link(RichText::new(crate::widgets::shorten(shown, 48)).small()).on_hover_text(url).clicked() {
                    acts.push(Action::OpenMeeting(url.clone()));
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Webex Meeting puts the room as the place (one typed is kept) and at the end of the notes,
    /// once however often it's pressed, and Join then shows.
    #[test]
    fn webex_meeting_puts_the_room_in_once() {
        let room = "https://acme.webex.com/meet/ana";
        let mut f = EventForm::default();
        add_webex(&mut f, room);
        assert_eq!((f.location.as_str(), f.join.as_deref()), (room, Some(room)));
        assert!(f.description.starts_with(tl!("Join the Webex meeting:")) && f.description.ends_with(&format!("{room}\n")), "{:?}", f.description);
        let mut f = EventForm { location: "Salle B".into(), description: "Ordre du jour\n".into(), ..Default::default() };
        add_webex(&mut f, room);
        add_webex(&mut f, room);
        assert_eq!(f.location, "Salle B");
        assert!(f.description.starts_with("Ordre du jour\n\n"), "{:?}", f.description);
        assert_eq!(f.description.matches(room).count(), 1);
        let e = quatrelle_engine::model::ical::CalEvent { location: f.location.clone(), description: f.description.clone(), ..Default::default() };
        assert_eq!(e.join_link().as_deref(), Some(room), "what Join finds once it's saved");
    }
}
