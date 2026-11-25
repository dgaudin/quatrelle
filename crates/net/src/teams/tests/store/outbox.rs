// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The outbox kept in the copy: its records in order and read back, those under another number's
//! name or past the outbox's caps passed over (and never written), no more than 500 read, emptied
//! without the rest of the copy, and gone with a copy set aside for another identity; what goes
//! with a message (files and pictures, the pictures' bytes beside it) kept and read back the same.

use quatrelle_mime::TEAMS_RUNS;
use quatrelle_model::pim::teams::{
    FILE_MAX, LOCAL_MAX, MAX_MENTIONS, Mention, NAME_CHARS, OUTBOX_MAX, OutKind, OutPart, OutState, Outgoing, PATH_MAX, PICTURE_INLINE_MAX, SEND_MAX,
    Unsent,
};

use super::*;
use crate::teams::store::{PartRecord, Uploaded};

fn seqs(c: &Copy) -> Vec<u64> {
    c.ops().iter().map(|o| o.seq).collect()
}

#[test]
fn ops_are_kept_in_order_and_read_back() {
    let (c, s) = copy();
    assert!(c.ops().is_empty());
    assert_eq!(c.next_seq(), 1, "numbered from 1");
    let unknown = OpRecord { state: OpState::Unknown, tries: 2, ..op(2, B) };
    let forbidden = OutState::NotSent { reason: Unsent::Refused, code: Some("HTTP 403 Forbidden".into()) };
    let refused = OpRecord { state: OpState::NotSent { reason: Unsent::Refused, code: Some("HTTP 403 Forbidden".into()) }, after: None, ..op(3, A) };
    // Written out of order, in two chats: read back by number.
    for o in [&refused, &op(1, A), &unknown] {
        c.put_op(o).unwrap();
    }
    assert_eq!(c.ops(), [op(1, A), unknown.clone(), refused.clone()]);
    assert_eq!(c.next_seq(), 4);
    assert!(s.account_file(TEAMS_ACCOUNT, "outbox/00000000000000000003.json").unwrap().is_some(), "its number in 20 digits");
    // As the conversation shows them: one being checked is being sent.
    let shown: Vec<OutState> = c.ops().iter().map(|o| o.outgoing().state).collect();
    assert_eq!(shown, [OutState::Waiting, OutState::Sending, forbidden]);
    assert_eq!(
        unknown.outgoing(),
        Outgoing {
            seq: 2,
            local: "l-2.0".into(),
            chat: B.into(),
            body: text("Reply 2"),
            mentions: Vec::new(),
            what: OutKind::Send,
            state: OutState::Sending,
            parts: Vec::new()
        }
    );
    // Written again as its state moves: the same record.
    c.put_op(&OpRecord { state: OpState::Sending, ..op(1, A) }).unwrap();
    assert_eq!(seqs(&c), [1, 2, 3]);
    assert_eq!(c.ops()[0].state, OpState::Sending);
    assert_eq!(c.ops()[0].outgoing().state, OutState::Sending);
    c.remove_op(2).unwrap();
    assert_eq!(seqs(&c), [1, 3]);
    assert_eq!(c.next_seq(), 4);
    c.remove_op(2).unwrap();
    // Kept with the copy: opened again, the same.
    drop(c);
    let (c, opened) = Copy::open(s, &who("u-1")).unwrap();
    assert_eq!(opened, Opened::Same);
    assert_eq!(seqs(&c), [1, 3]);
}

