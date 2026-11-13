// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a calendar's menu in the side pane — shown alone, its colour and its place kept
//! on this computer, and a calendar made, renamed or deleted where the account's server allows it
//! (Microsoft 365), refused elsewhere and for the calendars that aren't the account's to change.

use quatrelle_model::ical::CalEvent;
use quatrelle_model::pim::{CalendarInfo, DavSync, StoredEvent};

use super::*;
use crate::{ACCOUNT_COLOR, CalendarRefusal, PALETTE_COLOR};

pub(super) const MAIN: &str = "AAMkMain=";
pub(super) const TRIPS: &str = "AAMkTrips=";
pub(super) const BIRTHDAYS: &str = "AAMkBirthdays=";
pub(super) const TEAM: &str = "AAMkTeam=";
pub(super) const HOLIDAYS: &str = "AAMkHolidays=";

pub(super) fn cal(id: &str, name: &str, color: u8) -> CalendarInfo {
    CalendarInfo { id: id.into(), name: name.into(), color, ..Default::default() }
}

/// The sample made a Microsoft 365 account (`graph`), its calendars as the server lists them: the
/// main one, two of its own, a team's in a group, a read-only one and a colleague's; an
/// appointment in Trips.
pub(super) fn graph_session(name: &str, graph: bool) -> Session {
    let mut s = sample_session(name);
    if let Some(a) = s.accounts.iter_mut().find(|a| a.config.id == "sample") {
        a.config.graph = graph.then(|| "https://graph.microsoft.com/v1.0".into());
    }
    let calendars = vec![
        CalendarInfo { main: true, ..cal(MAIN, "Calendar", 0) },
        cal(TRIPS, "Trips", 2),
        cal(BIRTHDAYS, "Birthdays", 3),
        CalendarInfo { group: "Team".into(), ..cal(TEAM, "Team", 4) },
        CalendarInfo { read_only: true, ..cal(HOLIDAYS, "Holidays", 5) },
        CalendarInfo { read_only: true, ..cal("shared:ana@contoso.example", "Ana", 6) },
        CalendarInfo { not_own: true, ..cal("AAMkZoe=", "Zoe Park", 7) },
    ];
    let trip = CalEvent { uid: "trip-1".into(), summary: "Train to Lyon".into(), start: NOW + 3600, end: NOW + 7200, ..Default::default() };
    let sync = DavSync {
        caldav_home: Some("graph".into()),
        calendars: Some(calendars),
        events: vec![StoredEvent { calendar: TRIPS.into(), href: "AAMkEv1=".into(), event: trip, ..Default::default() }],
        synced_calendars: vec![TRIPS.into()],
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: "sample".into(), sync: Box::new(sync) });
    // Shown, every one (a team's comes unticked).
    for c in [MAIN, TRIPS, BIRTHDAYS, TEAM, HOLIDAYS] {
        s.run("calendar.show", &json!({"account": "sample", "calendar": c, "shown": true})).unwrap();
    }
    s
}

pub(super) fn shown_ids(s: &Session) -> Vec<String> {
    s.shown_calendars().into_iter().map(|c| c.1).collect()
}

fn applied(s: &mut Session) -> Vec<Op> {
    s.take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            Job::Apply { op, .. } => Some(op),
            _ => None,
        })
        .collect()
}

#[test]
fn show_only_hides_every_other_calendar_of_every_account() {
    let mut s = graph_session("arranging-show-only", true);
    assert!(shown_ids(&s).len() > 2, "{:?}", shown_ids(&s));
    let out = s.run("calendar.showOnly", &json!({"account": "sample", "calendar": TRIPS})).unwrap();
    assert_eq!(shown_ids(&s), [TRIPS]);
    assert!(out["hidden"].as_u64().unwrap() >= 5, "{out}");
    // Shown again one by one as before.
    s.run("calendar.show", &json!({"account": "sample", "calendar": MAIN})).unwrap();
    assert_eq!(shown_ids(&s), [MAIN, TRIPS]);
    assert!(matches!(s.run("calendar.showOnly", &json!({"account": "sample", "calendar": "nope"})), Err(CmdError::Params(_))));
}

