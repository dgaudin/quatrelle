//! Engine tests: what Add calendar and Open another mailbox take: a kind the session knows that
//! the place can't open is refused, an address nobody knows goes through.

use super::names::{answer, with_exchange};
use super::*;
use crate::FieldId;
use quatrelle_model::scheduling::Room;
use quatrelle_model::{Addressee, AddresseeKind};

/// The session with the Exchange account, and a distribution list the directory told us about.
fn knowing_a_list(name: &str) -> (Session, String) {
    let (mut s, id) = with_exchange(name);
    let team = Addressee::new("Team", "team@contoso.example", AddresseeKind::DistributionList);
    s.apply(Event::DirectoryPeople { account: id.clone(), query: "team".into(), people: vec![team], more: false, error: None });
    (s, id)
}

#[test]
fn a_known_list_is_refused_by_open_calendar() {
    let (mut s, _) = knowing_a_list("places-calendar");
    let refused = s.run("calendar.openShared", &json!({"email": "team@contoso.example"}));
    assert!(
        matches!(&refused, Err(CmdError::Params(m)) if m == "a distribution list's calendar can't be opened: team@contoso.example"),
        "{refused:?}"
    );
    s.run("calendar.openShared", &json!({"email": "ana@contoso.example"})).unwrap();
    done(s);
}

#[test]
fn a_known_list_is_refused_by_open_mailbox() {
    let (mut s, id) = knowing_a_list("places-mailbox");
    let refused = s.run("account.openMailbox", &json!({"email": "team@contoso.example"}));
    assert!(matches!(&refused, Err(CmdError::Params(m)) if m == "not a mailbox to open: team@contoso.example"), "{refused:?}");
    assert!(s.account(&id).unwrap().config.shared_mailboxes.is_empty(), "nothing was opened");
    let seine = Room { email: "room.seine@contoso.example".into(), name: "Salle Seine".into(), list: "Paris".into() };
    s.apply(Event::Rooms { account: id.clone(), lists: Vec::new(), rooms: vec![seine], error: None });
    s.run("account.openMailbox", &json!({"email": "room.seine@contoso.example"})).unwrap();
    s.run("account.openMailbox", &json!({"email": "ana@contoso.example"})).unwrap();
    assert_eq!(s.account(&id).unwrap().config.shared_mailboxes, ["room.seine@contoso.example", "ana@contoso.example"]);
    done(s);
}

#[test]
fn the_new_fields_are_known() {
    for name in ["calendar", "mailbox", "rule-one", "rule-any", "search-people"] {
        assert!(FieldId::parse(1, name).is_some(), "{name}");
    }
}

/// Whether `NAMES_NOT_FOUND` was said since the notice numbered `from`.
fn said_not_found(s: &Session, from: usize) -> bool {
    s.notices.get(from..).unwrap_or_default().iter().any(|n| n.text == crate::NAMES_NOT_FOUND)
}

/// A rule's or a Search Folder's field keeps a name nothing matches as typed (it's looked for as
/// text): checking it says nothing of names not found, here or once the directory answered no
/// one. A message's To still says it.
#[test]
fn a_name_nothing_matches_is_no_news_where_it_is_kept_as_typed() {
    // Nothing but the book: settled at once.
    let mut s = sample_session("as-typed-local");
    for field in ["rule-one", "rule-any", "search-people"] {
        let from = s.notices.len();
        s.run("people.resolve", &json!({"window": 1u64 << 51, "field": field, "text": "Zz"})).unwrap();
        assert!(!said_not_found(&s, from), "{field}");
    }
    let from = s.notices.len();
    s.run("people.resolve", &json!({"window": 1u64 << 51, "field": "calendar", "text": "Zz"})).unwrap();
    assert!(said_not_found(&s, from), "a calendar's field still says it");
    done(s);
    // The directory answering no one.
    let (mut s, gal) = with_exchange("as-typed-directory");
    let from = s.notices.len();
    s.run("people.resolve", &json!({"window": 1u64 << 51, "field": "rule-any", "text": "Zz", "account": gal})).unwrap();
    answer(&mut s, &gal, "zz", &[]);
    assert!(!said_not_found(&s, from), "{:?}", s.notices.get(from..));
    s.run("mail.new", &json!({})).unwrap();
    let id = s.compose.as_ref().map_or(0, |c| c.id);
    s.run("compose.update", &json!({"account": gal, "to": "Zy"})).unwrap();
    let from = s.notices.len();
    s.run("people.resolve", &json!({"window": id, "field": "to", "account": gal})).unwrap();
    answer(&mut s, &gal, "zy", &[]);
    assert!(said_not_found(&s, from), "a message's To still says it");
    done(s);
}
