// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The local copy of the Teams chats (`Teams.quatrelle`) read back as the hostile file it may be:
//! its identity, chats' records, pages of messages and the outbox's records made from the input
//! (records under another's key, pages that don't hold together, more than a page's worth, bodies
//! past their cap, messages going out past the outbox's caps), then opened, listed, read a window
//! and a page at a time, written to, and its outbox read back and emptied. Never a panic, and
//! what's read back within its caps. The input is a chat id, keys and their bytes, and a message,
//! as `quatrelle_net::teams::fuzz_teams_copy` reads them.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Err(broken) = quatrelle_net::teams::fuzz_teams_copy(data) {
        panic!("{broken}");
    }
});
