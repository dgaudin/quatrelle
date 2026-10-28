//! Engine tests: meeting invitations that change one occurrence, and invitations that mustn't
//! be applied (from the wrong person, or older than what the calendar holds).

use quatrelle_model::ical::{Attendee, CalEvent};
use quatrelle_model::pim::{CalendarInfo, DavSync, StoredEvent};

use super::*;
use crate::InviteProblem;

const WEEK: i64 = 7 * 86_400;
const ANA: (&str, &str) = ("Ana Lima", "ana@soundlab.example");

/// Put a message carrying a calendar in the sample Inbox and open it.
fn deliver(s: &mut Session, n: u32, from: (&str, &str), method: &str, events: &[CalEvent], stamp: i64) {
    let ics = quatrelle_model::ical::write(events, Some(method), stamp);
    let d = quatrelle_mime::Draft {
        to: vec![quatrelle_model::EmailAddress::new("Riley", "riley@studio.example")],
        subject: format!("{method} {n}"),
        body: "See the calendar.".into(),
        attachments: vec![quatrelle_mime::OutAttachment {
            name: "invite.ics".into(),
            mime: format!("text/calendar; method={method}"),
            data: ics.into_bytes(),
        }],
        ..Default::default()
    };
    let raw = quatrelle_mime::build(&d, &quatrelle_model::EmailAddress::new(from.0, from.1), NOW, &format!("<i{n}@x>")).unwrap();
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = quatrelle_model::MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9000 + n };
    inbox.messages.insert(0, quatrelle_mime::summary_from_full(&raw, key.clone(), quatrelle_model::Flags::default(), 0, NOW));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, &raw).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
}

fn me(partstat: &str) -> Attendee {
    Attendee {
        email: "riley@studio.example".into(),
        name: "Riley".into(),
        partstat: partstat.into(),
        rsvp: true,
        cutype: String::new(),
        role: String::new(),
    }
}

fn parts<'a>(s: &'a Session, uid: &str) -> Vec<&'a StoredEvent> {
    s.pim.get("sample").map(|p| p.events.iter().filter(|e| e.event.uid == uid).collect()).unwrap_or_default()
}

/// A weekly meeting Ana organizes, in a server calendar: the series and one moved occurrence in
/// one item (as CalDAV keeps them), and one more moved occurrence stored as its own item (as
/// Exchange and Microsoft Graph keep them).
fn weekly_series(s: &mut Session) -> CalEvent {
    let cal = "https://dav.example.org/cal/work/".to_string();
    let master = CalEvent {
        uid: "weekly@soundlab.example".into(),
        summary: "Mix review".into(),
        start: NOW + 86_400,
        end: NOW + 86_400 + 3600,
        rrule: Some("FREQ=WEEKLY;COUNT=8".into()),
        organizer: Some(Attendee { email: ANA.1.into(), name: ANA.0.into(), ..Default::default() }),
        attendees: vec![me("ACCEPTED")],
        sequence: 1,
        ..Default::default()
    };
    let moved = |week: i64| CalEvent {
        recurrence_id: Some(master.start + week * WEEK),
        start: master.start + week * WEEK + 7200,
        end: master.start + week * WEEK + 10_800,
        rrule: None,
        ..master.clone()
    };
    let stored = |e: CalEvent, href: &str| StoredEvent {
        calendar: cal.clone(),
        href: href.into(),
        etag: "\"1\"".into(),
        event: e,
        dirty: false,
        notify: false,
        respond: None,
    };
    let sync = DavSync {
        caldav_home: Some("https://dav.example.org/cal/".into()),
        calendars: Some(vec![CalendarInfo {
            id: cal.clone(),
            name: "Work".into(),
            color: 1,
            read_only: false,
            ctag: "a".into(),
            group: String::new(),
        }]),
        events: vec![
            stored(master.clone(), &format!("{cal}weekly.ics")),
            stored(moved(1), &format!("{cal}weekly.ics")),
            stored(moved(5), "graph-occurrence-5"),
        ],
        synced_calendars: vec![cal.clone()],
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: "sample".into(), sync: Box::new(sync) });
    assert_eq!(parts(s, &master.uid).len(), 3);
    master
}

