// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: Teams, as the engine keeps it: the rows, the open chat's window, the module's
//! fallback to Mail, and the `teams.*` commands.

use quatrelle_model::pim::teams::{self, ChatKind, ChatMessage, ChatRow, MAX_CHATS, Picture, Presence, Status, WINDOW_MAX};

use super::*;
use crate::Module;

const CLIENT: &str = "00000000-0000-0000-0000-00000000c11e";
const TENANT: &str = "contoso.example";

fn row(i: usize, at: i64) -> ChatRow {
    ChatRow {
        id: format!("19:chat{i}@thread.v2"),
        name: format!("Chat {i}"),
        kind: ChatKind::Group,
        last_line: String::new(),
        last_external: false,
        at: Some(at),
        unread: 0,
        hidden: false,
        left: false,
        unreadable: false,
        external: false,
        members: 3,
        person: None,
    }
}

fn msg(chat: &str, i: i64) -> ChatMessage {
    ChatMessage { id: format!("m{i}"), chat: chat.into(), created: 1_000_000 + i, modified: 1_000_000 + i, ..Default::default() }
}

fn teams_event(event: teams::Event) -> Event {
    Event::Teams { event }
}

fn teams_jobs(s: &mut Session) -> Vec<teams::Job> {
    s.take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            Job::Teams { job } => Some(job),
            _ => None,
        })
        .collect()
}

/// A session with Teams on and signed in, holding `rows`.
fn signed_in(rows: Vec<ChatRow>) -> Session {
    let mut s = Session::blank();
    s.prefs.teams.on = true;
    s.prefs.teams.signed_in = true;
    s.prefs.teams.client_id = CLIENT.into();
    s.prefs.teams.tenant = TENANT.into();
    for part in rows.chunks(teams::EVENT_ITEMS) {
        s.apply(teams_event(teams::Event::Chats { rows: part.to_vec() }));
    }
    s.take_jobs();
    s
}

fn open(s: &mut Session, chat: &str) {
    s.run("teams.open", &json!({"chat": chat})).unwrap();
}

fn messages(s: &mut Session, chat: &str, range: std::ops::Range<i64>) {
    let all: Vec<ChatMessage> = range.map(|i| msg(chat, i)).collect();
    for part in all.chunks(teams::EVENT_ITEMS) {
        s.apply(teams_event(teams::Event::Messages { chat: chat.into(), messages: part.to_vec(), fresh: false, older: true }));
    }
}

fn window_ids(s: &Session) -> Vec<i64> {
    s.teams.window.iter().map(|m| m.created - 1_000_000).collect()
}

#[test]
fn conversations_falls_back_to_mail_when_teams_is_off() {
    // The module remembered as `conversations`, with Teams off: Quatrelle opens on Mail.
    assert_eq!(serde_json::from_value::<Module>(json!("conversations")).unwrap(), Module::Conversations);
    let mut s = Session::blank();
    s.prefs = serde_json::from_value(json!({"teams": {"on": false, "signed_in": true, "client_id": CLIENT, "tenant": TENANT}})).unwrap();
    s.module = Module::Conversations;
    s.teams_start();
    assert_eq!(s.module, Module::Mail);
    assert!(teams_jobs(&mut s).is_empty(), "nothing of Teams runs while it's off");
    // On but nobody signed in: the same.
    s.prefs.teams.on = true;
    s.prefs.teams.signed_in = false;
    s.module = Module::Conversations;
    s.teams_start();
    assert_eq!(s.module, Module::Mail);
    assert!(teams_jobs(&mut s).is_empty());
    // On and signed in: started, the module kept.
    s.prefs.teams.signed_in = true;
    s.module = Module::Conversations;
    s.teams_start();
    assert_eq!(s.module, Module::Conversations);
    assert!(matches!(&teams_jobs(&mut s)[..], [teams::Job::Start { settings }] if settings.on && settings.signed_in));
    // Switched off while in it: back to Mail, and the worker told to stop.
    s.run("teams.settings", &json!({"on": false})).unwrap();
    assert_eq!(s.module, Module::Mail);
    assert_eq!(teams_jobs(&mut s), [teams::Job::Stop]);
    assert!(!s.prefs.teams.on);
    // Off, the module can't be chosen.
    assert!(matches!(s.run("view.module", &json!({"name": "conversations"})), Err(CmdError::Disabled(_))));
    assert_eq!(s.module, Module::Mail);
}

