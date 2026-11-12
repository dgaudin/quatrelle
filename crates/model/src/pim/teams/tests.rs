// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
use super::*;
use crate::pim::teams;
use crate::{Event, Job};

fn member(id: &str, name: &str) -> Member {
    Member {
        user_id: id.into(),
        name: name.into(),
        address: String::new(),
        kind: "aadUserConversationMember".into(),
        tenant_id: Some("t-1".into()),
        roles: vec!["owner".into()],
    }
}

fn chat(kind: ChatKind, topic: Option<&str>, members: Vec<Member>) -> Chat {
    Chat {
        id: "19:a@thread.v2".into(),
        kind,
        topic: topic.map(Into::into),
        members,
        last_read: None,
        hidden: false,
        left: false,
        unreadable: None,
        preview: None,
        last_activity: None,
        updated: None,
        web_url: None,
        tenant_id: None,
    }
}

#[test]
fn internal_only_when_proven_so() {
    let m = |kind: &str, tenant: Option<&str>, roles: &[&str]| Member {
        user_id: "u1".into(),
        name: "Ana".into(),
        address: String::new(),
        kind: kind.into(),
        tenant_id: tenant.map(Into::into),
        roles: roles.iter().map(|r| r.to_string()).collect(),
    };
    assert!(is_internal(&m("aadUserConversationMember", Some("T-1"), &["owner"]), "t-1"));
    assert!(!is_internal(&m("aadUserConversationMember", Some("T-2"), &["owner"]), "t-1"));
    assert!(!is_internal(&m("aadUserConversationMember", None, &[]), "t-1"));
    assert!(!is_internal(&m("aadUserConversationMember", Some("t-1"), &["guest"]), "t-1"));
    assert!(!is_internal(&m("anonymousGuestConversationMember", Some("t-1"), &[]), "t-1"));
    assert!(!is_internal(&m("microsoftAccountUserConversationMember", Some("t-1"), &[]), "t-1"));
    assert!(!is_internal(&m("aadUserConversationMember", Some(""), &[]), ""), "no tenant known proves nothing");
}

#[test]
fn a_stranger_not_in_the_members_is_external_unless_an_aad_user_of_the_tenant() {
    let u = |t: Option<&str>, kind: Option<&str>| Sender::User {
        id: "x".into(),
        name: "X".into(),
        identity_type: kind.map(Into::into),
        tenant_id: t.map(Into::into),
    };
    assert!(sender_internal(&u(Some("t-1"), Some("aadUser")), &[], "t-1"));
    assert!(!sender_internal(&u(Some("t-1"), Some("anonymousGuest")), &[], "t-1"));
    assert!(!sender_internal(&u(None, Some("aadUser")), &[], "t-1"));
    assert!(!sender_internal(&Sender::App { id: "a".into(), name: "Bot".into() }, &[], "t-1"));
    assert!(!sender_internal(&Sender::Nobody, &[], "t-1"));
}

#[test]
fn a_member_is_judged_by_its_membership_not_by_what_the_message_says() {
    let mut guest = member("x", "X");
    guest.roles = vec!["guest".into()];
    let says_internal = Sender::User { id: "x".into(), name: "X".into(), identity_type: Some("aadUser".into()), tenant_id: Some("t-1".into()) };
    assert!(!sender_internal(&says_internal, &[guest], "t-1"));
    let says_nothing = Sender::User { id: "x".into(), name: "X".into(), identity_type: None, tenant_id: None };
    assert!(sender_internal(&says_nothing, &[member("x", "X")], "t-1"));
}

#[test]
fn names_are_one_line_without_bidi_cut_at_80() {
    assert_eq!(one_line("Ana\u{202E}lim\nRossi\t "), "Analim Rossi");
    assert_eq!(one_line(&"é".repeat(100)).chars().count(), 80);
    assert!(one_line(&"é".repeat(100)).ends_with('…'));
    assert_eq!(one_line(&"é".repeat(80)), "é".repeat(80), "80 characters fit");
    assert_eq!(one_line("  a\u{200E}\u{200F}\u{2066}\u{2069}\u{0}b  \r\n c "), "ab c");
    assert_eq!(one_line("\u{202A}\u{202B}\u{202C}\u{202D}\n\t"), "");
}

