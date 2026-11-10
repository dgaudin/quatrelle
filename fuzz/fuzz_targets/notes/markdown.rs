// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A note's Markdown (a note from Exchange or a data file is hostile text): its lines and
//! segments inside the text, in order, covering it, on characters' boundaries, within the caps;
//! the window's edits (marks, Enter, Tab) at positions taken from the same bytes, their selection
//! inside the text they give, and a tick changing one byte or none.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_model::pim::markdown::{self, Edit, MAX_SPANS, Mark};

/// The text's lines and segments cover it, in order, each on characters' boundaries.
fn check(text: &str) {
    let mut end = 0;
    for (i, l) in markdown::read(text).iter().enumerate() {
        assert!(l.range.start <= l.range.end && l.range.end <= text.len());
        assert!(text.is_char_boundary(l.range.start) && text.is_char_boundary(l.range.end));
        let gap = text.get(end..l.range.start);
        assert!(if i == 0 { l.range.start == 0 } else { gap == Some("\n") || gap == Some("\r\n") });
        assert!(l.spans.len() <= MAX_SPANS);
        let mut at = l.range.start;
        for s in &l.spans {
            assert!(s.range.start == at && s.range.end > s.range.start && text.is_char_boundary(s.range.end));
            at = s.range.end;
        }
        assert_eq!(at, l.range.end);
        end = l.range.end;
    }
    assert_eq!(end, text.len());
}

fn check_edit(e: &Edit) {
    assert!(e.select.start <= e.select.end && e.select.end <= e.text.len());
    assert!(e.text.is_char_boundary(e.select.start) && e.text.is_char_boundary(e.select.end));
}

fuzz_target!(|data: &[u8]| {
    // The first four bytes place a selection; the rest is the note.
    let (head, body) = data.split_at(data.len().min(4));
    let at = |i: usize| usize::from(head.get(i).copied().unwrap_or(0)) * 64 + usize::from(head.get(i + 1).copied().unwrap_or(0));
    let text = String::from_utf8_lossy(body);
    check(&text);
    let (a, b) = (at(0), at(2));
    for mark in [Mark::Bold, Mark::Italic, Mark::Heading, Mark::List, Mark::Tick, Mark::Link] {
        let e = markdown::toggle_mark(&text, a..b, mark);
        check_edit(&e);
        check(&e.text);
    }
    if let Some(e) = markdown::enter(&text, a) {
        check_edit(&e);
    }
    for out in [false, true] {
        if let Some(e) = markdown::indent(&text, a..b, out) {
            check_edit(&e);
            check(&e.text);
        }
    }
    let mut t = text.to_string();
    let ticked = markdown::toggle_tick(&mut t, b);
    let differ = text.bytes().zip(t.bytes()).filter(|(x, y)| x != y).count();
    assert_eq!((t.len(), differ), (text.len(), usize::from(ticked)));
    let _ = markdown::link_at(&text, a);
});
