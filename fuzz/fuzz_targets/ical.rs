// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A calendar file (an invitation, a CalDAV event) read, written back and read again; and the
//! WebDAV answers CalDAV and CardDAV servers send.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_model::ical;
use quatrelle_net::dav;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let cal = ical::parse(&text, 3600);
    for e in cal.events.iter().take(16) {
        let _ = ical::reminder_minutes(e);
        let _ = e.links();
        let _ = e.join_link();
        // Its rule as an Exchange account sends it (Graph's recurrence, which EWS's is made from).
        if let Some(rule) = &e.rrule {
            let _ = quatrelle_net::graph_pim::recurrence_of(rule, e.start);
        }
        // Its files (R23), as the appointment's window lists and opens them.
        for i in 0..quatrelle_model::pim::files::event_files(e).len().min(4) {
            let _ = quatrelle_model::pim::files::event_file_data(e, i);
        }
    }
    let written = ical::write(&cal.events, cal.method.as_deref(), 0);
    let _ = ical::parse(&written, 3600);
    let _ = ical::duration(&text);
    let _ = ical::unescape(&text);
    let _ = ical::split_text_list(&text);
    let _ = dav::multistatus(&text);
});
