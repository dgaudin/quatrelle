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

/// Security review 1.5 (content n° 1): a sender the members don't have is External, whatever the
/// message says of them: a guest of the tenant is an `aadUser` of it too, and one who left the
/// chat is no longer among its members; nor is anyone while the members couldn't be read.
#[test]
fn a_sender_not_among_the_members_is_external_whatever_the_message_says() {
    let u = |t: Option<&str>, kind: Option<&str>| Sender::User {
        id: "x".into(),
        name: "X".into(),
        identity_type: kind.map(Into::into),
        tenant_id: t.map(Into::into),
    };
    assert!(!sender_internal(&u(Some("t-1"), Some("aadUser")), &[], "t-1"), "the members unknown");
    let gone_guest = u(Some("t-1"), Some("aadUser"));
    assert!(!sender_internal(&gone_guest, &[member("a", "Ana"), member("b", "Bruno")], "t-1"), "no longer a member");
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
    // Final review M1: an id is matched to the members ignoring case, as everywhere else.
    let upper = Sender::User { id: "X".into(), name: "X".into(), identity_type: Some("aadUser".into()), tenant_id: Some("t-1".into()) };
    let mut guest = member("x", "X");
    guest.roles = vec!["guest".into()];
    assert!(!sender_internal(&upper, &[guest], "t-1"));
}

