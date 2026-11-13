// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a calendar's menu in the side pane — shown alone, its colour and its place kept
//! on this computer, and a calendar made, renamed or deleted where the account's server allows it
//! (Microsoft 365), refused elsewhere and for the calendars that aren't the account's to change.

use quatrelle_model::ical::CalEvent;
use quatrelle_model::pim::{CalendarInfo, DavSync, StoredEvent};

use super::*;
use crate::{ACCOUNT_COLOR, CalendarRefusal, PALETTE_COLOR};

const MAIN: &str = "AAMkMain=";
const TRIPS: &str = "AAMkTrips=";
const BIRTHDAYS: &str = "AAMkBirthdays=";
const TEAM: &str = "AAMkTeam=";
const HOLIDAYS: &str = "AAMkHolidays=";

fn cal(id: &str, name: &str, color: u8) -> CalendarInfo {
    CalendarInfo { id: id.into(), name: name.into(), color, ..Default::default() }
}

/// The sample made a Microsoft 365 account (`graph`), its calendars as the server lists them: the
/// main one, two of its own, a team's in a group, a read-only one and a colleague's; an
/// appointment in Trips.
fn graph_session(name: &str, graph: bool) -> Session {
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

fn shown_ids(s: &Session) -> Vec<String> {
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
    // The sample (on this computer only), then an on-premises Exchange account.
    for ews in [false, true] {
        let mut s = graph_session(if ews { "arranging-ews" } else { "arranging-local" }, false);
        if ews && let Some(a) = s.accounts.iter_mut().find(|a| a.config.id == "sample") {
            a.config.ews = Some("https://mail.contoso.example/EWS/Exchange.asmx".into());
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