#[test]
fn a_colour_chosen_here_comes_before_the_servers() {
    let mut s = graph_session("arranging-colour", true);
    let colour = |s: &Session, id: &str| s.shown_calendars().into_iter().find(|c| c.1 == id).map(|c| c.3).unwrap();
    let event_colour =
        |s: &Session| s.agenda(NOW - 86_400, NOW + 86_400).into_iter().find(|i| i.summary == "Train to Lyon").map(|i| i.color).unwrap();
    assert_eq!(colour(&s, TRIPS), 2, "the server's");
    s.run("calendar.color", &json!({"account": "sample", "calendar": TRIPS, "color": 9})).unwrap();
    assert_eq!(colour(&s, TRIPS), PALETTE_COLOR + 9);
    assert_eq!(event_colour(&s), PALETTE_COLOR + 9, "its events with it");
    assert_eq!(s.prefs.calendar_colors.get("sample/AAMkTrips="), Some(&9), "kept on this computer");
    // The account's colour, then back to the server's.
    s.run("calendar.color", &json!({"account": "sample", "calendar": TRIPS, "color": 16})).unwrap();
    let account = s.account("sample").map(|a| a.config.color).unwrap();
    assert_eq!(colour(&s, TRIPS), ACCOUNT_COLOR + account);
    s.run("calendar.color", &json!({"account": "sample", "calendar": TRIPS, "color": null})).unwrap();
    assert_eq!(colour(&s, TRIPS), 2);
    assert!(s.prefs.calendar_colors.is_empty());
    for bad in [json!(17), json!(255), json!(-1), json!("red"), json!(1.5)] {
        assert!(matches!(s.run("calendar.color", &json!({"account": "sample", "calendar": TRIPS, "color": bad})), Err(CmdError::Params(_))), "{bad}");
    }
    assert!(matches!(s.run("calendar.color", &json!({"account": "sample", "calendar": TRIPS})), Err(CmdError::Params(_))), "color is asked for");
    assert_eq!(colour(&s, TRIPS), 2);
}

#[test]
fn calendars_move_within_their_group_and_not_past_its_ends() {
    let mut s = graph_session("arranging-order", true);
    let own = |s: &Session| shown_ids(s).into_iter().filter(|c| [MAIN, TRIPS, BIRTHDAYS].contains(&c.as_str())).collect::<Vec<_>>();
    assert_eq!(own(&s), [MAIN, TRIPS, BIRTHDAYS]);
    assert!(!s.can_move_calendar("sample", MAIN, true) && s.can_move_calendar("sample", MAIN, false));
    assert!(matches!(s.run("calendar.moveUp", &json!({"account": "sample", "calendar": MAIN})), Err(CmdError::Disabled(_))));
    s.run("calendar.moveUp", &json!({"account": "sample", "calendar": BIRTHDAYS})).unwrap();
    assert_eq!(own(&s), [MAIN, BIRTHDAYS, TRIPS]);
    s.run("calendar.moveUp", &json!({"account": "sample", "calendar": BIRTHDAYS})).unwrap();
    assert_eq!(own(&s), [BIRTHDAYS, MAIN, TRIPS]);
    // The read-only one is listed with them (no group); the team's alone in its group, and a
    // colleague's among other people's.
    assert!(!s.can_move_calendar("sample", TEAM, true) && !s.can_move_calendar("sample", TEAM, false), "alone in its group");
    assert!(!s.can_move_calendar("sample", "shared:ana@contoso.example", true), "first of other people's");
    assert!(matches!(s.run("calendar.moveDown", &json!({"account": "sample", "calendar": TEAM})), Err(CmdError::Disabled(_))));
    // The order stays when the server lists them again in its own.
    let again = DavSync {
        caldav_home: Some("graph".into()),
        calendars: Some(vec![cal(TRIPS, "Trips", 2), CalendarInfo { main: true, ..cal(MAIN, "Calendar", 0) }, cal(BIRTHDAYS, "Birthdays", 3)]),
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: "sample".into(), sync: Box::new(again) });
    assert_eq!(own(&s), [BIRTHDAYS, MAIN, TRIPS]);
    assert_eq!(s.ordered_calendars("sample").first().map(|c| c.id.as_str()), Some(BIRTHDAYS));
}

