// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: Teams, people: the directory searched (only the last query's answer taken, each
//! person checked again and remembered), a conversation created (one at a time, online only, its
//! first message sent once it exists, a chat already there opened instead), and a call's link built
//! from the open chat's members.

use quatrelle_model::pim::teams::{ChatKind, ChatRow, Member, NotCreated, Op, Person};

use super::*;
use crate::{TEAMS_NEEDS_NETWORK, TEAMS_NO_CALL, TeamsCreating, TeamsPage};

const ME: &str = "00000000-0000-0000-0000-0000000000e1";
const ANA: &str = "00000000-0000-0000-0000-0000000000a1";
const BRUNO: &str = "00000000-0000-0000-0000-0000000000b1";
const CHAT: &str = "19:chat1@thread.v2";

fn person(id: &str, name: &str) -> Person {
    Person {
        id: id.into(),
        name: name.into(),
        address: format!("{}@contoso.example", name.split(' ').next().unwrap_or("x").to_lowercase()),
        guest: false,
    }
}

fn ana() -> Person {
    person(ANA, "Ana Lima")
}

fn bruno() -> Person {
    person(BRUNO, "Bruno Reis")
}

fn session() -> Session {
    let mut s = signed_in(vec![]);
    s.prefs.teams.object_id = ME.into();
    s
}

/// The directory asked for `query` and answering `people`.
fn found(s: &mut Session, query: &str, people: Vec<Person>) {
    s.run("teams.people", &json!({"query": query})).unwrap();
    s.apply(teams_event(teams::Event::People { query: query.into(), people, failed: false }));
    s.take_jobs();
}

fn sends(jobs: &[teams::Job]) -> Vec<&Op> {
    jobs.iter().filter_map(|j| if let teams::Job::Op { op } = j { Some(op) } else { None }).collect()
}

fn one_on_one(person: &str) -> ChatRow {
    ChatRow { id: CHAT.into(), kind: ChatKind::OneOnOne, person: Some(person.into()), members: 2, ..row(1, 10) }
}

/// A path of the test's own, absolute wherever the tests run.
fn tmp(name: &str) -> String {
    std::env::temp_dir().join("q").join(name).to_string_lossy().into_owned()
}

fn local_of(v: &Value) -> String {
    v["creating"].as_str().unwrap_or_default().to_string()
}

#[test]
fn people_found_are_kept_for_the_last_query_only() {
    let mut s = session();
    s.run("teams.people", &json!({"query": "an"})).unwrap();
    s.run("teams.people", &json!({"query": "ana"})).unwrap();
    assert_eq!(teams_jobs(&mut s).len(), 2);
    s.apply(teams_event(teams::Event::People { query: "an".into(), people: vec![bruno()], failed: false }));
    let t = s.teams.people.clone().unwrap();
    assert!(t.waiting && t.people.is_empty() && t.query == "ana", "{t:?}");
    s.apply(teams_event(teams::Event::People { query: "ana".into(), people: vec![ana()], failed: false }));
    let t = s.teams.people.clone().unwrap();
    assert_eq!((t.people, t.waiting, t.failed), (vec![ana()], false, false));
    // The same query asked while it waits is asked once.
    s.run("teams.people", &json!({"query": "bru"})).unwrap();
    s.take_jobs();
    assert_eq!(s.run("teams.people", &json!({"query": " bru "})).unwrap()["asked"], false);
    assert!(teams_jobs(&mut s).is_empty());
    // An empty query forgets the people.
    s.run("teams.people", &json!({"query": ""})).unwrap();
    assert!(s.teams.people.is_none());
    assert_eq!(teams_jobs(&mut s), [teams::Job::Directory { query: String::new() }]);
    // A query past 64 characters is refused.
    assert!(matches!(s.run("teams.people", &json!({"query": "x".repeat(65)})), Err(CmdError::Params(_))));
}