#[test]
fn the_badge_counts_unread_of_listed_chats_only() {
    let mut s = signed_in(vec![]);
    assert_eq!(s.teams.unread_badge(), None);
    let mut a = row(1, 30);
    a.unread = 3;
    let mut hidden = row(2, 20);
    hidden.unread = 40;
    hidden.hidden = true;
    let mut b = row(3, 10);
    b.unread = 2;
    s.apply(teams_event(teams::Event::Chats { rows: vec![a.clone(), hidden, b] }));
    assert_eq!(s.teams.unread_badge().as_deref(), Some("5"), "hidden chats not counted");
    a.unread = 148;
    s.apply(teams_event(teams::Event::Chats { rows: vec![a] }));
    assert_eq!(s.teams.unread_badge().as_deref(), Some("99+"));
}

#[test]
fn a_rows_event_of_50_changes_50_rows_and_nothing_else() {
    let rows: Vec<ChatRow> = (0..20_000).map(|i| row(i, 1_000_000 - i as i64)).collect();
    let mut s = signed_in(rows.clone());
    assert_eq!(s.teams.rows, rows, "newest activity first, as sent");
    // Fifty rows whose unread count and last line change, at the same time: in place.
    let changed: Vec<ChatRow> = (0..50)
        .map(|k| {
            let mut r = row(k * 400, 1_000_000 - (k * 400) as i64);
            r.unread = 1;
            r.last_line = "Paul: it's fixed".into();
            r
        })
        .collect();
    let before = crate::session::pim::teams_row_scans();
    s.apply(teams_event(teams::Event::Chats { rows: changed.clone() }));
    assert_eq!(crate::session::pim::teams_row_scans(), before, "found by the index, never by going through the rows");
    let differ: Vec<usize> = s.teams.rows.iter().zip(&rows).enumerate().filter(|(_, (a, b))| a != b).map(|(i, _)| i).collect();
    assert_eq!(differ, (0..50).map(|k| k * 400).collect::<Vec<_>>());
    assert_eq!(s.teams.rows.get(400), changed.get(1));
    // Fifty rows with new activity: they go to the top, in one pass, the rest in order.
    let moved: Vec<ChatRow> = (0..50).map(|k| row(19_999 - k, 2_000_000 + k as i64)).collect();
    s.apply(teams_event(teams::Event::Chats { rows: moved }));
    assert_eq!(crate::session::pim::teams_row_scans(), before + 1, "one pass for the event, not one per row");
    assert_eq!(s.teams.rows.len(), 20_000);
    assert_eq!(s.teams.rows.first().map(|r| r.id.as_str()), Some("19:chat19950@thread.v2"));
    assert!(s.teams.rows.windows(2).all(|w| w[0].at >= w[1].at));
    // A chat past MAX_CHATS, older than every row: not kept.
    assert_eq!(MAX_CHATS, 20_000);
    s.apply(teams_event(teams::Event::Chats { rows: vec![row(30_000, 1)] }));
    assert_eq!(s.teams.rows.len(), MAX_CHATS);
    assert!(s.run("teams.open", &json!({"chat": "19:chat30000@thread.v2"})).is_err());
}

#[test]
fn the_window_keeps_at_most_500_and_takes_only_the_open_chat() {
    let mut s = signed_in(vec![row(1, 10), row(2, 5)]);
    let chat = "19:chat1@thread.v2";
    open(&mut s, chat);
    assert_eq!(teams_jobs(&mut s), [teams::Job::Open { chat: Some(chat.into()) }]);
    // The history read while the chat is open, newest pages first: the newest 500 kept.
    for page in (0..6).rev() {
        messages(&mut s, chat, page * 100..(page + 1) * 100);
    }
    assert_eq!(s.teams.window.len(), WINDOW_MAX);
    assert_eq!(window_ids(&s), (100..600).collect::<Vec<_>>(), "in creation order, the newest kept");
    // Another chat's messages change nothing.
    let before = s.teams.window.clone();
    messages(&mut s, "19:chat2@thread.v2", 0..50);
    assert_eq!(s.teams.window, before);
    // An edit replaces its message in place.
    let mut edited = msg(chat, 300);
    edited.edited = Some(2_000_000);
    s.apply(teams_event(teams::Event::Messages { chat: chat.into(), messages: vec![edited.clone()], fresh: true, older: false }));
    assert_eq!(s.teams.window.len(), WINDOW_MAX);
    assert_eq!(s.teams.window.get(200), Some(&edited));
    // Opening another chat empties the window.
    open(&mut s, "19:chat2@thread.v2");
    assert!(s.teams.window.is_empty());
}