#[test]
fn an_op_under_another_key_or_past_the_caps_is_passed_over() {
    let (c, s) = copy();
    let json = |o: &OpRecord| serde_json::to_vec(o).unwrap();
    let local = |l: &str| OpRecord { local: l.into(), ..op(1, A) };
    let code = |code: &str| OpRecord { state: OpState::NotSent { reason: Unsent::Refused, code: Some(code.into()) }, ..op(1, A) };
    let empty_runs = ChatText { blocks: vec![Block { kind: BlockKind::Paragraph, runs: vec![Run::default(); TEAMS_RUNS + 1] }] };
    // Past the caps: refused when written, passed over when read back.
    let broken = [
        ("number 0", OpRecord { seq: 0, ..op(1, A) }),
        ("the last number", OpRecord { seq: u64::MAX, ..op(1, A) }),
        ("a chat without an id", OpRecord { chat: String::new(), ..op(1, A) }),
        ("a chat id with a control character", OpRecord { chat: "19:a\u{7}b@thread.v2".into(), ..op(1, A) }),
        ("a chat id past MAX_ID", OpRecord { chat: "x".repeat(MAX_ID + 1), ..op(1, A) }),
        ("no local id", local("")),
        ("a local id past LOCAL_MAX", local(&"a".repeat(LOCAL_MAX + 1))),
        ("a capital in the local id", local("L-1")),
        ("a slash in the local id", local("a/b")),
        ("a space in the local id", local("a b")),
        ("an accent in the local id", local("é")),
        ("a body past SEND_MAX once written", OpRecord { body: text(&"<".repeat(SEND_MAX / 4)), ..op(1, A) }),
        ("a body of more runs than TEAMS_RUNS", OpRecord { body: empty_runs, ..op(1, A) }),
        ("ten tries", OpRecord { tries: 10, ..op(1, A) }),
        ("an empty code", code("")),
        ("a code past 64 characters", code(&"x".repeat(65))),
        ("a code on two lines", code("HTTP 403\nForbidden")),
        ("markup in a code", code("<b>403</b>")),
        ("an accent in a code", code("Refusé")),
    ];
    for (what, o) in &broken {
        assert!(c.put_op(o).is_err(), "{what}: refused when written");
        s.write_account_files(TEAMS_ACCOUNT, &[(op_key(o.seq), json(o))]).unwrap();
        assert!(c.ops().is_empty(), "{what}: passed over");
        c.clear_ops().unwrap();
    }
    // Planted: under the wrong name, damaged, too big.
    let mut padded = json(&op(1, A));
    padded.resize(padded.len() + (1 << 20), b' ');
    assert_eq!(serde_json::from_slice::<OpRecord>(&padded).unwrap(), op(1, A), "only its size is wrong");
    let planted = [
        ("under another number's key", op_key(1), json(&op(2, A))),
        ("a name not of 20 digits", "outbox/1.json".into(), json(&op(1, A))),
        ("a name past the numbers", "outbox/99999999999999999999.json".into(), json(&op(1, A))),
        ("a damaged record", op_key(1), b"{\"seq\":1,".to_vec()),
        ("a record past 1 MiB", op_key(1), padded),
        ("a chat's record", op_key(1), serde_json::to_vec(&ChatRecord::new(chat(A))).unwrap()),
    ];
    for (what, key, bytes) in planted {
        s.write_account_files(TEAMS_ACCOUNT, &[(key, bytes)]).unwrap();
        assert!(c.ops().is_empty(), "{what}: passed over");
        c.clear_ops().unwrap();
    }
    // At the caps: kept.
    let at_caps = OpRecord {
        local: "z".repeat(LOCAL_MAX),
        tries: 9,
        state: OpState::NotSent { reason: Unsent::Unconfirmed, code: Some("Az09 _.-".repeat(8)) },
        body: text(&"<".repeat(SEND_MAX / 4 - 2)),
        ..op(u64::MAX - 1, A)
    };
    c.put_op(&at_caps).unwrap();
    assert_eq!(c.ops(), [at_caps]);
    assert_eq!(c.next_seq(), u64::MAX);
}

/// Mention `id` of `user`, written as `text`.
fn mention(id: u32, user: Option<&str>, text: &str) -> Mention {
    Mention { id, text: text.into(), user_id: user.map(Into::into) }
}

/// A body of one run, naming mention `id`.
fn naming(id: u32, words: &str) -> ChatText {
    ChatText { blocks: vec![Block { kind: BlockKind::Paragraph, runs: vec![Run { text: words.into(), mention: Some(id), ..Run::default() }] }] }
}

/// An edit of message `m` (made at `created`) to `body`, its mentions as given.
fn edit(seq: u64, body: ChatText, mentions: Vec<Mention>) -> OpRecord {
    OpRecord { body, mentions, what: OutKind::Edit { message: "m-1".into(), created: T0 }, ..op(seq, A) }
}

fn delete(seq: u64) -> OpRecord {
    OpRecord { body: ChatText::default(), what: OutKind::Delete { message: "m-1".into(), created: T0 }, ..op(seq, A) }
}

fn react(seq: u64, emoji: &str) -> OpRecord {
    OpRecord { body: ChatText::default(), what: OutKind::React { message: "m-1".into(), created: T0, emoji: emoji.into(), on: true }, ..op(seq, A) }
}

#[test]
fn records_of_delivery_two_read_back_as_sends() {
    let (c, s) = copy();
    // As delivery 2 wrote it: no `what`, no `mentions`.
    let raw = r#"{"seq":1,"local":"l-1.0","chat":"19:a1b2@thread.v2","body":{"blocks":[{"kind":{"block":"paragraph"},"runs":[{"text":"Old"}]}]},
        "state":{"state":"waiting"},"tries":0,"after":1790812800000}"#;
    s.write_account_files(TEAMS_ACCOUNT, &[(op_key(1), raw.as_bytes().to_vec())]).unwrap();
    let ops = c.ops();
    assert_eq!(ops.len(), 1, "read back");
    assert_eq!((&ops[0].what, ops[0].mentions.len()), (&OutKind::Send, 0));
    assert_eq!(ops[0].body, text("Old"));
    assert_eq!(ops[0].outgoing().what, OutKind::Send);
}