#[test]
fn row_names_follow_the_chat_kind() {
    let me = "me-1";
    let one = chat(ChatKind::OneOnOne, Some("ignored"), vec![member("me-1", "Me Myself"), member("u2", "Paul\u{202E} Martin\n")]);
    assert_eq!(row_name(&one, me), "Paul Martin");
    assert_eq!(row_name(&one, "ME-1"), "Paul Martin", "ids compared ignoring case");
    let alone = chat(ChatKind::OneOnOne, None, vec![member("me-1", "Me")]);
    assert_eq!(row_name(&alone, me), NO_NAME);

    let group = chat(ChatKind::Group, None, vec![member("me-1", "Me"), member("u2", "Paul Martin"), member("u3", "Ana Rossi")]);
    assert_eq!(row_name(&group, me), "Paul, Ana");
    let topic = chat(ChatKind::Group, Some(" Launch\nplan "), group.members.clone());
    assert_eq!(row_name(&topic, me), "Launch plan");
    let many = chat(ChatKind::Group, None, (0..5_000).map(|i| member(&format!("u{i}"), &format!("Person{i} X"))).collect());
    let name = row_name(&many, me);
    assert_eq!(name.chars().count(), NAME_CHARS);
    assert!(name.starts_with("Person0, Person1, "));
    assert_eq!(row_name(&chat(ChatKind::Group, Some(" "), vec![member("me-1", "Me")]), me), NO_NAME);

    assert_eq!(row_name(&chat(ChatKind::Meeting, Some("Weekly"), vec![]), me), "Weekly");
    assert_eq!(row_name(&chat(ChatKind::Meeting, None, group.members.clone()), me), NO_NAME);
    assert_eq!(row_name(&chat(ChatKind::Other { name: "unknownFutureValue".into() }, None, vec![]), me), NO_NAME);
}

#[test]
fn chat_kinds_and_presences_from_graph() {
    assert_eq!(ChatKind::of_graph("oneOnOne"), ChatKind::OneOnOne);
    assert_eq!(ChatKind::of_graph("group"), ChatKind::Group);
    assert_eq!(ChatKind::of_graph("meeting"), ChatKind::Meeting);
    assert_eq!(ChatKind::of_graph("unknownFutureValue"), ChatKind::Other { name: "unknownFutureValue".into() });
    for odd in ["", "a b", "x\u{202E}", "é", &"a".repeat(33)] {
        assert_eq!(ChatKind::of_graph(odd), ChatKind::Other { name: "unreadable".into() }, "{odd:?}");
    }
    assert_eq!(ChatKind::of_graph(&"a".repeat(32)), ChatKind::Other { name: "a".repeat(32) });

    for (graph, presence) in [
        ("Available", Presence::Available),
        ("AvailableIdle", Presence::Available),
        ("Busy", Presence::Busy),
        ("BusyIdle", Presence::Busy),
        ("Away", Presence::Away),
        ("BeRightBack", Presence::Away),
        ("DoNotDisturb", Presence::DoNotDisturb),
        ("Offline", Presence::Offline),
        ("PresenceUnknown", Presence::Unknown),
        ("", Presence::Unknown),
        ("Sleeping", Presence::Unknown),
    ] {
        assert_eq!(Presence::of_graph(graph), presence, "{graph}");
    }
}

#[test]
fn plain_text_joins_runs_and_blocks() {
    let run = |t: &str| Run { text: t.into(), ..Default::default() };
    let t = ChatText {
        blocks: vec![
            Block { kind: BlockKind::Paragraph, runs: vec![run("Hello "), Run { text: "Ana".into(), mention: Some(0), ..Default::default() }] },
            Block { kind: BlockKind::Item { ordered: true, depth: 1 }, runs: vec![run("one")] },
        ],
    };
    assert_eq!(t.plain(), "Hello Ana\none");
    assert_eq!(ChatText::default().plain(), "");
}

