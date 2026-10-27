//! Engine tests: appointments, reminders, and the calendars an account shows.

use super::*;

mod web;

/// An appointment read from Exchange carries its start in its name (`…#<start>`), and a dialog
/// opened before it moved still holds the old one: it finds the appointment all the same, when
/// only one answers to the name without its start, instead of "no event `…`".
#[test]
fn an_appointment_answers_to_the_name_a_dialog_opened_with() {
    let mut s = sample_session("event-name");
    let uid = s.run("event.create", &json!({"summary": "test dga", "start": "2026-10-23T11:00", "end": "2026-10-23T12:00"})).unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    // What a copy read from Exchange looks like.
    {
        let account = "sample".to_string();
        let p = s.pim_mut(&account);
        if let Some(e) = p.events.iter_mut().find(|e| e.event.uid == uid) {
            e.event.uid = "040000008200E00074C5B7101A82E00800000000A0FB60422C61DD01#1792569600".into();
        }
    }
    let old = "040000008200E00074C5B7101A82E00800000000A0FB60422C61DD01#1792569600";
    let moved = "040000008200E00074C5B7101A82E00800000000A0FB60422C61DD01#1792574700";
    s.run("event.update", &json!({"uid": old, "summary": "test dga déplacé"})).unwrap();
    let r = s.run("event.update", &json!({"uid": moved, "location": "Bureau"})).expect("the name of a moved appointment still finds it");
    assert_eq!(r["uid"], old, "the appointment keeps its own name");
    let (_, e) = s.find_event(old).expect("still there");
    assert_eq!((e.event.summary.as_str(), e.event.location.as_str()), ("test dga déplacé", "Bureau"));
    // Two appointments answering to the same name without its start: nothing is guessed.
    assert!(s.run("event.update", &json!({"uid": "nobody#1", "summary": "x"})).is_err());
    done(s);
}

/// A reminder is minutes before the start; it's kept through a change, and taken off when asked.
#[test]
fn a_reminder_is_set_kept_and_taken_off() {
    let mut s = sample_session("reminder");
    let uid =
        s.run("event.create", &json!({"summary": "Review", "start": "2026-10-23T11:00", "end": "2026-10-23T12:00", "reminder": 15})).unwrap()["uid"]
            .as_str()
            .unwrap()
            .to_string();
    assert_eq!(s.find_event(&uid).unwrap().1.event.reminder, Some(15));
    s.run("event.update", &json!({"uid": uid, "reminder": 1440})).unwrap();
    assert_eq!(s.find_event(&uid).unwrap().1.event.reminder, Some(1440), "a day before");
    // Kept as written, a VALARM in its calendar file.
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    let mut again = Session::open(Some(Store::open(&root).unwrap()), quatrelle_model::dates::Clock::new(NOW, 0));
    assert_eq!(again.find_event(&uid).unwrap().1.event.reminder, Some(1440));
    again.run("event.update", &json!({"uid": uid, "reminder": null})).unwrap();
    assert_eq!(again.find_event(&uid).unwrap().1.event.reminder, None, "taken off");
    assert!(again.run("event.update", &json!({"uid": uid, "reminder": 99_999})).is_err(), "nonsense is refused, not a panic");
    assert_eq!(again.find_event(&uid).unwrap().1.event.reminder, None);
    done(s);
}

/// Outlook's Show As, Private, optional guests and Response Options are set by the commands, kept
/// with the event, and changed one at a time; nonsense is refused.
#[test]
fn a_meetings_show_as_privacy_optional_guests_and_options_are_kept() {
    use quatrelle_model::ical::ShowAs;
    let mut s = sample_session("meeting-options");
    let made = s
        .run(
            "event.create",
            &json!({"summary": "Budget", "start": "2026-10-23T11:00", "end": "2026-10-23T12:00", "attendees": "Ana <ana@studio.example>",
                    "optional": "Zoe <zoe@studio.example>", "showAs": "outOfOffice", "private": true, "newTimes": false}),
        )
        .unwrap();
    let uid = made["uid"].as_str().unwrap().to_string();
    let e = s.find_event(&uid).unwrap().1.event.clone();
    assert_eq!((e.show_as, e.private, e.no_new_times, e.no_responses), (ShowAs::OutOfOffice, true, true, false));
    let optional: Vec<&str> = e.attendees.iter().filter(|a| a.is_optional()).map(|a| a.email.as_str()).collect();
    let required: Vec<&str> = e.attendees.iter().filter(|a| !a.is_optional() && !a.is_resource()).map(|a| a.email.as_str()).collect();
    assert_eq!((optional, required), (vec!["zoe@studio.example"], vec!["ana@studio.example"]));
    // The optional guests alone, changed: the required one stays.
    s.run("event.update", &json!({"uid": uid, "optional": "", "showAs": "free", "responses": false})).unwrap();
    let e = s.find_event(&uid).unwrap().1.event.clone();
    assert_eq!((e.show_as, e.no_responses), (ShowAs::Free, true));
    assert_eq!(e.attendees.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["ana@studio.example"]);
    assert!(s.run("event.update", &json!({"uid": uid, "showAs": "asleep"})).is_err(), "nonsense is refused");
    done(s);
}