#[test]
fn change_records_are_kept_and_read_back() {
    let (c, _) = copy();
    let ana = mention(0, Some("u-ana"), "Ana Exemple");
    let all = [edit(1, naming(0, "Ana Exemple"), vec![ana.clone()]), delete(2), react(3, "\u{1F44D}")];
    for o in &all {
        c.put_op(o).unwrap();
    }
    assert_eq!(c.ops(), all);
    assert_eq!(c.ops()[0].outgoing().mentions, [ana]);
    assert_eq!(c.ops()[2].outgoing().what, all[2].what);
}

#[test]
fn a_change_record_past_its_rules_is_passed_over() {
    let (c, s) = copy();
    let ana = || mention(0, Some("u-ana"), "Ana Exemple");
    let kind = |what: OutKind| OpRecord { what, ..delete(1) };
    let at = |created: i64| (OutKind::Delete { message: "m-1".into(), created }, created);
    let many: Vec<Mention> = (0..=MAX_MENTIONS as u32).map(|i| mention(i, Some("u-ana"), "Ana")).collect();
    let name_line = "Ana\nLima";
    let broken = [
        ("a reaction of a name, not an emoji", react(1, "like")),
        ("a reaction with a bidi mark", react(1, "\u{1F44D}\u{202E}")),
        ("a reaction of no emoji", react(1, "")),
        ("a deletion with a body", OpRecord { body: text("x"), ..delete(1) }),
        ("a deletion with mentions", OpRecord { mentions: vec![ana()], ..delete(1) }),
        ("a reaction with a body", OpRecord { body: text("x"), ..react(1, "\u{1F44D}") }),
        ("a reaction with mentions", OpRecord { mentions: vec![ana()], ..react(1, "\u{1F44D}") }),
        ("an edit with no body", edit(1, ChatText::default(), Vec::new())),
        ("an edit of a body with no words", edit(1, ChatText { blocks: vec![Block::default()] }, Vec::new())),
        ("a message id of none", kind(OutKind::Delete { message: String::new(), created: T0 })),
        ("a message id with a slash", kind(OutKind::Delete { message: "a/b".into(), created: T0 })),
        ("a message id of two dots", kind(OutKind::Delete { message: "..".into(), created: T0 })),
        ("a message id with a control character", kind(OutKind::Delete { message: "a\u{7}b".into(), created: T0 })),
        ("a message id past MAX_ID", kind(OutKind::Delete { message: "x".repeat(MAX_ID + 1), created: T0 })),
        ("a deletion created before 1970", kind(at(-1).0)),
        ("a deletion created past year 9999", kind(OutKind::Delete { message: "m-1".into(), created: i64::MAX })),
        (
            "an edit created past year 9999",
            OpRecord { what: OutKind::Edit { message: "m-1".into(), created: 253_402_300_800_000 }, ..edit(1, text("x"), Vec::new()) },
        ),
        (
            "a reaction created at i64::MIN",
            OpRecord { what: OutKind::React { message: "m-1".into(), created: i64::MIN, emoji: "\u{1F44D}".into(), on: true }, ..react(1, "x") },
        ),
        ("a mention without a user", edit(1, naming(0, "Ana"), vec![mention(0, None, "Ana")])),
        ("a mention of a user without a usable id", edit(1, naming(0, "Ana"), vec![mention(0, Some(""), "Ana")])),
        ("a mention of no text", edit(1, text("x"), vec![mention(0, Some("u-ana"), "")])),
        ("a mention past NAME_CHARS", edit(1, text("x"), vec![mention(0, Some("u-ana"), &"a".repeat(NAME_CHARS + 1))])),
        ("a mention on two lines", edit(1, text("x"), vec![mention(0, Some("u-ana"), name_line)])),
        ("a mention with a bidi mark", edit(1, text("x"), vec![mention(0, Some("u-ana"), "Ana\u{202E}")])),
        ("two mentions of one id", edit(1, text("x"), vec![ana(), mention(0, Some("u-bo"), "Bo")])),
        ("a run naming mention 7", edit(1, naming(7, "Ana"), vec![ana()])),
        ("a send naming a mention of no list", OpRecord { body: naming(0, "Ana"), ..op(1, A) }),
        ("51 mentions", edit(1, text("x"), many)),
        ("an edit past SEND_MAX once written", edit(1, text(&"<".repeat(SEND_MAX / 4)), Vec::new())),
        ("a time to look after before 1970", OpRecord { after: Some(-1), ..op(1, A) }),
        ("a time to look after past year 9999", OpRecord { after: Some(253_402_300_800_000), ..op(1, A) }),
        ("a time to look after of i64::MAX", OpRecord { after: Some(i64::MAX), ..op(1, A) }),
        ("a deletion in a chat without an id", OpRecord { chat: String::new(), ..delete(1) }),
    ];
    for (what, o) in &broken {
        assert!(c.put_op(o).is_err(), "{what}: refused when written");
        s.write_account_files(TEAMS_ACCOUNT, &[(op_key(o.seq), serde_json::to_vec(o).unwrap())]).unwrap();
        assert!(c.ops().is_empty(), "{what}: passed over");
        c.clear_ops().unwrap();
    }
    // At the caps: kept.
    let ok = [
        OpRecord { after: Some(0), ..op(1, A) },
        OpRecord { after: Some(253_402_300_799_999), ..op(1, A) },
        edit(1, naming(49, "Ana"), (0..MAX_MENTIONS as u32).map(|i| mention(i, Some("u-ana"), &"a".repeat(NAME_CHARS))).collect()),
        OpRecord { what: OutKind::Delete { message: "m-1".into(), created: 253_402_300_799_999 }, ..delete(1) },
        OpRecord { what: OutKind::React { message: "m-1".into(), created: 0, emoji: "\u{2764}\u{FE0F}".into(), on: false }, ..react(1, "x") },
    ];
    for o in ok {
        c.put_op(&o).unwrap();
        assert_eq!(c.ops(), [o], "kept at its caps");
    }
}

