//! Engine tests: what an address stands for (the contacts field): rooms, from the room lists.

use super::names::with_exchange;
use super::*;
use crate::session::scheduling::Fetched;
use quatrelle_model::AddresseeKind;
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
