// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The chats the user left: by the whole list's absence (twice running), by a read refused, and
//! back when the list has them again; never by an answer that says nothing of the chat.

use super::*;

/// The hour's pass, asked now, and the time it takes.
fn hourly(r: &mut Run) {
    r.w.limiter.prompt(Cadence::Hourly);
    r.turns(Duration::from_secs(15));
}

fn removed(r: &Run, from: usize) -> Vec<String> {
    r.events().into_iter().skip(from).filter_map(|e| if let model::Event::Removed { chat } = e { Some(chat) } else { None }).collect()
}

/// Final review I2 (a): a chat one whole list lacks may have moved in it while it was read (a
/// message came to a chat past the first page): left only when the next whole list lacks it too.
#[test]
fn a_chat_absent_from_one_whole_list_is_not_left() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 2), chat(OTHER, T0 + MINUTE, 2)]));
    r.turns(Duration::from_secs(20));
    let from = r.events().len();
    let gone = r.fake(|t| {
        let i = t.chats.iter().position(|c| c.id == OTHER).unwrap();
        t.chats.remove(i)
    });
    hourly(&mut r);
    assert!(!r.row(OTHER).left, "absent once");
    // Back, then absent once more: not twice running.
    r.fake(|t| t.chats.push(gone));
    hourly(&mut r);
    r.fake(|t| t.chats.retain(|c| c.id != OTHER));
    hourly(&mut r);
    assert!(!r.row(OTHER).left, "listed between");
    assert!(removed(&r, from).is_empty());
    hourly(&mut r);
    assert_eq!(removed(&r, from), [OTHER]);
    assert!(r.row(OTHER).left && !r.row(CHAT).left);
}

/// Final review I2 (b): a chat marked left that the whole list has again is read: served, it is
/// back; refused, it stays left, said once.
#[test]
fn a_chat_left_then_listed_again_is_read_and_back_within_the_hour() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 2), chat(OTHER, T0 + MINUTE, 2)]));
    let from = r.events().len();
    r.fake(|t| t.forbid_reads.push(CHAT.into()));
    r.post(CHAT, "u-ana", NOW + MINUTE, "Bye");
    r.turns(Duration::from_secs(11));
    assert!(r.row(CHAT).left);
    // Listed by the hour's pass, still refused: read, kept left, not said again.
    let reads = r.reads(CHAT);
    hourly(&mut r);
    assert_eq!(r.reads(CHAT), reads + 1, "read once by the whole list");
    assert!(r.row(CHAT).left);
    assert_eq!(removed(&r, from), [CHAT]);
    // Served again, nothing new in it: back at the next hour.
    r.fake(|t| t.forbid_reads.clear());
    hourly(&mut r);
    assert!(!r.row(CHAT).left && !r.record(CHAT).chat.left, "{:?}", r.row(CHAT));
    assert_eq!(removed(&r, from), [CHAT]);
}

/// Final review I2 (c): while the list itself is refused (an account-wide 403), a chat's read
/// refused says nothing of that chat: no read is asked, and none is marked.
#[test]
fn while_the_list_is_refused_no_chat_is_read_or_marked() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 2)]));
    r.fake(|t| t.forbid_list = true);
    r.turns(Duration::from_secs(11));
    let (from, reads) = (r.events().len(), r.reads(CHAT));
    r.fake(|t| t.forbid_reads.push(CHAT.into()));
    r.w.job(Job::Open { chat: Some(CHAT.into()) });
    r.turns(Duration::from_secs(70));
    assert_eq!(r.reads(CHAT), reads, "no read while the list is refused");
    assert!(removed(&r, from).is_empty());
    let row = r.row(CHAT);
    assert!(!row.left && !row.unreadable, "{row:?}");
}

/// Final review I2 (e): a list page cut at the most items taken isn't the list's end: the chats
/// past the cut aren't taken for gone.
#[test]
fn a_list_page_cut_short_leaves_no_chat() {
    let chats: Vec<FakeChat> = (0..210).map(|i| chat(&format!("19:m{i}@thread.v2"), T0 + i * MINUTE, 1)).collect();
    let mut r = run(teams_with(chats));
    r.turns_every(Duration::from_secs(300), STEP);
    let from = r.events().len();
    r.fake(|t| t.list_page = Some(250));
    hourly(&mut r);
    hourly(&mut r);
    assert!(removed(&r, from).is_empty(), "{:?}", removed(&r, from));
}