#[test]
fn message_at_finds_a_message_by_its_time() {
    let (c, _) = copy();
    let all: Vec<ChatMessage> = (0..1000).map(|i| msg(A, i)).collect();
    c.upsert(A, &all).unwrap();
    assert!(c.chat(A).unwrap().pages.len() >= 10, "ten pages at least");
    let want = &all[299];
    assert_eq!(c.message_at(A, &want.id, want.created).as_ref(), Some(want));
    // The first and the last of the chat, and the first of a page.
    for m in [&all[0], &all[999]].into_iter().chain(c.chat(A).unwrap().pages.get(3).and_then(|p| all.iter().find(|m| m.created == *p))) {
        assert_eq!(c.message_at(A, &m.id, m.created).as_ref(), Some(m));
    }
    // The id of another time, a day off either way, another chat, another id.
    assert_eq!(c.message_at(A, &want.id, want.created + 24 * 60 * MINUTE), None);
    assert_eq!(c.message_at(A, &want.id, want.created - 24 * 60 * MINUTE), None);
    assert_eq!(c.message_at(B, &want.id, want.created), None);
    assert_eq!(c.message_at(A, "nothing", want.created), None);
}

#[test]
fn no_more_than_500_ops_are_read_back() {
    let (c, _) = copy();
    for seq in 1..=520 {
        c.put_op(&op(seq, if seq % 2 == 0 { A } else { B })).unwrap();
    }
    assert_eq!(OUTBOX_MAX, 500);
    assert_eq!(seqs(&c), (1..=500).collect::<Vec<u64>>(), "the first ones, by number");
    assert_eq!(c.next_seq(), 521, "past every one kept");
}

#[test]
fn clear_ops_keeps_the_rest_of_the_copy() {
    let (c, s) = copy();
    c.put_chat(&ChatRecord::new(chat(B))).unwrap();
    c.upsert(A, &[msg(A, 0), msg(A, 1)]).unwrap();
    for seq in 1..=3 {
        c.put_op(&op(seq, A)).unwrap();
    }
    c.clear_ops().unwrap();
    assert!(c.ops().is_empty());
    assert!(s.account_files(TEAMS_ACCOUNT, "outbox").unwrap().is_empty());
    // As briefed, the numbers start again from 1 (the engine tells its notices apart by local id).
    assert_eq!(c.next_seq(), 1);
    assert_eq!(c.chats().len(), 2);
    assert_eq!(numbers(&c.last_pages(A, 2)), [0, 1]);
    c.clear_ops().unwrap();
    drop(c);
    assert_eq!(Copy::open(s, &who("u-1")).unwrap().1, Opened::Same, "still the user's copy");
}