/// Two appointments the server keeps under one UID — some Exchange series arrive as separate
/// items that all carry the series' UID — are both kept: Friday's meeting must not vanish
/// behind its siblings, and a re-sync doesn't double them.
#[test]
fn appointments_sharing_a_uid_are_all_kept() {
    use quatrelle_model::dates::Clock;
    use quatrelle_model::ical::CalEvent;
    use quatrelle_model::pim::{CalendarInfo, DavSync, StoredEvent};

    let mut s = Session::open(Some(temp("shared-uid")), Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let cal = "ews:calendar";
    let item = |href: &str, start: i64| StoredEvent {
        calendar: cal.into(),
        href: href.into(),
        event: CalEvent {
            uid: "040000008200E00074C5".into(), summary: "Comité de sécurité".into(), start, end: start + 7_200, ..Default::default()
        },
        ..Default::default()
    };
    let sync = DavSync {
        calendars: Some(vec![CalendarInfo { id: cal.into(), name: "Calendar".into(), ..Default::default() }]),
        synced_calendars: vec![cal.into()],
        events: vec![item("I1", NOW), item("I2", NOW + 7 * 86_400)],
        ..Default::default()
    };
    s.apply_dav(&id, &sync);
    let titles = |s: &Session| s.agenda(NOW - 10, NOW + 8 * 86_400).into_iter().map(|a| a.start).collect::<Vec<_>>();
    assert_eq!(titles(&s), [NOW, NOW + 7 * 86_400], "both meetings, on their own days");
    s.apply_dav(&id, &sync);
    assert_eq!(titles(&s), [NOW, NOW + 7 * 86_400], "and once only after another sync");
    done(s);
}

/// Colleagues' calendars that come with Outlook's calendar groups, and public folders', come
/// unticked the first time (a dozen at once are slow to draw); this account's own, and one
/// opened here by address, come ticked; a tick given later stays.
#[test]
fn calendars_from_groups_come_unticked() {
    use quatrelle_model::pim::{CalendarInfo, DavSync, shared_calendar_id};
    let mut s = Session::open(Some(temp("cal-groups-unticked")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let cal = |id: &str, group: &str| CalendarInfo { id: id.into(), name: id.into(), group: group.into(), ..Default::default() };
    let ana = shared_calendar_id("ana@example.org");
    let tom = shared_calendar_id("tom@example.org");
    let sync = |cals: Vec<CalendarInfo>| DavSync { calendars: Some(cals), checked: NOW, ..Default::default() };
    s.apply_dav(&id, &sync(vec![cal("calendar", ""), cal(&ana, "Équipe : Studio"), cal(&tom, ""), cal("public:F1", "")]));
    assert!(s.calendar_shown(&id, "calendar"));
    assert!(!s.calendar_shown(&id, &ana), "a group's calendar comes unticked");
    assert!(s.calendar_shown(&id, &tom), "one opened by address comes ticked");
    assert!(!s.calendar_shown(&id, "public:F1"));
    // Ticked: it stays so at the next sync.
    s.run("calendar.show", &json!({"account": id, "calendar": ana, "shown": true})).unwrap();
    s.apply_dav(&id, &sync(vec![cal("calendar", ""), cal(&ana, "Équipe : Studio"), cal(&tom, ""), cal("public:F1", "")]));
    assert!(s.calendar_shown(&id, &ana));
}

/// Away from the calendar, an Exchange account's sync reads its own calendar only; opening the
/// calendar reads the calendars ticked at once, and one unticked isn't read.
#[test]
fn calendars_are_read_when_shown() {
    use quatrelle_model::pim::{CalendarInfo, DavSync, shared_calendar_id};
    let mut s = Session::open(Some(temp("cal-read-when-shown")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "riley@contoso.example", "ews": "mail.contoso.example", "username": "CONTOSO\\riley", "password": "pw"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let ana = shared_calendar_id("ana@contoso.example");
    let cal = |id: &str, group: &str| CalendarInfo { id: id.into(), name: id.into(), group: group.into(), ..Default::default() };
    s.apply_dav(&id, &DavSync { calendars: Some(vec![cal("calendar", ""), cal(&ana, "Équipe")]), checked: NOW, ..Default::default() });
    s.take_jobs();
    let pim = |s: &Session| s.pim.get(&id).cloned().unwrap_or_default();
    assert!(pim(&s).own_only, "in Mail");
    assert!(pim(&s).not_read.contains(&ana), "unticked");
    // The calendar opens: the ticked ones are read now.
    s.run("view.module", &json!({"name": "calendar"})).unwrap();
    let jobs = s.take_jobs();
    assert!(!pim(&s).own_only);
    assert!(jobs.iter().any(|j| matches!(j, Job::SyncDav { account } if *account == id)), "{jobs:?}");
    // Ticking Ana's reads it.
    s.run("calendar.show", &json!({"account": id, "calendar": ana, "shown": true})).unwrap();
    let jobs = s.take_jobs();
    assert!(pim(&s).not_read.is_empty());
    assert!(jobs.iter().any(|j| matches!(j, Job::SyncDav { .. })));
    // Back to Mail: nothing more to read until the calendar opens again.
    s.run("view.module", &json!({"name": "mail"})).unwrap();
    let jobs = s.take_jobs();
    assert!(pim(&s).own_only);
    assert!(!jobs.iter().any(|j| matches!(j, Job::SyncDav { .. })));
    // A sync that read this account's own calendar only keeps the others as they were.
    s.apply_dav(&id, &DavSync { calendars: Some(vec![cal("calendar", "")]), partial: true, checked: NOW, ..Default::default() });
    assert!(pim(&s).calendars.iter().any(|c| c.id == ana));
}

/// Dragged to another hour, one occurrence of a series becomes an instance of its own
/// (`RECURRENCE-ID`): the rest of the series stays put, a second drag changes the same
/// instance, Undo puts it back, and the whole series moved later carries it along.
#[test]
fn one_occurrence_moves_on_its_own() {
    let mut s = sample_session("move-occurrence");
    let uid = s
        .run("event.create", &json!({"summary": "Stand-up", "start": "2026-10-19T09:00", "end": "2026-10-19T09:30", "repeat": "weekly"}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let monday = crate::parse_local("2026-10-19T09:00", &quatrelle_model::dates::Clock::new(0, 0)).unwrap().0;
    let starts = |s: &Session| {
        let mut v: Vec<i64> = s.agenda(monday - 10, monday + 28 * 86_400).into_iter().filter(|i| i.summary == "Stand-up").map(|i| i.start).collect();
        v.sort_unstable();
        v
    };
    let second = monday + 7 * 86_400;
    assert_eq!(starts(&s), [monday, second, second + 7 * 86_400, second + 14 * 86_400]);
    // The second week's, an hour later.
    let r = s.run("event.move", &json!({"uid": uid, "occurrence": second, "start": second + 3600})).unwrap();
    assert_eq!((r["occurrence"].as_i64(), r["start"].as_i64(), r["told"].as_str()), (Some(second), Some(second + 3600), Some("none")));
    assert_eq!(starts(&s), [monday, second + 3600, second + 7 * 86_400, second + 14 * 86_400]);
    let instances = |s: &Session| s.pim["sample"].events.iter().filter(|e| e.event.uid == uid && e.event.recurrence_id.is_some()).count();
    assert_eq!(instances(&s), 1);
    let moved = s.agenda(second, second + 86_400).into_iter().find(|i| i.summary == "Stand-up").unwrap();
    assert_eq!((moved.occurrence, moved.end - moved.start, moved.recurring), (second, 1800, true), "still named by its place in the series");
    // Again: the same instance, not a second one; and it can be made longer.
    s.run("event.move", &json!({"uid": uid, "occurrence": second, "start": second + 7200, "end": second + 9000})).unwrap();
    assert_eq!(instances(&s), 1);
    let now_at = s.agenda(second, second + 86_400).into_iter().find(|i| i.summary == "Stand-up").map(|i| (i.start, i.end));
    assert_eq!(now_at, Some((second + 7200, second + 9000)));
    // Undo, twice: back an hour, then back where it began.
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(starts(&s), [monday, second + 3600, second + 7 * 86_400, second + 14 * 86_400]);
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(starts(&s), [monday, second, second + 7 * 86_400, second + 14 * 86_400]);
    // An occurrence the rule doesn't give is refused; so is one taken out of the series.
    assert!(s.run("event.move", &json!({"uid": uid, "occurrence": second + 3600, "start": second + 7200})).is_err());
    s.run("event.delete", &json!({"uid": uid, "occurrence": second + 7 * 86_400})).unwrap();
    assert!(s.run("event.move", &json!({"uid": uid, "occurrence": second + 7 * 86_400, "start": second + 7 * 86_400 + 3600})).is_err());
    // The whole series an hour later: its instance keeps its place in it, the day taken out too.
    s.run("event.move", &json!({"uid": uid, "occurrence": second, "start": second + 1800})).unwrap();
    s.run("event.move", &json!({"uid": uid, "start": monday + 3600})).unwrap();
    assert_eq!(starts(&s), [monday + 3600, second + 1800 + 3600, second + 14 * 86_400 + 3600]);
    let master = s.find_event(&uid).unwrap().1.event.clone();
    assert_eq!(master.exdates, [second + 7 * 86_400 + 3600]);
    // Deleting the instance's occurrence takes the instance away.
    s.run("event.delete", &json!({"uid": uid, "occurrence": second + 3600})).unwrap();
    assert_eq!(instances(&s), 0);
    assert_eq!(starts(&s), [monday + 3600, second + 14 * 86_400 + 3600]);
    // A plain event moves whole, occurrence or not; junk is refused.
    let lunch = s.agenda(NOW - 86_400, NOW + 2 * 86_400).into_iter().find(|i| i.summary == "Lunch with Jun").unwrap();
    s.run("event.move", &json!({"uid": lunch.uid, "occurrence": lunch.occurrence, "start": lunch.start + 3600})).unwrap();
    assert_eq!(s.find_event(&lunch.uid).unwrap().1.event.start, lunch.start + 3600);
    assert!(s.run("event.move", &json!({"uid": lunch.uid})).is_err());
    assert!(s.run("event.move", &json!({"uid": lunch.uid, "start": "noon"})).is_err());
    assert!(s.run("event.move", &json!({"uid": "nobody", "start": NOW})).is_err());
    done(s);
}

/// The guests of a meeting the user organizes are told once: Exchange sends the update
/// itself (the change is marked for the server), so nothing is mailed from here; elsewhere a
/// `REQUEST` goes by mail with its `SEQUENCE` raised. "Save without sending" tells no one.
#[test]
fn exchange_tells_the_guests_itself_and_mail_tells_them_once() {
    use quatrelle_model::pim::CalendarInfo;
    let mut s = Session::open(Some(temp("ews-meeting")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    // As after a first sync: the server's calendar is where new events go.
    s.pim_mut(&id).calendars = vec![CalendarInfo { id: "ews:calendar".into(), name: "Calendar".into(), ..Default::default() }];
    let uid = s
        .run("event.create", &json!({"summary": "Budget", "start": "2026-10-21T15:00", "attendees": "Ana <ana@contoso.example>", "account": id}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let jobs = s.take_jobs();
    assert!(!jobs.iter().any(|j| matches!(j, Job::Send { .. })), "nothing mailed from here: {jobs:?}");
    assert!(jobs.iter().any(|j| matches!(j, Job::SyncDav { .. })));
    let stored = |s: &Session| s.pim[&id].events.iter().find(|e| e.event.uid == uid).cloned().unwrap();
    assert!(stored(&s).notify && stored(&s).dirty, "the server sends the invitations");
    // Moved with "Send update": told by the server, still nothing by mail.
    let at = crate::parse_local("2026-10-21T16:00", &quatrelle_model::dates::Clock::new(0, 0)).unwrap().0;
    let r = s.run("event.move", &json!({"uid": uid, "start": at, "send": true})).unwrap();
    assert_eq!(r["told"], "server");
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Send { .. })));
    assert!(stored(&s).notify);
    // "Save without sending": the server is told to keep quiet.
    let r = s.run("event.move", &json!({"uid": uid, "start": at + 3600, "send": false})).unwrap();
    assert_eq!(r["told"], "none");
    assert!(!stored(&s).notify);
    // Once on the server, the mark is spent.
    s.run("event.move", &json!({"uid": uid, "start": at, "send": true})).unwrap();
    let pushed = quatrelle_model::pim::StoredEvent { href: "I1".into(), etag: "ck".into(), dirty: false, notify: false, ..stored(&s) };
    s.apply_dav(&id, &quatrelle_model::pim::DavSync { pushed_events: vec![pushed], ..Default::default() });
    assert!(!stored(&s).notify && !stored(&s).dirty);
    assert!(s.server_tells_guests(&id) && !s.server_tells_guests("sample"));
    done(s);

    // A calendar with no server to speak for it: the REQUEST goes by mail, once, SEQUENCE up.
    let mut s = sample_session("mail-meeting");
    let uid = s
        .run("event.create", &json!({"summary": "Trailer review", "start": "2026-10-21T15:00", "attendees": "Zoe Park <zoe@studio.example>"}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    // The invitations mailed so far: each one's method and the event's SEQUENCE (by SEQUENCE:
    // sent in the same second, the folder lists them in no fixed order).
    let sent = |s: &Session| -> Vec<(String, u32)> {
        let folder = s.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
        let mut v: Vec<(String, u32)> = folder
            .messages
            .iter()
            .filter(|m| m.subject == "Invitation: Trailer review")
            .filter_map(|m| s.message_calendar(&m.key))
            .map(|c| (c.method.unwrap_or_default(), c.events.first().map_or(0, |e| e.sequence)))
            .collect();
        v.sort();
        v
    };
    assert_eq!(sent(&s), [("REQUEST".to_string(), 0)]);
    let at = crate::parse_local("2026-10-21T16:00", &quatrelle_model::dates::Clock::new(0, 0)).unwrap().0;
    let r = s.run("event.move", &json!({"uid": uid, "start": at, "send": true})).unwrap();
    assert_eq!(r["told"], "mail");
    assert_eq!(sent(&s), [("REQUEST".to_string(), 0), ("REQUEST".to_string(), 1)], "one update, its SEQUENCE raised");
    let count = |s: &Session| sent(s).len();
    s.run("event.move", &json!({"uid": uid, "start": at + 3600, "send": false})).unwrap();
    assert_eq!(count(&s), 2, "saved without sending");
    // Someone else's meeting isn't the user's to tell about, nor to drag.
    s.update_event(&uid, |e| e.organizer = Some(quatrelle_model::ical::Attendee { email: "zoe@studio.example".into(), ..Default::default() }))
        .unwrap();
    let r = s.run("event.move", &json!({"uid": uid, "start": at, "send": true})).unwrap();
    assert_eq!(r["told"], "none");
    assert_eq!(count(&s), 2);
    let item = s.agenda(at - 86_400, at + 86_400).into_iter().find(|i| i.summary == "Trailer review").unwrap();
    assert!(!item.movable && !item.organized, "a guest doesn't drag the organizer's meeting");
    done(s);
}

/// An Exchange account's invitations are answered by the server, on the meeting it placed in the
/// calendar (kept here as `UID#start`): no second copy of the meeting, nothing mailed from here;
/// the answer clears once the server gave it; and a copy made here before (accepted from the
/// invitation, never sent, shown beside the server's meeting) becomes the answer on it.
#[test]
fn an_exchange_invitation_is_answered_on_the_servers_meeting() {
    use quatrelle_model::ical::CalEvent;
    use quatrelle_model::pim::{DavSync, StoredEvent};
    let mut s = sample_session("ews-invite");
    if let Some(a) = s.accounts.iter_mut().find(|a| a.config.id == "sample") {
        a.config.ews = Some("https://mail.studio.example/EWS/Exchange.asmx".into());
    }
    // The server's meeting, as Exchange's is kept: `UID#start`, with its item's address.
    let uid = "mix@sample.quatrelle";
    let server_uid = {
        let p = s.pim_mut("sample");
        let e = p.events.iter_mut().find(|e| e.event.uid == uid).expect("the sample's meeting");
        e.event.uid = format!("{uid}#{}", e.event.start);
        e.href = "E1".into();
        e.event.uid.clone()
    };
    let count = |s: &Session| s.pim["sample"].events.len();
    let before = count(&s);
    let k = s.list.iter().find(|m| m.subject == "Sound mix, version 3").map(|m| m.key.clone()).unwrap();
    s.run("list.open", &json!({"key": k.to_id()})).unwrap();
    s.take_jobs();
    let r = s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(r["byServer"], true);
    assert_eq!(count(&s), before, "no second copy of the meeting");
    let e = s.pim["sample"].events.iter().find(|e| e.event.uid == server_uid).unwrap().clone();
    assert_eq!((e.respond.as_deref(), e.dirty), (Some("ACCEPTED"), true), "the server is to answer");
    assert!(e.event.attendees.iter().any(|a| a.email == "riley@studio.example" && a.partstat == "ACCEPTED"));
    let jobs = s.take_jobs();
    assert!(!jobs.iter().any(|j| matches!(j, Job::Send { .. })), "nothing mailed from here: {jobs:?}");
    assert!(jobs.iter().any(|j| matches!(j, Job::SyncDav { .. })));
    // The server gave it.
    s.apply_dav("sample", &DavSync { responded: vec![(server_uid.clone(), "ACCEPTED".into())], ..Default::default() });
    let e = s.pim["sample"].events.iter().find(|e| e.event.uid == server_uid).unwrap().clone();
    assert_eq!((e.respond.as_deref(), e.dirty), (None, false));
    // A copy made by an older version (from the invitation, never sent), beside the server's.
    let copy =
        StoredEvent { calendar: e.calendar.clone(), event: CalEvent { uid: uid.into(), ..e.event.clone() }, dirty: true, ..Default::default() };
    s.pim_mut("sample").events.push(copy);
    s.take_jobs();
    s.apply_dav("sample", &DavSync::default());
    let p = &s.pim["sample"];
    assert!(!p.events.iter().any(|e| e.href.is_empty() && e.event.uid == uid), "the copy goes");
    let e = p.events.iter().find(|e| e.event.uid == server_uid).unwrap();
    assert_eq!(e.respond.as_deref(), Some("ACCEPTED"), "its answer is given on the server's meeting");
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { .. })));
    done(s);
}

/// A meeting is under one UID in the user's calendar and in each colleague's shared calendar
/// that has it. Moved or deleted in the user's calendar, it's the user's copy that changes: the
/// first one found was taken, a colleague's as often as not, and a deletion took them all (the
/// server asked to delete each where the user may change the calendar). An occurrence changed
/// here whose series the server no longer has can be deleted, and after a sync it's let go,
/// said once, rather than refused at every sync.
#[test]
fn a_meeting_in_colleagues_calendars_changes_only_in_ones_own() {
    use quatrelle_model::ical::CalEvent;
    use quatrelle_model::pim::{CalendarInfo, DavSync, StoredEvent};
    let mut s = Session::open(Some(temp("own-copy")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let (start, uid) = (1_792_569_600, "040000008200E00074C5B7101A82E008#1792569600");
    let (mine, theirs) = ("ews:calendar", "shared:ana@contoso.example");
    let copy = |calendar: &str, href: &str| StoredEvent {
        calendar: calendar.into(),
        href: href.into(),
        etag: "e".into(),
        event: CalEvent { uid: uid.into(), summary: "Safety board".into(), start, end: start + 3600, ..Default::default() },
        ..Default::default()
    };
    {
        let p = s.pim_mut(&id);
        // A colleague's calendar the user may change, listed first.
        p.calendars = vec![
            CalendarInfo { id: theirs.into(), name: "Ana".into(), ..Default::default() },
            CalendarInfo { id: mine.into(), name: "Calendar".into(), ..Default::default() },
        ];
        p.events = vec![copy(theirs, "ana-item"), copy(mine, "my-item")];
    }
    let in_cal = |s: &Session, cal: &str| s.pim[&id].events.iter().find(|e| e.calendar == cal && e.event.uid == uid).cloned();
    s.run("event.move", &json!({"uid": uid, "start": start + 7200})).unwrap();
    assert_eq!(in_cal(&s, mine).map(|e| (e.event.start, e.dirty)), Some((start + 7200, true)), "the user's copy moves");
    assert_eq!(in_cal(&s, theirs).map(|e| (e.event.start, e.dirty)), Some((start, false)), "the colleague's copy is theirs");
    s.run("event.delete", &json!({"uid": uid})).unwrap();
    assert!(in_cal(&s, mine).is_none());
    assert!(in_cal(&s, theirs).is_some(), "deleted in the user's calendar only");
    let asked: Vec<&str> = s.pim[&id].deleted.iter().map(|t| t.href.as_str()).collect();
    assert_eq!(asked, ["my-item"], "only the user's item is deleted on the server");
    // An occurrence changed here, of the series as Exchange once read it; read since as single
    // appointments, the series is no longer anywhere.
    let lone = |at: i64| StoredEvent {
        calendar: mine.into(),
        dirty: true,
        event: CalEvent {
            uid: "040000008200E00074C5B7101A82E008".into(),
            recurrence_id: Some(at),
            summary: "Safety board".into(),
            start: at,
            end: at + 3600,
            ..Default::default()
        },
        ..Default::default()
    };
    s.pim_mut(&id).events.push(lone(start + 7 * 86_400));
    s.run("event.delete", &json!({"uid": "040000008200E00074C5B7101A82E008", "occurrence": start + 7 * 86_400})).expect("deleted alone");
    assert!(!s.pim[&id].events.iter().any(|e| e.event.recurrence_id.is_some()));
    assert!(in_cal(&s, theirs).is_some(), "not the colleague's appointment under the series' name");
    s.pim_mut(&id).events.push(lone(start + 14 * 86_400));
    s.apply_dav(&id, &DavSync::default());
    assert!(!s.pim[&id].events.iter().any(|e| e.event.recurrence_id.is_some()), "let go after a sync");
    assert_eq!(s.notices.iter().filter(|n| n.text.contains("its series is no longer on the server")).count(), 1);
    s.apply_dav(&id, &DavSync::default());
    assert_eq!(s.notices.iter().filter(|n| n.text.contains("its series is no longer on the server")).count(), 1, "said once");
}

/// Other people's calendars set aside while the calendar isn't shown, as Outlook reads them from
/// its offline file only to show them: out of memory and of the calendar's document (into a
/// document of their own) when it's left, back when it's shown. One changed here and not yet on
/// the server stays; a calendar read again meanwhile keeps what was read.
#[test]
fn other_peoples_calendars_are_set_aside_away_from_the_calendar() {
    use quatrelle_model::pim::{Pim, StoredEvent};
    let mut s = sample_session("parking");
    let account = sample::SAMPLE_ACCOUNT;
    let theirs = |s: &Session| s.pim.get(account).map_or(0, |p| p.events.iter().filter(|e| e.calendar.starts_with("shared:")).count());
    let store = s.store.clone().unwrap();
    let on_disk = |store: &Store| store.load_account_doc::<Pim>(account, crate::session::pim::PIM_DOC).unwrap().events.len();
    let aside = |store: &Store| store.load_account_doc::<Vec<StoredEvent>>(account, "pim-parked.json").unwrap_or_default().len();
    // Mail on screen from the start: set aside at once (the session's first jobs).
    assert_eq!(theirs(&s), 0, "set aside at the start");
    assert_eq!(aside(&store), 7);
    let own = on_disk(&store);
    // The calendar shown: back, and nothing left aside.
    s.run("view.calendar", &json!({})).unwrap();
    s.take_jobs();
    assert_eq!(theirs(&s), 7);
    assert_eq!(aside(&store), 0, "nothing left aside");
    // The calendar's document takes them at its next save.
    s.run("calendar.goto", &json!({"date": "2026-10-14"})).ok();
    s.save_pim(account);
    assert_eq!(on_disk(&store), own + 7);
    let today = s.agenda(NOW - 86_400, NOW + 86_400);
    assert!(today.iter().any(|i| i.summary == "Dailies"), "shown in the calendar");
    // Left a moment: still there; five minutes: set aside, but for one changed here.
    s.run("view.mail", &json!({})).unwrap();
    s.take_jobs();
    assert_eq!(theirs(&s), 7, "not at once");
    if let Some(e) = s.pim.get_mut(account).and_then(|p| p.events.iter_mut().find(|e| e.event.summary == "Foley session")) {
        e.dirty = true;
    }
    s.set_clock(NOW + 301, 0);
    s.take_jobs();
    assert_eq!((theirs(&s), aside(&store)), (1, 6), "the one to send stays");
    // A calendar read again meanwhile (its pass came while aside) keeps what was read.
    let zoe = quatrelle_model::pim::shared_calendar_id("zoe@studio.example");
    let read = StoredEvent {
        calendar: zoe.clone(),
        event: quatrelle_model::ical::CalEvent { uid: "new".into(), summary: "Read since".into(), start: NOW, end: NOW + 3600, ..Default::default() },
        href: "item:new".into(),
        ..Default::default()
    };
    let sync = quatrelle_model::pim::DavSync {
        events: vec![read],
        synced_calendars: vec![zoe.clone()],
        calendars: Some(Vec::new()),
        partial: true,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: account.into(), sync: Box::new(sync) });
    s.run("view.calendar", &json!({})).unwrap();
    s.take_jobs();
    let zoes: Vec<String> = s.pim[account].events.iter().filter(|e| e.calendar == zoe).map(|e| e.event.summary.clone()).collect();
    assert_eq!(zoes, ["Read since"]);
    assert_eq!(theirs(&s), 1 + 1 + 4, "the rooms' come back, the one kept not doubled");
    done(s);
}

/// The Webex personal room is kept as typed, only a Webex link, and taken off with nothing.
#[test]
fn a_webex_personal_room_is_set_and_taken_off() {
    let mut s = sample_session("webex-room");
    let r = s.run("calendar.webexRoom", &json!({"url": " https://acme.webex.com/meet/ana "})).unwrap();
    assert_eq!((r["url"].as_str(), s.prefs.webex_room.as_str()), (Some("https://acme.webex.com/meet/ana"), "https://acme.webex.com/meet/ana"));
    for not in ["https://evil.example/meet/ana", "http://acme.webex.com/meet/ana", "acme.webex.com/meet/ana", "https://us02web.zoom.us/j/1"] {
        assert!(s.run("calendar.webexRoom", &json!({"url": not})).is_err(), "{not}");
    }
    assert_eq!(s.prefs.webex_room, "https://acme.webex.com/meet/ana", "a refused one leaves it be");
    s.run("calendar.webexRoom", &json!({"url": ""})).unwrap();
    assert!(s.prefs.webex_room.is_empty());
}

/// A sync that finds nothing new writes nothing: the calendar isn't saved again, nor the address
/// book worked out again (25,000 other people's appointments were written again, compressed, on
/// the window's thread, every minute or two while the calendar was shown); one that changes an
/// appointment is saved. Each appointment read is found by an index: it was looked for through
/// all of those kept, twice.
#[test]
fn a_sync_that_finds_nothing_new_writes_nothing() {
    use quatrelle_model::ical::CalEvent;
    use quatrelle_model::pim::{CalendarInfo, DavSync, StoredEvent};
    let mut s = Session::open(Some(temp("sync-nothing-new")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let event = |cal: &str, n: i64, etag: &str| StoredEvent {
        calendar: cal.into(),
        href: format!("{cal}/I{n}"),
        etag: etag.into(),
        event: CalEvent {
            uid: format!("{cal}/U{n}"),
            summary: format!("Meeting {n}"),
            start: NOW + n * 3_600,
            end: NOW + n * 3_600 + 1_800,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut ids = vec!["calendar".to_string()];
    ids.extend((0..25).map(|k| format!("shared:{k}")));
    let events: Vec<StoredEvent> = ids.iter().flat_map(|c| (0..if c == "calendar" { 700 } else { 1_000 }).map(move |n| event(c, n, "1"))).collect();
    let sync = |events: Vec<StoredEvent>| DavSync {
        calendars: Some(ids.iter().map(|c| CalendarInfo { id: c.clone(), name: c.clone(), ..Default::default() }).collect()),
        synced_calendars: ids.clone(),
        events,
        checked: NOW,
        ..Default::default()
    };
    s.apply_dav(&id, &sync(events.clone()));
    let kept = s.pim.get(&id).map_or(0, |p| p.events.len());
    assert_eq!(kept, 25_700);
    // The same again: nothing written, quickly.
    for _ in 0..1_000 {
        if !s.contacts_pending() {
            break;
        }
        s.contacts_step(10_000);
    }
    assert!(!s.contacts_pending());
    let again = sync(events.clone());
    let (revision, began) = (s.pim_revision, std::time::Instant::now());
    s.apply_dav(&id, &again);
    let took = began.elapsed();
    assert_eq!(s.pim_revision, revision, "nothing new: not saved");
    assert!(!s.contacts_pending(), "nor the address book worked out again");
    assert_eq!(s.pim.get(&id).map_or(0, |p| p.events.len()), kept);
    assert!(took < std::time::Duration::from_secs(3), "25,700 appointments merged in {took:?}");
    // One changed on the server: saved, and shown as it is now.
    let mut changed = events;
    if let Some(e) = changed.iter_mut().find(|e| e.calendar == "calendar" && e.event.summary == "Meeting 3") {
        e.etag = "2".into();
        e.event.summary = "Meeting 3, moved room".into();
    }
    s.apply_dav(&id, &sync(changed));
    assert_ne!(s.pim_revision, revision, "a change is saved");
    let titles: Vec<String> = s.agenda(NOW, NOW + 5 * 3_600).into_iter().filter(|a| a.calendar == "calendar").map(|a| a.summary).collect();
    assert!(titles.iter().any(|t| t == "Meeting 3, moved room"), "{titles:?}");
    assert_eq!(s.pim.get(&id).map_or(0, |p| p.events.len()), kept, "in place of the one it was");
    done(s);
}

/// Showing the calendar and leaving it changes which calendars the next sync reads: kept in a
/// document of its own, the calendar isn't written again for it, nor the address book worked out
/// again (a second at each switch, with 25,000 other people's appointments in the calendar).
#[test]
fn leaving_the_calendar_writes_only_what_to_read() {
    use quatrelle_model::pim::{CalendarInfo, DavSync, PIM_READING_DOC, PimReading};
    let mut s = Session::open(Some(temp("reading-doc")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let cal = CalendarInfo { id: "calendar".into(), name: "Calendar".into(), ..Default::default() };
    s.apply_dav(&id, &DavSync { calendars: Some(vec![cal]), checked: NOW, ..Default::default() });
    s.run("view.module", &json!({"name": "calendar"})).unwrap();
    s.take_jobs();
    for _ in 0..1_000 {
        if !s.contacts_pending() {
            break;
        }
        s.contacts_step(10_000);
    }
    let revision = s.pim_revision;
    s.run("view.module", &json!({"name": "mail"})).unwrap();
    s.take_jobs();
    assert_eq!(s.pim_revision, revision, "the calendar isn't written again");
    assert!(!s.contacts_pending(), "nor the address book worked out again");
    let store = s.store.clone().unwrap();
    let kept: Option<PimReading> = store.load_account_doc(&id, PIM_READING_DOC).unwrap();
    assert_eq!(kept.map(|r| r.own_only), Some(true), "the next sync reads the account's own calendar only");
    // Started again: read as it was left.
    drop(s);
    let again = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    assert!(again.pim.get(&id).is_some_and(|p| p.own_only));
    done(again);
}