#[test]
fn scrolling_back_drops_the_newest_end_and_asks_each_page_once() {
    let mut s = signed_in(vec![row(1, 10)]);
    let chat = "19:chat1@thread.v2";
    open(&mut s, chat);
    messages(&mut s, chat, 1000..1200);
    teams_jobs(&mut s);
    s.run("teams.older", &json!({"chat": chat})).unwrap();
    assert_eq!(teams_jobs(&mut s), [teams::Job::Page { chat: chat.into(), before: 1_001_000 }]);
    s.run("teams.older", &json!({"chat": chat})).unwrap();
    assert!(teams_jobs(&mut s).is_empty(), "the same page isn't asked twice");
    // Pages back: past 500, the newest end goes (the user reads the oldest).
    for page in 0..4 {
        messages(&mut s, chat, 900 - page * 100..1000 - page * 100);
        s.run("teams.older", &json!({"chat": chat})).unwrap();
    }
    assert_eq!(s.teams.window.len(), WINDOW_MAX);
    assert_eq!(window_ids(&s), (600..1100).collect::<Vec<_>>());
    assert!(s.teams.newer_left_out);
    // A new message isn't put after a gap: reopening the chat shows it.
    s.apply(teams_event(teams::Event::Messages { chat: chat.into(), messages: vec![msg(chat, 5000)], fresh: true, older: false }));
    assert_eq!(window_ids(&s).last(), Some(&1099));
    assert!(s.run("teams.older", &json!({"chat": "19:other@thread.v2"})).is_err(), "only the open chat");
}

#[test]
fn settings_refuse_ids_that_cant_be_ids() {
    let mut s = Session::blank();
    let before = s.prefs.teams.clone();
    assert!(matches!(s.run("teams.settings", &json!({"clientId": "not a guid", "on": true})), Err(CmdError::Params(_))));
    assert!(matches!(s.run("teams.settings", &json!({"tenant": "-contoso", "on": true})), Err(CmdError::Params(_))));
    assert_eq!(s.prefs.teams, before, "nothing changed");
    assert!(teams_jobs(&mut s).is_empty());
    s.run("teams.settings", &json!({"clientId": CLIENT, "tenant": TENANT, "notify": false, "notifyOnlyDirect": true})).unwrap();
    assert_eq!((s.prefs.teams.client_id.as_str(), s.prefs.teams.tenant.as_str()), (CLIENT, TENANT));
    assert!(!s.prefs.teams.notify && s.prefs.teams.notify_only_direct);
    s.run("teams.settings", &json!({"clientId": "", "tenant": " "})).unwrap();
    assert!(s.prefs.teams.client_id.is_empty() && s.prefs.teams.tenant.is_empty(), "empty is allowed");
    // Switched on, nobody signed in: nothing started yet.
    s.run("teams.settings", &json!({"on": true})).unwrap();
    assert!(s.prefs.teams.on);
    assert!(teams_jobs(&mut s).is_empty());
}

