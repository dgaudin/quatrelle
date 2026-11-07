// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: searches the summaries answer (flags, senders, sizes, categories) read no
//! message.

use super::*;

/// What every folder of the sample mailbox holds that `keep` keeps, counted from the summaries.
pub(super) fn counted(s: &mut Session, keep: impl Fn(&Summary) -> bool) -> usize {
    let mut n = 0;
    for (a, f) in s.scope_folders(SearchScope::All) {
        n += s.folder_state(&a, &f).messages.iter().filter(|m| keep(m)).count();
    }
    n
}

/// Flags, senders, sizes: the summaries answer, and no message is read for them (a big data
/// file's every message was read again for `is:unread`), nor for a Search Folder asking them.
#[test]
fn summary_searches_read_no_message() {
    let mut s = sample_session("summary-search");
    let unread = counted(&mut s, |m| !m.flags.seen);
    let flagged = counted(&mut s, |m| m.flags.flagged);
    let from_zoe = counted(&mut s, |m| m.from.name.to_lowercase().contains("zoe") || m.from.email.contains("zoe"));
    assert!(unread > 0 && flagged > 0 && from_zoe > 0, "the sample has some of each");
    let reads = s.store.as_ref().unwrap().reads();
    let all = |s: &mut Session, q: &str| s.run("search.run", &json!({"query": q, "scope": "all"})).unwrap()["results"].as_u64().unwrap() as usize;
    assert_eq!(all(&mut s, "is:unread"), unread);
    assert_eq!(all(&mut s, "is:flagged"), flagged);
    assert_eq!(all(&mut s, "from:zoe"), from_zoe);
    all(&mut s, "has:attachment -is:read larger:1KB subject:storyboard");
    s.run("searchFolder.create", &json!({"name": "Unread", "query": "is:unread"})).unwrap();
    assert!(!s.list.is_empty());
    assert_eq!(s.store.as_ref().unwrap().reads(), reads, "no message was read");
    // Words anywhere need the text: those still read it.
    assert_eq!(all(&mut s, "projector"), 1);
    let reads = s.store.as_ref().unwrap().reads();
    assert!(reads > 0);
    // Each index stays for its kind of search: going back and forth reads nothing again.
    assert_eq!(all(&mut s, "is:unread"), unread);
    assert_eq!(all(&mut s, "projector"), 1);
    assert_eq!(all(&mut s, "is:unread projector"), all(&mut s, "projector is:unread"));
    assert_eq!(s.store.as_ref().unwrap().reads(), reads, "read once");
    done(s);
}
