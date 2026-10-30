//! Engine tests: the contacts field: what an address stands for (rooms, from the room lists),
//! suggestions, and names resolved for any field (Check Names generalised).

use super::names::{answer, asked, with_exchange};
use super::*;
use crate::session::scheduling::Fetched;
use crate::{FieldId, INDEX_MAX, MAX_SHOWN_MEMBERS, MAX_TOTAL, Outcome, Source};
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

/// Check Names with the new search scope (the directory, then the Contacts folder): a person both
/// have is one match, written once, and a contact group found alone is written as its reference.
#[test]
fn check_names_with_the_contacts_scope_finds_a_person_once_and_offers_a_contact_group() {
    let (mut s, gal) = with_exchange("scope-contacts");
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "joanna; family"})).unwrap();
    s.run("compose.checkNames", &json!({})).unwrap();
    assert_eq!(asked(&mut s), ["joanna"]);
    // The directory and the Contacts folder both have her: one person, written once.
    answer(&mut s, &gal, "joanna", &[("Joanna Smith", "jo@contoso.example"), ("Joanna Smith", "JO@contoso.example")]);
    assert_eq!(s.compose.as_ref().unwrap().to, "Joanna Smith <jo@contoso.example>; family; ");
    assert_eq!(asked(&mut s), ["family"]);
    let group = Addressee { name: "Dupont family".into(), address: String::new(), kind: AddresseeKind::ContactGroup, item_id: "AAMk=".into() };
    s.apply(Event::DirectoryPeople { account: gal.clone(), query: "family".into(), people: vec![group.clone()], more: false, error: None });
    // Nothing here knows jo@contoso.example by a name of its own: she keeps the directory's.
    assert_eq!(
        s.compose.as_ref().unwrap().to,
        "Joanna Smith <jo@contoso.example>; Dupont family <group:AAMk=>; ",
        "a group found alone is written as its reference"
    );
    let id = s.compose.as_ref().unwrap().id;
    assert!(s.name_choice(id).is_none());
    assert_eq!(
        s.take_people_outcomes(FieldId { window: id, name: "to" }).last().map(|(t, o)| (t.as_str(), o.clone())),
        Some(("family", Outcome::Found(group)))
    );
    done(s);
}

/// Review focus 2: the on-prem Exchange unreachable. Addresses never need the directory (one
/// known takes its name), a malformed one is said, and a name the directory couldn't be asked
/// about is told apart from one it doesn't have; the text stays as typed.
#[test]
fn addresses_alone_resolve_without_the_directory_and_a_name_unreached_is_said() {
    let (mut s, gal) = with_exchange("resolve-offline");
    s.run("mail.new", &json!({})).unwrap();
    let id = s.compose.as_ref().unwrap().id;
    let f = FieldId { window: id, name: "to" };
    s.run("compose.update", &json!({"to": "zoe@studio.example; a@; dupont"})).unwrap();
    let r = s.people_resolve(f, Some(&gal), &Accepts::RECIPIENTS, None).unwrap();
    assert_eq!((r.resolved, r.unresolved.as_slice(), r.searching.as_slice()), (1, &["a@".to_string()][..], &["dupont".to_string()][..]));
    assert_eq!(s.compose.as_ref().unwrap().to, "Zoe Park <zoe@studio.example>; a@; dupont; ", "a known address takes its name; nothing asked for it");
    assert_eq!(asked(&mut s), ["dupont"]);
    assert_eq!(s.people_pending(f), 1);
    s.apply(Event::DirectoryPeople { account: gal.clone(), query: "dupont".into(), people: Vec::new(), more: false, error: Some("offline".into()) });
    let out = s.take_people_outcomes(f);
    assert!(out.contains(&("a@".into(), Outcome::NotAnAddress)) && out.contains(&("dupont".into(), Outcome::Unreached("offline".into()))), "{out:?}");
    assert_eq!(s.people_pending(f), 0);
    assert_eq!(s.compose.as_ref().unwrap().to, "Zoe Park <zoe@studio.example>; a@; dupont; ", "the text stays");
    done(s);
}