#[test]
fn sign_in_needs_both_ids() {
    let mut s = Session::blank();
    assert!(matches!(s.run("teams.signIn", &json!({})), Err(CmdError::Disabled(_))), "Teams off");
    s.run("teams.settings", &json!({"on": true, "clientId": CLIENT, "tenant": ""})).unwrap();
    assert!(matches!(s.run("teams.signIn", &json!({})), Err(CmdError::Disabled(_))), "no tenant");
    assert!(teams_jobs(&mut s).is_empty());
    s.run("teams.settings", &json!({"tenant": TENANT})).unwrap();
    s.run("teams.signIn", &json!({})).unwrap();
    assert_eq!(teams_jobs(&mut s), [teams::Job::SignIn { client_id: CLIENT.into(), tenant: TENANT.into() }]);
    assert_eq!(s.teams.status, Status::SigningIn);
    assert!(s.run("teams.signIn", &json!({})).is_err(), "already signing in");
    assert!(teams_jobs(&mut s).is_empty());
    // Who signed in goes into the settings.
    s.apply(teams_event(teams::Event::Account {
        name: "Ana\u{202E} Lima".into(),
        address: "ana@contoso.example".into(),
        object_id: "u-1".into(),
        tenant_id: "t-1".into(),
        drive_host: Some("contoso-my.sharepoint.com".into()),
    }));
    assert!(s.prefs.teams.signed_in);
    assert_eq!((s.prefs.teams.name.as_str(), s.prefs.teams.object_id.as_str()), ("Ana Lima", "u-1"));
    assert_eq!(s.teams.drive_host.as_deref(), Some("contoso-my.sharepoint.com"));
}

#[test]
fn sign_out_asks_the_worker_to_delete_the_copy_when_told() {
    let mut s = signed_in(vec![row(1, 10)]);
    s.prefs.teams.name = "Ana".into();
    s.run("teams.signOut", &json!({"deleteCopy": true})).unwrap();
    assert_eq!(teams_jobs(&mut s), [teams::Job::SignOut { delete_copy: true }]);
    assert!(!s.prefs.teams.signed_in && s.prefs.teams.name.is_empty());
    assert!(s.teams.rows.is_empty(), "nothing of the copy kept in memory");
    assert!(s.run("teams.signOut", &json!({})).is_err(), "nobody signed in");
    let mut s = signed_in(vec![row(1, 10)]);
    s.run("teams.signOut", &json!({})).unwrap();
    assert_eq!(teams_jobs(&mut s), [teams::Job::SignOut { delete_copy: false }], "the copy kept unless told");
    // Rows still on their way from before the sign-out aren't taken.
    s.apply(teams_event(teams::Event::Chats { rows: vec![row(2, 10)] }));
    assert!(s.teams.rows.is_empty());
}

#[test]
fn signed_out_the_module_falls_back_to_mail() {
    let mut s = signed_in(vec![row(1, 10)]);
    s.run("view.module", &json!({"name": "conversations"})).unwrap();
    s.run("teams.signOut", &json!({})).unwrap();
    assert_eq!(s.module, Module::Mail, "signed out by the user");
    let mut s = signed_in(vec![row(1, 10)]);
    s.run("view.module", &json!({"name": "conversations"})).unwrap();
    s.apply(teams_event(teams::Event::Status { status: Status::SignedOut }));
    assert_eq!(s.module, Module::Mail, "signed out, as the worker says");
}

#[test]
fn a_picture_dropped_unseen_can_be_asked_again() {
    let mut s = signed_in(vec![]);
    for i in 0..70 {
        let user = format!("u-{i}");
        s.run("teams.picture", &json!({"kind": "photo", "user": user})).unwrap();
        s.apply(teams_event(teams::Event::Picture { picture: Picture::Photo { user }, bytes: vec![1] }));
    }
    assert_eq!(s.teams.pictures.len(), 64);
    teams_jobs(&mut s);
    s.run("teams.picture", &json!({"kind": "photo", "user": "u-0"})).unwrap();
    assert_eq!(teams_jobs(&mut s), [teams::Job::Picture { picture: Picture::Photo { user: "u-0".into() } }], "dropped before the window took it");
    s.run("teams.picture", &json!({"kind": "photo", "user": "u-69"})).unwrap();
    assert!(teams_jobs(&mut s).is_empty(), "still waiting for the window");
}

#[test]
fn erased_teams_ids_stay_erased() {
    let mut p = crate::Prefs::default();
    assert!(serde_json::to_value(&p).unwrap().get("teams").is_some_and(|t| t.is_object()), "saved even as it was at first");
    p.teams.client_id.clear();
    p.teams.tenant.clear();
    let back: crate::Prefs = serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
    assert!(back.teams.client_id.is_empty() && back.teams.tenant.is_empty());
    assert_eq!(back.teams, p.teams);
}

