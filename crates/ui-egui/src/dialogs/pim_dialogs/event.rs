//! The event form: an appointment or meeting being added or edited, with the rooms booked for
//! it ([`super::rooms`]) and when everyone is free ([`super::scheduling`]).

use egui::Color32;
use quatrelle_engine::model::addr;
use quatrelle_engine::model::ical::ShowAs;

use super::*;

/// An event being added or edited.
#[derive(Clone, Debug, Default)]
pub struct EventForm {
    /// `None`: a new event.
    pub uid: Option<String>,
    /// The account an existing event is in (a new one's is its calendar's).
    pub account: String,
    /// The occurrence opened (its start), for deleting just that one.
    pub occurrence: Option<i64>,
    pub recurring: bool,
    /// The calendars that can take it: (account, calendar id, name).
    pub calendars: Vec<(String, String, String)>,
    pub calendar: usize,
    pub summary: String,
    pub location: String,
    pub description: String,
    /// Local `YYYY-MM-DD` and `HH:MM`.
    pub date: String,
    pub start: String,
    pub end_date: String,
    pub end: String,
    pub all_day: bool,
    /// `""` (never), `daily`, `weekly`, `monthly`, `yearly`, or a rule kept as it was.
    pub repeat: String,
    pub organizer: Option<String>,
    /// (who, their answer).
    pub attendees: Vec<(String, String)>,
    /// The guests, as typed (`Name <address>, …`): editable on the user's own events.
    pub guests: String,
    /// The rooms booked for it (name, address): resource attendees, which book themselves
    /// on Exchange; picked in the Room Finder.
    pub rooms: Vec<(String, String)>,
    /// The Room Finder, while it's open.
    pub finder: Option<RoomFinderUi>,
    /// The Scheduling Assistant is shown; the local day it shows (`None`: the meeting's); a
    /// drag of the meeting's slot in it.
    pub scheduling: bool,
    pub sched_day: Option<i64>,
    pub sched_drag: Option<SchedDrag>,
    /// The user can change the guests (a new event, or one they organize).
    pub can_invite: bool,
    /// The user was invited and hasn't answered.
    pub needs_reply: bool,
    /// Its reminder: minutes before the start (0: when it starts); `None`: none.
    pub reminder: Option<i64>,
    pub read_only: bool,
    /// Its categories, and every category there is (name, colour) to pick from.
    pub categories: Vec<String>,
    pub all_categories: Vec<(String, u8)>,
    /// The web addresses in its place and notes, and the one that joins it online (Teams, Zoom…):
    /// shown as links to open, as the notes box can't open them.
    pub links: Vec<String>,
    pub join: Option<String>,
    /// The notes shown to read, their links to open, rather than in the box that edits them:
    /// an invitation's (issue #1), or any with links.
    pub reading_notes: bool,
    pub error: Option<String>,
    /// The optional guests, as typed (Outlook's Optional line).
    pub optional: String,
    /// The guests' lines are shown (Outlook's Invite Attendees: an appointment becomes a meeting).
    pub inviting: bool,
    /// Outlook's Show As and Private, and its Response Options: the guests are asked to answer,
    /// and may propose another time.
    pub show_as: ShowAs,
    pub private: bool,
    pub responses: bool,
    pub new_times: bool,
    /// The number of the window it's open in (0: none, as a dialog).
    pub window: u64,
    /// Outlook's Appointment Recurrence, while it's open over the window.
    pub recurrence: Option<super::recurrence::RecurrenceUi>,
}

impl EventForm {
    /// Opens Outlook's Appointment Recurrence over its window, on how it repeats now.
    pub fn open_recurrence(&mut self) {
        let day = super::pickers::day_of(&self.date).unwrap_or(0);
        self.recurrence = Some(super::recurrence::RecurrenceUi::open(&self.repeat, day));
    }

