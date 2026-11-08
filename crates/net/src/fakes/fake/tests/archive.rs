// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Archiving IMAP mail onto this computer, over the fake IMAP server: the copy, the read-back and
//! the deletion on the server, Gmail's Trash included.

use super::*;

fn count(st: &Arc<Mutex<State>>, name: &str) -> usize {
    st.lock().unwrap().mailbox(name).map_or(0, |b| b.msgs.len())
}

/// Gmail keeps every message in All Mail: an expunge in a label folder only drops the label, and
/// one in Trash removes the message everywhere. A refused expunge says no, as many times as told.
#[test]
fn the_fake_keeps_gmails_all_mail() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.gmail();
        s.add("INBOX", "a", &[]);
        s.add("INBOX", "b", &[]);
    }
    let mut c = login(&st);
    c.select("INBOX", false).unwrap();
    c.uid_store("1", true, "\\Deleted").unwrap();
    c.expunge("1").unwrap();
    assert_eq!((count(&st, "INBOX"), count(&st, "[Gmail]/All Mail"), count(&st, "[Gmail]/Trash")), (1, 1, 0), "the label goes, the message stays");
    // Moved to Trash and expunged there: gone from Trash, and from All Mail with it.
    let raw = st.lock().unwrap().mailbox("[Gmail]/All Mail").unwrap().msgs[0].raw.clone();
    st.lock().unwrap().add_raw("[Gmail]/Trash", raw);
    c.select("[Gmail]/Trash", false).unwrap();
    c.uid_store("1", true, "\\Deleted").unwrap();
    c.expunge("1").unwrap();
    assert_eq!((count(&st, "[Gmail]/All Mail"), count(&st, "[Gmail]/Trash")), (0, 0));
    // A refused expunge leaves the message where it is, then works again.
    st.lock().unwrap().refuse_expunge(1);
    c.select("INBOX", false).unwrap();
    c.uid_store("2", true, "\\Deleted").unwrap();
    assert!(c.expunge("2").is_err());
    assert_eq!(count(&st, "INBOX"), 1);
    c.expunge("2").unwrap();
    assert_eq!(count(&st, "INBOX"), 0);
}
