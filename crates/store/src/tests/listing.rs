// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Tests: a list written a page at a time is kept as it stands part-way, and gone on with.

use super::*;

fn head(n: u32) -> FolderState {
    FolderState { validity: 3, uid_next: n + 1, ..Default::default() }
}

/// A list saved part-way reads back with what was handed over so far, and goes on to the same
/// list as one written in one go: small, across a page, and from a full page.
#[test]
fn a_list_saved_part_way_reads_back_and_goes_on() {
    let page = u32::try_from(PAGE_MESSAGES).unwrap();
    for (first, n) in [(3, 10), (page - 5, page + 9), (page, 2 * page + 1), (page + 7, page + 8)] {
        let s = Store::memory();
        let mut w = s.list_writer("a1", "INBOX");
        w.push(&s, (1..=first).map(|uid| msg(uid, uid % 2 == 0)).collect()).unwrap();
        assert_eq!(w.save(&s, &head(first)).unwrap(), (first.div_ceil(2), first));
        let back = s.load_folder("a1", "INBOX").unwrap();
        assert_eq!((back.messages.len(), back.uid_next), (first as usize, first + 1), "saved part-way");
        // It goes on in the same writer, saved again…
        w.push(&s, (first + 1..=first + 2).map(|uid| msg(uid, true)).collect()).unwrap();
        w.save(&s, &head(first + 2)).unwrap();
        drop(w);
        // …or, after a stop, in another: the messages under the number it goes on from stay,
        // those from there on (written, but not noted) are written again.
        let mut w = s.list_writer_from("a1", "INBOX", first + 1).unwrap();
        w.push(&s, (first + 1..=n).map(|uid| msg(uid, uid % 2 == 0)).collect()).unwrap();
        assert_eq!(w.finish(&s, &head(n)).unwrap(), (n.div_ceil(2), n));
        let resumed = s.load_folder("a1", "INBOX").unwrap();
        let whole = Store::memory();
        let mut w = whole.list_writer("a1", "INBOX");
        w.push(&whole, (1..=n).map(|uid| msg(uid, uid % 2 == 0)).collect()).unwrap();
        w.finish(&whole, &head(n)).unwrap();
        assert_eq!(resumed, whole.load_folder("a1", "INBOX").unwrap(), "{first} then to {n}");
    }
}

/// A message marked read between the stop and the going on stays read, and is counted so.
#[test]
fn going_on_keeps_the_marks_made_since() {
    let page = u32::try_from(PAGE_MESSAGES).unwrap();
    let s = Store::memory();
    let mut w = s.list_writer("a1", "INBOX");
    w.push(&s, (1..=page + 4).map(|uid| msg(uid, false)).collect()).unwrap();
    w.finish(&s, &head(page + 4)).unwrap();
    for uid in [7, page + 2] {
        let op =
            Op::SetFlag { keys: vec![MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid }], flag: Flag::Seen, on: true };
        s.apply_op("a1", &op).unwrap();
    }
    let mut w = s.list_writer_from("a1", "INBOX", page + 5).unwrap();
    w.push(&s, vec![msg(page + 5, false)]).unwrap();
    assert_eq!(w.finish(&s, &head(page + 5)).unwrap(), (page + 3, page + 5));
    let back = s.load_folder("a1", "INBOX").unwrap();
    assert!(back.find(7).unwrap().flags.seen && back.find(page + 2).unwrap().flags.seen && !back.find(8).unwrap().flags.seen);
    // A folder with no list yet goes on from nothing.
    let w = s.list_writer_from("a1", "Other", 1).unwrap();
    assert_eq!(w.finish(&s, &head(0)).unwrap(), (0, 0));
}