    /// A new event at `at` (an hour long), or at the next full hour.
    pub fn new(s: &Session, at: Option<i64>) -> EventForm {
        let start = at.unwrap_or_else(|| (s.clock.now / 3600 + 1) * 3600);
        let calendars = writable_calendars(s);
        let current = s.current_account().map(|a| a.config.id.clone()).unwrap_or_default();
        EventForm {
            calendar: calendars.iter().position(|c| c.0 == current).unwrap_or(0),
            calendars,
            account: current,
            date: local_date(start, s),
            start: local_time(start, s),
            end_date: local_date(start + 3600, s),
            end: local_time(start + 3600, s),
            can_invite: true,
            all_categories: all_categories(s, &[]),
            // As Outlook sets a new appointment's reminder.
            reminder: Some(DEFAULT_REMINDER),
            responses: true,
            new_times: true,
            ..Default::default()
        }
    }

    /// An existing event, opened at one of its occurrences.
    pub fn open(s: &Session, uid: &str, occurrence: i64) -> Option<EventForm> {
        let (account, stored) = s.find_event(uid)?;
        let e = &stored.event;
        let calendars = writable_calendars(s);
        let read_only = s.pim.get(account).is_some_and(|p| p.calendars.iter().any(|c| c.id == stored.calendar && c.read_only));
        // A recurring event is edited as a series: its first occurrence's dates.
        let (start, end) = if e.all_day { (s.clock.utc(e.start), s.clock.utc(e.end - 86_400)) } else { (e.start, e.end) };
        let mine: Vec<String> = s.accounts.iter().map(|a| a.config.email.to_ascii_lowercase()).collect();
        let shown = |a: &quatrelle_engine::model::ical::Attendee| if a.name.is_empty() { a.email.clone() } else { a.name.clone() };
        Some(EventForm {
            uid: Some(uid.to_string()),
            account: account.to_string(),
            occurrence: Some(occurrence),
            recurring: e.rrule.is_some(),
            calendar: calendars.iter().position(|c| c.0 == account && c.1 == stored.calendar).unwrap_or(0),
            calendars,
            summary: e.summary.clone(),
            location: e.location.clone(),
            description: e.description.clone(),
            date: local_date(start, s),
            start: local_time(start, s),
            end_date: local_date(end, s),
            end: local_time(end, s),
            all_day: e.all_day,
            reminder: quatrelle_engine::model::ical::reminder_minutes(e),
            repeat: match e.rrule.as_deref() {
                None => String::new(),
                Some(r) => match r.to_ascii_uppercase().as_str() {
                    "FREQ=DAILY" => "daily".into(),
                    "FREQ=WEEKLY" => "weekly".into(),
                    "FREQ=MONTHLY" => "monthly".into(),
                    "FREQ=YEARLY" => "yearly".into(),
                    _ => r.to_string(),
                },
            },
            organizer: e.organizer.as_ref().map(|o| if o.name.is_empty() { o.email.clone() } else { format!("{} <{}>", o.name, o.email) }),
            attendees: e.attendees.iter().filter(|a| !a.is_resource()).map(|a| (shown(a), a.partstat.clone())).collect(),
            needs_reply: e.attendees.iter().any(|a| mine.contains(&a.email.to_ascii_lowercase()) && a.partstat == "NEEDS-ACTION"),
            guests: guest_list(&e.attendees, false),
            optional: guest_list(&e.attendees, true),
            inviting: e.attendees.iter().any(|a| !a.is_resource()),
            show_as: e.show_as,
            private: e.private,
            responses: !e.no_responses,
            new_times: !e.no_new_times,
            rooms: e.attendees.iter().filter(|a| a.is_resource()).map(|a| (shown(a), a.email.clone())).collect(),
            can_invite: e.organizer.as_ref().is_none_or(|o| mine.contains(&o.email.to_ascii_lowercase())),
            links: e.links(),
            join: e.join_link(),
            reading_notes: read_only || !e.links().is_empty(),
            read_only,
            categories: e.categories.clone(),
            all_categories: all_categories(s, &e.categories),
            error: None,
            ..Default::default()
        })
    }

