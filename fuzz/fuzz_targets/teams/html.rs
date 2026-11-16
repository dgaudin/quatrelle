// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A Teams message's HTML (its sender writes it) read into its text: never a panic, the text
//! and the runs within their caps, a hosted image only by an id the worker can put back in
//! Graph's address (no `/`, not only dots), links only to the web; the same bytes as a
//! plain-text body. Then written back as for sending, and the same bytes as typed in the box
//! with bold and italic ranges and a mention: at most `SEND_MAX` bytes of the writer's own markup alone.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_mime::{
    TEAMS_HTML_MAX, TEAMS_RUNS, TooLong, Typed, teams_compose_typed, teams_foreign_markup, teams_html, teams_text, teams_typed, teams_write,
    teams_write_mentions,
};
use quatrelle_model::pim::teams::{MAX_ID, SEND_MAX};

fn check(written: Result<String, TooLong>) {
    match written {
        Ok(html) => {
            assert!(html.len() <= SEND_MAX, "{} bytes", html.len());
            assert!(!html.contains("<img") && !html.contains("<script"), "{html}");
            if let Some(piece) = teams_foreign_markup(&html) {
                panic!("not the writer's markup: {piece:?} in {html:?}");
            }
        }
        Err(TooLong { bytes }) => assert!(bytes > SEND_MAX, "too long at {bytes} bytes"),
    }
}

fuzz_target!(|data: &[u8]| {
    let html = String::from_utf8_lossy(data);
    let text = teams_html(&html, "19:f@thread.v2", "1");
    assert!(text.plain().len() <= TEAMS_HTML_MAX);
    let runs: Vec<_> = text.blocks.iter().flat_map(|b| b.runs.iter()).collect();
    assert!(runs.len() <= TEAMS_RUNS);
    for run in runs {
        if let Some(id) = &run.hosted {
            assert!(!id.contains('/') && id.chars().count() <= MAX_ID, "hosted id {id:?}");
            assert!(!id.chars().all(|c| c == '.'), "hosted id of dots only {id:?}");
        }
        for address in [&run.link, &run.outside].into_iter().flatten() {
            assert!(address.starts_with("http://") || address.starts_with("https://"), "address {address:?}");
        }
    }
    assert!(teams_text(&html).plain().len() <= TEAMS_HTML_MAX);
    check(teams_write(&text));
    // The bytes as typed, with ranges that may fall off the text or inside a character.
    let (n, third) = (html.len(), html.len() / 3);
    let typed = Typed {
        text: html.to_string(),
        bold: vec![third..n, 1..2],
        italic: vec![0..third.saturating_mul(2), n..third],
        mentions: vec![(third..third.saturating_mul(2), "u-f".into())],
    };
    let (composed, mentions) = teams_compose_typed(&typed);
    assert!(composed.blocks.iter().map(|b| b.runs.len()).sum::<usize>() <= TEAMS_RUNS);
    check(teams_write_mentions(&composed, &mentions));
    // A message read back into the box never panics, and what it holds composes within the caps.
    if let Some(back) = teams_typed(&text, &[]) {
        let (again, _) = teams_compose_typed(&back);
        assert!(again.blocks.iter().map(|b| b.runs.len()).sum::<usize>() <= TEAMS_RUNS);
    }
});
