//! Engine tests: what an address stands for (the contacts field): rooms, from the room lists.

use super::names::{answer, with_exchange};
use super::*;
use crate::session::scheduling::Fetched;
use crate::{FieldId, INDEX_MAX, Source};
use quatrelle_model::addr::Accepts;
use quatrelle_model::pim::{CalendarInfo, DavSync, shared_calendar_id};
use quatrelle_model::scheduling::{Room, RoomList};
use quatrelle_model::{Addressee, AddresseeKind};

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

/// Suggestions come from the book, the directory's answers kept, the room lists and the shared
/// mailboxes: an exact match first, then a name or an address that starts with the text, then the
/// most used; a field that takes only rooms is offered only rooms, and never more than eight.
#[test]
fn suggestions_are_ordered_exact_then_prefix_then_most_used_and_only_accepted_kinds() {
    let (mut s, gal) = with_exchange("suggest-order");
    // The window reads the mail into the book a slice per frame (a pass only starts it): here,
    // in one go.
    s.contacts_step(usize::MAX);
    answer(&mut s, &gal, "res", &[("Proginov Reseau", "reseaux@contoso.example")]);
    let team = Addressee::new("Team", "team@contoso.example", AddresseeKind::DistributionList);
    s.apply(Event::DirectoryPeople { account: gal.clone(), query: "team".into(), people: vec![team], more: false, error: None });
    s.apply(seine(&gal));
    let f = FieldId { window: 1, name: "to" };
    let got = s.people_suggest(f, Some(&gal), &Accepts::RECIPIENTS, "zoe");
    assert_eq!(got[0].who.address, "zoe@studio.example");
    assert_eq!(got[0].source, Source::Recent, "saved in Contacts, and written to: the mail counts first");
    s.run("contact.create", &json!({"name": "Quentin Saved", "emails": ["quentin@saved.example"]})).unwrap();
    s.contacts_step(usize::MAX);
    let got = s.people_suggest(f, Some(&gal), &Accepts::RECIPIENTS, "quentin");
    assert_eq!((got[0].who.address.as_str(), got[0].source), ("quentin@saved.example", Source::Contacts), "known from Contacts alone");
    // Written to, so ranked above anything from the directory, but neither exact nor the start of
    // a first or last name.
    s.contacts_sent(&[EmailAddress::new("Old Reseaux", "reseaux@contoso.example.org"), EmailAddress::new("Anna Teresa Lopez", "alopez@x.example")]);
    let got = s.people_suggest(f, Some(&gal), &Accepts::RECIPIENTS, "reseaux@contoso.example");
    assert_eq!((got[0].who.name.as_str(), got[0].source), ("Proginov Reseau", Source::Directory), "an exact match first");
    assert_eq!(got.get(1).map(|x| x.who.address.as_str()), Some("reseaux@contoso.example.org"));
    let got = s.people_suggest(f, Some(&gal), &Accepts::RECIPIENTS, "te");
    let at = |a: &str| got.iter().position(|x| x.who.address == a);
    assert!(at("team@contoso.example").is_some() && got.iter().any(|x| x.who.kind == AddresseeKind::DistributionList), "{got:?}");
    assert!(at("alopez@x.example").is_none_or(|l| at("team@contoso.example") < Some(l)), "a first name before a middle one: {got:?}");
    // Equal otherwise, the kind the field lists first comes first.
    let sales = vec![
        Addressee::new("Sales List", "a-sales@contoso.example", AddresseeKind::DistributionList),
        Addressee::new("Sales Person", "b-sales@contoso.example", AddresseeKind::Person),
    ];
    s.apply(Event::DirectoryPeople { account: gal.clone(), query: "sales".into(), people: sales, more: false, error: None });
    let got = s.people_suggest(f, Some(&gal), &Accepts::RECIPIENTS, "sales");
    assert_eq!(got.iter().map(|x| x.who.kind).collect::<Vec<_>>(), [AddresseeKind::Person, AddresseeKind::DistributionList]);
    let lists_first = Accepts { kinds: &[AddresseeKind::DistributionList, AddresseeKind::Person], one: false };
    assert_eq!(s.people_suggest(f, Some(&gal), &lists_first, "sales")[0].who.kind, AddresseeKind::DistributionList);
    let only_rooms = Accepts { kinds: &[AddresseeKind::Room, AddresseeKind::Equipment], one: false };
    let got = s.people_suggest(f, Some(&gal), &only_rooms, "s");
    assert!(!got.is_empty() && got.iter().all(|x| x.who.kind == AddresseeKind::Room) && got[0].source == Source::RoomList, "{got:?}");
    assert!(s.people_suggest(f, Some(&gal), &Accepts::RECIPIENTS, "zzzz-nobody").is_empty());
    assert!(s.people_suggest(f, Some(&gal), &Accepts::RECIPIENTS, "z").len() <= 8);
    assert!(s.people_suggest(f, Some(&gal), &Accepts::RECIPIENTS, "  ").is_empty(), "nothing typed, nothing offered");
    done(s);
}

