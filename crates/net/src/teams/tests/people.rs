// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The worker's people against the fake Graph, the worker driven by hand: the directory searched
//! once for the last thing typed, a second apart, its answers bounded and clean; a conversation
//! found in the copy without a request, or created, listed and said; refused, unanswered and
//! never made twice on its own; people who aren't people; offline and signed out; one creation
//! at a time; and what the log says.

use std::time::Duration;

use quatrelle_model::pim::teams::{self as model, ChatKind, Job, NotCreated, Person};
use serde_json::json;

use super::sync::{Run, STEP, run};
use super::{CHAT, T0, TENANT};
use crate::fake_graph::{FakeChat, FakeMessage, FakeTeams, FakeUser, rfc3339};

/// The user and the people of the directory, by object ids.
const MORGAN: &str = "00000000-0000-0000-0000-0000000000ee";
const ANA: &str = "00000000-0000-0000-0000-0000000000a1";
const BRUNO: &str = "00000000-0000-0000-0000-0000000000a2";
const CARLA: &str = "00000000-0000-0000-0000-0000000000a3";
const GROUP: &str = "19:g5h6@thread.v2";
const MEETING: &str = "19:m7n8@meeting_x@thread.v2";

fn morgan() -> FakeUser {
    FakeUser::new(MORGAN, "Morgan Lee")
}

fn user(id: &str, name: &str) -> FakeUser {
    FakeUser::new(id, name)
}

fn person(id: &str, name: &str, guest: bool) -> Person {
    let first = name.split_whitespace().next().unwrap().to_lowercase();
    Person { id: id.into(), name: name.into(), address: format!("{first}@contoso.example"), guest }
}

fn ana() -> Person {
    person(ANA, "Ana Exemple", false)
}

fn bruno() -> Person {
    person(BRUNO, "Bruno Reis", false)
}

fn carla() -> Person {
    Person { address: "carla@fabrikam.example".into(), ..person(CARLA, "Carla Dias", true) }
}

/// A directory of Ana, Bruno, Carla (a guest) and the user, and the chats given.
fn teams(chats: Vec<FakeChat>) -> FakeTeams {
    let mut t = FakeTeams::new(morgan(), TENANT);
    let mut carla = user(CARLA, "Carla Dias");
    carla.mail = "carla@fabrikam.example".into();
    t.directory = vec![user(ANA, "Ana Exemple"), user(BRUNO, "Bruno Reis"), carla, user(MORGAN, "Morgan Lee")];
    t.guests = vec![CARLA.into()];
    t.chats = chats;
    t
}

/// A chat of `kind` with `members`, one message of the first other at `at`.
fn chat_of_kind(id: &str, kind: &'static str, topic: Option<&str>, members: Vec<FakeUser>, at: i64) -> FakeChat {
    let from = members[1].id.clone();
    let mut c = FakeChat::new(id, kind, topic, members);
    c.messages.push(FakeMessage::new(&at.to_string(), Some(&from), &rfc3339(at), "<p>Hello</p>"));
    c
}

fn one_to_one(id: &str) -> FakeChat {
    chat_of_kind(id, "oneOnOne", None, vec![morgan(), user(ANA, "Ana Exemple")], T0)
}

fn group(id: &str, kind: &'static str, at: i64) -> FakeChat {
    chat_of_kind(id, kind, Some("Plans"), vec![morgan(), user(ANA, "Ana Exemple"), user(BRUNO, "Bruno Reis")], at)
}

impl Run {
    fn create(&mut self, local: &str, people: Vec<Person>, topic: Option<&str>) {
        self.w.job(Job::Create { local: local.into(), people, topic: topic.map(str::to_string) });
    }

    fn directory(&mut self, query: &str) {
        self.w.job(Job::Directory { query: query.into() });
    }

    fn peoples(&self) -> Vec<(String, Vec<Person>, bool)> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                model::Event::People { query, people, failed } => Some((query, people, failed)),
                _ => None,
            })
            .collect()
    }

    /// Every creation said, as (local, how it went): the chat when made, else why not.
    fn made(&self) -> Vec<(String, Result<String, (NotCreated, Option<String>)>)> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                model::Event::Created { local, chat } => Some((local, Ok(chat))),
                model::Event::NotCreated { local, why, code } => Some((local, Err((why, code)))),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn a_search_asks_once_for_the_last_thing_typed() {
    let mut r = run(teams(Vec::new()));
    for q in ["a", "an", "ana"] {
        r.directory(q);
    }
    r.turns(Duration::from_secs(3));
    assert_eq!(r.hits("users"), 1);
    assert_eq!(r.peoples(), vec![("ana".to_string(), vec![ana()], false)]);
    // Nothing is asked again unasked; an empty query forgets what waited.
    r.directory("bruno");
    r.directory("");
    r.turns(Duration::from_secs(3));
    assert_eq!((r.hits("users"), r.peoples().len()), (1, 1));
}