#[test]
fn names_are_one_line_without_bidi_cut_at_80() {
    assert_eq!(one_line("Ana\u{202E}lim\nRossi\t "), "Analim Rossi");
    assert_eq!(one_line(&"é".repeat(100)).chars().count(), 80);
    assert!(one_line(&"é".repeat(100)).ends_with('…'));
    assert_eq!(one_line(&"é".repeat(80)), "é".repeat(80), "80 characters fit");
    assert_eq!(one_line("  a\u{200E}\u{200F}\u{2066}\u{2069}\u{0}b  \r\n c "), "ab c");
    assert_eq!(one_line("\u{202A}\u{202B}\u{202C}\u{202D}\n\t"), "");
    // Final review M3: the Arabic letter mark too.
    assert_eq!(one_line("a\u{061C}b"), "ab");
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

/// Settings written before meetings could be allowed read back with meetings not allowed.
#[test]
fn settings_without_meetings_read_as_not_allowed() {
    let before = concat!(
        r#"{"on":true,"client_id":"c","tenant":"organizations","signed_in":true,"name":"Morgan Lee","address":"morgan@contoso.example","#,
        r#""object_id":"o","tenant_id":"t","drive_host":null,"notify":true,"notify_only_direct":false,"show_unreadable":false}"#,
    );
    let read: TeamsSettings = serde_json::from_str(before).unwrap();
    assert!(read.on && read.signed_in && !read.meetings);
    assert!(!TeamsSettings::default().meetings);
}

/// A sign-in asked before meetings could be allowed reads back as one without them.
#[test]
fn a_sign_in_job_without_meetings_reads_false() {
    let job: teams::Job = serde_json::from_str(r#"{"teams":"signIn","client_id":"c","tenant":"organizations"}"#).unwrap();
    assert_eq!(job, teams::Job::SignIn { client_id: "c".into(), tenant: "organizations".into(), meetings: false });
}

#[test]
fn meeting_jobs_and_events_round_trip_through_json() {
    fn round<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(v: T) {
        assert_eq!(serde_json::from_str::<T>(&serde_json::to_string(&v).unwrap()).unwrap(), v);
    }
    let fields = |title: &str| TeamsMeetingFields { title: title.into(), start: 1_795_165_200, end: 1_795_168_800 };
    // A passcode is written redacted: an empty one reads back as it was.
    let meeting = TeamsMeeting {
        id: "MSpkYzE3Njc0Yy04MWQ5".into(),
        join_url: "https://teams.microsoft.com/meet/1".into(),
        meeting_id: "1234567890".into(),
        passcode: crate::job::Secret::default(),
        toll_number: "+1 323-555-0166".into(),
        conference_id: "123 456 789 0".into(),
        options_url: "https://teams.microsoft.com/meetingOptions/?organizerId=o".into(),
    };
    for job in [
        teams::Job::SignIn { client_id: "c".into(), tenant: "organizations".into(), meetings: true },
        teams::Job::MeetingCreate { ask: 7, fields: fields("Budget") },
        teams::Job::MeetingUpdate { id: meeting.id.clone(), before: fields("Budget"), after: fields("Budget 2027") },
        teams::Job::MeetingDelete { id: meeting.id.clone() },
    ] {
        round(job);
    }
    for event in [
        teams::Event::MeetingCreated { ask: 7, meeting: Some(meeting.clone()), error: None },
        teams::Event::MeetingCreated { ask: 8, meeting: None, error: Some(TeamsMeetingError::Refused { status: 400, code: "BadRequest".into() }) },
        teams::Event::MeetingUpdated { id: meeting.id.clone(), meeting: None, error: Some(TeamsMeetingError::Untrusted) },
        teams::Event::MeetingDeleted { id: meeting.id.clone(), error: Some(TeamsMeetingError::Offline) },
        teams::Event::Meetings { allowed: true },
    ] {
        round(event);
    }
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
        teams::Job::SignIn { client_id: "c".into(), tenant: "organizations".into(), meetings: false },
        teams::Job::SignOut { delete_copy: true },
        teams::Job::Stop,
        teams::Job::Open { chat: None },
        teams::Job::Page { chat: "19:a@thread.v2".into(), before: 5 },
        teams::Job::Visible { users: vec!["u2".into()] },
        teams::Job::Search { query: "plan".into() },
        teams::Job::Picture { picture: Picture::Photo { user: "u2".into() } },
        teams::Job::Picture { picture: Picture::Outside { url: "https://contoso.example/a.png".into() } },
        teams::Job::DeleteSetAside { path: "/data/Teams-0123abcd.quatrelle.old".into() },
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

#[test]
fn delivery_two_jobs_and_events_round_trip_through_json() {
    let body = ChatText { blocks: vec![Block { kind: BlockKind::Paragraph, runs: vec![Run { text: "hi".into(), ..Run::default() }] }] };
    for job in [
        teams::Job::Op {
            op: Op::Send { chat: "19:a@thread.v2".into(), local: "1731500000-1".into(), body: body.clone(), mentions: Vec::new(), parts: Vec::new() },
        },
        teams::Job::Op { op: Op::MarkRead { chat: "19:a@thread.v2".into() } },
        teams::Job::Retry { seq: 7 },
        teams::Job::Discard { seq: 7 },
        teams::Job::DiscardUnnumbered { local: "1731500000-1".into() },
        teams::Job::Find { chat: "19:a@thread.v2".into(), message: "1730000000000".into() },
    ] {
        let j = Job::Teams { job };
        assert_eq!(serde_json::from_str::<Job>(&serde_json::to_string(&j).unwrap()).unwrap(), j);
    }
    let out = Outgoing {
        seq: 3,
        local: "l".into(),
        chat: "19:a@thread.v2".into(),
        body,
        mentions: Vec::new(),
        what: OutKind::Send,
        parts: Vec::new(),
        state: OutState::NotSent { reason: Unsent::Refused, code: Some("HTTP 403 Forbidden".into()) },
    };
    let news = News {
        chat: "19:a@thread.v2".into(),
        message: "1730000000000".into(),
        sender: "Ana Exemple".into(),
        external: true,
        app: false,
        mentions_me: true,
        chat_name: Some("Agenda".into()),
        direct: false,
        hidden: false,
        text: "Lunch?".into(),
        at: 1_730_000_000_000,
    };
    let unread = News { message: String::new(), chat_name: None, direct: true, app: true, ..news.clone() };
    let mut waiting = out.clone();
    waiting.state = OutState::Waiting;
    let mut sending = out.clone();
    sending.state = OutState::Sending;
    let states = [Unsent::Refused, Unsent::TooLong, Unsent::TooMany, Unsent::Left, Unsent::Unconfirmed].into_iter().map(|reason| Outgoing {
        seq: 0,
        state: OutState::NotSent { reason, code: None },
        ..out.clone()
    });
    let items: Vec<Outgoing> = [waiting, sending].into_iter().chain(states).collect();
    for event in [
        teams::Event::Outbox { chat: out.chat.clone(), items: vec![out], took: Some("l".into()) },
        teams::Event::Outbox { chat: "19:a@thread.v2".into(), items, took: None },
        teams::Event::Unsent { count: 2 },
        teams::Event::News { news: vec![news, unread] },
        teams::Event::SignInPage { url: "https://login.microsoftonline.com/organizations/oauth2/v2.0/authorize?x=1".into() },
        teams::Event::NoPicture { picture: Picture::Photo { user: "u1".into() } },
        teams::Event::Placed { chat: "c".into(), message: "m".into(), newest: false },
    ] {
        let e = Event::Teams { event };
        assert_eq!(serde_json::from_str::<Event>(&serde_json::to_string(&e).unwrap()).unwrap(), e);
    }
}

#[test]
fn first_line_stops_at_the_line_without_bidi_cut_at_max() {
    assert_eq!(first_line("Lunch\u{202E} at\tnoon?\nsecond line", 200), "Lunch at noon?");
    assert_eq!(first_line(&"é".repeat(300), NEWS_CHARS).chars().count(), NEWS_CHARS);
    assert!(first_line(&"é".repeat(300), NEWS_CHARS).ends_with('…'));
    assert_eq!(first_line(&"é".repeat(NEWS_CHARS), NEWS_CHARS), "é".repeat(NEWS_CHARS), "max characters fit");
    assert_eq!(first_line("\nsecond line", 200), "", "an empty first line is empty");
    assert_eq!(first_line("  a\r\nb", 200), "a");
    assert_eq!(first_line("a\u{2066}\u{061C}\u{0}b  c", 200), "ab c");
    assert_eq!(first_line("abc", 1), "…");
    assert_eq!(first_line("abc", 0), "", "nothing past max, even the ellipsis");
    assert_eq!(first_line("", 200), "");
    // A name is still cut at its own length.
    assert_eq!(one_line(&"é".repeat(300)).chars().count(), NAME_CHARS);
}

#[test]
fn delivery_three_ops_and_outgoing_round_trip_through_json() {
    let body = ChatText {
        blocks: vec![Block { kind: BlockKind::Paragraph, runs: vec![Run { text: "Ana Exemple".into(), mention: Some(0), ..Run::default() }] }],
    };
    let mentions = vec![Mention { id: 0, text: "Ana Exemple".into(), user_id: Some("u-ana".into()) }];
    for op in [
        Op::Send { chat: "19:a@thread.v2".into(), local: "1-1".into(), body: body.clone(), mentions: mentions.clone(), parts: Vec::new() },
        Op::Edit {
            chat: "19:a@thread.v2".into(),
            local: "1-2".into(),
            message: "1730000000000".into(),
            created: 1_730_000_000_000,
            body: body.clone(),
            mentions: mentions.clone(),
        },
        Op::Delete { chat: "19:a@thread.v2".into(), local: "1-3".into(), message: "1730000000000".into(), created: 1_730_000_000_000 },
        Op::React {
            chat: "19:a@thread.v2".into(),
            local: "1-4".into(),
            message: "1730000000000".into(),
            created: 1_730_000_000_000,
            emoji: QUICK_REACTIONS[1].into(),
            on: false,
        },
    ] {
        let j = Job::Teams { job: teams::Job::Op { op } };
        assert_eq!(serde_json::from_str::<Job>(&serde_json::to_string(&j).unwrap()).unwrap(), j);
    }
    let o = Outgoing {
        seq: 4,
        local: "1-4".into(),
        chat: "19:a@thread.v2".into(),
        body,
        mentions,
        what: OutKind::React { message: "m".into(), created: 5, emoji: "\u{1F44D}".into(), on: true },
        state: OutState::Waiting,
        parts: Vec::new(),
    };
    assert_eq!(serde_json::from_str::<Outgoing>(&serde_json::to_string(&o).unwrap()).unwrap(), o);
}

#[test]
fn what_delivery_two_wrote_reads_back_as_a_send() {
    let o: Outgoing =
        serde_json::from_str(r#"{"seq":3,"local":"1-1","chat":"19:a@thread.v2","body":{"blocks":[]},"state":{"state":"waiting"}}"#).unwrap();
    assert_eq!((o.what, o.mentions.len()), (OutKind::Send, 0));
    let op: Op = serde_json::from_str(r#"{"op":"send","chat":"19:a@thread.v2","local":"1-1","body":{"blocks":[]}}"#).unwrap();
    assert!(matches!(op, Op::Send { mentions, .. } if mentions.is_empty()));
}

#[test]
fn a_reaction_is_an_emoji_never_a_name() {
    for e in QUICK_REACTIONS {
        assert!(reactable(e), "{e}");
    }
    assert!(reactable("\u{1F498}"));
    for e in ["", "like", "custom", "a\u{1F44D}", "\u{1F44D} ", "\u{1F44D}\u{202E}", "\u{1F44D}\u{7}"] {
        assert!(!reactable(e), "{e:?}");
    }
    assert!(!reactable(&"\u{1F600}".repeat(REACTION_CHARS + 1)));
    assert!(same_reaction("\u{2764}", "\u{2764}\u{FE0F}") && !same_reaction("\u{1F44D}", "\u{1F44E}"));
}

#[test]
fn only_a_member_of_the_organisation_may_be_mentioned() {
    let m = |kind: &str, tenant: Option<&str>| Member {
        user_id: "u-ana".into(),
        kind: kind.into(),
        tenant_id: tenant.map(str::to_string),
        ..Member::default()
    };
    assert!(mentionable(&m("aadUserConversationMember", Some("TENANT-1")), "tenant-1"));
    assert!(!mentionable(&m("aadUserConversationMember", Some("tenant-2")), "tenant-1"));
    assert!(!mentionable(&m("aadUserConversationMember", None), "tenant-1"));
    assert!(!mentionable(&m("anonymousGuestConversationMember", Some("tenant-1")), "tenant-1"));
    assert!(!mentionable(&m("aadUserConversationMember", Some("")), ""));
}

#[test]
fn delivery_four_ops_jobs_and_events_round_trip_through_json() {
    fn round<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(v: T) {
        assert_eq!(serde_json::from_str::<T>(&serde_json::to_string(&v).unwrap()).unwrap(), v);
    }
    let ana =
        Person { id: "00000000-0000-0000-0000-0000000000a1".into(), name: "Ana Exemple".into(), address: "ana@contoso.example".into(), guest: false };
    round(Op::Send {
        chat: "19:a@thread.v2".into(),
        local: "l1".into(),
        body: ChatText::default(),
        mentions: vec![],
        parts: vec![Part::File { path: "a.txt".into() }, Part::Picture { path: "p.png".into() }],
    });
    for job in [
        teams::Job::File { chat: "c".into(), message: "m".into(), created: 5, attachment: "a".into() },
        teams::Job::Directory { query: "an".into() },
        teams::Job::Create { local: "n1".into(), people: vec![ana.clone()], topic: Some("Studio".into()) },
    ] {
        round(job);
    }
    let whys = [FileWhy::TooBig, FileWhy::NoRoom, FileWhy::Refused, FileWhy::Offline, FileWhy::Stalled, FileWhy::Busy, FileWhy::Gone];
    for outcome in
        whys.into_iter().map(|why| FileOutcome::Failed { why, code: Some("HTTP 403".into()) }).chain([FileOutcome::Saved { path: "x".into() }])
    {
        round(teams::Event::File { chat: "c".into(), message: "m".into(), attachment: "a".into(), outcome });
    }
    round(teams::Event::People { query: "an".into(), people: vec![ana], failed: false });
    round(teams::Event::Created { local: "n1".into(), chat: "c".into() });
    for why in [NotCreated::Offline, NotCreated::Refused, NotCreated::TooMany] {
        round(teams::Event::NotCreated { local: "n1".into(), why, code: None });
        round(why);
    }
    let parts =
        vec![OutPart { name: "a.txt".into(), size: 10, picture: false, sent: 4 }, OutPart { name: "p.png".into(), size: 20, picture: true, sent: 0 }];
    let mut o = Outgoing {
        seq: 1,
        local: "l1".into(),
        chat: "c".into(),
        body: ChatText::default(),
        mentions: vec![],
        what: OutKind::Send,
        parts,
        state: OutState::Uploading,
    };
    round(o.clone());
    for reason in [Unsent::FileGone, Unsent::FileTooBig, Unsent::NotShared] {
        o.state = OutState::NotSent { reason, code: None };
        round(o.clone());
    }
}

#[test]
fn what_delivery_three_wrote_reads_back_without_parts() {
    let op: Op = serde_json::from_str(r#"{"op":"send","chat":"c","local":"l","body":{"blocks":[]}}"#).unwrap();
    assert!(matches!(op, Op::Send { parts, .. } if parts.is_empty()));
    let o: Outgoing = serde_json::from_str(r#"{"seq":1,"local":"l","chat":"c","body":{"blocks":[]},"state":{"state":"waiting"}}"#).unwrap();
    assert!(o.parts.is_empty());
}

#[test]
fn a_person_is_a_guid_a_line_and_an_address() {
    let ok =
        Person { id: "00000000-0000-0000-0000-0000000000a1".into(), name: "Ana Exemple".into(), address: "ana@contoso.example".into(), guest: false };
    assert!(person_ok(&ok));
    let with = |f: &dyn Fn(&mut Person)| {
        let mut p = ok.clone();
        f(&mut p);
        person_ok(&p)
    };
    for id in ["u-ana", "a/b", "x')"] {
        assert!(!with(&|p| p.id = id.into()), "{id}");
    }
    for name in ["", "Ana  Exemple", "Ana\u{202E}"] {
        assert!(!with(&|p| p.name = name.into()), "{name:?}");
    }
    assert!(!with(&|p| p.address = "ana @contoso.example".into()));
    assert!(!with(&|p| p.address = format!("{}@x.example", "a".repeat(320))));
    assert!(is_object_id("00000000-0000-0000-0000-0000000000a1") && !is_object_id("0000000-0000-0000-0000-0000000000a1"));
}

#[test]
fn an_object_id_and_an_address_are_checked_to_the_character() {
    let id = "00000000-0000-0000-0000-0000000000a1";
    assert!(is_object_id(id) && is_object_id("ABCDEF01-2345-6789-abcd-ef0123456789"));
    for bad in [
        "0000000g-0000-0000-0000-0000000000a1",
        "00000000-0000-0000-0000-0000000000a'",
        "00000000-000-00000-0000-0000000000a1",
        "00000000-0000-0000-000-00000000000a1",
        "00000000-0000-0000-0000-0000000000a",
        "00000000-0000-0000-0000-0000000000a12",
        "",
        "00000000-0000-0000-0000",
    ] {
        assert!(!is_object_id(bad), "{bad:?}");
    }
    let with = |address: String| person_ok(&Person { id: id.into(), name: "Ana Exemple".into(), address, guest: false });
    assert!(with("ana@contoso.example".into()));
    assert!(!with("an\u{7}a@contoso.example".into()) && !with("an\u{202E}a@contoso.example".into()) && !with("an\ta@contoso.example".into()));
    let of_len = |n: usize| format!("{}@contoso.example", "a".repeat(n - "@contoso.example".len()));
    assert!(with(of_len(320)) && !with(of_len(321)));
}