#[test]
fn a_run_is_written_without_its_defaults_and_read_back_whole() {
    let plain = Run { text: "a".into(), ..Default::default() };
    assert_eq!(serde_json::to_string(&plain).unwrap(), r#"{"text":"a"}"#);
    let styled = Run {
        text: "b".into(),
        bold: true,
        italic: true,
        underline: true,
        strike: true,
        code: true,
        link: Some("https://contoso.example/".into()),
        mention: Some(0),
        hosted: Some("h".into()),
        outside: Some("https://contoso.example/a.png".into()),
        attachment: Some("a".into()),
    };
    for run in [plain, styled, Run::default()] {
        assert_eq!(serde_json::from_str::<Run>(&serde_json::to_string(&run).unwrap()).unwrap(), run);
    }
    // As written before the defaults were left out, too.
    let full = r#"{"text":"c","bold":false,"italic":true,"underline":false,"strike":false,"code":false,"link":null,"mention":null,"hosted":null,"outside":null,"attachment":null}"#;
    assert_eq!(serde_json::from_str::<Run>(full).unwrap(), Run { text: "c".into(), italic: true, ..Default::default() });
    // Twenty thousand one-letter runs are kept in well under the cap on a body.
    let many = ChatText { blocks: vec![Block { kind: BlockKind::Paragraph, runs: vec![Run { text: "x".into(), ..Default::default() }; 20_000] }] };
    assert!(serde_json::to_vec(&many).unwrap().len() < MAX_BODY, "{}", serde_json::to_vec(&many).unwrap().len());
}

#[test]
fn settings_default_announce_and_are_default() {
    let s = TeamsSettings::default();
    assert!(s.notify && !s.on && !s.notify_only_direct);
    assert!(s.is_default());
    assert!(!TeamsSettings { on: true, ..TeamsSettings::default() }.is_default());
    let read: TeamsSettings = serde_json::from_str("{\"on\":true}").unwrap();
    assert!(read.on && read.notify, "missing fields take their defaults");
}

#[test]
fn jobs_and_events_round_trip_through_json() {
    let j = Job::Teams {
        job: teams::Job::Picture { picture: Picture::Hosted { chat: "19:a@thread.v2".into(), message: "1".into(), hosted: "h".into() } },
    };
    assert_eq!(serde_json::from_str::<Job>(&serde_json::to_string(&j).unwrap()).unwrap(), j);
    let e = Event::Teams { event: teams::Event::Status { status: Status::Copying { done: 3, of: 9 } } };
    assert_eq!(serde_json::from_str::<Event>(&serde_json::to_string(&e).unwrap()).unwrap(), e);
    assert_eq!(j.account(), "");
}

#[test]
fn every_payload_round_trips_through_json() {
    let message = ChatMessage {
        id: "1".into(),
        chat: "19:a@thread.v2".into(),
        sender: Sender::User { id: "u2".into(), name: "Paul".into(), identity_type: Some("aadUser".into()), tenant_id: Some("t-1".into()) },
        created: 1,
        modified: 2,
        edited: Some(2),
        deleted: None,
        kind: MessageKind::Message,
        body: ChatText {
            blocks: vec![Block {
                kind: BlockKind::Quote,
                runs: vec![Run {
                    text: "x".into(),
                    bold: true,
                    link: Some("https://x.example/".into()),
                    hosted: Some("h".into()),
                    ..Default::default()
                }],
            }],
        },
        mentions: vec![Mention { id: 0, text: "Ana".into(), user_id: Some("u1".into()) }],
        reactions: vec![Reaction { emoji: "👍".into(), users: vec!["u1".into()] }],
        attachments: vec![
            Attachment::File { id: "a".into(), name: "f.docx".into(), url: "https://contoso.example/f.docx".into(), size: Some(3) },
            Attachment::Hosted { id: "h".into() },
            Attachment::Card,
        ],
        unshowable: false,
    };
    let row = ChatRow {
        id: "19:a@thread.v2".into(),
        name: "Paul".into(),
        kind: ChatKind::Other { name: "unknownFutureValue".into() },
        last_line: "Paul: hi".into(),
        last_external: false,
        at: Some(1),
        unread: 2,
        hidden: false,
        left: false,
        unreadable: true,
        external: false,
        members: 2,
        person: Some("u2".into()),
    };
    let jobs = vec![
        teams::Job::Start { settings: TeamsSettings::default() },
        teams::Job::SignIn { client_id: "c".into(), tenant: "organizations".into() },
        teams::Job::SignOut { delete_copy: true },
        teams::Job::Stop,
        teams::Job::Open { chat: None },
        teams::Job::Page { chat: "19:a@thread.v2".into(), before: 5 },
        teams::Job::Visible { users: vec!["u2".into()] },
        teams::Job::Search { query: "plan".into() },
        teams::Job::Picture { picture: Picture::Photo { user: "u2".into() } },
        teams::Job::Picture { picture: Picture::Outside { url: "https://contoso.example/a.png".into() } },
    ];
    for job in jobs {
        let j = Job::Teams { job };
        assert_eq!(serde_json::from_str::<Job>(&serde_json::to_string(&j).unwrap()).unwrap(), j);
    }
    let events = vec![
        teams::Event::Status { status: Status::PermissionMissing { scope: "Chat.Read".into() } },
        teams::Event::Status { status: Status::Failed { why: "no".into() } },
        teams::Event::Status { status: Status::NoKeychain },
        teams::Event::Account {
            name: "Ana".into(),
            address: "ana@contoso.example".into(),
            object_id: "o".into(),
            tenant_id: "t".into(),
            drive_host: None,
        },
        teams::Event::Chats { rows: vec![row] },
        teams::Event::Messages { chat: "19:a@thread.v2".into(), messages: vec![message], fresh: true, older: false },
        teams::Event::Presence { presences: vec![("u2".into(), Presence::DoNotDisturb)] },
        teams::Event::Found { query: "plan".into(), hits: vec![("19:a@thread.v2".into(), "1".into())], indexing: true, partial: false },
        teams::Event::Picture { picture: Picture::Photo { user: "u2".into() }, bytes: vec![1, 2, 3] },
        teams::Event::Removed { chat: "19:a@thread.v2".into() },
        teams::Event::SetAside { path: "/data/Teams-1a2b3c4d.quatrelle.old".into() },
    ];
    for event in events {
        let e = Event::Teams { event };
        assert_eq!(serde_json::from_str::<Event>(&serde_json::to_string(&e).unwrap()).unwrap(), e);
    }
    let preview = Preview { sender: Sender::Nobody, text: "hi".into(), at: 1 };
    let mut c = chat(ChatKind::Meeting, Some("Weekly"), vec![member("u2", "Paul")]);
    c.preview = Some(preview);
    assert_eq!(serde_json::from_str::<Chat>(&serde_json::to_string(&c).unwrap()).unwrap(), c);
}