/// A field outside the message being written (the address book's To row) hands its text in and
/// gets the rewrite back; its choices and outcomes are kept by its window and name, apart from
/// the message's.
#[test]
fn a_dialogs_field_resolves_its_own_text_and_chooses_by_field() {
    let (mut s, gal) = with_exchange("resolve-dialog");
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "dupont"})).unwrap();
    let f = FieldId { window: 7, name: "book-to" };
    let r = s.run("people.resolve", &json!({"window": 7, "field": "book-to", "text": "Zoe Park; dupont", "account": gal})).unwrap();
    assert_eq!(r["text"], "Zoe Park <zoe@studio.example>; dupont; ");
    assert_eq!((r["resolved"].as_u64(), &r["unresolved"], &r["searching"]), (Some(1), &json!([]), &json!(["dupont"])));
    assert_eq!(s.compose.as_ref().unwrap().to, "dupont", "the message being written is untouched");
    assert_eq!(asked(&mut s), ["dupont"]);
    answer(&mut s, &gal, "dupont", &[("Anne Dupont", "anne.dupont@contoso.example"), ("Marc Dupont", "marc.dupont@contoso.example")]);
    assert_eq!(s.compose.as_ref().unwrap().to, "dupont", "still untouched");
    let choice = s.name_choice_for(f).cloned().expect("two people to choose from");
    assert_eq!((choice.field, choice.token.as_str(), choice.people.len()), (f, "dupont", 2));
    assert_eq!(s.name_choice(7), Some(&choice));
    let marc = json!({"field": "book-to", "token": "dupont", "name": "Marc Dupont", "email": "marc.dupont@contoso.example"});
    let mut other = marc.clone();
    other["window"] = json!(8);
    assert!(matches!(s.run("people.choose", &other), Err(CmdError::Params(_))), "no choice in window 8");
    let mut chosen = marc.clone();
    chosen["window"] = json!(7);
    let out = s.run("people.choose", &chosen).unwrap();
    assert_eq!(out["token"], "dupont");
    let found = s.take_people_outcomes(f);
    assert!(
        found
            .iter()
            .any(|(t, o)| t == "dupont" && matches!(o, Outcome::Found(a) if a.name == "Marc Dupont" && a.address == "marc.dupont@contoso.example")),
        "{found:?}"
    );
    assert!(s.name_choice(7).is_none());
    assert!(s.take_people_outcomes(f).is_empty(), "taken once");
    assert!(
        matches!(s.run("people.resolve", &json!({"window": 7, "field": "book-to"})), Err(CmdError::Params(_))),
        "a dialog's field hands its text in"
    );
    done(s);
}

/// Of several that fit a name, a field keeps the kinds it takes when any is left (Salle Seine in
/// a field for rooms); a field that takes everyone asks which was meant.
#[test]
fn of_several_that_fit_a_field_keeps_the_kinds_it_takes() {
    let (mut s, gal) = with_exchange("resolve-kinds");
    s.apply(seine(&gal));
    let rooms = Accepts { kinds: &[AddresseeKind::Room, AddresseeKind::Equipment], one: false };
    let f = FieldId { window: 9, name: "cli" };
    let r = s.people_resolve(f, Some(&gal), &rooms, Some("seine")).unwrap();
    assert_eq!(r.searching, ["seine"]);
    assert_eq!(asked(&mut s), ["seine"]);
    let people = vec![
        Addressee::new("Salle Seine", "room.seine@contoso.example", AddresseeKind::Person),
        Addressee::new("Seine Martin", "seine.martin@contoso.example", AddresseeKind::Person),
    ];
    s.apply(Event::DirectoryPeople { account: gal.clone(), query: "seine".into(), people, more: false, error: None });
    let room = Addressee::new("Salle Seine", "room.seine@contoso.example", AddresseeKind::Room);
    assert_eq!(s.take_people_outcomes(f), [("seine".to_string(), Outcome::Found(room))], "the room lists tell it's a room");
    // Answered before: settled at once, from what the directory said.
    let to = FieldId { window: 9, name: "to" };
    let r = s.people_resolve(to, Some(&gal), &Accepts::RECIPIENTS, Some("seine")).unwrap();
    assert!(r.searching.is_empty() && asked(&mut s).is_empty());
    assert_eq!(s.name_choice_for(to).map(|c| c.people.len()), Some(2));
    assert_eq!(s.take_people_outcomes(to), [("seine".to_string(), Outcome::Ambiguous)]);
    done(s);
}

