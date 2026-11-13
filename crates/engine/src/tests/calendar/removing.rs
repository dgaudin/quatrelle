// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a calendar removed from the list on this computer (never on the server) — out
//! of the side pane, the calendar, the search and the reminders, no longer read by the sync, and
//! brought back; the account's main calendar, and those with a Remove of their own, refused.

use quatrelle_model::pim::{CalendarInfo, LOCAL, PIM_READING_DOC, PimReading};

use super::arranging::{BIRTHDAYS, HOLIDAYS, MAIN, TEAM, TRIPS, cal, ews_main, ews_session, ews_synced, graph_session, sent, shown_ids};
use super::*;
use crate::{EventWhen, RemovalRefusal};

fn removed_names(s: &Session, account: &str) -> Vec<String> {
    s.removed_calendars(account).iter().map(|c| c.name.clone()).collect()
}

fn trips_listed(s: &Session) -> bool {
    s.ordered_calendars("sample").iter().any(|c| c.id == TRIPS)
}

/// Removed: not in the side pane's list, not drawn, not found, no reminder, its events kept as
/// they were; the next sync told to leave it unread. Brought back: listed and shown as before,
/// and read again at once.
#[test]
fn a_removed_calendar_leaves_the_list_and_the_sync_then_comes_back() {
    let mut s = graph_session("removing-graph", true);
    let account = "sample".to_string();
    if let Some(e) = s.pim_mut(&account).events.iter_mut().find(|e| e.calendar == TRIPS) {
        e.event.reminder = Some(15);
    }
    s.run("view.module", &json!({"name": "calendar"})).unwrap();
    s.take_jobs();
    let due = |s: &Session| s.due_reminders(NOW + 3000).iter().any(|r| r.uid == "trip-1");
    assert!(trips_listed(&s) && due(&s) && !s.find_events("Lyon", EventWhen::All, None, None, 10).is_empty(), "before");

    let out = s.run("calendar.remove", &json!({"account": "sample", "calendar": TRIPS})).unwrap();
    assert_eq!(out["removed"], json!(true), "{out}");
    assert!(s.prefs.removed_calendars.contains(&Session::calendar_key("sample", TRIPS)));
    assert!(!trips_listed(&s), "out of the side pane's list");
    assert!(!s.calendar_shown("sample", TRIPS) && !shown_ids(&s).contains(&TRIPS.to_string()));
    assert!(s.agenda(NOW, NOW + 86_400).iter().all(|i| i.calendar != TRIPS), "not drawn");
    assert!(s.find_events("Lyon", EventWhen::All, None, None, 10).is_empty(), "not found");
    assert!(s.find_events("Lyon", EventWhen::All, Some("sample"), Some(TRIPS), 10).is_empty(), "not found, even asked by name");
    assert!(!due(&s), "no reminder from what's no longer read");
    assert_eq!(removed_names(&s, "sample"), ["Trips"]);
    assert!(s.pim.get("sample").is_some_and(|p| p.events.iter().any(|e| e.calendar == TRIPS)), "its events kept here");
    let jobs = s.take_jobs();
    let p = s.pim.get("sample").unwrap();
    assert_eq!(p.removed, [TRIPS], "the sync leaves it unread");
    assert!(p.not_read.contains(&TRIPS.to_string()));
    assert!(!jobs.iter().any(|j| matches!(j, Job::SyncCalendars { .. })), "nothing read for it: {jobs:?}");
    // Twice: still once.
    s.run("calendar.remove", &json!({"account": "sample", "calendar": TRIPS})).unwrap();
    assert_eq!(s.prefs.removed_calendars.len(), 1);

    let out = s.run("calendar.restore", &json!({"account": "sample", "calendar": TRIPS})).unwrap();
    assert_eq!(out["restored"], json!(true), "{out}");
    assert!(trips_listed(&s) && shown_ids(&s).contains(&TRIPS.to_string()) && due(&s));
    assert!(s.removed_calendars("sample").is_empty() && s.prefs.removed_calendars.is_empty());
    let jobs = s.take_jobs();
    assert!(s.pim.get("sample").is_some_and(|p| p.removed.is_empty() && !p.not_read.contains(&TRIPS.to_string())));
    assert!(jobs.iter().any(|j| matches!(j, Job::SyncCalendars { account, .. } if account == "sample")), "read again: {jobs:?}");
    // Brought back when it wasn't removed: nothing to do.
    let out = s.run("calendar.restore", &json!({"account": "sample", "calendar": TRIPS})).unwrap();
    assert_eq!(out["restored"], json!(false));
    done(s);
}

/// The account's main calendar stays, by the menu and by the command; one opened by its owner's
/// address and one subscribed to by its link have their own Remove; every other one goes.
#[test]
fn the_main_calendar_and_those_with_a_remove_of_their_own_stay() {
    let mut s = graph_session("removing-refused", true);
    let account = "sample".to_string();
    {
        let p = s.pim_mut(&account);
        p.shared.push("ana@contoso.example".into());
        p.calendars.push(CalendarInfo { id: "web:moons".into(), name: "Moons".into(), ..Default::default() });
    }
    let remove = |s: &mut Session, c: &str| s.run("calendar.remove", &json!({"account": "sample", "calendar": c}));
    assert_eq!(s.calendar_removable("sample", MAIN), Err(RemovalRefusal::Main));
    assert!(matches!(remove(&mut s, MAIN), Err(CmdError::Disabled(_))));
    for own in ["shared:ana@contoso.example", "web:moons"] {
        assert_eq!(s.calendar_removable("sample", own), Err(RemovalRefusal::OwnRemove), "{own}");
        assert!(matches!(remove(&mut s, own), Err(CmdError::Disabled(_))), "{own}");
    }
    assert!(s.prefs.removed_calendars.is_empty());
    // Its own others, Birthdays, a team's in a group, a read-only one, a colleague's.
    for c in [TRIPS, BIRTHDAYS, TEAM, HOLIDAYS, "AAMkZoe="] {
        assert_eq!(s.calendar_removable("sample", c), Ok(()), "{c}");
    }
    assert!(matches!(remove(&mut s, "AAMkNope="), Err(CmdError::Params(_))));
    assert!(matches!(s.run("calendar.remove", &json!({"account": "nobody", "calendar": TRIPS})), Err(CmdError::Params(_))));
    assert!(matches!(s.run("calendar.restore", &json!({"account": "nobody", "calendar": TRIPS})), Err(CmdError::Params(_))));
    done(s);
}

