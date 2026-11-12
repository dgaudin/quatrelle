// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A Teams message's HTML (its sender writes it) read into its text: never a panic, the text
//! and the runs within their caps, a hosted image only by an id the worker can put back in
//! Graph's address, links only to the web; the same bytes as a plain-text body.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_mime::{TEAMS_HTML_MAX, TEAMS_RUNS, teams_html, teams_text};
use quatrelle_model::pim::teams::MAX_ID;

fuzz_target!(|data: &[u8]| {
    let html = String::from_utf8_lossy(data);
    let text = teams_html(&html, "19:f@thread.v2", "1");
    assert!(text.plain().len() <= TEAMS_HTML_MAX);
    let runs: Vec<_> = text.blocks.iter().flat_map(|b| b.runs.iter()).collect();
    assert!(runs.len() <= TEAMS_RUNS);
    for run in runs {
        if let Some(id) = &run.hosted {
            assert!(!id.contains('/') && id.chars().count() <= MAX_ID, "hosted id {id:?}");
        }
        for address in [&run.link, &run.outside].into_iter().flatten() {
            assert!(address.starts_with("http://") || address.starts_with("https://"), "address {address:?}");
        }
    }
    assert!(teams_text(&html).plain().len() <= TEAMS_HTML_MAX);
});
