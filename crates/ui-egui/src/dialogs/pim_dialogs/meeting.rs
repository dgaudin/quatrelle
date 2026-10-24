//! An appointment or a meeting in a window of its own, as Outlook opens one: a ribbon (the
//! Appointment or Meeting tab, and the Scheduling Assistant), then its title, guests, times and
//! place, and its notes taking the rest of the window. The form is the dialog's ([`EventForm`]);
//! only where it's shown differs.

use quatrelle_engine::model::ical::ShowAs;

use super::event::{reminder_choice, repeat_options};
use super::*;
use crate::widgets::Groups;

/// The window's content. `false` once it's done with from its ribbon (Close, for one that can't
/// be changed); saving, sending and deleting close it once the engine has done them.
pub fn meeting(ui: &mut Ui, f: &mut EventForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    tabs(ui, t, f);
    ui.add_space(2.0);
    let mut keep = true;
    egui::ScrollArea::horizontal().id_salt("meeting-ribbon").show(ui, |ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            keep = ribbon(ui, t, f, acts);
        });
    });
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.border);
    ui.add_space(8.0);
    if f.scheduling && !f.all_day {
        super::scheduling::view(ui, f, s, t, acts);
        ui.add_space(8.0);
        // Outlook's assistant keeps the times under it.
        ui.add_enabled_ui(!f.read_only, |ui| times(ui, t, f, s));
    } else {
        header(ui, t, f, s, acts);
        body(ui, t, f, acts);
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

/// The Appointment (or Meeting) tab's groups. `false`: closed from it.
fn ribbon(ui: &mut Ui, t: &Tokens, f: &mut EventForm, acts: &mut Vec<Action>) -> bool {
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
    // An online meeting (Teams, Webex, Zoom…): Join, as Outlook has it.
    if let Some(url) = f.join.clone() {
        if widgets::big_button(ui, Icon::People, "Join meeting", &t, true).on_hover_text(&url).clicked() {
            acts.push(Action::OpenUrl(url));
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
    egui::Popup::menu(&r).show(|ui| repeat_options(ui, f));
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

/// The title, the guests' lines, the times, the place and the rooms, and the calendar a new one
/// goes in; then what's said of it (organizer, answers).
fn header(ui: &mut Ui, t: &Tokens, f: &mut EventForm, s: &Session, acts: &mut Vec<Action>) {
    ui.add_enabled_ui(!f.read_only, |ui| {
        form_row(ui, t, false, None, "Title", |ui| {
            field(ui, &mut f.summary, tl!("Add a title"), f32::INFINITY);
        });
        if f.inviting && f.can_invite {
            form_row(ui, t, false, None, "Required", |ui| {
                field(ui, &mut f.guests, tl!("Names <addresses>, separated by ; or ,"), f32::INFINITY);
            });
            form_row(ui, t, false, None, "Optional", |ui| {
                field(ui, &mut f.optional, tl!("Names <addresses>, separated by ; or ,"), f32::INFINITY);
            });
        }
        times(ui, t, f, s);
        form_row(ui, t, false, None, "Location", |ui| {
            field(ui, &mut f.location, tl!("Add a place"), f32::INFINITY);
        });
        if f.can_invite {
            form_row(ui, t, false, None, "Rooms", |ui| super::rooms::row(ui, f, t));
            if f.finder.is_some() {
                super::rooms::finder(ui, f, s, t, acts);
                ui.add_space(8.0);
            }
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
                    acts.push(Action::OpenUrl(url.clone()));
                }
            }
        });
    }
}
