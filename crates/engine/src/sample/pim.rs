//! The sample mailbox's calendar, address book, notes and tasks.

use super::*;

/// The sample calendar (a week around `now`, in local time) and address book.
pub fn sample_pim(now: i64, offset: i64) -> quatrelle_model::pim::Pim {
    use quatrelle_model::ical::{Attendee, CalEvent};
    use quatrelle_model::pim::{LOCAL, Pim, StoredCard, StoredEvent};
    use quatrelle_model::vcard::{Card, Typed};
    let today = (now + offset).div_euclid(86_400);
    let monday = today - (today + 3).rem_euclid(7);
    let at = |day: i64, h: i64, m: i64| day * 86_400 + h * 3600 + m * 60 - offset;
    let who = |name: &str, email: &str, partstat: &str| Attendee {
        email: email.into(),
        name: name.into(),
        partstat: partstat.into(),
        rsvp: false,
        cutype: String::new(),
        role: String::new(),
    };
    let ev = |uid: &str, summary: &str, start: i64, end: i64| CalEvent {
        uid: format!("{uid}@sample.quatrelle"),
        summary: summary.into(),
        start,
        end,
        ..Default::default()
    };
    let events = vec![
        CalEvent {
            rrule: Some("FREQ=WEEKLY;BYDAY=MO,WE,FR".into()),
            location: "Studio floor".into(),
            ..ev("standup", "Team stand-up", at(monday - 14, 9, 30), at(monday - 14, 9, 45))
        },
        CalEvent {
            location: "Screening room".into(),
            reminder: Some(15),
            description: "Revised frames for scenes 4 to 7, and the colour key.".into(),
            organizer: Some(who("Zoe Park", "zoe@studio.example", "ACCEPTED")),
            // Held in a room booked for it (a resource attendee, as Exchange books rooms).
            attendees: vec![
                who("Riley Moreno", ME.1, "ACCEPTED"),
                who("Sam Okafor", "sam@studio.example", "ACCEPTED"),
                Attendee { cutype: "ROOM".into(), ..who("Screening room", ROOMS[0].0, "ACCEPTED") },
            ],
            ..ev("review", "Storyboard review", at(monday + 3, 14, 0), at(monday + 3, 15, 30))
        },
        CalEvent { location: "Café on the corner".into(), ..ev("lunch", "Lunch with Jun", at(today, 12, 30), at(today, 13, 30)) },
        CalEvent {
            location: "Sound Lab".into(),
            organizer: Some(who("Ana Lima", "ana@soundlab.example", "ACCEPTED")),
            attendees: vec![who("Riley Moreno", ME.1, "NEEDS-ACTION")],
            ..ev("mix", "Sound mix playback", at(today + 1, 11, 0), at(today + 1, 12, 0))
        },
        CalEvent { all_day: true, ..ev("poster", "Poster to the print shop", (today + 2) * 86_400, (today + 3) * 86_400) },
        CalEvent { location: "Community Gallery".into(), ..ev("opening", "Winter group show opening", at(today + 9, 18, 0), at(today + 9, 21, 0)) },
    ];
    let person = |uid: &str, name: &str, email: &str, org: &str, title: &str, phone: &str| Card {
        uid: format!("{uid}@sample.quatrelle"),
        name: name.into(),
        emails: vec![Typed { value: email.into(), kind: "work".into() }],
        phones: if phone.is_empty() { Vec::new() } else { vec![Typed { value: phone.into(), kind: "cell".into() }] },
        org: org.into(),
        title: title.into(),
        ..Default::default()
    };
    let cards = vec![
        person("zoe", "Zoe Park", "zoe@studio.example", "Studio Lantern", "Director", "+1 555 0142"),
        person("sam", "Sam Okafor", "sam@studio.example", "Studio Lantern", "Editor", "+1 555 0177"),
        person("ana", "Ana Lima", "ana@soundlab.example", "Sound Lab", "Sound designer", ""),
        person("jun", "Jun Watanabe", "jun@studio.example", "Studio Lantern", "Animator", "+1 555 0108"),
        person("print", "Print Shop", "orders@printshop.example", "Print Shop", "", "+1 555 0190"),
    ];
    // The studio's own calendar beside the personal one (to see side by side, or hide).
    let studio = vec![
        CalEvent { location: "Edit suite".into(), ..ev("edit", "Edit session: act two", at(today, 10, 0), at(today, 12, 0)) },
        CalEvent { location: "Machine room".into(), ..ev("render", "Render farm maintenance", at(today + 1, 9, 0), at(today + 1, 13, 0)) },
    ];
    let mut p = Pim::default();
    p.ensure_defaults();
    // In a group of its own, as a team's calendars are kept in Outlook.
    p.calendars.push(quatrelle_model::pim::CalendarInfo {
        id: STUDIO_CALENDAR.into(),
        name: "Studio".into(),
        color: 3,
        group: "Studio team".into(),
        ..Default::default()
    });
    // The studio's rooms, each with a calendar of its own in a group of their own, as Outlook
    // keeps room calendars (unticked until wanted): the Room Finder offers them, and the
    // Scheduling Assistant shows when each is booked. And a colleague's calendar, shared.
    let shared_id = quatrelle_model::pim::shared_calendar_id;
    for (n, (email, name)) in ROOMS.iter().enumerate() {
        p.calendars.push(quatrelle_model::pim::CalendarInfo {
            id: shared_id(email),
            name: (*name).into(),
            color: u8::try_from(5 + n).unwrap_or(5),
            read_only: true,
            group: "Rooms".into(),
            ..Default::default()
        });
    }
    p.calendars.push(quatrelle_model::pim::CalendarInfo {
        id: shared_id("zoe@studio.example"),
        name: "Zoe Park".into(),
        color: 7,
        read_only: true,
        group: "Studio team".into(),
        ..Default::default()
    });
    let booked = |cal: &str, event: CalEvent| StoredEvent { calendar: shared_id(cal), event, ..Default::default() };
    let elsewhere = vec![
        booked(ROOMS[0].0, ev("dailies", "Dailies", at(today, 9, 0), at(today, 10, 0))),
        booked(ROOMS[0].0, ev("client-screening", "Client screening", at(today, 15, 0), at(today, 17, 0))),
        booked(ROOMS[0].0, ev("review-room", "Storyboard review", at(monday + 3, 14, 0), at(monday + 3, 15, 30))),
        booked(ROOMS[1].0, ev("foley", "Foley session", at(today, 9, 30), at(today, 12, 0))),
        booked(ROOMS[1].0, ev("mix-room", "Sound mix playback", at(today + 1, 11, 0), at(today + 1, 12, 0))),
        booked("zoe@studio.example", ev("zoe-producers", "Producers' call", at(today, 11, 0), at(today, 12, 0))),
        booked(
            "zoe@studio.example",
            CalEvent { status: "TENTATIVE".into(), ..ev("zoe-casting", "Casting session", at(today, 14, 0), at(today, 16, 0)) },
        ),
    ];
    p.events = events
        .into_iter()
        .map(|event| StoredEvent { calendar: LOCAL.into(), event, ..Default::default() })
        .chain(studio.into_iter().map(|event| StoredEvent { calendar: STUDIO_CALENDAR.into(), event, ..Default::default() }))
        .chain(elsewhere)
        .collect();
    // Categories colour some of them (as in Outlook).
    for e in &mut p.events {
        match e.event.uid.split('@').next().unwrap_or("") {
            "review" | "edit" => e.event.categories = vec!["Work".into()],
            "lunch" => e.event.categories = vec!["Personal".into()],
            _ => {}
        }
    }
    p.cards = cards.into_iter().map(|card| StoredCard { book: LOCAL.into(), card, ..Default::default() }).collect();
    // Notes (Outlook's Notes), kept on this computer.
    let note = |uid: &str, text: &str, color: u8, ago: i64| quatrelle_model::pim::StoredNote {
        note: quatrelle_model::pim::Note { uid: format!("{uid}@sample.quatrelle"), text: text.into(), color, modified: now - ago },
        ..Default::default()
    };
    p.notes = vec![
        note("note-poster", "Festival poster\nApprove the proof by Thursday; ask about 170 gsm paper.", 3, 3600),
        note("note-mix", "Sound mix v3\nRain under the café scene: a little quieter.", 0, 86_400),
        note("note-fox", "Fox turnarounds\nSide view: longer tail.", 1, 3 * 86_400),
    ];
    // Tasks (Outlook's Tasks), kept on this computer: one late, some due this week, one done.
    let task = |uid: &str, subject: &str, due: Option<i64>| quatrelle_model::pim::StoredTask {
        task: quatrelle_model::pim::Task {
            uid: format!("{uid}@sample.quatrelle"),
            subject: subject.into(),
            due,
            modified: now,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut proof = task("task-proof", "Approve the poster proof", Some(today - 1));
    proof.task.importance = 1;
    let mut budget = task("task-budget", "Send the budget to Zoe", Some(today));
    budget.task.categories = vec!["Work".into()];
    budget.task.reminder = Some(at(today, 16, 0));
    let mut mix = task("task-mix", "Listen to sound mix v3", Some(today + 2));
    mix.task.status = quatrelle_model::pim::TaskStatus::InProgress;
    mix.task.percent = 50;
    let mut fox = task("task-fox", "Fox turnarounds: side view", None);
    fox.task.notes = "Longer tail.".into();
    let mut invoice = task("task-invoice", "Pay the print shop", Some(today - 3));
    invoice.task.set_complete(true, today - 2);
    p.tasks = vec![proof, budget, mix, task("task-paper", "Order 170 gsm paper", Some(monday + 9)), fox, invoice];
    p
}
