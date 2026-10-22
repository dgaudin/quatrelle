//! Engine tests: appointments, reminders, and the calendars an account shows.

use super::*;

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
    let monday = crate::parse_local("2026-10-19T09:00", 0).unwrap().0;
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
    let at = crate::parse_local("2026-10-21T16:00", 0).unwrap().0;
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
    // The invitations mailed so far: each one's method and the event's SEQUENCE.
    let sent = |s: &Session| -> Vec<(String, u32)> {
        let folder = s.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
        folder
            .messages
            .iter()
            .filter(|m| m.subject == "Invitation: Trailer review")
            .filter_map(|m| s.message_calendar(&m.key))
            .map(|c| (c.method.unwrap_or_default(), c.events.first().map_or(0, |e| e.sequence)))
            .collect()
    };
    assert_eq!(sent(&s), [("REQUEST".to_string(), 0)]);
    let at = crate::parse_local("2026-10-21T16:00", 0).unwrap().0;
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