    /// The account the event is in, or goes to (its calendar's).
    pub fn account_id(&self) -> String {
        if self.uid.is_some() {
            return self.account.clone();
        }
        self.calendars.get(self.calendar).map_or_else(|| self.account.clone(), |c| c.0.clone())
    }

    /// The rooms' names, as the place reads when the meeting is held in them.
    pub fn rooms_text(&self) -> String {
        self.rooms.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join("; ")
    }

    /// Book a room: it joins the rooms, and the place follows the rooms unless it was typed
    /// by hand.
    pub fn add_room(&mut self, name: &str, email: &str) {
        let email = email.trim().to_ascii_lowercase();
        if !email.contains('@') || self.rooms.iter().any(|(_, e)| *e == email) || self.rooms.len() >= 50 {
            return;
        }
        let before = self.rooms_text();
        let name = if name.trim().is_empty() { email.clone() } else { name.trim().to_string() };
        self.rooms.push((name, email));
        self.place_follows_rooms(&before);
    }

    /// Take a room off (the place follows, as above).
    pub fn remove_room(&mut self, email: &str) {
        let before = self.rooms_text();
        self.rooms.retain(|(_, e)| !e.eq_ignore_ascii_case(email));
        self.place_follows_rooms(&before);
    }

    fn place_follows_rooms(&mut self, before: &str) {
        if self.location.trim().is_empty() || self.location.trim() == before {
            self.location = self.rooms_text();
        }
    }

    /// When it starts and ends, from the fields (`None` while they're not dates).
    pub fn times(&self, s: &Session) -> Option<(i64, i64)> {
        if self.all_day {
            let (a, _) = quatrelle_engine::parse_local(self.date.trim(), &s.clock)?;
            let (b, _) = quatrelle_engine::parse_local(self.end_date.trim(), &s.clock)?;
            return Some((s.clock.utc(a), s.clock.utc(b + 86_400).max(s.clock.utc(a + 86_400))));
        }
        let (a, _) = quatrelle_engine::parse_local(&format!("{}T{}", self.date.trim(), self.start.trim()), &s.clock)?;
        let (b, _) = quatrelle_engine::parse_local(&format!("{}T{}", self.end_date.trim(), self.end.trim()), &s.clock)?;
        Some((a, b.max(a)))
    }

    /// Move it to `start`..`end` (the fields follow).
    pub fn set_times(&mut self, s: &Session, start: i64, end: i64) {
        self.date = local_date(start, s);
        self.start = local_time(start, s);
        self.end_date = local_date(end, s);
        self.end = local_time(end, s);
    }

    pub(crate) fn params(&self) -> Value {
        let (start, end) = if self.all_day {
            (self.date.trim().to_string(), self.end_date.trim().to_string())
        } else {
            (format!("{}T{}", self.date.trim(), self.start.trim()), format!("{}T{}", self.end_date.trim(), self.end.trim()))
        };
        let mut p = json!({
            "summary": self.summary.trim(),
            "location": self.location.trim(),
            "description": self.description,
            "start": start,
            "end": end,
            "allDay": self.all_day,
            "repeat": if self.repeat.is_empty() { "none" } else { self.repeat.as_str() },
            "reminder": self.reminder.map_or_else(|| json!("none"), |m| json!(m)),
            "categories": self.categories,
            "showAs": self.show_as.param(),
            "private": self.private,
        });
        if self.can_invite {
            p["attendees"] = json!(self.guests.trim());
            p["optional"] = json!(self.optional.trim());
            p["responses"] = json!(self.responses);
            p["newTimes"] = json!(self.new_times);
            let rooms: Vec<quatrelle_engine::model::EmailAddress> =
                self.rooms.iter().map(|(name, email)| quatrelle_engine::model::EmailAddress::new(name.clone(), email.clone())).collect();
            p["rooms"] = json!(addr::format_list(&rooms));
        }
        match &self.uid {
            Some(u) => p["uid"] = json!(u),
            None => {
                if let Some((a, c, _)) = self.calendars.get(self.calendar) {
                    p["account"] = json!(a);
                    p["calendar"] = json!(c);
                }
            }
        }
        p
    }
}