/// An account whose server names no main calendar (CalDAV): each calendar goes but the last one
/// listed under it, which keeps its heading (where the removed ones come back from); the one
/// kept here for an account without calendars stays.
#[test]
fn without_a_main_calendar_the_last_one_listed_stays() {
    let mut s = sample_session("removing-last");
    let account = "sample".to_string();
    let (home, work) = ("https://dav.contoso.example/cal/home/", "https://dav.contoso.example/cal/work/");
    s.pim_mut(&account).calendars =
        vec![cal(home, "Home", 1), cal(work, "Work", 2), CalendarInfo { group: "Team".into(), ..cal("AAMkT=", "Team", 3) }];
    s.run("calendar.remove", &json!({"account": "sample", "calendar": home})).unwrap();
    assert_eq!(s.calendar_removable("sample", work), Err(RemovalRefusal::Last));
    assert!(matches!(s.run("calendar.remove", &json!({"account": "sample", "calendar": work})), Err(CmdError::Disabled(_))));
    assert_eq!(s.calendar_removable("sample", "AAMkT="), Ok(()), "one in a group isn't under the account's heading");
    s.run("calendar.restore", &json!({"account": "sample", "calendar": home})).unwrap();
    assert_eq!(s.calendar_removable("sample", work), Ok(()));
    s.pim_mut(&account).calendars = vec![CalendarInfo { id: LOCAL.into(), name: "Calendar".into(), ..Default::default() }, cal(home, "Home", 1)];
    assert_eq!(s.calendar_removable("sample", LOCAL), Err(RemovalRefusal::Main));
    done(s);
}

/// Settings written before removed calendars were kept read with none; written with one, read
/// back the same; none isn't written.
#[test]
fn settings_from_before_read_with_nothing_removed() {
    let old: crate::Prefs = serde_json::from_value(json!({"hiddenCalendars": ["sample/x"], "calendarColors": {"sample/x": 3}})).unwrap();
    assert!(old.removed_calendars.is_empty() && old.hidden_calendars.contains("sample/x"));
    let mut p = crate::Prefs::default();
    assert!(serde_json::to_value(&p).unwrap().get("removedCalendars").is_none(), "none isn't written");
    p.removed_calendars.insert("a/b".into());
    let back: crate::Prefs = serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
    assert_eq!(back, p);
    // What a sync reads, kept from before: none removed.
    let reading: PimReading = serde_json::from_value(json!({"ownOnly": true, "notRead": ["x"]})).unwrap();
    assert!(reading.removed.is_empty() && reading.own_only);
}

/// An Exchange account's other calendar, its Birthdays and a colleague's in one of Outlook's
/// groups, removed: the sync is told to leave each unread, in the calendar and away from it,
/// and the worker finds them where it reads what to read.
#[test]
fn an_exchange_accounts_removed_calendars_go_to_its_sync() {
    let mut s = ews_session("removing-ews");
    let (trips, birthdays, ana) = ("ews:folder:AAMkTrips=", "ews:folder:AAMkBirthdays=", "shared:ana@contoso.example");
    let calendars = vec![
        ews_main(),
        cal(trips, "Trips", 2),
        CalendarInfo { read_only: true, ..cal(birthdays, "Birthdays", 3) },
        CalendarInfo { group: "Team".into(), ..cal(ana, "Ana Lima", 4) },
    ];
    ews_synced(&mut s, calendars, vec![sent("train", trips)], false);
    s.run("view.module", &json!({"name": "calendar"})).unwrap();
    s.take_jobs();
    for c in [trips, birthdays, ana] {
        s.run("calendar.remove", &json!({"account": "sample", "calendar": c})).unwrap();
    }
    assert_eq!(removed_names(&s, "sample"), ["Trips", "Birthdays", "Ana Lima"]);
    let shown = shown_ids(&s);
    assert!(shown.contains(&"ews:calendar".to_string()) && [trips, birthdays, ana].iter().all(|c| !shown.contains(&c.to_string())), "{shown:?}");
    s.take_jobs();
    let mut removed = s.pim.get("sample").map(|p| p.removed.clone()).unwrap_or_default();
    removed.sort();
    assert_eq!(removed, [birthdays, trips, ana]);
    let kept: Option<PimReading> = s.store.as_ref().and_then(|st| st.load_account_doc("sample", PIM_READING_DOC).ok()).flatten();
    assert_eq!(kept.map(|r| r.removed.len()), Some(3), "kept where the worker reads what to read");
    s.run("view.module", &json!({"name": "mail"})).unwrap();
    s.take_jobs();
    assert_eq!(s.pim.get("sample").map(|p| p.removed.len()), Some(3), "away from the calendar too");
    done(s);
}