#[test]
fn people_found_are_checked_again() {
    let mut s = session();
    s.run("teams.people", &json!({"query": "e"})).unwrap();
    let mut answer: Vec<Person> = (0..30).map(|i| person(&format!("00000000-0000-0000-0000-{i:012}"), &format!("Person {i}"))).collect();
    answer.insert(3, Person { id: "u-x".into(), ..ana() });
    answer.insert(4, Person { name: "Eve\u{202E}".into(), ..ana() });
    answer.insert(5, person(ME, "Me Myself"));
    s.apply(teams_event(teams::Event::People { query: "e".into(), people: answer, failed: false }));
    let t = s.teams.people.clone().unwrap();
    assert_eq!(t.people.len(), 25);
    assert!(t.people.iter().all(|p| p.id != "u-x" && p.id != ME && !p.name.contains('\u{202E}')), "{:?}", t.people);
    // A failed directory is said.
    s.run("teams.people", &json!({"query": "f"})).unwrap();
    s.apply(teams_event(teams::Event::People { query: "f".into(), people: vec![], failed: true }));
    let t = s.teams.people.clone().unwrap();
    assert!(t.failed && !t.waiting);
}

#[test]
fn only_people_the_directory_said_can_be_added() {
    let mut s = session();
    assert!(matches!(s.run("teams.create", &json!({"people": [ANA], "text": "hello"})), Err(CmdError::Params(_))));
    assert!(teams_jobs(&mut s).is_empty());
    assert!(s.teams.creating.is_none());
}

#[test]
fn a_person_already_chatted_with_opens_that_chat() {
    let mut s = session();
    s.apply(teams_event(teams::Event::Chats { rows: vec![one_on_one(ANA)] }));
    found(&mut s, "ana", vec![ana()]);
    let r = s.run("teams.create", &json!({"people": [ANA], "text": "hello"})).unwrap();
    assert_eq!(r["chat"], CHAT);
    assert!(r["local"].as_str().is_some_and(|l| !l.is_empty()), "{r}");
    assert_eq!(s.teams.open.as_deref(), Some(CHAT));
    let jobs = teams_jobs(&mut s);
    assert!(!jobs.iter().any(|j| matches!(j, teams::Job::Create { .. })), "{jobs:?}");
    assert!(matches!(&sends(&jobs)[..], [Op::Send { chat, .. }] if chat == CHAT), "{jobs:?}");
    assert!(s.teams.creating.is_none());
    // The window closes its pane on it (once).
    assert_eq!(s.take_teams_created().as_deref(), Some(CHAT));
    assert_eq!(s.take_teams_created(), None);
    // A chat left isn't reused: a new one is made.
    let mut left = one_on_one(ANA);
    left.left = true;
    s.apply(teams_event(teams::Event::Chats { rows: vec![left] }));
    let r = s.run("teams.create", &json!({"people": [ANA], "text": "hello"})).unwrap();
    assert!(r["creating"].is_string(), "{r}");
}

#[test]
fn a_new_chat_is_created_then_its_first_message_sent() {
    let mut s = session();
    found(&mut s, "ana", vec![ana()]);
    let r = s.run("teams.create", &json!({"people": [ANA], "text": "hello", "files": [tmp("report.pdf")]})).unwrap();
    let local = local_of(&r);
    let jobs = teams_jobs(&mut s);
    assert_eq!(jobs, [teams::Job::Create { local: local.clone(), people: vec![ana()], topic: None }]);
    assert_eq!(s.teams.creating, Some(TeamsCreating::Waiting { local: local.clone() }));
    assert_eq!(s.take_teams_created(), None);
    s.apply(teams_event(teams::Event::Chats { rows: vec![one_on_one(ANA)] }));
    s.take_jobs();
    s.apply(teams_event(teams::Event::Created { local, chat: CHAT.into() }));
    assert_eq!(s.teams.open.as_deref(), Some(CHAT));
    assert!(s.teams.creating.is_none());
    let jobs = teams_jobs(&mut s);
    let [Op::Send { chat, parts, body, .. }] = &sends(&jobs)[..] else { panic!("{jobs:?}") };
    assert_eq!(chat, CHAT);
    assert_eq!(parts.len(), 1);
    assert!(!body.blocks.is_empty());
    assert_eq!(s.take_teams_created().as_deref(), Some(CHAT));
    // Nothing is sent twice by a second answer.
    s.apply(teams_event(teams::Event::Created { local: "other".into(), chat: CHAT.into() }));
    assert!(teams_jobs(&mut s).iter().all(|j| !matches!(j, teams::Job::Op { .. })));
}