#[test]
fn a_calendar_is_made_renamed_and_deleted_through_the_journal() {
    let mut s = graph_session("arranging-journal", true);
    applied(&mut s);
    assert_eq!(s.calendar_change("sample", None), Ok(()));
    s.run("calendar.create", &json!({"account": "sample", "name": "  Garden\u{202E}  club \n"})).unwrap();
    assert_eq!(applied(&mut s), [Op::CreateCalendar { name: "Garden club".into() }]);
    assert!(
        matches!(s.run("calendar.create", &json!({"account": "sample", "name": "trips"})), Err(CmdError::Params(_))),
        "one with that name already"
    );

    s.run("calendar.rename", &json!({"account": "sample", "calendar": TRIPS, "name": "Travel"})).unwrap();
    assert_eq!(applied(&mut s), [Op::RenameCalendar { calendar: TRIPS.into(), name: "Travel".into() }]);
    assert_eq!(s.ordered_calendars("sample").iter().find(|c| c.id == TRIPS).map(|c| c.name.as_str()), Some("Travel"), "renamed here at once");
    s.run("calendar.rename", &json!({"account": "sample", "calendar": TRIPS, "name": "Travel "})).unwrap();
    assert!(applied(&mut s).is_empty(), "the same name: nothing to send");

    s.run("calendar.color", &json!({"account": "sample", "calendar": TRIPS, "color": 3})).unwrap();
    s.run("calendar.moveUp", &json!({"account": "sample", "calendar": TRIPS})).unwrap();
    let out = s.run("calendar.delete", &json!({"account": "sample", "calendar": TRIPS})).unwrap();
    assert_eq!(out["events"], json!(1));
    assert_eq!(applied(&mut s), [Op::DeleteCalendar { calendar: TRIPS.into() }]);
    assert!(!shown_ids(&s).contains(&TRIPS.to_string()));
    assert!(s.agenda(NOW - 86_400, NOW + 86_400).iter().all(|i| i.summary != "Train to Lyon"), "its events with it");
    let p = s.pim.get("sample").unwrap();
    assert!(p.events.iter().all(|e| e.calendar != TRIPS) && p.deleted.iter().all(|t| t.calendar != TRIPS), "none deleted on its own");
    assert!(!s.prefs.calendar_colors.contains_key("sample/AAMkTrips="));
    assert!(s.prefs.calendar_order.get("sample").is_some_and(|o| !o.iter().any(|c| c == TRIPS)));
    assert!(matches!(s.run("calendar.delete", &json!({"account": "sample", "calendar": TRIPS})), Err(CmdError::Params(_))));
}

#[test]
fn the_main_and_other_peoples_calendars_are_neither_renamed_nor_deleted() {
    let mut s = graph_session("arranging-refused", true);
    applied(&mut s);
    for (id, why) in [
        (MAIN, CalendarRefusal::Main),
        (HOLIDAYS, CalendarRefusal::NotOwn),
        (TEAM, CalendarRefusal::NotOwn),
        ("shared:ana@contoso.example", CalendarRefusal::NotOwn),
        ("local", CalendarRefusal::NotOwn),
        ("AAMkZoe=", CalendarRefusal::NotOwn),
    ] {
        assert_eq!(s.calendar_change("sample", Some(id)), Err(why), "{id}");
        assert!(
            matches!(
                s.run("calendar.rename", &json!({"account": "sample", "calendar": id, "name": "x"})),
                Err(CmdError::Disabled(_) | CmdError::Params(_))
            ),
            "{id}"
        );
        assert!(
            matches!(s.run("calendar.delete", &json!({"account": "sample", "calendar": id})), Err(CmdError::Disabled(_) | CmdError::Params(_))),
            "{id}"
        );
    }
    assert!(applied(&mut s).is_empty());
    assert_eq!(s.calendar_change("sample", Some(TRIPS)), Ok(()));
    // Kept from before the main one was recorded: none is touched until the server says.
    if let Some(c) = s.pim_mut("sample").calendars.iter_mut().find(|c| c.id == MAIN) {
        c.main = false;
    }
    assert_eq!(s.calendar_change("sample", Some(TRIPS)), Err(CalendarRefusal::NotRead));
}

#[test]
fn servers_that_dont_allow_it_refuse_every_one() {
    // The sample (on this computer only), then an IMAP account with a CalDAV server.
    for imap in [false, true] {
        let mut s = graph_session(if imap { "arranging-imap" } else { "arranging-local" }, false);
        if imap && let Some(a) = s.accounts.iter_mut().find(|a| a.config.id == "sample") {
            a.config.imap.host = "imap.contoso.example".into();
        }
        applied(&mut s);
        assert_eq!(s.calendar_change("sample", None), Err(CalendarRefusal::Server));
        assert_eq!(s.calendar_change("sample", Some(TRIPS)), Err(CalendarRefusal::Server));
        assert!(matches!(s.run("calendar.create", &json!({"account": "sample", "name": "Garden"})), Err(CmdError::Disabled(_))));
        assert!(matches!(s.run("calendar.rename", &json!({"account": "sample", "calendar": TRIPS, "name": "Travel"})), Err(CmdError::Disabled(_))));
        assert!(matches!(s.run("calendar.delete", &json!({"account": "sample", "calendar": TRIPS})), Err(CmdError::Disabled(_))));
        assert!(applied(&mut s).is_empty());
        // What stays on this computer works everywhere.
        s.run("calendar.moveDown", &json!({"account": "sample", "calendar": MAIN})).unwrap();
        s.run("calendar.color", &json!({"account": "sample", "calendar": MAIN, "color": 0})).unwrap();
    }
}