/// H12: what is typed is one quoted term in each of the two fields, whatever it holds.
#[test]
fn a_search_is_one_quoted_term() {
    let mut r = run(teams(Vec::new()));
    r.directory("ana\" OR \"mail:eve");
    r.turns(Duration::from_secs(3));
    let search = r.fake(|t| t.last_search.clone());
    assert_eq!(search.matches('"').count(), 4, "{search}");
    assert_eq!(r.hits("users"), 1);
}

/// H13: at most 25, none of an id that isn't an object id, nor the user, a guest said so.
#[test]
fn people_found_are_bounded_and_clean() {
    let mut t = teams(Vec::new());
    let mut carla = user(CARLA, "Contoso Carla");
    carla.mail = "carla@fabrikam.example".into();
    let mut list = vec![user("a/b", "Contoso Slash"), user(MORGAN, "Contoso Morgan"), carla];
    list.extend((0..37).map(|i| user(&format!("00000000-0000-0000-0000-000000000c{i:02}"), &format!("Contoso Person {i}"))));
    t.directory = list;
    let mut r = run(t);
    r.directory("contoso");
    r.turns(Duration::from_secs(3));
    let (_, found, failed) = r.peoples().pop().unwrap();
    assert!(!failed && !found.is_empty() && found.len() <= 25, "{}", found.len());
    assert!(found.iter().all(|p| p.id != "a/b" && !p.id.eq_ignore_ascii_case(MORGAN)), "{found:?}");
    assert!(found.iter().any(|p| p.id == CARLA && p.guest));
    assert!(found.iter().all(|p| p.id == CARLA || !p.guest));
}

#[test]
fn searches_go_a_second_apart() {
    let mut r = run(teams(Vec::new()));
    let mut hits = Vec::new();
    let mut last = r.hits("users");
    r.directory("ana");
    for i in 0..30 {
        if i == 1 {
            r.directory("bruno");
        }
        r.turns(STEP);
        if r.hits("users") > last {
            last = r.hits("users");
            hits.push(r.at);
        }
    }
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(hits[1] - hits[0] >= Duration::from_secs(1), "{hits:?}");
    assert_eq!(r.peoples().len(), 2);
}

#[test]
fn a_person_already_chatted_with_is_found_without_a_request() {
    let mut r = run(teams(vec![one_to_one(CHAT)]));
    assert_eq!(r.record(CHAT).chat.members.len(), 2, "its members are in the copy");
    r.create("1-1", vec![ana()], None);
    assert_eq!(r.made(), vec![("1-1".to_string(), Ok(CHAT.to_string()))], "said at once");
    r.turns(Duration::from_secs(3));
    assert_eq!(r.hits("createChat"), 0);
    // That chat left: Teams is asked, and gives it back.
    let mut rec = r.record(CHAT);
    rec.chat.left = true;
    r.w.copy.as_ref().unwrap().put_chat(&rec).unwrap();
    r.create("1-2", vec![ana()], None);
    r.turns(Duration::from_secs(5));
    assert_eq!(r.hits("createChat"), 1);
    assert_eq!(r.made().pop(), Some(("1-2".to_string(), Ok(CHAT.to_string()))));
    assert!(!r.record(CHAT).chat.left, "it is in the user's list again");
    assert_eq!(r.fake(|t| t.chats.len()), 1, "no second chat");
}

/// Review Focus 5.
#[test]
fn a_group_of_the_same_people_is_found_without_a_request() {
    // A meeting's chat of the same people, more recent, is never taken.
    let mut r = run(teams(vec![group(GROUP, "group", T0), group(MEETING, "meeting", T0 + 60_000)]));
    assert_eq!(r.record(GROUP).chat.members.len(), 3);
    r.create("1-1", vec![bruno(), ana()], None);
    assert_eq!(r.made(), vec![("1-1".to_string(), Ok(GROUP.to_string()))]);
    r.turns(Duration::from_secs(3));
    assert_eq!(r.hits("createChat"), 0);
    // Another set of people: a request each (Ana alone, then with Carla too).
    r.create("1-2", vec![ana()], None);
    r.turns(Duration::from_secs(3));
    assert_eq!(r.hits("createChat"), 1);
    r.create("1-3", vec![ana(), bruno(), carla()], None);
    r.turns(Duration::from_secs(3));
    assert_eq!(r.hits("createChat"), 2);
    let made = r.made();
    assert!(made.iter().all(|(_, m)| m.is_ok()), "{made:?}");
    assert_ne!(made[2].1.as_ref().unwrap(), GROUP);
}

