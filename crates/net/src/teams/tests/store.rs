// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The local copy (`Teams.quatrelle`): bound to the identity it was made for, messages in pages
//! found again by their creation time, deleted messages emptied, and the file read back as the
//! hostile input it may be.

use quatrelle_model::pim::teams::{
    Attachment, Block, BlockKind, Chat, ChatKind, ChatMessage, ChatText, MAX_BODY, MAX_ID, Mention, Reaction, Run, Sender, TEAMS_ACCOUNT,
};
use quatrelle_store::Store;

use super::{MINUTE, T0};
use crate::teams::store::{ChatRecord, Copy, Identity, Opened, PAGE, chat_hash, imgref_key};

const A: &str = "19:a1b2@thread.v2";
const B: &str = "19:c3d4@thread.v2";

fn who(user: &str) -> Identity {
    Identity { tenant_id: "t-1".into(), object_id: user.into() }
}

fn chat(id: &str) -> Chat {
    Chat {
        id: id.into(),
        kind: ChatKind::OneOnOne,
        topic: None,
        members: Vec::new(),
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

fn text(s: &str) -> ChatText {
    ChatText { blocks: vec![Block { kind: BlockKind::Paragraph, runs: vec![Run { text: s.into(), ..Default::default() }] }] }
}

/// Message `i` of a chat: made `i` minutes after T0, its id its time.
fn msg(chat: &str, i: i64) -> ChatMessage {
    let at = T0 + i * MINUTE;
    ChatMessage {
        id: at.to_string(),
        chat: chat.into(),
        sender: Sender::User { id: "u-ana".into(), name: "Ana Lima".into(), identity_type: Some("aadUser".into()), tenant_id: Some("t-1".into()) },
        created: at,
        modified: at,
        body: text(&format!("Message {i}")),
        ..ChatMessage::default()
    }
}

fn number(m: &ChatMessage) -> i64 {
    (m.created - T0) / MINUTE
}

fn numbers(msgs: &[ChatMessage]) -> Vec<i64> {
    msgs.iter().map(number).collect()
}

/// A copy in memory holding chat A, and its store.
fn copy() -> (Copy, Store) {
    let s = Store::memory();
    let (c, opened) = Copy::open(s.clone(), &who("u-1")).unwrap();
    assert_eq!(opened, Opened::New);
    c.put_chat(&ChatRecord::new(chat(A))).unwrap();
    (c, s)
}

fn temp(name: &str) -> std::path::PathBuf {
    let t = std::env::temp_dir().join(format!("quatrelle-teams-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    t
}

/// Every message of a chat, from the last pages back to the first, a page at a time.
fn all_back(c: &Copy, chat: &str) -> Vec<i64> {
    let mut out = c.last_pages(chat, 2);
    for _ in 0..1000 {
        let Some(first) = out.first() else { break };
        let older = c.before(chat, first.created);
        if older.is_empty() {
            break;
        }
        assert!(older.len() <= PAGE);
        out.splice(0..0, older);
    }
    numbers(&out)
}

#[test]
fn another_identitys_copy_is_set_aside_never_opened() {
    let t = temp("identity");
    let open = || Store::open_packed(t.join("app"), t.join("files")).unwrap();
    let s = open();
    let (c, opened) = Copy::open(s.clone(), &who("u-1")).unwrap();
    assert_eq!(opened, Opened::New);
    c.put_chat(&ChatRecord::new(chat(A))).unwrap();
    c.upsert(A, &[msg(A, 0)]).unwrap();
    let file = s.data_file(TEAMS_ACCOUNT).unwrap();
    assert_eq!(file.file_name().unwrap(), "Teams.quatrelle");
    drop(c);
    let (c, opened) = Copy::open(s.clone(), &who("u-1")).unwrap();
    assert_eq!(opened, Opened::Same);
    assert_eq!(numbers(&c.last_pages(A, 2)), [0]);
    drop(c);

    let (c, opened) = Copy::open(s.clone(), &who("u-2")).unwrap();
    let Opened::SetAside(aside) = opened else { panic!("{opened:?}") };
    let name = format!("Teams-{}.quatrelle.old", &chat_hash("t-1/u-1")[..8]);
    assert_eq!(aside, t.join("files").join(&name), "named for whose it is");
    assert!(aside.exists() && aside.to_string_lossy().ends_with(".quatrelle.old"));
    assert!(c.chats().is_empty() && c.chat(A).is_none() && c.last_pages(A, 2).is_empty(), "the new copy has nothing of the other's");
    assert_eq!(s.data_file(TEAMS_ACCOUNT).unwrap(), file, "a file of its own, under the same name");
    // After a restart (every handle on the store let go), the same identity again: its own copy,
    // still empty.
    drop((c, s));
    let s = open();
    let (c, opened) = Copy::open(s.clone(), &who("u-2")).unwrap();
    assert_eq!(opened, Opened::Same);
    assert!(c.chats().is_empty());
    // Deleting it takes the file, and one set aside as damaged.
    let damaged = file.with_extension("quatrelle.damaged");
    std::fs::write(&damaged, b"x").unwrap();
    c.delete().unwrap();
    assert!(!file.exists() && !damaged.exists() && aside.exists(), "the other identity's copy is the user's to keep");
    let _ = std::fs::remove_dir_all(&t);
}

#[test]
fn a_copy_that_doesnt_say_whose_it_is_isnt_read() {
    let record = serde_json::to_vec(&ChatRecord::new(chat(A))).unwrap();
    let chats = (format!("{TEAMS_ACCOUNT}/chats/{}.json", chat_hash(A)), record);
    for identity in [None, Some(b"{damaged".to_vec()), Some(br#"{"tenant_id":"t-1","object_id":"u-9"}"#.to_vec())] {
        let mut files = vec![chats.clone()];
        files.extend(identity.map(|i| (format!("{TEAMS_ACCOUNT}/identity.json"), i)));
        // A cache without data files can't set it aside: it goes.
        let (c, opened) = Copy::open(Store::memory_from(files), &who("u-1")).unwrap();
        assert_eq!(opened, Opened::New);
        assert!(c.chat(A).is_none() && c.chats().is_empty());
    }
    // No identity to keep a copy for.
    assert!(Copy::open(Store::memory(), &Identity { tenant_id: String::new(), object_id: "u-1".into() }).is_err());

    // In a data file: a damaged identity is set aside too.
    let t = temp("unknown");
    let s = Store::open_packed(t.join("app"), t.join("files")).unwrap();
    s.write_account_files(TEAMS_ACCOUNT, &[("identity.json".into(), b"{damaged".to_vec()), (format!("chats/{}.json", chat_hash(A)), chats.1)])
        .unwrap();
    let (c, opened) = Copy::open(s, &who("u-1")).unwrap();
    assert!(matches!(&opened, Opened::SetAside(p) if p.exists()), "{opened:?}");
    assert!(c.chat(A).is_none());
    let _ = std::fs::remove_dir_all(&t);
}

#[test]
fn pages_split_at_100_and_keep_creation_order() {
    let (c, _) = copy();
    // 250 messages out of order (97 is prime to 250), in Graph's pages of 50.
    let all: Vec<ChatMessage> = (0..250).map(|i| msg(A, i * 97 % 250)).collect();
    for batch in all.chunks(50) {
        assert_eq!(c.upsert(A, batch).unwrap().len(), 50);
    }
    let r = c.chat(A).unwrap();
    assert!(r.pages.len() >= 3, "{:?}", r.pages);
    assert!(r.pages.windows(2).all(|w| w[0] < w[1]));
    let window = c.last_pages(A, 2);
    assert!(!window.is_empty() && window.len() <= 2 * PAGE, "{}", window.len());
    let newest: Vec<i64> = (250 - window.len() as i64..250).collect();
    assert_eq!(numbers(&window), newest, "the newest, in creation order");
    let older = c.before(A, window[0].created);
    assert!(!older.is_empty() && older.len() <= PAGE);
    assert_eq!(numbers(&older), (newest[0] - older.len() as i64..newest[0]).collect::<Vec<_>>(), "the page before");
    assert_eq!(all_back(&c, A), (0..250).collect::<Vec<_>>(), "each once");
    assert!(c.upsert(A, &all[..50]).unwrap().is_empty(), "the same again changes nothing");

    // New messages one at a time fill the newest page; history read backwards fills the oldest.
    for forwards in [true, false] {
        let (c, _) = copy();
        for i in 0..250 {
            let i = if forwards { i } else { 249 - i };
            c.upsert(A, &[msg(A, i)]).unwrap();
        }
        let r = c.chat(A).unwrap();
        assert_eq!(r.pages.len(), 3, "forwards {forwards}: full pages");
        assert_eq!(c.last_pages(A, 2).len(), if forwards { 150 } else { 200 });
        assert_eq!(all_back(&c, A), (0..250).collect::<Vec<_>>());
    }
    // A message in the middle of a full page splits it in two.
    let (c, _) = copy();
    let evens: Vec<ChatMessage> = (0..300).map(|i| msg(A, i * 2)).collect();
    c.upsert(A, &evens).unwrap();
    assert_eq!(c.chat(A).unwrap().pages.len(), 3);
    c.upsert(A, &[msg(A, 201)]).unwrap();
    let r = c.chat(A).unwrap();
    assert_eq!(r.pages.len(), 4, "{:?}", r.pages);
    let mut want: Vec<i64> = (0..300).map(|i| i * 2).collect();
    want.insert(101, 201);
    assert_eq!(all_back(&c, A), want);
}

#[test]
fn an_edit_finds_its_page_by_creation_time() {
    let (c, s) = copy();
    let all: Vec<ChatMessage> = (0..250).map(|i| msg(A, i)).collect();
    for batch in all.chunks(50) {
        c.upsert(A, batch).unwrap();
    }
    s.track_changes();
    let mut edited = msg(A, 5);
    edited.body = text("Message 5, edited");
    edited.edited = Some(T0 + 300 * MINUTE);
    edited.modified = T0 + 300 * MINUTE;
    assert_eq!(c.upsert(A, std::slice::from_ref(&edited)).unwrap(), [edited.clone()]);
    let changes: Vec<String> = s.take_changes().into_iter().map(|(k, _)| k).collect();
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert!(changes[0].starts_with(&format!("{TEAMS_ACCOUNT}/msgs/{}/", chat_hash(A))), "{changes:?}");
    let page = c.before(A, msg(A, 6).created);
    assert_eq!(page.last(), Some(&edited));
    assert_eq!(numbers(&page), (0..6).collect::<Vec<_>>());
    // The same again: nothing changes, nothing is written.
    assert!(c.upsert(A, &[edited]).unwrap().is_empty());
    assert!(s.take_changes().is_empty());
}

#[test]
fn a_deleted_message_keeps_only_its_id_times_and_sender() {
    let (c, s) = copy();
    let mut m = msg(A, 1);
    m.body.blocks[0].runs.push(Run { text: String::new(), hosted: Some("h1".into()), ..Default::default() });
    m.attachments = vec![
        Attachment::Hosted { id: "h1".into() },
        Attachment::File { id: "f".into(), name: "plan.pdf".into(), url: "https://contoso.example/plan.pdf".into(), size: Some(3) },
    ];
    m.mentions = vec![Mention { id: 0, text: "Paul".into(), user_id: Some("u-paul".into()) }];
    m.reactions = vec![Reaction { emoji: "👍".into(), users: vec!["u-paul".into()] }];
    m.edited = Some(T0 + 2 * MINUTE);
    c.upsert(A, &[m.clone(), msg(A, 2)]).unwrap();
    c.put_hosted(A, &m.id, "h1", b"the picture's bytes").unwrap();
    assert_eq!(c.hosted(A, &m.id, "h1").as_deref(), Some(&b"the picture's bytes"[..]));

    // Graph says it was deleted, its content still in the answer.
    let mut gone = m.clone();
    gone.deleted = Some(T0 + 10 * MINUTE);
    gone.modified = T0 + 10 * MINUTE;
    let bare = ChatMessage {
        id: m.id.clone(),
        chat: A.into(),
        sender: m.sender.clone(),
        created: m.created,
        modified: gone.modified,
        edited: m.edited,
        deleted: gone.deleted,
        ..ChatMessage::default()
    };
    assert_eq!(c.upsert(A, &[gone]).unwrap(), std::slice::from_ref(&bare));
    assert_eq!(c.last_pages(A, 2), [bare, msg(A, 2)]);
    assert_eq!(c.hosted(A, &m.id, "h1"), None);
    assert_eq!(s.account_file(TEAMS_ACCOUNT, &imgref_key(A, &m.id, "h1")).unwrap(), None);
    assert!(s.account_files(TEAMS_ACCOUNT, "img").unwrap().is_empty(), "the picture's bytes go too");
}

#[test]
fn a_record_under_another_key_is_ignored() {
    let (c, s) = copy();
    let b = serde_json::to_vec(&ChatRecord::new(chat(B))).unwrap();
    s.write_account_files(TEAMS_ACCOUNT, &[(format!("chats/{}.json", chat_hash(A)), b)]).unwrap();
    assert_eq!(c.chat(A), None);
    assert!(c.chats().is_empty(), "nor listed under the key it's in");

    // A page under A's key holding B's messages, or a message of B in A's page, the same.
    c.put_chat(&ChatRecord::new(chat(A))).unwrap();
    c.upsert(A, &[msg(A, 0)]).unwrap();
    let key = format!("msgs/{}/{}.json", chat_hash(A), c.chat(A).unwrap().page_keys[0]);
    let page = |chat: &str, msgs: &[ChatMessage]| serde_json::to_vec(&serde_json::json!({ "chat": chat, "messages": msgs })).unwrap();
    s.write_account_files(TEAMS_ACCOUNT, &[(key.clone(), page(B, &[msg(B, 1)]))]).unwrap();
    assert!(c.last_pages(A, 2).is_empty());
    s.write_account_files(TEAMS_ACCOUNT, &[(key.clone(), page(A, &[msg(B, 1), msg(A, 2)]))]).unwrap();
    assert_eq!(numbers(&c.last_pages(A, 2)), [2]);
    // More than a page's worth in the file: a page's worth is read.
    let many: Vec<ChatMessage> = (0..150).map(|i| msg(A, i)).collect();
    s.write_account_files(TEAMS_ACCOUNT, &[(key, page(A, &many))]).unwrap();
    assert_eq!(c.last_pages(A, 2).len(), PAGE);
}

#[test]
fn pages_that_dont_hold_together_start_the_chat_again() {
    let (c, s) = copy();
    c.upsert(A, &(0..150).map(|i| msg(A, i)).collect::<Vec<_>>()).unwrap();
    let mut r = c.chat(A).unwrap();
    r.watermark = Some(T0);
    r.pages.reverse();
    s.write_account_files(TEAMS_ACCOUNT, &[(format!("chats/{}.json", chat_hash(A)), serde_json::to_vec(&r).unwrap())]).unwrap();
    let r = c.chat(A).unwrap();
    assert!(r.pages.is_empty() && r.page_keys.is_empty() && r.watermark.is_none(), "read again from the server");
    // What it had goes as the first message comes in again.
    c.upsert(A, &[msg(A, 7)]).unwrap();
    assert_eq!(all_back(&c, A), [7]);
    assert_eq!(s.account_files(TEAMS_ACCOUNT, &format!("msgs/{}", chat_hash(A))).unwrap().len(), 1);
}

#[test]
fn put_chat_keeps_the_pages_upsert_wrote() {
    let (c, _) = copy();
    let mut r = c.chat(A).unwrap();
    c.upsert(A, &[msg(A, 0), msg(A, 1)]).unwrap();
    r.watermark = Some(T0 + MINUTE);
    r.pages = vec![5];
    r.page_keys = vec![9];
    c.put_chat(&r).unwrap();
    let back = c.chat(A).unwrap();
    assert_eq!(back.watermark, Some(T0 + MINUTE));
    assert_eq!(numbers(&c.last_pages(A, 2)), [0, 1], "a record read before the messages went in doesn't lose them");
    // Messages of a chat not in the copy aren't kept.
    assert!(c.upsert(B, &[msg(B, 0)]).unwrap().is_empty());
    assert!(c.chat(B).is_none() && c.last_pages(B, 2).is_empty());
}

#[test]
fn a_message_without_a_usable_id_is_never_stored() {
    let (c, s) = copy();
    // As Graph's reader gives one whose id couldn't be read.
    let unreadable = ChatMessage { unshowable: true, ..msg(A, 0) };
    let ids = [String::new(), "a\u{7}b".into(), "x".repeat(MAX_ID + 1)];
    let bad: Vec<ChatMessage> = ids.iter().map(|id| ChatMessage { id: id.clone(), ..unreadable.clone() }).collect();
    assert!(c.upsert(A, &bad).unwrap().is_empty());
    assert!(c.last_pages(A, 2).is_empty());
    assert!(s.account_files(TEAMS_ACCOUNT, &format!("msgs/{}", chat_hash(A))).unwrap().is_empty());
    // Nor a chat, nor a picture.
    assert!(c.put_chat(&ChatRecord::new(chat(""))).is_err());
    assert!(c.put_hosted(A, "", "h1", b"x").is_err());
    // The same message, in another chat's batch, isn't this chat's.
    assert!(c.upsert(A, &[msg(B, 0)]).unwrap().is_empty());
}

#[test]
fn bodies_over_256_kib_are_cut_when_stored() {
    let (c, _) = copy();
    let styled = Run { text: "x".into(), bold: true, italic: true, link: Some("https://contoso.example/a".into()), ..Default::default() };
    let many = ChatText { blocks: vec![Block { kind: BlockKind::Paragraph, runs: vec![styled.clone(); 40_000] }] };
    // Quotes and controls take two and six bytes each as kept; an accented letter two in UTF-8.
    let long =
        |s: &str| ChatText { blocks: vec![Block { kind: BlockKind::Quote, runs: vec![Run { text: s.repeat(200_000), ..Default::default() }] }] };
    for (i, body) in [many, long("\""), long("\u{1}"), long("é"), text("short")].into_iter().enumerate() {
        let mut m = msg(A, i as i64);
        m.body = body.clone();
        c.upsert(A, &[m.clone()]).unwrap();
        let kept = c.last_pages(A, 2).into_iter().find(|k| k.id == m.id).unwrap();
        let size = serde_json::to_vec(&kept.body).unwrap().len();
        assert!(size <= MAX_BODY, "{i}: {size}");
        if serde_json::to_vec(&body).unwrap().len() <= MAX_BODY {
            assert_eq!(kept.body, body, "{i}: under the cap, kept whole");
            continue;
        }
        assert!(size > MAX_BODY - 64, "{i}: filled to the cap, {size}");
        let (runs, was) = (&kept.body.blocks[0].runs, &body.blocks[0].runs);
        let last = runs.len() - 1;
        assert!(runs[..last] == was[..last] && was[last].text.starts_with(&runs[last].text), "{i}: what's kept is the start");
    }
}

#[test]
#[cfg(unix)]
fn the_files_are_the_users_only() {
    use std::os::unix::fs::PermissionsExt;
    let t = temp("modes");
    let s = Store::open_packed(t.join("app"), t.join("files")).unwrap();
    let (c, _) = Copy::open(s.clone(), &who("u-1")).unwrap();
    c.put_chat(&ChatRecord::new(chat(A))).unwrap();
    let file = s.data_file(TEAMS_ACCOUNT).unwrap();
    assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(file.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    let _ = std::fs::remove_dir_all(&t);
}

#[test]
fn the_fuzz_entry_reaches_the_pages() {
    let record = serde_json::to_string(&ChatRecord { pages: vec![T0], page_keys: vec![0], ..ChatRecord::new(chat(A)) }).unwrap();
    let page = serde_json::to_string(&serde_json::json!({ "chat": A, "messages": [msg(A, 0), msg(A, 1)] })).unwrap();
    let message = serde_json::to_string(&msg(A, 2)).unwrap();
    let parts = [A, "identity.json", r#"{"tenant_id":"t-1","object_id":"u-1"}"#, "chats/=.json", &record, "msgs/=/0.json", &page, &message];
    assert_eq!(crate::teams::fuzz_teams_copy(parts.join("\0").as_bytes()), Ok(()));
    // A deleted message planted with a sender the reader would never give: held, not a crash.
    let ghost = ChatMessage {
        deleted: Some(T0),
        sender: Sender::User { id: String::new(), name: "a\nb".into(), identity_type: None, tenant_id: None },
        ..msg(A, 0)
    };
    let planted = serde_json::to_string(&serde_json::json!({ "chat": A, "messages": [ghost] })).unwrap();
    let parts = [A, "identity.json", r#"{"tenant_id":"t-1","object_id":"u-1"}"#, "chats/=.json", &record, "msgs/=/0.json", &planted];
    assert_eq!(crate::teams::fuzz_teams_copy(parts.join("\0").as_bytes()), Ok(()));
    for junk in [&b""[..], b"\0\0\0", b"19:x\0chats/=.json\0{\"chat\":1}", b"\0..\0x\0/abs\0y"] {
        assert_eq!(crate::teams::fuzz_teams_copy(junk), Ok(()));
    }
}

/// Fix round 1: messages made in the same millisecond are never parted by a page's edge, so the
/// page before a window holds every one made before the window's first.
#[test]
fn equal_creation_times_never_straddle_a_page() {
    let (c, _) = copy();
    // 199 a minute apart, three in the same millisecond at 199 (where a full page would end),
    // then 100 more: 302 in one go.
    let mut all: Vec<ChatMessage> = (0..199).map(|i| msg(A, i)).collect();
    for id in ["199a", "199b", "199c"] {
        all.push(ChatMessage { id: id.into(), ..msg(A, 199) });
    }
    all.extend((200..300).map(|i| msg(A, i)));
    c.upsert(A, &all).unwrap();
    let mut want: Vec<i64> = (0..199).collect();
    want.extend([199, 199, 199]);
    want.extend(200..300);
    assert_eq!(all_back(&c, A), want, "each once, none hidden by the page's edge");
    // And when the history comes backwards, one at a time.
    let (c, _) = copy();
    for m in all.iter().rev() {
        c.upsert(A, std::slice::from_ref(m)).unwrap();
    }
    assert_eq!(all_back(&c, A), want);
}

/// Fix round 1: what's read back from the file is held to the rules Graph's reader holds what
/// comes in to: names on one line, every id usable.
#[test]
fn what_is_read_back_keeps_the_readers_rules() {
    use quatrelle_model::pim::teams::{Member, Preview, one_line};
    let (c, s) = copy();
    let long = "x".repeat(MAX_ID + 1);
    let member =
        |id: &str, name: &str| Member { user_id: id.into(), name: name.into(), kind: "aadUserConversationMember".into(), ..Member::default() };
    let topic = format!("Plan\n\u{202E}lacitirc {}", "y".repeat(200));
    let user = |id: &str, name: &str| Sender::User { id: id.into(), name: name.into(), identity_type: None, tenant_id: None };
    let mut r = ChatRecord::new(chat(A));
    r.chat.topic = Some(topic.clone());
    r.chat.members = vec![member("", "Nobody"), member("u-ok", "Ana\u{202E}\nLima"), member(&long, "Long")];
    r.chat.preview = Some(Preview { sender: user("u-ok", "Ana\u{7}"), text: "hi\n\tthere".into(), at: T0 });
    r.pages = vec![T0];
    r.page_keys = vec![0];
    let mut m = msg(A, 0);
    m.sender = user("u-ok", "Ana\u{202E} Lima");
    m.mentions = vec![Mention { id: 0, text: "Paul\nx".into(), user_id: Some(long.clone()) }];
    m.reactions = vec![Reaction { emoji: "👍".into(), users: vec!["u-ok".into(), String::new(), long.clone()] }];
    m.attachments = vec![
        Attachment::File { id: String::new(), name: "a".into(), url: "https://contoso.example/a".into(), size: None },
        Attachment::Hosted { id: long.clone() },
        Attachment::Hosted { id: "h1".into() },
        Attachment::Card,
    ];
    m.body.blocks[0].runs.push(Run { text: String::new(), hosted: Some(long.clone()), ..Default::default() });
    let nobody = ChatMessage { sender: user("", "Ghost"), ..msg(A, 1) };
    // Deleted ones too: only their id, times and sender stay, and the sender is held the same.
    let deleted = |i: i64, sender: Sender| ChatMessage { sender, deleted: Some(T0 + i * MINUTE), body: ChatText::default(), ..msg(A, i) };
    let page =
        serde_json::json!({ "chat": A, "messages": [m, nobody, deleted(2, user("u-ok", "Ana\u{202E}\nLima")), deleted(3, user(&long, "Long"))] });
    s.write_account_files(
        TEAMS_ACCOUNT,
        &[
            (format!("chats/{}.json", chat_hash(A)), serde_json::to_vec(&r).unwrap()),
            (format!("msgs/{}/0.json", chat_hash(A)), serde_json::to_vec(&page).unwrap()),
        ],
    )
    .unwrap();
    for back in [c.chat(A).unwrap(), c.chats().remove(0)] {
        assert_eq!(back.chat.topic, Some(one_line(&topic)));
        assert_eq!(back.chat.members, [member("u-ok", "Ana Lima")]);
        let p = back.chat.preview.unwrap();
        assert_eq!((p.sender, p.text), (user("u-ok", "Ana"), "hi there".to_string()));
    }
    let read = c.last_pages(A, 2);
    let [m, nobody, deleted_named, deleted_long] = read.as_slice() else { panic!("{read:?}") };
    assert_eq!(deleted_named.sender, user("u-ok", "Ana Lima"));
    assert_eq!(deleted_long.sender, Sender::Nobody);
    assert_eq!(m.sender, user("u-ok", "Ana Lima"));
    assert_eq!(m.mentions, [Mention { id: 0, text: "Paul x".into(), user_id: None }]);
    assert_eq!(m.reactions[0].users, ["u-ok"]);
    assert_eq!(m.attachments, [Attachment::Hosted { id: "h1".into() }, Attachment::Card]);
    assert_eq!(m.body.blocks[0].runs[1].hosted, None);
    assert_eq!(nobody.sender, Sender::Nobody, "a sender without a usable id is no one");
}

/// Fix round 1: a run planted in the file far past the cap is cut without an index of each of
/// its characters (the reading's peak memory stays near the file's size). Linux only, and run
/// alone (`-- --ignored`): it reads the process's peak memory.
#[test]
#[ignore = "reads the process's peak memory: run alone with --ignored"]
#[cfg(target_os = "linux")]
fn a_huge_run_in_the_file_is_cut_without_a_copy_per_character() {
    let peak = || {
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        let line = status.lines().find(|l| l.starts_with("VmHWM:")).unwrap();
        line.split_whitespace().nth(1).unwrap().parse::<u64>().unwrap() << 10
    };
    let (c, s) = copy();
    let mut r = c.chat(A).unwrap();
    r.pages = vec![T0];
    r.page_keys = vec![0];
    let mut m = msg(A, 0);
    m.body = ChatText { blocks: vec![Block { kind: BlockKind::Paragraph, runs: vec![Run { text: "é".repeat(8 << 20), ..Default::default() }] }] };
    let page = serde_json::to_vec(&serde_json::json!({ "chat": A, "messages": [m] })).unwrap();
    s.write_account_files(
        TEAMS_ACCOUNT,
        &[(format!("chats/{}.json", chat_hash(A)), serde_json::to_vec(&r).unwrap()), (format!("msgs/{}/0.json", chat_hash(A)), page)],
    )
    .unwrap();
    drop(m);
    std::fs::write("/proc/self/clear_refs", "5").unwrap();
    let before = peak();
    let read = c.last_pages(A, 2);
    let grown = peak().saturating_sub(before);
    assert!(serde_json::to_vec(&read[0].body).unwrap().len() <= MAX_BODY);
    // 8 Mi characters, 16 MiB: an index of them would be 64 MiB more.
    assert!(grown < 56 << 20, "the read grew by {} MiB", grown >> 20);
}

/// Fix round 1: what a deleted message showed goes before its page is written: a write that
/// fails leaves no picture of it behind (the next read of the deletion writes the page).
#[test]
fn a_deleted_messages_picture_goes_even_when_its_page_cant_be_written() {
    let s = Store::failing();
    let (c, _) = Copy::open(s.clone(), &who("u-1")).unwrap();
    c.put_chat(&ChatRecord::new(chat(A))).unwrap();
    let mut m = msg(A, 1);
    m.attachments = vec![Attachment::Hosted { id: "h1".into() }];
    c.upsert(A, std::slice::from_ref(&m)).unwrap();
    c.put_hosted(A, &m.id, "h1", b"the picture").unwrap();
    s.fail(None, Some(".json"));
    let gone = ChatMessage { deleted: Some(T0 + 5 * MINUTE), ..m.clone() };
    assert!(c.upsert(A, &[gone]).is_err());
    assert_eq!(c.hosted(A, &m.id, "h1"), None);
    s.fail(None, None);
    assert!(s.account_files(TEAMS_ACCOUNT, "img").unwrap().is_empty());
}

/// Fix round 1: a copy whose chats can't be listed is neither read nor deleted: opening it fails.
#[test]
fn a_copy_whose_chats_cant_be_listed_isnt_deleted() {
    let s = Store::failing();
    let record = serde_json::to_vec(&ChatRecord::new(chat(A))).unwrap();
    s.write_account_files(TEAMS_ACCOUNT, &[(format!("chats/{}.json", chat_hash(A)), record)]).unwrap();
    s.fail(Some("chats"), None);
    assert!(Copy::open(s.clone(), &who("u-1")).is_err());
    s.fail(None, None);
    assert_eq!(s.account_files(TEAMS_ACCOUNT, "chats").unwrap().len(), 1, "still there");
}