#[test]
fn a_typed_name_is_trimmed_refused_empty_and_held_to_its_length() {
    let mut s = graph_session("arranging-names", true);
    for empty in ["", "   ", "\u{202E}\u{200F}", "\n\t"] {
        assert!(matches!(s.run("calendar.create", &json!({"account": "sample", "name": empty})), Err(CmdError::Params(_))), "{empty:?}");
    }
    let long = "a".repeat(quatrelle_model::pim::MAX_CALENDAR_NAME + 1);
    assert!(matches!(s.run("calendar.create", &json!({"account": "sample", "name": long})), Err(CmdError::Params(_))));
    assert!(matches!(s.run("calendar.rename", &json!({"account": "sample", "calendar": TRIPS, "name": long})), Err(CmdError::Params(_))));
    let longest = "é".repeat(quatrelle_model::pim::MAX_CALENDAR_NAME);
    s.run("calendar.create", &json!({"account": "sample", "name": longest})).unwrap();
    assert!(matches!(s.run("calendar.create", &json!({"account": "nobody", "name": "x"})), Err(CmdError::Params(_))));
}

/// Settings written before these two were read as they were, the two empty; written with them,
/// read back the same.
#[test]
fn settings_from_before_read_with_no_colour_or_order() {
    let old: crate::Prefs = serde_json::from_value(json!({"hiddenCalendars": ["sample/x"]})).unwrap();
    assert!(old.calendar_colors.is_empty() && old.calendar_order.is_empty());
    assert!(old.hidden_calendars.contains("sample/x"));
    let mut p = crate::Prefs::default();
    p.calendar_colors.insert("a/b".into(), 4);
    p.calendar_order.insert("a".into(), vec!["b".into(), "c".into()]);
    let back: crate::Prefs = serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
    assert_eq!(back, p);
}

/// The sample made an on-premises Exchange account.
pub(super) fn ews_session(name: &str) -> Session {
    let mut s = sample_session(name);
    if let Some(a) = s.accounts.iter_mut().find(|a| a.config.id == "sample") {
        a.config.ews = Some("https://mail.contoso.example/EWS/Exchange.asmx".into());
    }
    s
}

/// Exchange's main calendar, as its sync lists it.
pub(super) fn ews_main() -> CalendarInfo {
    CalendarInfo { main: true, ..cal("ews:calendar", "Calendar", 0) }
}

/// An appointment the server has (`uid`, in `calendar`).
pub(super) fn sent(uid: &str, calendar: &str) -> StoredEvent {
    StoredEvent {
        calendar: calendar.into(),
        href: format!("AAMk{uid}="),
        event: CalEvent { uid: uid.into(), summary: uid.into(), start: NOW + 3600, end: NOW + 7200, ..Default::default() },
        ..Default::default()
    }
}

/// A calendar sync of the sample's Exchange account listing `calendars` (each read whole), or
/// (`partial`) only some of them.
pub(super) fn ews_synced(s: &mut Session, calendars: Vec<CalendarInfo>, events: Vec<StoredEvent>, partial: bool) {
    let synced_calendars = calendars.iter().map(|c| c.id.clone()).collect();
    let sync = DavSync {
        caldav_home: Some("ews".into()),
        calendars: Some(calendars),
        events,
        synced_calendars,
        partial,
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: "sample".into(), sync: Box::new(sync) });
}