#[test]
fn search_hits_without_usable_ids_are_left_out() {
    let mut s = signed_in(vec![row(1, 10)]);
    s.run("teams.search", &json!({"query": "plan"})).unwrap();
    let long = "x".repeat(teams::MAX_ID + 1);
    let hits = vec![
        ("19:chat1@thread.v2".to_string(), "m1".to_string()),
        ("19:chat1@thread.v2".to_string(), String::new()),
        (String::new(), "m2".to_string()),
        (long.clone(), "m3".to_string()),
        ("19:chat1@thread.v2".to_string(), long),
    ];
    s.apply(teams_event(teams::Event::Found { query: "plan".into(), hits, indexing: false, partial: false }));
    let f = s.teams.found.as_ref().unwrap();
    assert_eq!(
        f.hits,
        [("19:chat1@thread.v2".to_string(), "m1".to_string()), ("19:chat1@thread.v2".to_string(), String::new())],
        "a chat's name is a hit with no message"
    );
}

#[test]
fn teams_open_refuses_an_unknown_chat() {
    let mut s = signed_in(vec![row(1, 10)]);
    assert!(matches!(s.run("teams.open", &json!({"chat": "19:nope@thread.v2"})), Err(CmdError::Params(_))));
    assert!(matches!(s.run("teams.open", &json!({"chat": 5})), Err(CmdError::Params(_))));
    assert!(teams_jobs(&mut s).is_empty());
    assert_eq!(s.teams.open, None);
    open(&mut s, "19:chat1@thread.v2");
    s.run("teams.open", &json!({"chat": null})).unwrap();
    assert_eq!(teams_jobs(&mut s), [teams::Job::Open { chat: Some("19:chat1@thread.v2".into()) }, teams::Job::Open { chat: None }]);
    assert_eq!(s.teams.open, None);
}

#[test]
fn no_keychain_is_said_once_and_kept_until_signed_out() {
    let mut s = signed_in(vec![]);
    s.apply(teams_event(teams::Event::Status { status: Status::NoKeychain }));
    s.apply(teams_event(teams::Event::Status { status: Status::Copying { done: 1, of: 9 } }));
    assert!(s.teams.not_remembered);
    assert_eq!(s.teams.status, Status::Copying { done: 1, of: 9 });
    s.apply(teams_event(teams::Event::Status { status: Status::UpToDate }));
    assert!(s.teams.not_remembered);
    s.apply(teams_event(teams::Event::Status { status: Status::SignedOut }));
    assert!(!s.teams.not_remembered);
}

#[test]
fn sign_in_again_keeps_the_copy_readable() {
    let mut s = signed_in(vec![row(1, 10)]);
    open(&mut s, "19:chat1@thread.v2");
    messages(&mut s, "19:chat1@thread.v2", 0..3);
    s.apply(teams_event(teams::Event::Status { status: Status::SignInAgain }));
    assert_eq!(s.teams.rows.len(), 1);
    assert_eq!(s.teams.window.len(), 3);
    assert!(s.prefs.teams.signed_in, "the copy opens again at the next start");
    s.run("teams.search", &json!({"query": "fixed"})).unwrap();
}

#[test]
fn a_left_chat_stays_listed_read_only() {
    let mut s = signed_in(vec![row(1, 10), row(2, 5)]);
    open(&mut s, "19:chat1@thread.v2");
    s.apply(teams_event(teams::Event::Removed { chat: "19:chat1@thread.v2".into() }));
    let mut left = row(1, 10);
    left.left = true;
    s.apply(teams_event(teams::Event::Chats { rows: vec![left.clone()] }));
    assert_eq!(s.teams.rows.first(), Some(&left));
    assert_eq!(s.teams.rows.len(), 2);
    assert_eq!(s.teams.open.as_deref(), Some("19:chat1@thread.v2"), "still open, to read");
    s.apply(teams_event(teams::Event::Removed { chat: "19:chat2@thread.v2".into() }));
    assert!(s.teams.rows.get(1).is_some_and(|r| r.left));
}

