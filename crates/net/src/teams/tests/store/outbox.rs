// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The outbox kept in the copy: its records in order and read back, those under another number's
//! name or past the outbox's caps passed over (and never written), no more than 500 read, emptied
//! without the rest of the copy, and gone with a copy set aside for another identity.

use quatrelle_mime::TEAMS_RUNS;
use quatrelle_model::pim::teams::{LOCAL_MAX, MAX_MENTIONS, Mention, NAME_CHARS, OUTBOX_MAX, OutKind, OutState, Outgoing, SEND_MAX, Unsent};

use super::*;

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
            state: OutState::Sending
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
    let ana = mention(0, Some("u-ana"), "Ana Lima");
    let all = [edit(1, naming(0, "Ana Lima"), vec![ana.clone()]), delete(2), react(3, "\u{1F44D}")];
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
    let ana = || mention(0, Some("u-ana"), "Ana Lima");
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
        let op = OpRecord { seq, local, chat, body: ChatText { blocks }, mentions, what, state, tries, after };
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