/// An Exchange account's own other calendars (`ews:folder:…`): made, renamed and deleted through
/// the journal, each change then read back from the server as Exchange lists them; one the server
/// no longer lists goes with its appointments, but not on a sync that read only the main one.
#[test]
fn an_exchange_accounts_other_calendars_round_trip() {
    let mut s = ews_session("arranging-ews");
    let (main, event, synced) = (ews_main, sent, ews_synced);
    let trips = "ews:folder:AAMkTrips=";
    synced(&mut s, vec![main(), cal(trips, "Trips", 2)], vec![event("train", trips)], false);
    applied(&mut s);
    assert_eq!(s.calendar_change("sample", Some("ews:calendar")), Err(CalendarRefusal::Main));
    assert_eq!(s.calendar_change("sample", Some(trips)), Ok(()));

    s.run("calendar.create", &json!({"account": "sample", "name": "Garden club"})).unwrap();
    assert_eq!(applied(&mut s), [Op::CreateCalendar { name: "Garden club".into() }]);
    let garden = "ews:folder:AAMkGarden=";
    synced(&mut s, vec![main(), cal(trips, "Trips", 2), cal(garden, "Garden club", 3)], vec![event("train", trips), event("seeds", garden)], false);
    assert!(shown_ids(&s).contains(&garden.to_string()), "made, read back, shown");

    s.run("calendar.rename", &json!({"account": "sample", "calendar": trips, "name": "Travel"})).unwrap();
    assert_eq!(applied(&mut s), [Op::RenameCalendar { calendar: trips.into(), name: "Travel".into() }]);
    s.run("calendar.delete", &json!({"account": "sample", "calendar": trips})).unwrap();
    assert_eq!(applied(&mut s), [Op::DeleteCalendar { calendar: trips.into() }]);
    synced(&mut s, vec![main(), cal(garden, "Garden club", 3)], vec![event("seeds", garden)], false);
    assert!(!shown_ids(&s).contains(&trips.to_string()));

    // Away from the calendar the sync reads the main one alone: Garden club stays.
    synced(&mut s, vec![main()], Vec::new(), true);
    assert!(s.pim.get("sample").is_some_and(|p| p.events.iter().any(|e| e.calendar == garden)), "kept on a partial sync");
    // Deleted in Outlook: the server lists it no more, and its appointments go with it.
    synced(&mut s, vec![main()], Vec::new(), false);
    assert!(!shown_ids(&s).contains(&garden.to_string()));
    assert!(s.pim.get("sample").is_some_and(|p| p.events.iter().all(|e| e.calendar != garden)), "its appointments with it");
}

/// A calendar deleted elsewhere: an appointment made here and never sent that was filed in it
/// goes to the account's main calendar, still to send, said once; one the server has already goes
/// with its calendar.
#[test]
fn an_unsent_appointment_outlives_its_calendar_deleted_elsewhere() {
    let mut s = ews_session("arranging-orphans");
    let trips = "ews:folder:AAMkTrips=";
    ews_synced(&mut s, vec![ews_main(), cal(trips, "Trips", 2)], vec![sent("train", trips)], false);
    let museum = CalEvent { uid: "museum".into(), summary: "Museum".into(), start: NOW + 3600, end: NOW + 7200, ..Default::default() };
    s.pim_mut("sample").events.push(StoredEvent { calendar: trips.into(), event: museum, dirty: true, ..Default::default() });
    let said = s.notices.len();
    ews_synced(&mut s, vec![ews_main()], Vec::new(), false);
    let p = s.pim.get("sample").unwrap();
    let moved = p.events.iter().find(|e| e.event.uid == "museum").map(|e| (e.calendar.clone(), e.dirty, e.href.clone()));
    assert_eq!(moved, Some(("ews:calendar".to_string(), true, String::new())), "in the main calendar, still to send");
    assert!(p.events.iter().all(|e| e.event.uid != "train"), "the one the server had goes with its calendar");
    assert!(s.agenda(NOW - 86_400, NOW + 86_400).iter().any(|i| i.summary == "Museum"), "shown");
    assert_eq!(s.notices.len(), said + 1, "said once: {:?}", s.notices);
    // Read again before it's sent: nothing more is said.
    ews_synced(&mut s, vec![ews_main()], Vec::new(), false);
    assert_eq!(s.notices.len(), said + 1, "{:?}", s.notices);
}