/// A window closed takes its fields' state with it: what waited is no longer settled there when
/// the answer comes (the question already out stays, another field may share it), and its
/// choices and outcomes are gone.
#[test]
fn a_window_forgotten_keeps_nothing_and_a_late_answer_settles_nothing_there() {
    let (mut s, gal) = with_exchange("forget-window");
    s.run("mail.new", &json!({})).unwrap();
    let id = s.compose.as_ref().unwrap().id;
    let to = FieldId { window: id, name: "to" };
    s.run("compose.update", &json!({"to": "dupont; martin; a@"})).unwrap();
    s.run("compose.checkNames", &json!({})).unwrap();
    assert_eq!(asked(&mut s), ["dupont"]);
    answer(&mut s, &gal, "dupont", &[("Anne Dupont", "anne.dupont@contoso.example"), ("Marc Dupont", "marc.dupont@contoso.example")]);
    assert_eq!(asked(&mut s), ["martin"]);
    assert!(s.name_choice(id).is_some() && s.people_pending(to) == 1);
    s.forget_people_window(id);
    assert!(s.name_choice(id).is_none(), "its choices go");
    assert_eq!(s.people_pending(to), 0, "what it waited on goes");
    assert!(s.take_people_outcomes(to).is_empty(), "its outcomes go");
    answer(&mut s, &gal, "martin", &[("Paul Martin", "paul.martin@contoso.example")]);
    assert_eq!(s.compose.as_ref().unwrap().to, "dupont; martin; a@; ", "nothing written");
    assert!(s.take_people_outcomes(to).is_empty() && s.name_choice(id).is_none(), "nothing recorded");
    done(s);
}

/// Past the fields whose outcomes are kept, only the one whose outcomes were recorded longest
/// ago goes; a field still recording keeps its place.
#[test]
fn past_the_fields_kept_only_the_oldest_fields_outcomes_go() {
    let mut s = sample_session("outcome-fields");
    let field = |window: u64| FieldId { window, name: "cli" };
    for w in 1..=64 {
        s.people_resolve(field(w), None, &Accepts::RECIPIENTS, Some("a@")).unwrap();
    }
    // The first field records again: now the second is the oldest.
    s.people_resolve(field(1), None, &Accepts::RECIPIENTS, Some("b@")).unwrap();
    s.people_resolve(field(65), None, &Accepts::RECIPIENTS, Some("a@")).unwrap();
    assert!(s.take_people_outcomes(field(2)).is_empty(), "the oldest went");
    assert_eq!(s.take_people_outcomes(field(1)).len(), 2, "recording again kept it");
    for w in [3, 64, 65] {
        assert_eq!(s.take_people_outcomes(field(w)), [("a@".to_string(), Outcome::NotAnAddress)], "window {w}");
    }
    done(s);
}

/// A name given up past the names that may wait (or whose account goes) is unreached, never
/// silently settled: the send gate reads `people_pending` and the outcomes.
#[test]
fn a_name_given_up_while_waiting_is_unreached() {
    let (mut s, gal) = with_exchange("pending-evicted");
    let field = |window: u64| FieldId { window, name: "cli" };
    s.people_resolve(field(1), Some(&gal), &Accepts::RECIPIENTS, Some("n0")).unwrap();
    for w in 2..=5u64 {
        let text: Vec<String> = (0..500).map(|i| format!("m{w}x{i}")).collect();
        let r = s.people_resolve(field(w), Some(&gal), &Accepts::RECIPIENTS, Some(&text.join("; "))).unwrap();
        assert_eq!(r.searching.len(), 500);
    }
    assert_eq!(s.people_pending(field(1)), 0, "the oldest was given up");
    assert_eq!(s.take_people_outcomes(field(1)), [("n0".to_string(), Outcome::Unreached("too many names waiting".into()))]);
    assert_eq!(s.people_pending(field(5)), 500);
    s.run("account.remove", &json!({"account": gal})).unwrap();
    assert_eq!(s.people_pending(field(5)), 0);
    let out = s.take_people_outcomes(field(5));
    assert!(out.len() == 500 && out.iter().all(|(_, o)| matches!(o, Outcome::Unreached(_))), "{:?}", out.first());
    done(s);
}

