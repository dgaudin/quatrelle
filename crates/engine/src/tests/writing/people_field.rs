//! Engine tests: what an address stands for (the contacts field): rooms, from the room lists.

use super::names::with_exchange;
use super::*;
use crate::session::scheduling::Fetched;
use quatrelle_model::AddresseeKind;
use quatrelle_model::pim::{CalendarInfo, DavSync, shared_calendar_id};
use quatrelle_model::scheduling::{Room, RoomList};

/// The accounts whose room lists are being asked.
fn asked_rooms(s: &mut Session) -> Vec<String> {
    s.take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            Job::GetRooms { account } => Some(account),
            _ => None,
        })
        .collect()
}

/// A room calendar kept in the account's Rooms group: a room the calendar knows, so that a failed
/// ask has something to fall back on that is not the server's set.
fn with_calendar_room(s: &mut Session, account: &str) {
    let info = CalendarInfo {
        id: shared_calendar_id("room.lyon@contoso.example"),
        name: "Salle Lyon".into(),
        group: "Salles".into(),
        read_only: true,
        ..Default::default()
    };
    s.apply_dav(account, &DavSync { calendars: Some(vec![info]), checked: NOW, ..Default::default() });
    s.take_jobs();
}

fn seine(account: &str) -> Event {
    Event::Rooms {
        account: account.into(),
        lists: Vec::new(),
        rooms: vec![Room { email: "room.seine@contoso.example".into(), name: "Salle Seine".into(), list: "Paris".into() }],
        error: None,
    }
}

fn timed_out(account: &str) -> Event {
    Event::Rooms { account: account.into(), lists: Vec::new(), rooms: Vec::new(), error: Some("timeout".into()) }
}

/// The room lists are what tells a room from a person: loaded as the account comes online and
/// once a day after, a failed ask keeps the last set (and the Room Finder's) and is tried again
/// sooner.
#[test]
fn rooms_are_known_from_the_lists_loaded_at_start_up_and_a_day_later() {
    let (mut s, gal) = with_exchange("rooms-startup");
    s.tick();
    assert!(asked_rooms(&mut s).is_empty(), "offline: nothing asked");
    s.apply(Event::Status { account: gal.clone(), status: SyncStatus::Online { at: NOW } });
    s.tick();
    assert_eq!(asked_rooms(&mut s), std::slice::from_ref(&gal));
    s.tick();
    assert!(asked_rooms(&mut s).is_empty(), "asked once");
    s.apply(Event::Rooms {
        account: gal.clone(),
        lists: vec![RoomList { email: "rooms.paris@contoso.example".into(), name: "Paris".into() }],
        rooms: vec![Room { email: "room.seine@contoso.example".into(), name: "Salle Seine".into(), list: "Paris".into() }],
        error: None,
    });
    assert_eq!(s.addressee_kind(Some(&gal), "Room.Seine@contoso.example"), AddresseeKind::Room);
    assert_eq!(s.room_name(&gal, "room.seine@contoso.example"), Some("Salle Seine"));
    assert!(matches!(s.rooms_now(&gal), Some(Fetched::Ready(_))), "the Room Finder was never opened, and has them");
    s.set_clock(NOW + 86_401, 0);
    s.tick();
    assert_eq!(asked_rooms(&mut s), std::slice::from_ref(&gal), "a day later");
    assert!(matches!(s.rooms_now(&gal), Some(Fetched::Ready(_))), "a Room Finder that is open isn't blanked");
    with_calendar_room(&mut s, &gal);
    s.apply(Event::Rooms { account: gal.clone(), lists: Vec::new(), rooms: Vec::new(), error: Some("timeout".into()) });
    assert_eq!(s.addressee_kind(Some(&gal), "room.seine@contoso.example"), AddresseeKind::Room, "a failed refresh keeps the last set");
    assert!(matches!(s.rooms_now(&gal), Some(Fetched::Ready(_))));
    s.set_clock(NOW + 86_401 + 601, 0);
    s.tick();
    assert_eq!(asked_rooms(&mut s), std::slice::from_ref(&gal), "a failure is tried again sooner");
    // The address book is built when first wanted (the window's suggestions do); `addressee_kind`
    // only reads it.
    assert!(!s.suggest_contacts("zoe", 1).is_empty());
    assert_eq!(s.addressee_kind(Some(&gal), "zoe@studio.example"), AddresseeKind::Person, "known from mail");
    assert_eq!(s.addressee_kind(Some(&gal), "nobody@elsewhere.example"), AddresseeKind::Unknown);
    done(s);
}

/// An account removed takes its rooms with it, and an answer arriving late does not bring them
/// back: its rooms are no longer rooms.
#[test]
fn a_removed_accounts_rooms_are_forgotten_and_a_late_answer_is_ignored() {
    let (mut s, gal) = with_exchange("rooms-removed");
    s.apply(seine(&gal));
    assert_eq!(s.addressee_kind(None, "room.seine@contoso.example"), AddresseeKind::Room);
    s.run("account.remove", &json!({"account": gal})).unwrap();
    assert_eq!(s.addressee_kind(None, "room.seine@contoso.example"), AddresseeKind::Unknown, "gone with the account");
    s.apply(seine(&gal));
    assert_eq!(s.addressee_kind(None, "room.seine@contoso.example"), AddresseeKind::Unknown, "a late answer brings nothing back");
    assert!(s.rooms_now(&gal).is_none());
    done(s);
}

/// A timeout while the owner's refresh is out (the Room Finder reads Loading) leaves the
/// server's rooms rooms, even though the calendar offers some of its own.
#[test]
fn a_failed_answer_to_a_refresh_keeps_the_servers_rooms() {
    let (mut s, gal) = with_exchange("rooms-refresh-failed");
    with_calendar_room(&mut s, &gal);
    s.apply(seine(&gal));
    s.request_rooms(&gal, true).unwrap();
    assert!(matches!(s.rooms_now(&gal), Some(Fetched::Loading)));
    s.apply(timed_out(&gal));
    assert_eq!(s.addressee_kind(Some(&gal), "room.seine@contoso.example"), AddresseeKind::Room, "the server's set stays");
    done(s);
}

/// The owner's refresh and the quiet one never queue two asks for an account.
#[test]
fn the_owners_refresh_and_the_quiet_one_never_ask_twice() {
    let (mut s, gal) = with_exchange("rooms-no-double");
    s.apply(Event::Status { account: gal.clone(), status: SyncStatus::Online { at: NOW } });
    s.tick();
    s.request_rooms(&gal, true).unwrap();
    assert_eq!(asked_rooms(&mut s).len(), 1, "the quiet ask is out; the owner's refresh waits for it");
    s.apply(seine(&gal));
    s.request_rooms(&gal, true).unwrap();
    assert_eq!(asked_rooms(&mut s).len(), 1, "answered: a refresh asks");
    s.set_clock(NOW + 86_401, 0);
    s.request_rooms(&gal, true).unwrap();
    s.tick();
    assert_eq!(asked_rooms(&mut s).len(), 1, "the owner's ask is out and the quiet one is due: one ask");
    done(s);
}