#[test]
fn answering_one_occurrence_keeps_the_series() {
    let mut s = sample_session("invite-one");
    let master = weekly_series(&mut s);
    let uid = master.uid.clone();
    let week = |n: i64| master.start + n * WEEK;
    let one = |n: i64, seq: u32| CalEvent {
        recurrence_id: Some(week(n)),
        start: week(n) + 3600,
        end: week(n) + 7200,
        rrule: None,
        sequence: seq,
        summary: format!("Mix review (week {n})"),
        attendees: vec![me("NEEDS-ACTION")],
        ..master.clone()
    };

    // Ana moves week 3 only: accepting it adds that occurrence and keeps the rest.
    deliver(&mut s, 1, ANA, "REQUEST", &[one(3, 2)], NOW);
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    let now = parts(&s, &uid);
    assert_eq!(now.len(), 4, "the series, weeks 1, 3 and 5: {now:?}");
    assert!(now.iter().any(|e| e.event.recurrence_id.is_none() && e.event.rrule.is_some()), "the series is kept");
    assert!(now.iter().any(|e| e.event.recurrence_id == Some(week(1))), "week 1 is kept");
    assert!(now.iter().any(|e| e.event.recurrence_id == Some(week(3)) && e.dirty), "week 3 goes to the server");
    assert!(s.pim["sample"].deleted.is_empty(), "nothing is deleted on the server");

    // Declining week 4 only: an exception on the series, nothing deleted.
    deliver(&mut s, 2, ANA, "REQUEST", &[one(4, 2)], NOW);
    s.run("invite.respond", &json!({"answer": "declined"})).unwrap();
    let m = s.find_event(&uid).expect("the series stays").1.clone();
    assert!(m.event.exdates.contains(&week(4)) && m.dirty, "{m:?}");
    assert_eq!(parts(&s, &uid).len(), 4);
    assert!(s.pim["sample"].deleted.is_empty(), "nothing is deleted on the server");

    // Ana cancels week 1 (stored in the series' item): the item is written again without it.
    deliver(&mut s, 3, ANA, "CANCEL", &[CalEvent { status: "CANCELLED".into(), ..one(1, 3) }], NOW);
    s.run("invite.apply", &json!({})).unwrap();
    let m = s.find_event(&uid).expect("the series stays").1.clone();
    assert!(m.event.exdates.contains(&week(1)) && m.dirty);
    assert!(!parts(&s, &uid).iter().any(|e| e.event.recurrence_id == Some(week(1))), "the moved week 1 is gone");
    assert!(s.pim["sample"].deleted.is_empty(), "the series' item isn't deleted");

    // ... and week 5 (an item of its own on the server): only that item is deleted.
    deliver(&mut s, 4, ANA, "CANCEL", &[CalEvent { status: "CANCELLED".into(), ..one(5, 3) }], NOW);
    s.run("invite.apply", &json!({})).unwrap();
    let hrefs: Vec<&str> = s.pim["sample"].deleted.iter().map(|t| t.href.as_str()).collect();
    assert_eq!(hrefs, ["graph-occurrence-5"]);
    assert!(s.find_event(&uid).is_some_and(|(_, m)| m.event.exdates.contains(&week(5))));
    assert_eq!(parts(&s, &uid).len(), 2, "the series and week 3");

    // A message with the series itself still replaces it.
    let whole = CalEvent { summary: "Mix review (new room)".into(), sequence: 4, attendees: vec![me("NEEDS-ACTION")], ..master.clone() };
    deliver(&mut s, 5, ANA, "REQUEST", &[whole], NOW);
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    let all = parts(&s, &uid);
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!(all[0].event.summary, "Mix review (new room)");
    done(s);
}