#[test]
fn a_creation_for_a_chat_never_listed_fails() {
    let mut s = session();
    found(&mut s, "ana", vec![ana()]);
    let local = local_of(&s.run("teams.create", &json!({"people": [ANA], "text": "hello"})).unwrap());
    s.take_jobs();
    s.apply(teams_event(teams::Event::Created { local, chat: "19:nowhere@thread.v2".into() }));
    assert_eq!(s.teams.creating, Some(TeamsCreating::Failed { why: NotCreated::Refused, code: None }));
    assert!(sends(&teams_jobs(&mut s)).is_empty());
    assert_eq!(s.take_teams_created(), None);
}

#[test]
fn a_creation_not_made_says_why_and_sends_nothing() {
    let mut s = session();
    found(&mut s, "ana", vec![ana()]);
    let local = local_of(&s.run("teams.create", &json!({"people": [ANA], "text": "hello"})).unwrap());
    s.take_jobs();
    s.apply(teams_event(teams::Event::NotCreated { local, why: NotCreated::Refused, code: Some("HTTP 403 Forbidden".into()) }));
    assert_eq!(s.teams.creating, Some(TeamsCreating::Failed { why: NotCreated::Refused, code: Some("HTTP 403 Forbidden".into()) }));
    assert!(sends(&teams_jobs(&mut s)).is_empty());
    // A new one is taken.
    let r = s.run("teams.create", &json!({"people": [ANA], "text": "hello"})).unwrap();
    assert!(r["creating"].is_string());
    assert!(matches!(s.teams.creating, Some(TeamsCreating::Waiting { .. })));
}

#[test]
fn one_creation_at_a_time() {
    let mut s = session();
    found(&mut s, "ana", vec![ana(), bruno()]);
    s.run("teams.create", &json!({"people": [ANA], "text": "hello"})).unwrap();
    s.take_jobs();
    assert!(matches!(s.run("teams.create", &json!({"people": [BRUNO], "text": "hi"})), Err(CmdError::Disabled(_))));
    assert!(teams_jobs(&mut s).is_empty());
}

#[test]
fn nothing_is_created_while_offline() {
    let mut s = session();
    found(&mut s, "ana", vec![ana()]);
    s.apply(teams_event(teams::Event::Status { status: teams::Status::Offline }));
    assert!(matches!(s.run("teams.create", &json!({"people": [ANA], "text": "hello"})), Err(CmdError::Disabled(m)) if m == TEAMS_NEEDS_NETWORK));
    assert!(teams_jobs(&mut s).is_empty());
    assert!(s.teams.creating.is_none());
}

