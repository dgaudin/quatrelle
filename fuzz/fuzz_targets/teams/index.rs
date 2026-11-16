// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A chat's search tranche (`index/<hash>/<n>.json` in `Teams.quatrelle`) read back as the hostile
//! file it may be, held to its caps, a message's words taken out and put in, cut as the writer
//! cuts it and written: each piece read back the same, within its caps and a tranche's count, the
//! pieces holding what the tranche held, no edge between two messages of one millisecond unless a
//! piece holds only that millisecond. Never a panic. The input is a chat id, a tranche's bytes, a
//! message and a control byte, as `quatrelle_net::teams::fuzz_teams_index` reads them.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Err(broken) = quatrelle_net::teams::fuzz_teams_index(data) {
        panic!("{broken}");
    }
});
