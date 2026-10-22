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