/// The directory unreachable is never "not found": neither when its answer comes, nor when the
/// failure kept is read again (within the minute it isn't asked again).
#[test]
fn a_directory_that_cant_be_reached_never_says_names_not_found() {
    let (mut s, gal) = with_exchange("unreached-not-found");
    s.run("mail.new", &json!({})).unwrap();
    let id = s.compose.as_ref().unwrap().id;
    s.run("compose.update", &json!({"to": "dupont"})).unwrap();
    s.run("compose.checkNames", &json!({})).unwrap();
    assert_eq!(asked(&mut s), ["dupont"]);
    let notices = s.notices.len();
    s.apply(Event::DirectoryPeople { account: gal.clone(), query: "dupont".into(), people: Vec::new(), more: false, error: Some("offline".into()) });
    let out = s.run("compose.checkNames", &json!({})).unwrap();
    assert!(asked(&mut s).is_empty(), "the failure is kept a minute");
    assert_eq!(out["unresolved"], json!(["dupont"]));
    let said: Vec<&str> = s.notices[notices..].iter().map(|n| n.text.as_str()).collect();
    assert!(!said.contains(&crate::NAMES_NOT_FOUND), "{said:?}");
    let to = s.take_people_outcomes(FieldId { window: id, name: "to" });
    assert_eq!(to, [("dupont".to_string(), Outcome::Unreached("offline".into()))]);
    done(s);
}

/// The same name waiting in two fields (the message's To and its Select Names row) is one
/// question to the server, settled in each field apart.
#[test]
fn one_name_in_two_fields_is_asked_once_and_settled_in_each() {
    let (mut s, gal) = with_exchange("two-fields-one-question");
    s.run("mail.new", &json!({})).unwrap();
    let id = s.compose.as_ref().unwrap().id;
    let (to, book) = (FieldId { window: id, name: "to" }, FieldId { window: id, name: "book-to" });
    s.run("compose.update", &json!({"to": "dupont"})).unwrap();
    s.run("compose.checkNames", &json!({})).unwrap();
    let r = s.people_resolve(book, Some(&gal), &Accepts::RECIPIENTS, Some("dupont")).unwrap();
    assert_eq!(r.searching, ["dupont"]);
    assert_eq!(asked(&mut s), ["dupont"], "one question");
    assert_eq!((s.people_pending(to), s.people_pending(book)), (1, 1));
    answer(&mut s, &gal, "dupont", &[("Marc Dupont", "marc.dupont@contoso.example")]);
    assert_eq!((s.people_pending(to), s.people_pending(book)), (0, 0));
    assert_eq!(s.compose.as_ref().unwrap().to, "Marc Dupont <marc.dupont@contoso.example>; ");
    for f in [to, book] {
        let out = s.take_people_outcomes(f);
        assert!(matches!(out.as_slice(), [(t, Outcome::Found(a))] if t == "dupont" && a.address == "marc.dupont@contoso.example"), "{f:?}: {out:?}");
    }
    assert!(asked(&mut s).is_empty());
    done(s);
}

/// A name typed several times in a field is looked up once.
#[test]
fn a_name_repeated_in_a_field_is_looked_up_once() {
    let mut s = sample_session("repeated-name");
    let before = s.people.lookups;
    let r = s.people_resolve(FieldId { window: 1, name: "cli" }, None, &Accepts::RECIPIENTS, Some("zoe; zoe; zoe")).unwrap();
    assert_eq!(r.text, "Zoe Park <zoe@studio.example>; Zoe Park <zoe@studio.example>; Zoe Park <zoe@studio.example>; ");
    assert_eq!(r.resolved, 3);
    assert_eq!(s.people.lookups - before, 1);
    done(s);
}

/// The jobs asking for a list's members, whatever else was asked.
fn expand_jobs(s: &mut Session) -> Vec<Job> {
    s.take_jobs().into_iter().filter(|j| matches!(j, Job::ExpandList { .. })).collect()
}

fn person(name: &str, email: &str) -> Addressee {
    Addressee::new(name, email, AddresseeKind::Person)
}