#[test]
fn a_new_chat_is_created_listed_and_said() {
    let mut r = run(teams(Vec::new()));
    r.create("1-1", vec![ana()], None);
    r.turns(Duration::from_secs(4));
    let chat = r.made().pop().unwrap().1.unwrap();
    let base = r.server.base.clone();
    let member = |id: &str, role: &str| json!({"@odata.type": "#microsoft.graph.aadUserConversationMember", "roles": [role], "user@odata.bind": format!("{base}/users('{id}')")});
    assert_eq!(r.fake(|t| t.creations.clone()), vec![json!({"chatType": "oneOnOne", "members": [member(MORGAN, "owner"), member(ANA, "owner")]})]);
    let events = r.events();
    let row = events.iter().position(|e| matches!(e, model::Event::Chats { rows } if rows.iter().any(|x| x.id == chat))).expect("its row said");
    let created = events.iter().position(|e| matches!(e, model::Event::Created { .. })).unwrap();
    assert!(row < created, "the row, then Created: {events:?}");
    let row = r.row(&chat);
    assert_eq!((row.name.as_str(), row.kind, row.person.as_deref()), ("Ana Exemple", ChatKind::OneOnOne, Some(ANA)));
    assert_eq!(r.record(&chat).chat.members.len(), 2);

    r.create("1-2", vec![ana(), bruno()], Some("Launch"));
    r.turns(Duration::from_secs(4));
    let group = r.made().pop().unwrap().1.unwrap();
    let body = r.fake(|t| t.creations[1].clone());
    assert_eq!((body["chatType"].as_str(), body["topic"].as_str()), (Some("group"), Some("Launch")));
    assert_eq!(r.row(&group).name, "Launch");
    assert_eq!(r.record(&group).chat.members.len(), 3);
}

#[test]
fn a_guest_is_added_as_a_guest() {
    let mut r = run(teams(Vec::new()));
    r.create("1-1", vec![carla()], None);
    r.turns(Duration::from_secs(4));
    assert!(r.made().pop().unwrap().1.is_ok());
    assert_eq!(r.fake(|t| t.creations[0]["members"][1]["roles"].clone()), json!(["guest"]));
}

#[test]
fn a_creation_refused_or_unanswered_is_said_and_not_made_again() {
    let mut r = run(teams(Vec::new()));
    r.fake(|t| t.forbid_create = true);
    r.create("1-1", vec![ana()], None);
    r.turns(Duration::from_secs(3));
    assert_eq!(r.made(), vec![("1-1".to_string(), Err((NotCreated::Refused, Some("HTTP 403 Forbidden".to_string()))))]);
    r.turns(Duration::from_secs(60));
    assert_eq!(r.hits("createChat"), 1, "never made again on its own");

    // The answer lost with the request carried out: said, and never asked again.
    let mut r = run(teams(Vec::new()));
    r.fake(|t| t.cut_after.insert("createChat".into(), 1));
    r.create("1-1", vec![bruno()], None);
    r.turns(Duration::from_secs(3));
    assert_eq!(r.made(), vec![("1-1".to_string(), Err((NotCreated::Offline, None)))]);
    r.turns(Duration::from_secs(60));
    assert_eq!(r.hits("createChat"), 1);
    assert_eq!(r.fake(|t| t.chats.len()), 1, "the fake did make it once");

    // A server's error is Graph's answer: refused, with its code.
    let mut r = run(teams(Vec::new()));
    r.fake(|t| t.fail_next.insert("createChat".into(), 1));
    r.create("1-1", vec![bruno()], None);
    r.turns(Duration::from_secs(60));
    assert!(matches!(r.made().as_slice(), [(_, Err((NotCreated::Refused, Some(code))))] if code.starts_with("HTTP 500")), "{:?}", r.made());
    assert_eq!(r.hits("createChat"), 1);
}

/// H15: what isn't a person is never asked for.
#[test]
fn a_person_who_isnt_one_is_never_asked_for() {
    let mut r = run(teams(Vec::new()));
    let tap = crate::fakes::logs::tap(|l| l.starts_with("teams: ") && l.contains("dropped"));
    let eve = Person { id: "x')/../me".into(), ..person(ANA, "Eve Moss", false) };
    let many: Vec<Person> = (0..21).map(|i| person(&format!("00000000-0000-0000-0000-0000000002{i:02}"), "Someone Else", false)).collect();
    let me = person(MORGAN, "Morgan Lee", false);
    let upper = Person { id: ANA.to_uppercase(), ..ana() };
    r.create("1-1", vec![eve], None);
    r.create("1-2", many, None);
    r.create("1-3", vec![me], None);
    r.create("1-4", vec![ana(), upper], None);
    r.create("1-5", Vec::new(), None);
    r.create("BAD ID", vec![bruno()], None);
    r.turns(Duration::from_secs(5));
    assert_eq!(r.made(), Vec::new());
    assert_eq!(r.hits("createChat"), 0);
    assert!(tap.lines().len() >= 6, "a warning each: {:?}", tap.lines());
}

