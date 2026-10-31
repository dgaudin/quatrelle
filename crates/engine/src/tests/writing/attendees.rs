//! Engine tests: the contacts field of a meeting: what a rooms field refuses, and the names of the
//! meeting's fields.

use super::*;
use crate::FieldId;
use quatrelle_model::{Addressee, AddresseeKind};

/// A list the directory told us about is no room; an address nobody knows goes through, as before
/// (the check is best effort, the server has the last word).
#[test]
fn a_known_list_is_refused_as_a_room_and_an_unknown_address_goes_through() {
    let mut s = sample_session("rooms-refuse-list");
    s.index_directory(&[Addressee::new("Team", "team@contoso.example", AddresseeKind::DistributionList)]);
    let refused = s.run("event.create", &json!({"summary": "Sync", "start": "2026-10-16T14:00", "rooms": "Team <team@contoso.example>"}));
    assert!(matches!(&refused, Err(CmdError::Params(m)) if m == "not a room: team@contoso.example"), "{refused:?}");
    let uid = s.run("event.create", &json!({"summary": "Sync", "start": "2026-10-16T14:00", "rooms": "beamer@contoso.example"})).unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let e = s.find_event(&uid).unwrap().1.event.clone();
    let room = e.attendees.iter().find(|a| a.is_resource()).unwrap();
    assert_eq!((room.email.as_str(), room.cutype.as_str()), ("beamer@contoso.example", "ROOM"));
    // An update refuses it the same way, and keeps the room it had.
    let refused = s.run("event.update", &json!({"uid": uid, "rooms": "team@contoso.example"}));
    assert!(matches!(&refused, Err(CmdError::Params(m)) if m == "not a room: team@contoso.example"), "{refused:?}");
    done(s);
}

#[test]
fn the_meeting_fields_are_known_by_name() {
    for name in ["from", "required", "optional", "rooms", "att-required", "att-optional", "att-rooms"] {
        assert!(FieldId::parse(5, name).is_some(), "{name}");
    }
    assert!(FieldId::parse(5, "nobody").is_none());
}