/// The nine commands of a calendar's menu with junk params, on a Microsoft 365 account and an
/// Exchange one (where New, Rename and Delete get past their `when` to their bodies): each answers
/// an error or a result, none fails unexpectedly.
#[test]
fn the_calendar_commands_survive_junk_params_on_exchange_and_microsoft_365() {
    let junk = [
        json!(null),
        json!({}),
        json!({"account": 5, "calendar": [], "name": {"a": 1}, "color": "red"}),
        json!({"account": "sample", "calendar": "AAMkTrips=", "name": 42, "color": -3}),
        json!({"account": "sample", "calendar": null, "name": "", "color": 1e300}),
        json!({"account": "sample", "calendar": "\u{202E}", "name": "\u{0}\u{7}", "color": 16}),
        json!({"account": "nobody", "calendar": "ews:calendar", "name": "x".repeat(10_000), "color": null}),
    ];
    let commands = [
        "calendar.showOnly",
        "calendar.color",
        "calendar.moveUp",
        "calendar.moveDown",
        "calendar.create",
        "calendar.rename",
        "calendar.delete",
        "calendar.remove",
        "calendar.restore",
    ];
    for mut s in [graph_session("arranging-junk-graph", true), {
        let mut s = ews_session("arranging-junk-ews");
        ews_synced(&mut s, vec![ews_main(), cal("ews:folder:AAMkTrips=", "Trips", 2)], Vec::new(), false);
        s
    }] {
        for id in commands {
            for p in &junk {
                if let Err(CmdError::Failed(why)) = s.run(id, p) {
                    panic!("{id} {p}: {why}");
                }
            }
        }
        done(s);
    }
}

/// Someone else's calendar gone from the listing (opened by address, or kept in a group of
/// Outlook's): an appointment made in it and not yet sent never becomes the account's own (it would
/// invite from the account); it stays as it was, unmoved and unsaid.
#[test]
fn an_unsent_appointment_in_someone_elses_calendar_isnt_made_the_accounts() {
    let mut s = ews_session("arranging-orphans-theirs");
    let ana = "shared:ana@contoso.example";
    let team = CalendarInfo { group: "Team".into(), ..cal("shared:zoe@contoso.example", "Zoe Park", 3) };
    ews_synced(&mut s, vec![ews_main(), cal(ana, "Ana Lima", 2), team], Vec::new(), false);
    for (uid, calendar) in [("lunch", ana), ("review", "shared:zoe@contoso.example")] {
        let event = CalEvent { uid: uid.into(), summary: uid.into(), start: NOW + 3600, end: NOW + 7200, ..Default::default() };
        s.pim_mut("sample").events.push(StoredEvent { calendar: calendar.into(), event, dirty: true, ..Default::default() });
    }
    let said = s.notices.len();
    ews_synced(&mut s, vec![ews_main()], Vec::new(), false);
    let p = s.pim.get("sample").unwrap();
    for (uid, calendar) in [("lunch", ana), ("review", "shared:zoe@contoso.example")] {
        let kept = p.events.iter().find(|e| e.event.uid == uid).map(|e| e.calendar.as_str());
        assert_eq!(kept, Some(calendar), "{uid} stays where it was made");
    }
    assert_eq!(s.notices.len(), said, "nothing said: {:?}", s.notices);
}

/// A server that names no main calendar (CalDAV, JMAP): which calendar is the account's own can't
/// be told, so an unsent appointment whose calendar went from the listing stays as it was.
#[test]
fn without_a_main_calendar_an_unsent_appointment_stays_where_it_was() {
    let mut s = sample_session("arranging-orphans-caldav");
    let (work, home) = ("https://dav.contoso.example/cal/work/", "https://dav.contoso.example/cal/home/");
    let synced = |s: &mut Session, calendars: Vec<CalendarInfo>| {
        let synced_calendars = calendars.iter().map(|c| c.id.clone()).collect();
        let sync = DavSync {
            caldav_home: Some("https://dav.contoso.example/cal/".into()),
            calendars: Some(calendars),
            synced_calendars,
            checked: NOW,
            ..Default::default()
        };
        s.apply(Event::DavSynced { account: "sample".into(), sync: Box::new(sync) });
    };
    synced(&mut s, vec![cal(work, "Work", 1), cal(home, "Home", 2)]);
    let event = CalEvent { uid: "dentist".into(), summary: "Dentist".into(), start: NOW + 3600, end: NOW + 7200, ..Default::default() };
    s.pim_mut("sample").events.push(StoredEvent { calendar: home.into(), event, dirty: true, ..Default::default() });
    let said = s.notices.len();
    synced(&mut s, vec![cal(work, "Work", 1)]);
    let kept = s.pim.get("sample").and_then(|p| p.events.iter().find(|e| e.event.uid == "dentist")).map(|e| e.calendar.clone());
    assert_eq!(kept.as_deref(), Some(home), "not moved into another calendar");
    assert_eq!(s.notices.len(), said, "{:?}", s.notices);
}