#[test]
fn another_identitys_outbox_goes_with_its_copy() {
    let t = temp("outbox-identity");
    let s = Store::open_packed(t.join("app"), t.join("files")).unwrap();
    let (c, _) = Copy::open(s.clone(), &who("u-1")).unwrap();
    c.put_chat(&ChatRecord::new(chat(A))).unwrap();
    c.put_op(&op(1, A)).unwrap();
    c.put_op(&op(2, A)).unwrap();
    assert_eq!(seqs(&c), [1, 2]);
    drop(c);
    let (c, opened) = Copy::open(s.clone(), &who("u-2")).unwrap();
    assert!(matches!(&opened, Opened::SetAside(p) if p.exists()), "{opened:?}");
    assert!(c.ops().is_empty(), "nothing of the other's goes out");
    assert_eq!(c.next_seq(), 1);
    drop((c, s));
    let _ = std::fs::remove_dir_all(&t);
}

/// Any state of a message going out.
fn any_op_state() -> impl proptest::strategy::Strategy<Value = OpState> {
    use proptest::prelude::*;
    let reason = proptest::sample::select(vec![Unsent::Refused, Unsent::TooLong, Unsent::TooMany, Unsent::Left, Unsent::Unconfirmed]);
    prop_oneof![
        Just(OpState::Waiting),
        Just(OpState::Sending),
        Just(OpState::Unknown),
        (reason, proptest::option::of("[A-Za-z0-9 _.-]{1,64}")).prop_map(|(reason, code)| OpState::NotSent { reason, code }),
    ]
}

/// Any kind of item of the outbox, its message and time within what the outbox keeps.
fn any_out_kind() -> impl proptest::strategy::Strategy<Value = OutKind> {
    use proptest::prelude::*;
    let message = "[a-z0-9-]{1,20}";
    let created = 0..=253_402_300_799_999i64;
    prop_oneof![
        Just(OutKind::Send),
        (message, created.clone()).prop_map(|(message, created)| OutKind::Edit { message, created }),
        (message, created.clone()).prop_map(|(message, created)| OutKind::Delete { message, created }),
        (message, created, proptest::sample::select(vec!["\u{1F44D}", "\u{2764}\u{FE0F}", "\u{1F602}"]), any::<bool>())
            .prop_map(|(message, created, emoji, on)| OutKind::React { message, created, emoji: emoji.into(), on }),
    ]
}

/// Any part of a message within what the outbox keeps: a file uploaded or not, a picture. Names
/// begin with `x`, never one Windows keeps for itself (`con`, `nul`…), so each is its own drive
/// name.
fn any_part() -> impl proptest::strategy::Strategy<Value = PartRecord> {
    use proptest::prelude::*;
    let up = proptest::option::of(("[A-Z0-9]{1,12}", any::<u128>(), "[a-z]{1,8}", any::<bool>()).prop_map(|(item, g, page, shared)| Uploaded {
        item,
        guid: guid(g),
        url: format!("https://contoso-my.sharepoint.com/personal/riley/x{page}.pdf"),
        name: format!("x{page}.pdf"),
        shared,
    }));
    prop_oneof![
        ("[a-z]{1,8}", 1..=FILE_MAX, -253_402_300_799_999..=253_402_300_799_999i64, up).prop_map(|(name, size, modified, up)| PartRecord::File {
            path: absolute(&format!("x{name}.pdf")),
            name: format!("x{name}.pdf"),
            size,
            modified,
            up
        }),
        ("[a-z]{1,8}", proptest::sample::select(vec!["image/png", "image/jpeg"]), 1..=PICTURE_INLINE_MAX as u32)
            .prop_map(|(name, mime, len)| PartRecord::Picture { name: format!("x{name}.png"), mime: mime.into(), len }),
    ]
}

