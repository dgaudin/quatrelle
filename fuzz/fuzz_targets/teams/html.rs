// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A Teams message's HTML (its sender writes it) read into its text: never a panic, the text
//! and the runs within their caps, a hosted image only by an id the worker can put back in
//! Graph's address (no `/`, not only dots), links only to the web; the same bytes as a
//! plain-text body. Then written back as for sending, and the same bytes as typed in the box
//! with bold and italic ranges: at most `SEND_MAX` bytes of the writer's own markup alone.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_mime::{TEAMS_HTML_MAX, TEAMS_RUNS, TooLong, teams_compose, teams_html, teams_text, teams_write};
use quatrelle_model::pim::teams::{MAX_ID, SEND_MAX};

/// The writer's own markup: its tags and the entities it writes.
const OWN: &[&str] = &[
    "<p>", "</p>", "<b>", "</b>", "<i>", "</i>", "<u>", "</u>", "<s>", "</s>", "<code>", "</code>", "<br>", "</a>", "&amp;", "&lt;", "&gt;",
    "&quot;", "&#39;", "&nbsp;",
];

/// The first piece of `html` that isn't the writer's own markup, if any: a `<`, `>`, `"`, `'` or
/// `&` that doesn't begin one of its tags, a link to the web or one of its entities (escaped text
/// may say `src=` or `style=`; no attribute but a web link's `href` may be written).
fn foreign(html: &str) -> Option<&str> {
    let mut rest = html;
    while let Some(i) = rest.find(['<', '>', '"', '\'', '&']) {
        let at = &rest[i..];
        let Some(len) = OWN.iter().find(|own| at.starts_with(**own)).map(|own| own.len()).or_else(|| link_tag(at)) else { return Some(at) };
        rest = &at[len..];
    }
    None
}

/// The length of the tag `at` starts with when it's a link to the web, its address escaped.
fn link_tag(at: &str) -> Option<usize> {
    let href = at.strip_prefix("<a href=\"")?;
    let end = href.find('"')?;
    let address = &href[..end];
    let web = address.starts_with("http://") || address.starts_with("https://");
    (web && !address.contains(['<', '>', '\'']) && href[end..].starts_with("\">")).then(|| "<a href=\"".len() + end + 2)
}

fn check(written: Result<String, TooLong>) {
    match written {
        Ok(html) => {
            assert!(html.len() <= SEND_MAX, "{} bytes", html.len());
            assert!(!html.contains("<img") && !html.contains("<script"), "{html}");
            if let Some(piece) = foreign(&html) {
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
    let composed = teams_compose(&html, &[third..n, 1..2], &[0..third.saturating_mul(2), n..third]);
    assert!(composed.blocks.iter().map(|b| b.runs.len()).sum::<usize>() <= TEAMS_RUNS);
    check(teams_write(&composed));
});