/// The search bug of 2026-10-30 again would be a pass per frame: a text typed once costs one
/// pass, however often the window asks, over at most the book, the index and the rooms.
#[test]
fn suggestions_run_one_pass_per_change_of_text_over_a_book_at_its_cap() {
    let mut s = sample_session("suggest-bounded");
    let many: Vec<EmailAddress> = (0..20_000).map(|i| EmailAddress::new(format!("Ann {i}"), format!("ann{i}@big.example"))).collect();
    s.contacts_sent(&many);
    let f = FieldId { window: 1, name: "to" };
    let first = s.people_suggest(f, None, &Accepts::RECIPIENTS, "ann1");
    let after_one = s.people_suggest_passes();
    for _ in 0..9 {
        assert_eq!(s.people_suggest(f, None, &Accepts::RECIPIENTS, "ann1"), first);
    }
    let (passes, scanned) = s.people_suggest_passes();
    assert_eq!(passes, 1, "one pass for one text");
    assert_eq!(scanned, after_one.1, "the frames after it scan nothing");
    assert!(scanned <= 20_000 + 30_000 + 2_000, "{scanned}");
    s.people_suggest(f, None, &Accepts::RECIPIENTS, "ann12");
    assert_eq!(s.people_suggest_passes().0, 2);
    s.people_suggest(FieldId { window: 2, name: "to" }, None, &Accepts::RECIPIENTS, "ann12");
    assert_eq!(s.people_suggest_passes().0, 3, "another field is another pass");
    s.contacts_sent(&[EmailAddress::new("Ann New", "ann12new@big.example")]);
    s.people_suggest(f, None, &Accepts::RECIPIENTS, "ann12");
    assert_eq!(s.people_suggest_passes().0, 3, "the book is being read: under a second, the last answer stands");
    s.set_clock(NOW + 1, 0);
    s.people_suggest(f, None, &Accepts::RECIPIENTS, "ann12");
    assert_eq!(s.people_suggest_passes().0, 4, "the book changed: a second on, the same text is worked out again");
    done(s);
}

/// The directory's answers fill an index that suggestions and kinds read; suggestions never
/// write it, and it holds at most [`INDEX_MAX`] entries, the oldest going first (not cut with
/// the answers kept), until the account goes.
#[test]
fn the_directory_index_feeds_suggestions_and_is_never_filled_by_them() {
    let (mut s, gal) = with_exchange("suggest-index");
    s.contacts_step(usize::MAX);
    answer(&mut s, &gal, "res", &[("Proginov Reseau", "reseaux@contoso.example")]);
    assert!(s.directory_known("RESEAUX@contoso.example").is_some());
    let f = FieldId { window: 1, name: "to" };
    assert!(!s.people_suggest(f, Some(&gal), &Accepts::RECIPIENTS, "zo").is_empty());
    assert!(s.directory_known("zoe@studio.example").is_none(), "a suggestion never fills the index");
    // 300 answers of 100 (the answers kept, the index full), then one more person in an answer
    // asked again: the oldest goes.
    let person = |i: usize| Addressee::new(format!("Person {i}"), format!("p{i}@dir.example"), AddresseeKind::Person);
    for q in 0..299 {
        let people = (q * 100..q * 100 + 100).map(person).collect();
        s.apply(Event::DirectoryPeople { account: gal.clone(), query: format!("q{q}"), people, more: false, error: None });
    }
    assert!(s.directory_known("p0@dir.example").is_some());
    let people = (29_900..30_001).map(person).collect();
    s.apply(Event::DirectoryPeople { account: gal.clone(), query: "q0".into(), people, more: false, error: None });
    assert!(s.people.index.len() <= INDEX_MAX, "{}", s.people.index.len());
    assert!(s.directory_known("p0@dir.example").is_none(), "the oldest gone");
    assert!(s.directory_known("p30000@dir.example").is_some());
    assert_eq!(s.addressee_kind(Some(&gal), "p30000@dir.example"), AddresseeKind::Person, "the index tells the kind");
    // A new name asked past the answers kept: the oldest answer goes, the index keeps its own
    // order (only its FIFO cuts it).
    answer(&mut s, &gal, "fresh", &[("Fresh Face", "fresh@dir.example")]);
    assert!(s.directory_known("p30000@dir.example").is_some(), "not cleared with the answers");
    assert!(s.directory_known("p100@dir.example").is_some());
    assert!(s.directory_known("p1@dir.example").is_none(), "the next oldest made room");
    assert!(s.directory_known("fresh@dir.example").is_some());
    assert_eq!(s.people.index.len(), INDEX_MAX);
    // The command.
    let r = s.run("people.suggest", &json!({"window": 3, "field": "to", "text": "zo"})).unwrap();
    assert_eq!(r[0]["email"], "zoe@studio.example");
    assert_eq!(r[0]["name"], "Zoe Park");
    assert_eq!(r[0]["kind"], "person");
    assert_eq!(r[0]["source"], "recent");
    let r = s.run("people.suggest", &json!({"window": 3, "field": "to", "text": "fresh", "accepts": ["room"]})).unwrap();
    assert_eq!((r[0]["kind"].as_str(), r[0]["source"].as_str()), (Some("unknown"), Some("directory")), "an unknown kind is always taken");
    let r = s.run("people.suggest", &json!({"window": 3, "field": "to", "text": "zo", "accepts": ["room"]})).unwrap();
    assert_eq!(r, json!([]), "a person isn't offered where only rooms go");
    assert!(matches!(s.run("people.suggest", &json!({"window": 3, "field": "nope", "text": "zo"})), Err(CmdError::Params(_))));
    assert!(matches!(s.run("people.suggest", &json!({"window": 3, "field": "to", "text": "zo", "accepts": ["nope"]})), Err(CmdError::Params(_))));
    // contacts.suggest answers through the same pass, its keys kept and the kind and source added.
    let r = s.run("contacts.suggest", &json!({"query": "zo"})).unwrap();
    assert_eq!(r[0]["address"], "Zoe Park <zoe@studio.example>");
    assert_eq!(r[0]["kind"], "person");
    assert_eq!(r[0]["source"], "recent");
    // The account goes: so does what its directory answered.
    s.run("account.remove", &json!({"account": gal})).unwrap();
    assert!(s.directory_known("fresh@dir.example").is_none() && s.people.index.is_empty());
    let r = s.run("people.suggest", &json!({"window": 3, "field": "to", "text": "fresh"})).unwrap();
    assert_eq!(r, json!([]));
    done(s);
}