/// How a repeat reads: Outlook's simple choices, a rule set elsewhere as Custom.
fn repeat_label(r: &str) -> String {
    match r {
        "" => tl!("Never").to_string(),
        "daily" => tl!("Every day").to_string(),
        "weekly" => tl!("Every week").to_string(),
        "monthly" => tl!("Every month").to_string(),
        "yearly" => tl!("Every year").to_string(),
        _ => tl!("Custom").to_string(),
    }
}

/// How it repeats, among Outlook's simple choices (one set elsewhere is kept as Custom).
pub(super) fn repeat_choice(ui: &mut Ui, f: &mut EventForm, width: f32) {
    egui::ComboBox::from_id_salt("event-repeat").width(width).selected_text(repeat_label(&f.repeat)).show_ui(ui, |ui| repeat_options(ui, f));
}

/// The repeats to pick from, the one chosen ticked (a menu or a list's).
pub(super) fn repeat_options(ui: &mut Ui, f: &mut EventForm) {
    let custom = !matches!(f.repeat.as_str(), "" | "daily" | "weekly" | "monthly" | "yearly");
    for r in ["", "daily", "weekly", "monthly", "yearly"] {
        ui.selectable_value(&mut f.repeat, r.to_string(), repeat_label(r));
    }
    if custom {
        let keep = f.repeat.clone();
        ui.selectable_value(&mut f.repeat, keep, tl!("Custom"));
    }
}

/// Outlook's reminder: how long before it starts, or none.
pub(super) fn reminder_choice(ui: &mut Ui, f: &mut EventForm, width: f32) {
    let label = |m: Option<i64>| match m {
        None => tl!("None").to_string(),
        Some(m) => crate::calendar::reminder_text(m),
    };
    let mut choices: Vec<Option<i64>> = std::iter::once(None).chain(REMINDER_CHOICES.iter().map(|m| Some(*m))).collect();
    // One set elsewhere (another app, a server) stays offered as it is.
    if !choices.contains(&f.reminder) {
        choices.push(f.reminder);
    }
    egui::ComboBox::from_id_salt("event-reminder").width(width).selected_text(label(f.reminder)).show_ui(ui, |ui| {
        for c in choices {
            ui.selectable_value(&mut f.reminder, c, label(c));
        }
    });
}