proptest::proptest! {
    /// Any record reads back from its JSON as it was; one within the caps, from the copy too.
    #[test]
    fn op_records_round_trip(
        seq in 1..u64::MAX,
        local in "[0-9a-z.-]{1,64}",
        chat in "\\PC{1,40}",
        words in proptest::collection::vec(("\\PC{0,20}", proptest::prelude::any::<bool>(), proptest::prelude::any::<bool>()), 0..6),
        state in any_op_state(),
        tries in 0u8..=9,
        after in proptest::option::of(0..=253_402_300_799_999i64),
        anything in proptest::prelude::any::<String>(),
        any_seq in proptest::prelude::any::<u64>(),
        any_tries in proptest::prelude::any::<u8>(),
        what in any_out_kind(),
        people in proptest::collection::vec(("[a-z]{1,8}", "[a-z]{1,8}"), 0..4),
        parts in proptest::collection::vec(any_part(), 0..=3),
    ) {
        let mentions = people.into_iter().enumerate().map(|(i, (user, name))| Mention { id: i as u32, text: name, user_id: Some(user) }).collect();
        let mut runs: Vec<Run> = words.into_iter().map(|(text, bold, italic)| Run { text, bold, italic, ..Run::default() }).collect();
        // A change within the rules: a deletion or a reaction has no words, an edit has some.
        let (mentions, runs) = match &what {
            OutKind::Send => (mentions, runs),
            OutKind::Edit { .. } => (mentions, { runs.push(Run { text: "x".into(), ..Run::default() }); runs }),
            OutKind::Delete { .. } | OutKind::React { .. } => (Vec::new(), Vec::new()),
        };
        let blocks = if runs.is_empty() && !matches!(what, OutKind::Send | OutKind::Edit { .. }) { Vec::new() } else { vec![Block { kind: BlockKind::Paragraph, runs }] };
        // Parts go with a message sent only.
        let parts = if what == OutKind::Send { parts } else { Vec::new() };
        let op = OpRecord { seq, local, chat, body: ChatText { blocks }, mentions, what, state, tries, after, parts };
        let loose = OpRecord { seq: any_seq, local: anything.clone(), chat: anything, tries: any_tries, ..op.clone() };
        proptest::prop_assert_eq!(serde_json::from_slice::<OpRecord>(&serde_json::to_vec(&loose).unwrap()).unwrap(), loose);
        let (c, _) = Copy::open(Store::memory(), &who("u-1")).unwrap();
        c.put_op(&op).unwrap();
        proptest::prop_assert_eq!(c.ops(), vec![op.clone()]);
        proptest::prop_assert_eq!(c.next_seq(), seq + 1);
    }
}

/// Fix round 1: an outbox that can't be listed (a folder it can't read but can write in) gives
/// the next message a number `put_op` refuses, never one that writes over a message waiting.
#[test]
#[cfg(unix)]
fn an_outbox_that_cant_be_listed_numbers_nothing_over_a_record() {
    use std::os::unix::fs::PermissionsExt;
    let t = temp("outbox-unlisted");
    let s = Store::open(t.join("app")).unwrap();
    let (c, _) = Copy::open(s.clone(), &who("u-1")).unwrap();
    c.put_op(&op(1, A)).unwrap();
    let dir = s.account_path(TEAMS_ACCOUNT).join("outbox");
    let mode = |m: u32| std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(m)).unwrap();
    mode(0o300);
    if std::fs::read_dir(&dir).is_ok() {
        // Run as root: nothing is refused to it.
        mode(0o700);
        let _ = std::fs::remove_dir_all(&t);
        return;
    }
    let next = OpRecord { seq: c.next_seq(), local: "l-9.0".into(), ..op(1, A) };
    let written = c.put_op(&next);
    mode(0o700);
    assert!(written.is_err(), "numbered {}: refused, not written over", next.seq);
    assert_eq!(c.ops(), [op(1, A)], "the message waiting is still the user's");
    let _ = std::fs::remove_dir_all(&t);
}

/// A path of the temporary folder: an absolute one, wherever the tests run.
fn absolute(name: &str) -> String {
    std::env::temp_dir().join(name).to_string_lossy().into_owned()
}