fn expanded(account: &str, address: &str, item_id: Option<&str>, members: Vec<Addressee>, more: bool, total: usize) -> Event {
    Event::ListExpanded { account: account.into(), address: address.into(), item_id: item_id.map(String::from), members, more, total, error: None }
}

/// A list is asked once and kept; a nested list stays a list (its members found in the index);
/// a contact group is asked by its item id; an empty list is said, a refusal is said and asked
/// again after a minute; the sample account has no directory.
#[test]
fn a_list_is_expanded_by_address_a_group_by_item_id_a_nested_list_stays_a_list_and_an_empty_one_is_said() {
    let (mut s, gal) = with_exchange("expand-lists");
    let team = json!({"email": "team@contoso.example"});
    let out = s.run("people.expand", &team).unwrap();
    assert_eq!(out["state"], "loading");
    assert_eq!(s.take_jobs(), [Job::ExpandList { account: gal.clone(), address: "team@contoso.example".into(), item_id: None }]);
    assert_eq!(s.run("people.expand", &team).unwrap()["state"], "loading");
    assert!(s.take_jobs().is_empty(), "asked once");

    let sub = Addressee::new("Sub team", "sub@contoso.example", AddresseeKind::DistributionList);
    s.apply(expanded(&gal, "team@contoso.example", None, vec![person("Ann Lee", "ann@contoso.example"), sub.clone()], true, 700));
    let out = s.run("people.expand", &team).unwrap();
    assert_eq!(out["state"], "ready");
    assert_eq!(out["members"][0]["email"], "ann@contoso.example");
    assert_eq!(out["members"][1]["kind"], "distributionList");
    assert_eq!((out["more"].clone(), out["total"].clone()), (json!(true), json!(700)));
    assert_eq!(s.directory_known("sub@contoso.example"), Some(&sub));
    assert!(expand_jobs(&mut s).is_empty(), "an answer is kept");
    let who = Addressee::new("", "TEAM@contoso.example", AddresseeKind::DistributionList);
    assert!(matches!(s.expansion_now(&gal, &who), Some(Fetched::Ready(x)) if x.members.len() == 2), "found by the address in any case");

    // A contact group has no address: asked by its item id.
    let family = json!({"itemId": "AAMk=", "name": "Family", "kind": "contactGroup"});
    assert_eq!(s.run("people.expand", &family).unwrap()["state"], "loading");
    assert_eq!(s.take_jobs(), [Job::ExpandList { account: gal.clone(), address: String::new(), item_id: Some("AAMk=".into()) }]);
    s.apply(expanded(&gal, "", Some("AAMk="), Vec::new(), false, 0));
    let out = s.run("people.expand", &family).unwrap();
    assert_eq!(
        (out["state"].clone(), out["members"].clone(), out["more"].clone(), out["total"].clone()),
        (json!("ready"), json!([]), json!(false), json!(0))
    );

    // A refusal is said, and the next ask tries again.
    let closed = json!({"email": "closed@contoso.example"});
    s.run("people.expand", &closed).unwrap();
    s.take_jobs();
    s.apply(Event::ListExpanded {
        account: gal.clone(),
        address: "closed@contoso.example".into(),
        item_id: None,
        members: Vec::new(),
        more: false,
        total: 0,
        error: Some("ErrorAccessDenied".into()),
    });
    let out = s.run("people.expand", &closed).unwrap();
    assert_eq!((out["state"].clone(), out["error"].clone()), (json!("failed"), json!("ErrorAccessDenied")));
    assert!(expand_jobs(&mut s).is_empty(), "a failure is said, not asked again at once");
    s.set_clock(NOW + 61, 0);
    assert_eq!(s.run("people.expand", &closed).unwrap()["state"], "loading", "asked again after a minute");
    assert_eq!(expand_jobs(&mut s).len(), 1);

    // Not a list, or nothing to ask by.
    assert!(matches!(s.run("people.expand", &json!({"email": "ann@contoso.example", "kind": "person"})), Err(CmdError::Params(_))));
    assert!(matches!(s.run("people.expand", &json!({"kind": "distributionList"})), Err(CmdError::Params(_))));
    assert!(matches!(s.run("people.expand", &json!({"itemId": "not an id!", "kind": "contactGroup"})), Err(CmdError::Params(_))));
    done(s);

    let mut s = sample_session("expand-sample");
    assert!(matches!(s.run("people.expand", &team), Err(CmdError::Disabled(_))));
    assert!(s.take_jobs().is_empty());
    done(s);
}

