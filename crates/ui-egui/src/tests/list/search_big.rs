// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A mailbox the size of a big real one, made up ([`big`]: 50,000 messages in 40 folders, the
//! Inbox 12,000, every one downloaded), in the window: a search typed over all of it builds the
//! index in memory a slice a frame, the window drawing between them, and finds what the mailbox
//! holds; Down through the Inbox builds no rows. The times are checked in a release build only:
//! a debug build's frames are many times slower (its mailbox is a fifth of the size, so the
//! suite stays quick). The two tests here run one at a time (side by side, each one's frames
//! measured the other's work).

use quatrelle_engine::sample::big;

use super::*;

/// Held by each test here while it runs.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The mailbox: 50,000 messages in a release build, 10,000 in a debug one.
fn shape() -> big::Shape {
    if cfg!(debug_assertions) { big::Shape { messages: 10_000, inbox: 2_400, ..big::Shape::default() } } else { big::Shape::default() }
}

/// The window on the mailbox, its Inbox shown; nothing indexed yet.
fn big_app() -> (MailApp, big::Truth) {
    let store = Store::memory();
    let truth = big::generate(&store, NOW, shape()).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    (MailApp::new(s, Box::new(NullBackend), Box::new(NullServices(NOW))), truth)
}

/// The `pct` percentile of `times` (`99`: the 99th).
fn percentile(times: &[std::time::Duration], pct: usize) -> std::time::Duration {
    let mut t = times.to_vec();
    t.sort();
    t.get((t.len().saturating_sub(1)) * pct / 100).copied().unwrap_or_default()
}

/// Queries typed over the mailbox: the first builds the index a slice a frame and finds what the
/// mailbox holds, and in a release build no frame holds the window long; the second (`is:read`,
/// 35,000 found) reads the summaries alone.
#[test]
fn a_search_typed_over_fifty_thousand_never_holds_a_frame() {
    let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let ctx = egui::Context::default();
    let (mut a, t) = big_app();
    frame(&ctx, &mut a, vec![]);
    for (q, want) in [(big::RARE, t.rare), ("is:read", t.read)] {
        a.run("search.clear", json!({}));
        a.ui.search_text.clear();
        a.ui.focus_search = true;
        for _ in 0..2 {
            frame(&ctx, &mut a, vec![]);
        }
        // Typed a key a frame, then Enter (searched at once: the box's pause after the last key
        // goes by egui's clock, which a test's frames don't move).
        for c in q.chars() {
            frame(&ctx, &mut a, vec![Event::Text(c.to_string())]);
        }
        assert_eq!(a.ui.search_text, q);
        let mut times = Vec::new();
        let started = std::time::Instant::now();
        let mut events = vec![key(Key::Enter, Modifiers::NONE)];
        while (!events.is_empty() || a.session.search_pending) && times.len() < 100_000 {
            let at = std::time::Instant::now();
            frame(&ctx, &mut a, std::mem::take(&mut events));
            times.push(at.elapsed());
        }
        let worst = times.iter().max().copied().unwrap_or_default();
        let p99 = percentile(&times, 99);
        eprintln!(
            "{q}: {} frames in {:.2} s, p50 {:?}, p99 {p99:?}, worst {worst:?}",
            times.len(),
            started.elapsed().as_secs_f64(),
            percentile(&times, 50)
        );
        assert!(q == "is:read" || times.len() > 2, "{q}: spread over frames");
        assert!(!a.session.search_pending, "{q}: done");
        assert_eq!(a.session.search.as_ref().map(|(s, _)| s.as_str()), Some(q));
        assert_eq!(a.session.list.len(), want, "{q}");
        if !cfg!(debug_assertions) {
            assert!(p99 <= std::time::Duration::from_millis(40), "{q}: frames took {p99:?} (p99)");
        }
    }
}

/// Down through the Inbox, a press a frame, each message read as it's landed on: the list's rows
/// are never built again, and a release build's frames keep to 30 ms (p99).
#[test]
fn down_through_twelve_thousand_builds_no_rows() {
    let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let ctx = egui::Context::default();
    let (mut a, _) = big_app();
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.list.len(), shape().inbox as usize);
    frame(&ctx, &mut a, vec![key(Key::Home, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    let (built, read) = (a.session.nav_structure(), a.session.messages_shown());
    let mut times = Vec::new();
    for _ in 0..60 {
        let started = std::time::Instant::now();
        frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
        times.push(started.elapsed());
    }
    let p99 = percentile(&times, 99);
    eprintln!("60 Downs: p50 {:?}, p99 {p99:?}, worst {:?}", percentile(&times, 50), times.iter().max().copied().unwrap_or_default());
    assert_eq!(a.session.messages_shown(), read + 60, "each one read");
    assert_eq!(a.session.nav_structure(), built, "no rows built");
    assert_eq!(a.session.row_keys().iter().position(|k| a.session.focus.as_ref() == Some(k)), Some(60));
    if !cfg!(debug_assertions) {
        assert!(p99 <= std::time::Duration::from_millis(30), "a Down's frame took {p99:?}");
    }
}