/// The required (or optional) guests, as the lines show them: `Name <address>, …`.
fn guest_list(attendees: &[quatrelle_engine::model::ical::Attendee], optional: bool) -> String {
    attendees
        .iter()
        .filter(|a| !a.is_resource() && a.is_optional() == optional)
        .map(|a| if a.name.is_empty() { a.email.clone() } else { format!("{} <{}>", a.name, a.email) })
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn event(ui: &mut Ui, f: &mut EventForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    // Wider with the Scheduling Assistant open: its hours need the room.
    ui.set_width(if f.scheduling { 780.0 } else { 540.0 });
    form_header(ui, t, Ok(Icon::Calendar), if f.uid.is_some() { "Event" } else { "New event" });
    // An online meeting (Teams, Zoom…): Join, as Outlook has it, read-only calendar or not.
    if let Some(url) = f.join.clone() {
        if ui.add(egui::Button::new(RichText::new(tl!("Join meeting")).color(t.on_accent).strong()).fill(t.accent)).on_hover_text(&url).clicked() {
            acts.push(Action::OpenUrl(url));
        }
        ui.add_space(8.0);
    }
    ui.add_enabled_ui(!f.read_only, |ui| {
        let w = ui.available_width();
        ui.add(
            egui::TextEdit::singleline(&mut f.summary)
                .hint_text(tl!("Add a title"))
                .font(theme::bold(16.0))
                .desired_width(w)
                .min_size(vec2(w, 36.0))
                .margin(Margin::symmetric(10, 8))
                .align(egui::Align2::LEFT_CENTER),
        );
        ui.add_space(12.0);
        form_row(ui, t, true, Some(Icon::Clock), "Starts", |ui| {
            field(ui, &mut f.date, "YYYY-MM-DD", 120.0);
            if !f.all_day {
                field(ui, &mut f.start, "09:00", 70.0);
            }
        });
        form_row(ui, t, true, None, "Ends", |ui| {
            field(ui, &mut f.end_date, "YYYY-MM-DD", 120.0);
            if !f.all_day {
                field(ui, &mut f.end, "10:00", 70.0);
            }
            ui.add_space(6.0);
            ui.allocate_ui_with_layout(vec2(ui.available_width(), FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.checkbox(&mut f.all_day, tl!("All day"));
            });
        });
        form_row(ui, t, true, Some(Icon::Sync), "Repeat", |ui| {
            ui.allocate_ui_with_layout(vec2(200.0, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| repeat_choice(ui, f, 194.0));
        });
        // Outlook's reminder: how long before it starts (or none).
        form_row(ui, t, true, Some(Icon::Reminder), "Reminder", |ui| {
            ui.allocate_ui_with_layout(vec2(200.0, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| reminder_choice(ui, f, 194.0));
        });
        form_row(ui, t, true, Some(Icon::Location), "Location", |ui| {
            field(ui, &mut f.location, tl!("Add a place"), f32::INFINITY);
        });
        // Categories colour it in the calendar (Outlook's Categorize on an appointment).
        if !f.all_categories.is_empty() {
            form_row(ui, t, true, Some(Icon::Tag), "Categories", |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (name, color) in f.all_categories.clone() {
                        let on = f.categories.iter().any(|c| c.eq_ignore_ascii_case(&name));
                        let c = theme::category_color(color, t.dark);
                        let text = RichText::new(format!("● {}", name)).color(if on { Color32::WHITE } else { c });
                        let r = ui.add(
                            egui::Button::new(text)
                                .fill(if on { c } else { Color32::TRANSPARENT })
                                .stroke(egui::Stroke::new(1.0, c))
                                .corner_radius(10),
                        );
                        if r.clicked() {
                            if on {
                                f.categories.retain(|x| !x.eq_ignore_ascii_case(&name));
                            } else {
                                f.categories.push(name.clone());
                            }
                        }
                    }
                });
            });
        }
        if f.can_invite {
            form_row(ui, t, true, Some(Icon::People), "Invite", |ui| {
                field(ui, &mut f.guests, tl!("Names <addresses>, separated by ; or ,"), f32::INFINITY);
            });
            // The rooms booked for it, and the Room Finder to pick one (Outlook's).
            form_row(ui, t, true, Some(Icon::Location), "Rooms", |ui| super::rooms::row(ui, f, t));
            if f.finder.is_some() {
                super::rooms::finder(ui, f, s, t, acts);
                ui.add_space(8.0);
            }
        }
        if f.uid.is_none() && f.calendars.len() > 1 {
            let name = f.calendars.get(f.calendar).map(|c| c.2.clone()).unwrap_or_default();
            form_row(ui, t, true, Some(Icon::Calendar), "Calendar", |ui| {
                ui.allocate_ui_with_layout(vec2(260.0, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("event-calendar").width(254.0).selected_text(name).show_ui(ui, |ui| {
                        for (i, c) in f.calendars.iter().enumerate() {
                            ui.selectable_value(&mut f.calendar, i, &c.2);
                        }
                    });
                });
            });
        }
        // When everyone (and the room) is free: Outlook's Scheduling Assistant, under the
        // fields, the meeting drawn over their hours and moved from there.
        if !f.all_day {
            ui.horizontal(|ui| {
                if ui.selectable_label(f.scheduling, tl!("Scheduling Assistant")).clicked() {
                    f.scheduling = !f.scheduling;
                    f.sched_day = None;
                }
            });
            if f.scheduling {
                ui.add_space(4.0);
                super::scheduling::view(ui, f, s, t, acts);
            }
            ui.add_space(8.0);
        }
        if !f.reading_notes {
            // Tall notes scroll in their box: the dialog keeps its buttons on screen.
            egui::ScrollArea::vertical().id_salt("event-notes").max_height(NOTES_MAX_H).show(ui, |ui| {
                area(ui, &mut f.description, tl!("Notes"), 4);
            });
        }
    });
    if f.reading_notes {
        ui.add_space(4.0);
        notes(ui, t, &f.description, acts);
        if !f.read_only && ui.small_button(tl!("Edit notes")).clicked() {
            f.reading_notes = false;
        }
    }
    // The notes' links, to open (the box above only edits them): a Teams invitation's "Join the
    // meeting now", a document, a dial-in page.
    if !f.links.is_empty() && !f.reading_notes {
        ui.add_space(4.0);
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
    if f.recurring {
        ui.add_space(4.0);
        ui.label(RichText::new(tl!("Changes apply to every occurrence.")).color(t.faint).small());
    }
    if let Some(o) = &f.organizer {
        ui.add_space(4.0);
        ui.label(RichText::new(tf!("Organizer: {name}", name = o)).color(t.dim).small());
    }
    if !f.attendees.is_empty() {
        let list: Vec<String> = f.attendees.iter().map(|(who, p)| format!("{who} ({})", tl!(partstat_label(p)))).collect();
        ui.label(RichText::new(tf!("Attendees: {names}", names = list.join(", "))).color(t.dim).small());
    }
    // Someone else's meeting: its rooms, read.
    if !f.can_invite && !f.rooms.is_empty() {
        ui.label(RichText::new(tf!("Rooms: {names}", names = f.rooms_text())).color(t.dim).small());
    }
    if f.needs_reply
        && let Some(uid) = &f.uid
    {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!("Your answer:")).color(t.text));
            for (label, code) in [("Accept", "accepted"), ("Tentative", "tentative"), ("Decline", "declined")] {
                if ui.button(tl!(label)).clicked() {
                    acts.push(Action::Submit("event.respond", json!({"uid": uid, "answer": code})));
                }
            }
        });
    }
    if let Some(e) = &f.error {
        ui.add_space(6.0);
        ui.label(RichText::new(e).color(t.danger));
    }
    footer_rule(ui, t);
    let mut keep = true;
    ui.horizontal(|ui| {
        if let Some(uid) = f.uid.clone().filter(|_| !f.read_only) {
            if f.recurring {
                if ui.button(tl!("Delete this occurrence")).clicked() {
                    acts.push(Action::Submit("event.delete", json!({"uid": uid, "occurrence": f.occurrence})));
                }
                if ui.button(tl!("Delete the series")).clicked() {
                    acts.push(Action::Submit("event.delete", json!({"uid": uid})));
                }
            } else if ui.button(tl!("Delete")).clicked() {
                acts.push(Action::Submit("event.delete", json!({"uid": uid})));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !f.read_only && widgets::accent_button(ui, crate::icons::Icon::Calendar, "Save", t, !f.summary.trim().is_empty()).clicked() {
                f.error = None;
                acts.push(Action::Submit(if f.uid.is_some() { "event.update" } else { "event.create" }, f.params()));
            }
            if ui.button(tl!(if f.read_only { "Close" } else { "Cancel" })).clicked() {
                keep = false;
            }
        });
    });
    keep
}