#[test]
fn nothing_is_created_offline_or_signed_out() {
    let mut r = run(teams(vec![one_to_one(CHAT)]));
    r.w.work_offline(true);
    r.create("1-1", vec![bruno()], None);
    r.directory("ana");
    r.create("1-2", vec![ana()], None);
    assert_eq!(r.made(), vec![("1-1".to_string(), Err((NotCreated::Offline, None))), ("1-2".to_string(), Err((NotCreated::Offline, None)))]);
    assert_eq!(r.peoples(), vec![("ana".to_string(), Vec::new(), true)]);
    r.turns(Duration::from_secs(3));
    assert_eq!((r.hits("createChat"), r.hits("users")), (0, 0));
    // Going offline with a creation waiting says so.
    r.w.work_offline(false);
    r.create("1-3", vec![bruno()], None);
    r.w.work_offline(true);
    assert_eq!(r.made().pop(), Some(("1-3".to_string(), Err((NotCreated::Offline, None)))));
    r.w.work_offline(false);
    r.turns(Duration::from_secs(3));
    assert_eq!(r.hits("createChat"), 0, "given up, not made later");

    r.w.job(Job::SignOut { delete_copy: false });
    r.create("1-4", vec![bruno()], None);
    r.directory("ana");
    assert_eq!(r.made().pop(), Some(("1-4".to_string(), Err((NotCreated::Offline, None)))));
    assert_eq!(r.peoples().pop(), Some(("ana".to_string(), Vec::new(), true)));
    r.turns(Duration::from_secs(3));
    assert_eq!((r.hits("createChat"), r.hits("users")), (0, 0));
}

#[test]
fn one_creation_at_a_time() {
    let mut r = run(teams(Vec::new()));
    r.create("1-1", vec![ana()], None);
    r.create("1-2", vec![bruno()], None);
    assert_eq!(r.made(), vec![("1-2".to_string(), Err((NotCreated::TooMany, None)))]);
    r.turns(Duration::from_secs(4));
    let made = r.made();
    assert_eq!(made.len(), 2, "{made:?}");
    assert!(matches!(&made[1], (local, Ok(_)) if local == "1-1"), "{made:?}");
    assert_eq!(r.hits("createChat"), 1);
}

/// H18: the log says counts and keyed hashes, never a name, an address, what was typed or a topic.
#[test]
fn logs_carry_no_name_address_or_query() {
    let mut t = teams(Vec::new());
    t.directory.push(user("00000000-0000-0000-0000-0000000000b1", "Quokka Zephyr"));
    t.directory.push(user("00000000-0000-0000-0000-0000000000b2", "Wombat Zephyr"));
    let mut r = run(t);
    let tap = crate::fakes::logs::tap(|l| l.starts_with("teams: "));
    r.directory("zephyr");
    r.turns(Duration::from_secs(3));
    let found = r.peoples().pop().unwrap().1;
    assert_eq!(found.len(), 2);
    r.create("1-1", found, Some("Wallaby plans"));
    r.turns(Duration::from_secs(4));
    assert!(r.made().pop().unwrap().1.is_ok());
    r.fake(|t| t.forbid_create = true);
    r.create("1-2", vec![ana()], Some("Bilby"));
    r.turns(Duration::from_secs(3));
    let lines = tap.lines();
    assert!(lines.iter().any(|l| l.contains("a directory search") && l.contains('2')), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("a conversation created")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("HTTP 403")), "{lines:?}");
    let port = r.server.base.rsplit(':').next().unwrap().split('/').next().unwrap().to_string();
    for line in &lines {
        let lower = line.to_lowercase();
        for private in [
            "quokka",
            "wombat",
            "zephyr",
            "wallaby",
            "bilby",
            "ana",
            "exemple",
            "contoso",
            "fabrikam",
            "@thread.v2",
            "unq.gbl",
            MORGAN,
            ANA,
            BRUNO,
            port.as_str(),
        ] {
            assert!(!lower.contains(&private.to_lowercase()), "{private:?} in the log: {line}");
        }
    }
}