/// A book being read a slice per frame changes on every frame: the same text is worked out
/// again at most once a second meanwhile, and once more when the reading is done.
#[test]
fn a_book_being_read_redoes_the_same_text_at_most_once_a_second() {
    let store = temp("suggest-reading");
    sample::seed_big_mailbox(&store, 4, 100, NOW).unwrap();
    let mut s = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    let f = FieldId { window: 1, name: "to" };
    s.people_suggest(f, None, &Accepts::RECIPIENTS, "a");
    assert!(s.contacts_pending(), "the first pass only starts the book");
    let (mut frames, mut read) = (0i64, false);
    while !read {
        // Ten frames a second.
        s.set_clock(NOW + frames / 10, 0);
        let before = s.people_suggest_passes().0;
        read = s.contacts_step(5);
        s.people_suggest(f, None, &Accepts::RECIPIENTS, "a");
        frames += 1;
        if read {
            assert_eq!(s.people_suggest_passes().0, before + 1, "the book read: one pass more");
        } else {
            let passes = s.people_suggest_passes().0;
            assert!(passes <= 1 + (frames / 10) as u64, "{passes} passes in {frames} frames");
        }
    }
    assert!(frames >= 30, "the reading took {frames} frames");
    let passes = s.people_suggest_passes().0;
    for _ in 0..5 {
        s.people_suggest(f, None, &Accepts::RECIPIENTS, "a");
    }
    assert_eq!(s.people_suggest_passes().0, passes, "read: nothing changes, nothing is worked out");
    s.people_suggest(f, None, &Accepts::RECIPIENTS, "an");
    assert_eq!(s.people_suggest_passes().0, passes + 1, "a change of text always runs a pass");
    done(s);
}

/// The book's best by score aren't cut before the match is ranked: thousands of colleagues
/// whose address has "pro" in it (`@proginov.fr`) don't push out the one whose name starts with it.
#[test]
fn a_name_that_starts_with_the_text_beats_many_addresses_that_merely_contain_it() {
    let mut s = sample_session("suggest-prefix");
    s.contacts_step(usize::MAX);
    let colleagues: Vec<EmailAddress> = (0..2_000).map(|i| EmailAddress::new(format!("Colleague {i}"), format!("c{i}@proginov.fr"))).collect();
    for _ in 0..3 {
        s.contacts_sent(&colleagues);
    }
    s.contacts_sent(&[EmailAddress::new("Prosper Martin", "pmartin@elsewhere.example")]);
    let got = s.people_suggest(FieldId { window: 1, name: "to" }, None, &Accepts::RECIPIENTS, "pro");
    assert_eq!(got.first().map(|x| x.who.address.as_str()), Some("pmartin@elsewhere.example"), "{got:?}");
    done(s);
}