#[test]
fn pictures_are_asked_once_and_outside_ones_only_over_https() {
    let mut s = signed_in(vec![row(1, 10)]);
    s.run("teams.picture", &json!({"kind": "photo", "user": "u-2"})).unwrap();
    s.run("teams.picture", &json!({"kind": "photo", "user": "u-2"})).unwrap();
    assert_eq!(
        teams_jobs(&mut s),
        [teams::Job::Picture { picture: Picture::Photo { user: "u-2".into() } }],
        "a missing photo isn't asked every frame"
    );
    let hosted = json!({"kind": "hosted", "chat": "19:chat1@thread.v2", "message": "m1", "hosted": "h1"});
    s.run("teams.picture", &hosted).unwrap();
    s.run("teams.picture", &hosted).unwrap();
    assert_eq!(teams_jobs(&mut s).len(), 1);
    for bad in ["http://pictures.example/a.png", "file:///etc/passwd", "https://", "https://a.example/\u{0}x", "javascript:alert(1)"] {
        assert!(matches!(s.run("teams.picture", &json!({"kind": "outside", "url": bad})), Err(CmdError::Params(_))), "{bad}");
    }
    assert!(s.run("teams.picture", &json!({"kind": "photo", "user": ""})).is_err());
    assert!(s.run("teams.picture", &json!({"kind": "photo", "user": "x".repeat(513)})).is_err());
    assert!(teams_jobs(&mut s).is_empty());
    // An outside picture is the user's click: asked each time.
    let url = "https://pictures.example/a.png";
    s.run("teams.picture", &json!({"kind": "outside", "url": url})).unwrap();
    s.run("teams.picture", &json!({"kind": "outside", "url": url})).unwrap();
    assert_eq!(teams_jobs(&mut s).len(), 2);
    // Answers wait for the UI, at most 64; one never asked for isn't taken.
    s.apply(teams_event(teams::Event::Picture { picture: Picture::Photo { user: "u-9".into() }, bytes: vec![1] }));
    assert!(s.teams.pictures.is_empty());
    for i in 0..70u8 {
        s.apply(teams_event(teams::Event::Picture { picture: Picture::Outside { url: url.into() }, bytes: vec![i] }));
        s.run("teams.picture", &json!({"kind": "outside", "url": url})).unwrap();
    }
    assert_eq!(s.teams.pictures.len(), 64);
    assert_eq!(s.teams.pictures.front().map(|p| p.1.clone()), Some(vec![6]), "the oldest dropped");
}

#[test]
fn the_visible_set_is_sent_only_when_it_changes() {
    let mut s = signed_in(vec![row(1, 10)]);
    s.run("teams.visible", &json!({"users": ["u-1", "u-2"]})).unwrap();
    s.run("teams.visible", &json!({"users": ["u-2", "u-1", "u-2"]})).unwrap();
    assert_eq!(teams_jobs(&mut s), [teams::Job::Visible { users: vec!["u-1".into(), "u-2".into()] }]);
    let many: Vec<String> = (0..651).map(|i| format!("u-{i}")).collect();
    assert!(matches!(s.run("teams.visible", &json!({"users": many})), Err(CmdError::Params(_))));
    assert!(s.run("teams.visible", &json!({"users": [5]})).is_err());
    assert!(teams_jobs(&mut s).is_empty());
    s.apply(teams_event(teams::Event::Presence { presences: vec![("u-1".into(), Presence::Busy)] }));
    assert_eq!(s.teams.presence.get("u-1"), Some(&Presence::Busy));
}

#[test]
fn search_takes_only_the_answer_to_the_last_query() {
    let mut s = signed_in(vec![row(1, 10)]);
    assert!(matches!(s.run("teams.search", &json!({"query": "x".repeat(201)})), Err(CmdError::Params(_))));
    s.run("teams.search", &json!({"query": "fix"})).unwrap();
    s.run("teams.search", &json!({"query": "fixed"})).unwrap();
    assert_eq!(teams_jobs(&mut s), [teams::Job::Search { query: "fix".into() }, teams::Job::Search { query: "fixed".into() }]);
    let found =
        |q: &str| teams::Event::Found { query: q.into(), hits: vec![("19:chat1@thread.v2".into(), "m1".into())], indexing: true, partial: false };
    s.apply(teams_event(found("fix")));
    assert!(s.teams.found.is_none(), "an answer to an earlier query");
    s.apply(teams_event(found("fixed")));
    let f = s.teams.found.as_ref().unwrap();
    assert_eq!((f.query.as_str(), f.hits.len(), f.indexing, f.partial), ("fixed", 1, true, false));
    s.run("teams.search", &json!({"query": " "})).unwrap();
    assert!(s.teams.found.is_none() && teams_jobs(&mut s).is_empty(), "an empty box: the rows again");
}

