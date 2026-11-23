// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The Outbox's files, when they can't be changed.

use super::*;

/// A message waiting in the Outbox.
fn waiting(id: u64) -> OutboxItem {
    OutboxItem { id, from: "zoe@studio.example".into(), recipients: vec!["ana@contoso.example".into()], ..Default::default() }
}

/// The names of the files in the account's Outbox.
fn files(s: &Store) -> Vec<String> {
    s.files.list(&s.outbox_dir("a1")).into_iter().map(|(n, _)| n).collect()
}

/// A message taken out of the Outbox whose files can't be removed isn't listed again (it would be
/// sent again); what's left goes once it can. A new message of the id isn't hidden by the mark.
#[test]
fn an_outbox_message_taken_out_stays_out() {
    let s = Store::failing();
    let item = waiting(7);
    // Both files held: the mark keeps it out. One: the other going is enough. The mark says to
    // remove what's left.
    for held in ["outbox/7.json|outbox/7.eml", "outbox/7.json", "outbox/7.eml"] {
        s.outbox_add("a1", &item, b"x").unwrap();
        s.fail(None, Some(held));
        s.outbox_remove("a1", 7).unwrap();
        assert!(s.outbox("a1").is_empty(), "{held}: listed again");
        s.outbox_sweep("a1");
        assert!(s.outbox("a1").is_empty(), "{held}: listed again once swept");
        s.fail(None, None);
        assert!(files(&s).iter().any(|p| p.ends_with("7.gone")), "{held}: {:?}", files(&s));
        s.outbox_sweep("a1");
        assert!(files(&s).is_empty(), "{held}: {:?}", files(&s));
    }
    // Nothing can be written there: still in the Outbox, and said.
    s.outbox_add("a1", &item, b"x").unwrap();
    s.fail(None, Some("outbox/7.json|outbox/7.eml|outbox/7.gone"));
    assert!(s.outbox_remove("a1", 7).is_err());
    assert_eq!(s.outbox("a1"), vec![item.clone()]);
    // Marked, then queued again with the same id: listed.
    s.fail(None, Some("outbox/7.json|outbox/7.eml"));
    s.outbox_remove("a1", 7).unwrap();
    s.fail(None, None);
    s.outbox_add("a1", &item, b"y").unwrap();
    assert_eq!(s.outbox("a1"), vec![item]);
    assert_eq!(s.outbox_raw("a1", 7).unwrap(), b"y");
}

/// A removal that answers as if done and leaves the file (a data file that couldn't look whether
/// it was there): the mark, written first, keeps the message out, and stays as long as one of its
/// files does. With no mark either, the message is said to be still there; and a mark that stays
/// the same way would hide a message queued again with the id, which isn't queued then.
#[test]
fn a_removal_that_says_done_and_leaves_the_files_keeps_the_message_out() {
    let s = Store::failing();
    let item = waiting(7);
    s.outbox_add("a1", &item, b"x").unwrap();
    s.fail_quietly(Some("outbox/7.json|outbox/7.eml"));
    s.outbox_remove("a1", 7).unwrap();
    assert!(s.outbox("a1").is_empty(), "listed again: {:?}", files(&s));
    s.outbox_sweep("a1");
    assert!(s.outbox("a1").is_empty(), "listed again once swept: {:?}", files(&s));
    s.fail_quietly(None);
    s.outbox_sweep("a1");
    assert!(files(&s).is_empty(), "{:?}", files(&s));
    // No mark either.
    s.outbox_add("a1", &item, b"x").unwrap();
    s.fail_quietly(Some("outbox/7.json|outbox/7.eml"));
    s.fail(None, Some("outbox/7.gone"));
    assert!(s.outbox_remove("a1", 7).is_err(), "said to be taken out");
    assert_eq!(s.outbox("a1"), vec![item.clone()]);
    // Marked, the mark then held as the files were.
    s.fail(None, None);
    s.outbox_remove("a1", 7).unwrap();
    s.fail_quietly(Some("outbox/7.gone"));
    assert!(s.outbox_add("a1", &item, b"y").is_err(), "queued, and never listed");
    s.fail_quietly(None);
    s.outbox_add("a1", &item, b"y").unwrap();
    assert_eq!(s.outbox("a1"), vec![item]);
}