#[test]
fn invitations_from_others_or_out_of_date_are_not_applied() {
    let mut s = sample_session("invite-checks");
    let uid = "mix@sample.quatrelle";
    let stored = s.find_event(uid).unwrap().1.event.clone();
    assert_eq!(stored.organizer.as_ref().map(|o| o.email.as_str()), Some(ANA.1));

    // Someone else cancelling Ana's meeting: refused, the event stays.
    let cancel = CalEvent { status: "CANCELLED".into(), sequence: stored.sequence + 1, ..stored.clone() };
    deliver(&mut s, 1, ("Mallory", "mallory@evil.example"), "CANCEL", std::slice::from_ref(&cancel), NOW);
    assert_eq!(s.invite().unwrap().problem, Some(InviteProblem::NotFromOrganizer), "the card says why");
    assert!(s.run("invite.apply", &json!({})).is_err());
    assert!(s.find_event(uid).is_some(), "not cancelled by a stranger");
    // ... or changing it.
    let fake = CalEvent { summary: "Moved to my place".into(), sequence: stored.sequence + 1, ..stored.clone() };
    deliver(&mut s, 2, ("Mallory", "mallory@evil.example"), "REQUEST", &[fake], NOW);
    assert!(s.run("invite.respond", &json!({"answer": "accepted"})).is_err());
    assert_eq!(s.find_event(uid).unwrap().1.event.summary, stored.summary);

    // Ana's update (sequence 5) is taken in.
    let newer = CalEvent { summary: "Sound mix playback (room 2)".into(), sequence: 5, ..stored.clone() };
    deliver(&mut s, 3, ANA, "REQUEST", &[newer], NOW);
    assert_eq!(s.invite().unwrap().problem, None);
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(s.find_event(uid).unwrap().1.event.summary, "Sound mix playback (room 2)");
    // An older one opened afterwards (a lower sequence) doesn't undo it.
    let older = CalEvent { summary: "Sound mix playback (room 1)".into(), sequence: 4, ..stored.clone() };
    deliver(&mut s, 4, ANA, "REQUEST", &[older], NOW);
    assert_eq!(s.invite().unwrap().problem, Some(InviteProblem::OutOfDate));
    assert!(s.run("invite.respond", &json!({"answer": "accepted"})).is_err());
    assert_eq!(s.find_event(uid).unwrap().1.event.summary, "Sound mix playback (room 2)");
    // Nor does one with the same sequence stamped earlier.
    let earlier = CalEvent { summary: "Sound mix playback (draft)".into(), sequence: 5, ..stored.clone() };
    deliver(&mut s, 5, ANA, "REQUEST", &[earlier], NOW - 3600);
    assert!(s.run("invite.respond", &json!({"answer": "accepted"})).is_err());
    assert_eq!(s.find_event(uid).unwrap().1.event.summary, "Sound mix playback (room 2)");
    // Nor an older cancellation.
    let stale = CalEvent { status: "CANCELLED".into(), sequence: 3, ..stored.clone() };
    deliver(&mut s, 6, ANA, "CANCEL", &[stale], NOW);
    assert!(s.run("invite.apply", &json!({})).is_err());
    assert!(s.find_event(uid).is_some());

    // A guest's answer counts only from that guest.
    let own = s
        .run("event.create", &json!({"summary": "Trailer review", "start": "2026-10-21T15:00", "attendees": "Zoe Park <zoe@studio.example>"}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let mut reply = s.find_event(&own).unwrap().1.event.clone();
    reply.attendees = vec![Attendee {
        email: "zoe@studio.example".into(),
        name: "Zoe Park".into(),
        partstat: "DECLINED".into(),
        rsvp: false,
        cutype: String::new(),
        role: String::new(),
    }];
    deliver(&mut s, 7, ("Mallory", "mallory@evil.example"), "REPLY", std::slice::from_ref(&reply), NOW);
    assert_eq!(s.invite().unwrap().problem, Some(InviteProblem::NotFromGuest));
    assert!(s.run("invite.apply", &json!({})).is_err());
    let zoe = |s: &Session| s.find_event(&own).unwrap().1.event.attendees.iter().find(|a| a.email == "zoe@studio.example").unwrap().partstat.clone();
    assert_eq!(zoe(&s), "NEEDS-ACTION");
    deliver(&mut s, 8, ("Zoe Park", "ZOE@studio.example"), "REPLY", &[reply], NOW);
    assert_eq!(s.invite().unwrap().problem, None, "the address is compared without case");
    s.run("invite.apply", &json!({})).unwrap();
    assert_eq!(zoe(&s), "DECLINED");
    done(s);
}

/// An invitation sent to a group the user is in names the group among its guests, not them. On
/// Exchange the server answers it: the answer shows at once and stays (as the server keeps it,
/// `MyResponseType`), and the invitation goes to Deleted Items, as Outlook does. It showed no
/// answer at all, and stayed in the Inbox: the owner found nothing happened (27/10).
#[test]
fn a_group_invitation_shows_its_answer_and_goes() {
    let mut s = sample_session("invite-group");
    if let Some(a) = s.accounts.iter_mut().find(|a| a.config.id == "sample") {
        a.config.ews = Some("https://mail.studio.example/EWS/Exchange.asmx".into());
    }
    let group =
        Attendee { email: "team@studio.example".into(), name: "Team".into(), partstat: "NEEDS-ACTION".into(), rsvp: true, ..Default::default() };
    let meeting = CalEvent {
        uid: "plan@soundlab.example".into(),
        summary: "Planning".into(),
        start: NOW + 86_400,
        end: NOW + 86_400 + 3600,
        organizer: Some(Attendee { email: ANA.1.into(), name: ANA.0.into(), ..Default::default() }),
        attendees: vec![group],
        sequence: 1,
        ..Default::default()
    };
    // Exchange placed it in the calendar, unanswered.
    let cal = "calendar".to_string();
    let read = |etag: &str, answer: &str| DavSync {
        calendars: Some(vec![CalendarInfo { id: cal.clone(), name: "Calendar".into(), ..Default::default() }]),
        events: vec![StoredEvent {
            calendar: cal.clone(),
            href: "AAMk-plan".into(),
            etag: etag.into(),
            event: CalEvent { my_answer: answer.into(), ..meeting.clone() },
            ..Default::default()
        }],
        synced_calendars: vec![cal.clone()],
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: "sample".into(), sync: Box::new(read("ck1", "NEEDS-ACTION")) });
    deliver(&mut s, 7, ANA, "REQUEST", std::slice::from_ref(&meeting), NOW);
    let key = s.reading.as_ref().map(|r| r.key.clone()).unwrap();
    assert_eq!(s.invite().map(|i| i.answer), Some("NEEDS-ACTION".into()));
    s.take_jobs();
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    // Gone from the Inbox to Deleted Items (the server moves it), the answer on its way.
    assert!(!s.list.iter().any(|m| m.key == key), "gone from the Inbox");
    let trash = s.account("sample").and_then(|a| a.folder(FolderRole::Trash)).map(|f| f.path.clone()).unwrap();
    let jobs = s.take_jobs();
    assert!(jobs.iter().any(|j| matches!(j, Job::Apply { op: Op::Move { to, .. }, .. } if *to == trash)), "{jobs:?}");
    assert!(parts(&s, &meeting.uid).iter().any(|e| e.respond.as_deref() == Some("ACCEPTED") && e.event.my_answer == "ACCEPTED"));
    // The server answered, and keeps the answer: the invitation shows it (sent again, say).
    let mut answered = read("ck2", "ACCEPTED");
    answered.responded = vec![(meeting.uid.clone(), "ACCEPTED".into())];
    s.apply(Event::DavSynced { account: "sample".into(), sync: Box::new(answered) });
    assert!(parts(&s, &meeting.uid).iter().all(|e| e.respond.is_none() && !e.dirty));
    deliver(&mut s, 8, ANA, "REQUEST", std::slice::from_ref(&meeting), NOW);
    assert_eq!(s.invite().map(|i| i.answer), Some("ACCEPTED".into()), "the server's answer, the group's guests aside");
    done(s);
}