#[test]
fn next_and_previous_go_through_the_listed_rows() {
    let mut hidden = row(2, 20);
    hidden.hidden = true;
    let mut s = signed_in(vec![row(1, 30), hidden, row(3, 10)]);
    assert!(s.run("teams.next", &json!({})).is_err(), "only in the module");
    s.run("view.module", &json!({"name": "conversations"})).unwrap();
    s.run("teams.next", &json!({})).unwrap();
    assert_eq!(s.teams.open.as_deref(), Some("19:chat1@thread.v2"));
    s.run("teams.next", &json!({})).unwrap();
    assert_eq!(s.teams.open.as_deref(), Some("19:chat3@thread.v2"), "the hidden one passed over");
    s.run("teams.next", &json!({})).unwrap();
    assert_eq!(s.teams.open.as_deref(), Some("19:chat3@thread.v2"), "the last stays");
    s.run("teams.previous", &json!({})).unwrap();
    assert_eq!(s.teams.open.as_deref(), Some("19:chat1@thread.v2"));
}

#[test]
fn a_copy_set_aside_can_be_deleted() {
    let mut s = signed_in(vec![]);
    assert!(s.run("teams.deleteSetAside", &json!({})).is_err(), "none set aside");
    let path = "/data/Teams-0123abcd.quatrelle.old";
    s.apply(teams_event(teams::Event::SetAside { path: path.into() }));
    assert_eq!(s.teams.set_aside.as_deref(), Some(path));
    s.run("teams.deleteSetAside", &json!({})).unwrap();
    assert_eq!(teams_jobs(&mut s), [teams::Job::DeleteSetAside { path: path.into() }]);
    assert_eq!(s.teams.set_aside, None);
    // Teams switched off: the copy set aside is still offered for deletion.
    s.apply(teams_event(teams::Event::SetAside { path: path.into() }));
    s.run("teams.settings", &json!({"on": false})).unwrap();
    assert_eq!(s.teams.set_aside.as_deref(), Some(path));
    s.run("teams.deleteSetAside", &json!({})).unwrap();
    assert!(teams_jobs(&mut s).contains(&teams::Job::DeleteSetAside { path: path.into() }));
}

#[test]
fn teams_events_are_ignored_while_it_is_off() {
    let mut s = Session::blank();
    s.apply(teams_event(teams::Event::Chats { rows: vec![row(1, 10)] }));
    s.apply(teams_event(teams::Event::Status { status: Status::UpToDate }));
    assert!(s.teams.rows.is_empty());
    assert_eq!(s.teams.status, Status::SignedOut);
}

#[test]
fn teams_commands_survive_junk_params_while_on() {
    let mut s = signed_in((0..5).map(|i| row(i, 100 - i as i64)).collect());
    s.module = Module::Conversations;
    let ids: Vec<&str> = s.registry().all().iter().map(|c| c.id).filter(|id| id.starts_with("teams.")).collect();
    assert_eq!(ids.len(), 11, "{ids:?}");
    let junk = [
        json!(null),
        json!({}),
        json!({"chat": [], "users": "u", "kind": 5, "query": {"a": 1}}),
        json!({"chat": "19:chat1@thread.v2", "kind": "hosted", "message": "", "hosted": "x".repeat(10_000)}),
        json!({"users": [null, 1, ""], "kind": "outside", "url": "https://\u{202E}/x\\y"}),
        json!({"query": "\u{0}".repeat(300), "deleteCopy": "yes", "on": 1, "clientId": 7, "tenant": []}),
        json!({"kind": "photo", "user": "\u{FFFF}", "chat": null}),
    ];
    for _ in 0..2 {
        for id in &ids {
            if *id == "teams.signOut" || *id == "teams.settings" {
                continue;
            }
            for p in &junk {
                let _ = s.run(id, p);
            }
        }
        open(&mut s, "19:chat1@thread.v2");
        messages(&mut s, "19:chat1@thread.v2", 0..3);
    }
    for p in &junk {
        let _ = s.run("teams.settings", p);
        let _ = s.run("teams.signOut", p);
    }
    assert!(s.prefs.teams.on, "no junk switches Teams off");
}
