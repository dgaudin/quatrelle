// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! When the server received the mail a cache holds from before that time was kept: asked once, at
//! an ordinary sync, with `UID FETCH … (UID INTERNALDATE)` (RFC 3501 §6.4.8), so no header or
//! body comes again. Until it is known the list falls back on the message's own `Date:`.

use std::io::{Read, Write};

use quatrelle_store::FolderState;

use crate::imap::{Client, uid_chunks, uid_set};

/// The most messages asked about at a sync (the rest at the next ones), and in one command.
const PER_SYNC: usize = 5_000;
const PER_COMMAND: usize = 500;

/// Give the cached messages that have no arrival time the server's. A server that refuses
/// skips this, the sync goes on and the next one asks again; a message it knows no date for
/// keeps the fall-back.
pub(super) fn fill<S: Read + Write>(c: &mut Client<S>, st: &mut FolderState) {
    let missing: Vec<u32> = st.messages.iter().filter(|m| m.received == 0).map(|m| m.key.uid).take(PER_SYNC).collect();
    for chunk in uid_chunks(&missing, PER_COMMAND) {
        let Ok(fetched) = c.uid_fetch(&uid_set(&chunk), "UID INTERNALDATE", "") else { return };
        for f in fetched {
            if let (Some(t), Some(m)) = (f.internal_date.filter(|t| *t > 0), st.find_mut(f.uid)) {
                m.received = t;
            }
        }
    }
}