/// An object id made of `n`'s hexadecimal digits.
fn guid(n: u128) -> String {
    let h = format!("{n:032x}");
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

/// A file of 4,000 bytes, uploaded when `up` says so.
fn file_part(up: Option<Uploaded>) -> PartRecord {
    PartRecord::File { path: absolute("report.pdf"), name: "report.pdf".into(), size: 4_000, modified: T0, up }
}

/// What OneDrive made of the file, shared with the chat or not.
fn uploaded(shared: bool) -> Uploaded {
    Uploaded {
        item: "01ITEM0001".into(),
        guid: "5b8c7d0e-0001-4000-8000-000000000001".into(),
        url: "https://contoso-my.sharepoint.com/personal/riley/Microsoft%20Teams%20Chat%20Files/report%201.pdf".into(),
        name: "report 1.pdf".into(),
        shared,
    }
}

fn picture_part(len: u32) -> PartRecord {
    PartRecord::Picture { name: "paste.png".into(), mime: "image/png".into(), len }
}

fn with_parts(seq: u64, parts: Vec<PartRecord>) -> OpRecord {
    OpRecord { parts, ..op(seq, A) }
}

#[test]
fn part_records_are_kept_and_read_back() {
    let (c, s) = copy();
    let going = with_parts(1, vec![file_part(None), file_part(Some(uploaded(true))), picture_part(1_000)]);
    c.put_op(&going).unwrap();
    assert_eq!(c.ops(), std::slice::from_ref(&going));
    // As the conversation shows them: a file's bytes all gone up once it is uploaded.
    let file = |sent: u64| OutPart { name: "report.pdf".into(), size: 4_000, picture: false, sent };
    assert_eq!(going.outgoing().parts, [file(0), file(4_000), OutPart { name: "paste.png".into(), size: 1_000, picture: true, sent: 0 }]);
    // A picture's bytes, beside the record, by its place among the parts.
    c.put_picture(1, 2, &[9u8; 1_000]).unwrap();
    assert_eq!(c.picture(1, 2), Some(vec![9u8; 1_000]));
    assert_eq!((c.picture(1, 0), c.picture(2, 2)), (None, None));
    assert!(s.account_file(TEAMS_ACCOUNT, "outbox-parts/00000000000000000001/2").unwrap().is_some(), "its number in 20 digits");
    // Never past 4 MiB, nor empty: refused when written, passed over when read back.
    let past = vec![0u8; PICTURE_INLINE_MAX as usize + 1];
    assert!(c.put_picture(1, 3, &past).is_err());
    assert!(c.put_picture(1, 3, &[]).is_err());
    s.write_account_files(TEAMS_ACCOUNT, &[("outbox-parts/00000000000000000001/3".into(), past)]).unwrap();
    assert_eq!(c.picture(1, 3), None);
    // At the caps: ten parts, a path of 4,096 bytes, 100 MiB, a picture of 4 MiB, times a date holds.
    let long = absolute(&"a".repeat(PATH_MAX));
    let long = format!("{}{}", &long[..PATH_MAX - 4], ".pdf");
    let at_caps = with_parts(
        2,
        vec![
            PartRecord::File { path: long, name: "a.pdf".into(), size: FILE_MAX, modified: -253_402_300_799_999, up: None },
            PartRecord::File { path: absolute("b.pdf"), name: "b.pdf".into(), size: 1, modified: 253_402_300_799_999, up: Some(uploaded(false)) },
            picture_part(PICTURE_INLINE_MAX as u32),
        ]
        .into_iter()
        .chain((0..7).map(|_| picture_part(1)))
        .collect(),
    );
    c.put_op(&at_caps).unwrap();
    assert_eq!(c.ops(), [going, at_caps]);
    c.put_picture(2, 2, &vec![0u8; PICTURE_INLINE_MAX as usize]).unwrap();
    assert_eq!(c.picture(2, 2).map(|b| b.len()), Some(PICTURE_INLINE_MAX as usize));
}

/// H16: a record whose parts break their rules is never written, and passed over when read back.
#[test]
fn part_records_past_their_rules_are_passed_over() {
    let (c, s) = copy();
    let file = |path: &str, name: &str, size: u64| PartRecord::File { path: path.into(), name: name.into(), size, modified: T0, up: None };
    let up = |change: fn(&mut Uploaded)| {
        let mut u = uploaded(true);
        change(&mut u);
        with_parts(1, vec![file_part(Some(u))])
    };
    let picture = |mime: &str, len: u32| with_parts(1, vec![PartRecord::Picture { name: "paste.png".into(), mime: mime.into(), len }]);
    let report = absolute("report.pdf");
    let broken = [
        ("eleven parts", with_parts(1, vec![picture_part(1); 11])),
        ("parts on an edit", OpRecord { parts: vec![file_part(None)], ..edit(1, text("x"), Vec::new()) }),
        ("parts on a reaction", OpRecord { parts: vec![picture_part(1)], ..react(1, "\u{1F44D}") }),
        ("a relative path", with_parts(1, vec![file("report.pdf", "report.pdf", 1)])),
        ("a path with a line break", with_parts(1, vec![file(&absolute("a\nb.pdf"), "report.pdf", 1)])),
        ("a path past PATH_MAX", with_parts(1, vec![file(&absolute(&"a".repeat(PATH_MAX)), "report.pdf", 1)])),
        ("a name that climbs", with_parts(1, vec![file(&report, "../x", 1)])),
        ("a name of none", with_parts(1, vec![file(&report, "", 1)])),
        ("a name OneDrive refuses", with_parts(1, vec![file(&report, "a#b.pdf", 1)])),
        ("a size of 0", with_parts(1, vec![file(&report, "report.pdf", 0)])),
        ("a size past FILE_MAX", with_parts(1, vec![file(&report, "report.pdf", FILE_MAX + 1)])),
        (
            "a time past what a date holds",
            with_parts(1, vec![PartRecord::File { path: report.clone(), name: "report.pdf".into(), size: 1, modified: i64::MAX, up: None }]),
        ),
        ("an uploaded file of GUID x", up(|u| u.guid = "x".into())),
        ("an uploaded file at an http address", up(|u| u.url = "http://contoso-my.sharepoint.com/x.pdf".into())),
        ("an uploaded file at an address with a space", up(|u| u.url = "https://contoso-my.sharepoint.com/a b.pdf".into())),
        ("an uploaded file at an address past 8 KiB", up(|u| u.url = format!("https://contoso-my.sharepoint.com/{}", "a".repeat(8 << 10)))),
        // H16: an address elsewhere than Microsoft's file hosts, as `transfer_url` says them.
        ("an uploaded file at an address elsewhere", up(|u| u.url = "https://evil.example/x.pdf".into())),
        ("an uploaded file at a look-alike address", up(|u| u.url = "https://sharepoint.com.evil.example/x".into())),
        ("an uploaded file at a host glued to the suffix", up(|u| u.url = "https://evilsharepoint.com/x.pdf".into())),
        ("an uploaded file at an IP address", up(|u| u.url = "https://10.0.0.1/x.pdf".into())),
        ("an uploaded file on another port", up(|u| u.url = "https://contoso-my.sharepoint.com:8443/x.pdf".into())),
        ("an uploaded file with user information", up(|u| u.url = "https://user@contoso-my.sharepoint.com/x.pdf".into())),
        ("an uploaded item a/b", up(|u| u.item = "a/b".into())),
        ("an uploaded item of none", up(|u| u.item = String::new())),
        ("an uploaded name that climbs", up(|u| u.name = "../x".into())),
        ("a picture of SVG", picture("image/svg+xml", 1)),
        ("a picture of 0 bytes", picture("image/png", 0)),
        ("a picture past PICTURE_INLINE_MAX", picture("image/jpeg", PICTURE_INLINE_MAX as u32 + 1)),
        ("a picture's name that climbs", with_parts(1, vec![PartRecord::Picture { name: "../x.png".into(), mime: "image/png".into(), len: 1 }])),
        ("a body and its places past SEND_MAX", OpRecord { body: text(&"<".repeat(SEND_MAX / 4 - 2)), ..with_parts(1, vec![file_part(None)]) }),
    ];
    for (what, o) in &broken {
        assert!(c.put_op(o).is_err(), "{what}: refused when written");
        s.write_account_files(TEAMS_ACCOUNT, &[(op_key(o.seq), serde_json::to_vec(o).unwrap())]).unwrap();
        assert!(c.ops().is_empty(), "{what}: passed over");
        c.clear_ops().unwrap();
    }
}

#[test]
fn records_of_delivery_three_read_back_without_parts() {
    let (c, s) = copy();
    // As delivery 3 wrote it: no `parts`.
    let raw = r#"{"seq":1,"local":"l-1.0","chat":"19:a1b2@thread.v2","body":{"blocks":[{"kind":{"block":"paragraph"},"runs":[{"text":"Old"}]}]},
        "mentions":[],"what":{"what":"send"},"state":{"state":"waiting"},"tries":0,"after":1790812800000}"#;
    s.write_account_files(TEAMS_ACCOUNT, &[(op_key(1), raw.as_bytes().to_vec())]).unwrap();
    let ops = c.ops();
    assert_eq!(ops.len(), 1, "read back");
    assert!(ops[0].parts.is_empty());
    assert!(ops[0].outgoing().parts.is_empty());
    // And a file uploaded before its share is read back with what OneDrive made of it.
    let raw = serde_json::to_vec(&with_parts(2, vec![file_part(Some(uploaded(false)))])).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(v["parts"][0]["part"], "file");
    assert_eq!(v["parts"][0]["up"]["shared"], false);
}

#[test]
fn removing_a_record_takes_its_pictures() {
    let (c, s) = copy();
    for seq in 1..=3 {
        c.put_op(&with_parts(seq, vec![picture_part(10), file_part(None), picture_part(10)])).unwrap();
        c.put_picture(seq, 0, &[1u8; 10]).unwrap();
        c.put_picture(seq, 2, &[2u8; 10]).unwrap();
    }
    c.remove_op(1).unwrap();
    assert_eq!((c.picture(1, 0), c.picture(1, 2)), (None, None));
    assert!(s.account_files(TEAMS_ACCOUNT, "outbox-parts/00000000000000000001").unwrap_or_default().is_empty());
    assert_eq!(c.picture(2, 2), Some(vec![2u8; 10]), "another's stay");
    // Their pictures alone, the records kept.
    c.remove_pictures(2).unwrap();
    assert_eq!((c.picture(2, 0), seqs(&c)), (None, vec![2, 3]));
    // The outbox emptied: every picture with it.
    c.clear_ops().unwrap();
    assert_eq!(c.picture(3, 0), None);
    assert!(s.account_files(TEAMS_ACCOUNT, "outbox-parts").unwrap_or_default().is_empty());
}