#[test]
fn a_topic_is_a_groups_only() {
    let topic = |people: &[&str], t: &str| {
        let mut s = session();
        found(&mut s, "a", vec![ana(), bruno()]);
        s.run("teams.create", &json!({"people": people, "text": "hello", "topic": t})).unwrap();
        match &teams_jobs(&mut s)[..] {
            [teams::Job::Create { topic, .. }] => topic.clone(),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(topic(&[ANA], "Launch"), None);
    assert_eq!(topic(&[ANA, BRUNO], "Launch").as_deref(), Some("Launch"));
    assert_eq!(topic(&[ANA, BRUNO], "  A \n B ").as_deref(), Some("A B"));
    assert_eq!(topic(&[ANA, BRUNO], &"x".repeat(200)).map(|t| t.chars().count()), Some(80));
    assert_eq!(topic(&[ANA, BRUNO], "  "), None);
}

#[test]
fn a_creation_needs_a_message_and_people() {
    let mut s = session();
    let many: Vec<Person> = (0..22).map(|i| person(&format!("00000000-0000-0000-0000-{i:012}"), &format!("Person {i}"))).collect();
    let mut all = vec![ana(), bruno(), person(ME, "Me Myself")];
    all.extend(many.clone());
    found(&mut s, "a", all);
    // The user isn't remembered from the directory's own answer.
    let ids: Vec<&str> = many.iter().map(|p| p.id.as_str()).collect();
    let bad = [
        json!({"people": [ANA], "text": ""}),
        json!({"people": [ANA], "text": " ", "files": [], "pictures": []}),
        json!({"people": [], "text": "hi"}),
        json!({"people": &ids[..21], "text": "hi"}),
        json!({"people": [ANA, ANA], "text": "hi"}),
        json!({"people": [ME], "text": "hi"}),
        json!({"text": "hi"}),
    ];
    for b in bad {
        assert!(matches!(s.run("teams.create", &b), Err(CmdError::Params(_))), "{b}");
    }
    assert!(teams_jobs(&mut s).is_empty());
    assert!(s.teams.creating.is_none());
    // Twenty are taken.
    s.run("teams.create", &json!({"people": &ids[..20], "text": "hi"})).unwrap();
}

fn call_session(kind: ChatKind, members: Vec<Member>) -> Session {
    let mut s = session();
    s.apply(teams_event(teams::Event::Chats { rows: vec![ChatRow { kind, ..row(1, 10) }] }));
    open(&mut s, "19:chat1@thread.v2");
    s.apply(teams_event(teams::Event::Members { chat: "19:chat1@thread.v2".into(), members, web_url: None, first: true }));
    s.take_jobs();
    s
}

fn member(id: &str, address: &str) -> Member {
    Member { user_id: id.into(), name: "x".into(), address: address.into(), kind: "aadUserConversationMember".into(), ..Default::default() }
}

#[test]
fn call_builds_teams_link_from_the_members_addresses() {
    let mut s = call_session(ChatKind::OneOnOne, vec![member(ME, "me@contoso.example"), member(ANA, "ana@contoso.example")]);
    let r = s.run("teams.call", &json!({})).unwrap();
    let audio = "https://teams.microsoft.com/l/call/0/0?users=ana%40contoso.example";
    assert_eq!(r["url"], audio);
    assert_eq!(s.take_teams_page(), Some(TeamsPage::Call(audio.into())));
    s.run("teams.call", &json!({"video": true})).unwrap();
    assert_eq!(s.take_teams_page(), Some(TeamsPage::Call(format!("{audio}&withVideo=true"))));
    assert_eq!(s.teams.callable(ME), Some(vec!["ana@contoso.example"]));
    let mut g = call_session(
        ChatKind::Group,
        vec![member(ME, "me@contoso.example"), member(ANA, "ana@contoso.example"), member(BRUNO, "bruno@contoso.example")],
    );
    g.run("teams.call", &json!({})).unwrap();
    assert_eq!(
        g.take_teams_page(),
        Some(TeamsPage::Call("https://teams.microsoft.com/l/call/0/0?users=ana%40contoso.example,bruno%40contoso.example".into()))
    );
}

#[test]
fn no_call_without_an_address_in_a_meeting_chat_or_past_twenty() {
    let no_call = |s: &mut Session| {
        assert!(matches!(s.run("teams.call", &json!({})), Err(CmdError::Disabled(m)) if m == TEAMS_NO_CALL));
        assert_eq!(s.take_teams_page(), None);
        assert_eq!(s.teams.callable(ME), None);
    };
    no_call(&mut call_session(ChatKind::Group, vec![member(ME, "me@contoso.example"), member(ANA, "")]));
    no_call(&mut call_session(ChatKind::Meeting, vec![member(ME, "me@contoso.example"), member(ANA, "ana@contoso.example")]));
    let mut left = call_session(ChatKind::OneOnOne, vec![member(ME, "me@contoso.example"), member(ANA, "ana@contoso.example")]);
    left.apply(teams_event(teams::Event::Chats { rows: vec![ChatRow { kind: ChatKind::OneOnOne, left: true, ..row(1, 10) }] }));
    no_call(&mut left);
    let mut many = vec![member(ME, "me@contoso.example")];
    many.extend((0..21).map(|i| member(&format!("00000000-0000-0000-0000-{i:012}"), &format!("p{i}@contoso.example"))));
    no_call(&mut call_session(ChatKind::Group, many));
    // Alone in it: nobody to call.
    no_call(&mut call_session(ChatKind::Group, vec![member(ME, "me@contoso.example")]));
    // No chat open.
    let mut none = session();
    assert!(matches!(none.run("teams.call", &json!({})), Err(CmdError::Disabled(m)) if m == TEAMS_NO_CALL));
    assert_eq!(none.take_teams_page(), None);
}

#[test]
fn a_member_address_cant_add_someone_to_the_call() {
    for bad in ["ana@contoso.example,eve@evil.example", "ana@contoso.example&withVideo=true", "ana@contoso.example#x"] {
        let mut s = call_session(ChatKind::OneOnOne, vec![member(ME, "me@contoso.example"), member(ANA, bad)]);
        assert!(matches!(s.run("teams.call", &json!({})), Err(CmdError::Disabled(_))), "{bad}");
        assert_eq!(s.take_teams_page(), None, "{bad}");
    }
}

#[test]
fn signing_out_forgets_people_and_a_creation() {
    let mut s = session();
    found(&mut s, "ana", vec![ana()]);
    s.run("teams.create", &json!({"people": [ANA], "text": "hello"})).unwrap();
    assert!(s.teams.people.is_some() && s.teams.creating.is_some());
    s.run("teams.signOut", &json!({})).unwrap();
    assert!(s.teams.people.is_none() && s.teams.creating.is_none());
    // The people remembered are gone too: the directory must say them again.
    s.prefs.teams.on = true;
    s.prefs.teams.signed_in = true;
    assert!(matches!(s.run("teams.create", &json!({"people": [ANA], "text": "hello"})), Err(CmdError::Params(_))));
}

/// The outbox full: a message to a listed chat is refused.
fn outbox_full(s: &mut Session) {
    s.teams.unsent = u32::try_from(quatrelle_model::pim::teams::OUTBOX_MAX).unwrap_or(u32::MAX);
}

#[test]
fn a_refused_first_message_in_an_existing_chat_changes_nothing() {
    let mut s = session();
    s.apply(teams_event(teams::Event::Chats { rows: vec![one_on_one(ANA), row(2, 5)] }));
    open(&mut s, "19:chat2@thread.v2");
    found(&mut s, "ana", vec![ana()]);
    outbox_full(&mut s);
    assert!(matches!(s.run("teams.create", &json!({"people": [ANA], "text": "hello"})), Err(CmdError::Disabled(_))));
    assert_eq!(s.teams.open.as_deref(), Some("19:chat2@thread.v2"));
    assert_eq!(s.take_teams_created(), None);
    assert!(teams_jobs(&mut s).is_empty());
    // Too long a body: the same.
    s.teams.unsent = 0;
    assert!(s.run("teams.create", &json!({"people": [ANA], "text": "x".repeat(200_000)})).is_err());
    assert_eq!(s.teams.open.as_deref(), Some("19:chat2@thread.v2"));
    assert_eq!(s.take_teams_created(), None);
}

#[test]
fn a_refused_first_message_in_a_created_chat_keeps_the_pane() {
    let mut s = session();
    found(&mut s, "ana", vec![ana()]);
    let local = local_of(&s.run("teams.create", &json!({"people": [ANA], "text": "hello"})).unwrap());
    s.apply(teams_event(teams::Event::Chats { rows: vec![one_on_one(ANA)] }));
    s.take_jobs();
    outbox_full(&mut s);
    s.apply(teams_event(teams::Event::Created { local, chat: CHAT.into() }));
    assert_eq!(s.teams.open, None, "no chat switched");
    assert_eq!(s.take_teams_created(), None, "the pane stays");
    assert!(matches!(s.teams.creating, Some(TeamsCreating::Failed { .. })), "{:?}", s.teams.creating);
    assert!(sends(&teams_jobs(&mut s)).is_empty());
    // Sending again is taken: the chat is there now, and is opened.
    s.teams.unsent = 0;
    let r = s.run("teams.create", &json!({"people": [ANA], "text": "hello"})).unwrap();
    assert_eq!(r["chat"], CHAT);
    assert_eq!(s.take_teams_created().as_deref(), Some(CHAT));
}