/// At most 500 members are kept; what is left out is counted from the total, whatever `more`
/// says: members without an address never reach the answer, so fewer than the total may come
/// with `more` unset. A hostile total is clamped.
#[test]
fn an_expansion_keeps_500_members_and_says_how_many_more_from_the_total() {
    let (mut s, gal) = with_exchange("expand-caps");
    let who = |addr: &str| Addressee::new("", addr, AddresseeKind::DistributionList);
    let ask = |s: &mut Session, addr: &str| {
        s.people_expand(None, &who(addr)).unwrap();
        s.take_jobs();
    };
    let many = |n: usize| (0..n).map(|i| person(&format!("P{i}"), &format!("p{i}@contoso.example"))).collect::<Vec<_>>();
    let got = |s: &Session, addr: &str| match s.expansion_now(&gal, &who(addr)) {
        Some(Fetched::Ready(x)) => (x.members.len(), x.more, x.total),
        other => panic!("{other:?}"),
    };

    ask(&mut s, "big@contoso.example");
    s.apply(expanded(&gal, "big@contoso.example", None, many(600), false, 600));
    assert_eq!(got(&s, "big@contoso.example"), (MAX_SHOWN_MEMBERS, true, 600), "100 left out are counted");

    // Three came, ten are said: the seven without an address are not shown, but counted.
    ask(&mut s, "gaps@contoso.example");
    s.apply(expanded(&gal, "gaps@contoso.example", None, many(3), false, 10));
    assert_eq!(got(&s, "gaps@contoso.example"), (3, true, 10));

    // A total below what came is raised to it; one that agrees leaves nothing more.
    ask(&mut s, "low@contoso.example");
    s.apply(expanded(&gal, "low@contoso.example", None, many(2), false, 0));
    assert_eq!(got(&s, "low@contoso.example"), (2, false, 2));

    // A hostile total is clamped.
    ask(&mut s, "huge@contoso.example");
    s.apply(expanded(&gal, "huge@contoso.example", None, many(1), false, usize::MAX));
    assert_eq!(got(&s, "huge@contoso.example"), (1, true, MAX_TOTAL));

    // 500 members indexed, no more.
    assert!(s.directory_known("p499@contoso.example").is_some());
    done(s);
}

/// A late answer (an account gone, a list nobody asked about) brings nothing back, and an ask
/// unanswered for a minute is asked again.
#[test]
fn a_late_or_unasked_expansion_is_ignored_and_a_lost_ask_is_retried() {
    let (mut s, gal) = with_exchange("expand-late");
    let team = json!({"email": "team@contoso.example"});
    s.apply(expanded(&gal, "unasked@contoso.example", None, vec![person("Ann", "ann@contoso.example")], false, 1));
    assert!(s.directory_known("ann@contoso.example").is_none(), "nobody asked");
    s.run("people.expand", &team).unwrap();
    s.take_jobs();
    s.set_clock(NOW + 30, 0);
    s.run("people.expand", &team).unwrap();
    assert!(s.take_jobs().is_empty());
    s.set_clock(NOW + 61, 0);
    s.run("people.expand", &team).unwrap();
    assert_eq!(s.take_jobs().len(), 1, "a lost ask is asked again");
    s.run("account.remove", &json!({"account": gal})).unwrap();
    assert!(s.people.expansions.is_empty());
    s.apply(expanded(&gal, "team@contoso.example", None, vec![person("Ann", "ann@contoso.example")], false, 1));
    assert!(s.people.expansions.is_empty() && s.directory_known("ann@contoso.example").is_none());
    done(s);
}

