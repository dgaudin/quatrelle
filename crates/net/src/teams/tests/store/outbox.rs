// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The outbox kept in the copy: its records in order and read back, those under another number's
//! name or past the outbox's caps passed over (and never written), no more than 500 read, emptied
//! without the rest of the copy, and gone with a copy set aside for another identity.

use quatrelle_mime::TEAMS_RUNS;
use quatrelle_model::pim::teams::{LOCAL_MAX, OUTBOX_MAX, OutKind, OutState, Outgoing, SEND_MAX, Unsent};

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
        after in proptest::option::of(proptest::prelude::any::<i64>()),
        anything in proptest::prelude::any::<String>(),
        any_seq in proptest::prelude::any::<u64>(),
        any_tries in proptest::prelude::any::<u8>(),
    ) {
        let runs = words.into_iter().map(|(text, bold, italic)| Run { text, bold, italic, ..Run::default() }).collect();
        let op = OpRecord { seq, local, chat, body: ChatText { blocks: vec![Block { kind: BlockKind::Paragraph, runs }] }, state, tries, after };
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