/// Junk params to `people.expand` on an Exchange account (the sample account refuses first):
/// refused, never a crash, and no ask goes out without a real list.
#[test]
fn people_expand_survives_junk_params_with_a_directory() {
    let (mut s, _gal) = with_exchange("expand-junk");
    for p in [
        json!(null),
        json!({}),
        json!({"email": "@"}),
        json!({"email": 5, "itemId": [], "kind": {"a": 1}}),
        json!({"itemId": "AAMk=", "kind": "room"}),
        json!({"email": format!("{}@contoso.example", "x".repeat(100_000))}),
        json!({"itemId": "A".repeat(600), "kind": "contactGroup"}),
        json!({"account": "nobody", "email": "team@contoso.example"}),
    ] {
        let _ = s.run("people.expand", &p);
    }
    assert!(expand_jobs(&mut s).is_empty(), "nothing asked about junk");
    done(s);
}

/// A late failure never replaces an answer got since: a first ask slow to fail, a second one
/// answered, then the first one's error arrives.
#[test]
fn a_late_error_never_replaces_an_answer() {
    let (mut s, gal) = with_exchange("expand-late-error");
    let team = json!({"email": "team@contoso.example"});
    s.run("people.expand", &team).unwrap();
    s.set_clock(NOW + 61, 0);
    s.run("people.expand", &team).unwrap();
    assert_eq!(expand_jobs(&mut s).len(), 2, "asked again after a minute");
    s.apply(expanded(&gal, "team@contoso.example", None, vec![person("Ann", "ann@contoso.example")], false, 1));
    s.set_clock(NOW + 90, 0);
    s.apply(Event::ListExpanded {
        account: gal.clone(),
        address: "team@contoso.example".into(),
        item_id: None,
        members: Vec::new(),
        more: false,
        total: 0,
        error: Some("timeout".into()),
    });
    let out = s.run("people.expand", &team).unwrap();
    assert_eq!((out["state"].clone(), out["members"].as_array().map(Vec::len)), (json!("ready"), Some(1)));
    assert!(out.get("error").is_none());
    done(s);
}

/// The total is held to what came, not to what was kept: 600 members received are 600 in all
/// (100 more) whatever the server's total says.
#[test]
fn the_total_is_at_least_the_members_received_not_the_members_kept() {
    let (mut s, gal) = with_exchange("expand-total-received");
    let who = Addressee::new("", "big@contoso.example", AddresseeKind::DistributionList);
    for (addr, claimed) in [("big@contoso.example", 0), ("big2@contoso.example", 300)] {
        let who = Addressee { address: addr.into(), ..who.clone() };
        s.people_expand(None, &who).unwrap();
        s.take_jobs();
        let members = (0..600).map(|i| person(&format!("P{i}"), &format!("p{i}@contoso.example"))).collect();
        s.apply(expanded(&gal, addr, None, members, false, claimed));
        match s.expansion_now(&gal, &who) {
            Some(Fetched::Ready(x)) => assert_eq!((x.members.len(), x.more, x.total), (MAX_SHOWN_MEMBERS, true, 600), "claimed {claimed}"),
            other => panic!("{other:?}"),
        }
    }
    done(s);
}

/// Item ids keep their case: two contact groups whose ids differ only by case are two lists.
#[test]
fn contact_groups_whose_ids_differ_by_case_are_two_lists() {
    let (mut s, gal) = with_exchange("expand-id-case");
    for id in ["AAMk=", "aamk="] {
        s.run("people.expand", &json!({"itemId": id, "kind": "contactGroup"})).unwrap();
    }
    assert_eq!(
        s.take_jobs(),
        [
            Job::ExpandList { account: gal.clone(), address: String::new(), item_id: Some("AAMk=".into()) },
            Job::ExpandList { account: gal.clone(), address: String::new(), item_id: Some("aamk=".into()) },
        ]
    );
    s.apply(expanded(&gal, "", Some("AAMk="), vec![person("Ann", "ann@contoso.example")], false, 1));
    let group = |id: &str| Addressee { item_id: id.into(), ..Addressee::new("", "", AddresseeKind::ContactGroup) };
    assert!(matches!(s.expansion_now(&gal, &group("AAMk=")), Some(Fetched::Ready(x)) if x.members.len() == 1));
    assert_eq!(s.expansion_now(&gal, &group("aamk=")), Some(&Fetched::Loading), "the other one is still out");
    assert_eq!(s.people.expansions.len(), 2);
    done(s);
}
