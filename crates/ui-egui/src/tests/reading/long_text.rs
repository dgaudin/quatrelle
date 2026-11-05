// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a long message laid out by what's in view (the tests of upstream 7020f7b), its
//! addresses links and its tables kept (the fork's), and a bench of its frames.

use super::*;
use std::time::{Duration, Instant};

/// A message of this source put first in the Inbox, and the Inbox shown.
fn put(a: &mut MailApp, uid: u32, raw: &[u8]) -> MessageKey {
    let store = a.session.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid };
    inbox.messages.insert(0, quatrelle_engine::mime::summary_from_full(raw, key.clone(), Default::default(), 0, NOW));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, raw).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    key
}

/// A plain-text message of about `chars` characters, in lines of 80, a web address now and then.
fn plain_message(chars: usize) -> Vec<u8> {
    let mut text = String::new();
    let mut n = 0usize;
    while text.len() < chars {
        n += 1;
        if n.is_multiple_of(40) {
            text.push_str(&format!("See https://studio.example/notes/{n} for the figures of line {n:>6}, as agreed.\r\n"));
        } else {
            text.push_str(&format!("Line {n:>6} of the long report: the grade is warmer now, the cut is tighter.\r\n"));
        }
    }
    format!(
        "From: Ana <ana@posteo.example>\r\nTo: Riley <riley@studio.example>\r\nSubject: Long report\r\n\
         Date: Fri, 16 Oct 2026 09:00:00 +0000\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}"
    )
    .into_bytes()
}

/// An HTML message of about `chars` characters of text, a paragraph a line (a link every
/// fortieth).
fn html_message(chars: usize) -> Vec<u8> {
    let mut html = String::new();
    let mut n = 0usize;
    while n * 56 < chars {
        n += 1;
        if n.is_multiple_of(40) {
            html.push_str(&format!("<p>Line {n:>6}: the <a href=\"https://studio.example/{n}\">figures</a> are here.</p>\r\n"));
        } else {
            html.push_str(&format!("<p>Line {n:>6}: the <b>grade</b> is warmer now, the cut is tighter.</p>\r\n"));
        }
    }
    format!(
        "From: Ana <ana@posteo.example>\r\nTo: Riley <riley@studio.example>\r\nSubject: Long newsletter\r\n\
         Date: Fri, 16 Oct 2026 09:00:00 +0000\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body>{html}</body></html>"
    )
    .into_bytes()
}

/// The frames that show a long message once it's open: the first, and the slowest of the next
/// ten. A timing test: run in release (`cargo test --release -p quatrelle-ui-egui long_text::bench
/// -- --ignored --nocapture`).
#[test]
#[ignore = "a bench: run it with --release, --ignored and --nocapture"]
fn bench_frames_of_a_long_message() {
    for (name, raw) in
        [("plain, 127,000", plain_message(127_000)), ("HTML, 127,000", html_message(127_000)), ("plain, 300,000", plain_message(300_000))]
    {
        let (mut a, dir) = app("long-bench");
        let ctx = egui::Context::default();
        frame(&ctx, &mut a, vec![]);
        frame(&ctx, &mut a, vec![]);
        let k = put(&mut a, 9800, &raw);
        // The list as it stands drawn first (its first frame isn't the message's).
        frame(&ctx, &mut a, vec![]);
        frame(&ctx, &mut a, vec![]);
        let t = Instant::now();
        a.run("list.open", json!({"key": k.to_id()}));
        let opened = t.elapsed();
        let mut frames = Vec::new();
        for _ in 0..11 {
            let t = Instant::now();
            frame(&ctx, &mut a, vec![]);
            frames.push(t.elapsed());
        }
        let text = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map_or(0, |p| p.text.chars().count());
        let worst = frames.iter().skip(1).max().copied().unwrap_or(Duration::ZERO);
        let ms = |d: Duration| d.as_secs_f64() * 1e3;
        println!(
            "{name} characters asked ({text} in its text): opened in {:.1} ms, first frame {:.1} ms, slowest of the next ten {:.1} ms",
            ms(opened),
            ms(frames[0]),
            ms(worst)
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Long text is cut in chunks at line ends, the break in neither, and a line longer than a
/// chunk at a character's edge: put back together, it is the text.
#[test]
fn long_text_is_cut_at_line_ends_and_character_edges() {
    use crate::reading::{CHUNK, chunks};
    let text = format!("{}{}\nend", "ab\ncd\n".repeat(2000), "\u{e9}".repeat(5000));
    let r = chunks(&text, CHUNK);
    assert!(r.len() > 3);
    assert!(r.iter().all(|c| !c.is_empty() && c.len() <= CHUNK && text.is_char_boundary(c.start) && text.is_char_boundary(c.end)));
    let (mut back, mut at) = (String::new(), 0);
    for c in &r {
        if c.start > at {
            assert_eq!(&text[at..c.start], "\n");
            back.push('\n');
        }
        back.push_str(&text[c.clone()]);
        at = c.end;
    }
    assert_eq!(back, text);
    assert!(r.iter().any(|c| text[c.clone()].chars().all(|ch| ch == '\u{e9}')), "the long line is cut");
    assert_eq!(chunks("short", CHUNK), vec![0..5]);
    assert!(chunks("", CHUNK).is_empty());
}

/// A message of a few hundred thousand characters, its lines numbered.
fn long_message(lines: usize) -> Vec<u8> {
    let body: String = (0..lines).map(|i| format!("Line {i:05} of a long message, with words enough to wrap now and then.\n")).collect();
    let d = quatrelle_engine::mime::Draft { subject: "Long".into(), body, ..Default::default() };
    let from = quatrelle_engine::model::EmailAddress::new("Sam", "sam@studio.example");
    quatrelle_engine::mime::build(&d, &from, NOW, "<long@x>").unwrap()
}

/// `raw` as the body of the first message, opened; its text, and how many parts the first frame
/// that showed it laid out.
fn open_long(a: &mut MailApp, ctx: &egui::Context, raw: Vec<u8>) -> (String, usize) {
    frame(ctx, a, vec![]);
    let key = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.session.open_message(&key);
    a.session.apply(MailEvent::Body { key, raw });
    // Until it has come in (it fades up from a little below).
    let mut first = None;
    for _ in 0..30 {
        frame(ctx, a, vec![]);
        if first.is_none() && !a.ui.long_text.is_empty() {
            first = Some(long_state(a).laid);
        }
    }
    let text = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.text.clone()).unwrap();
    (text, first.expect("laid out by what's in view"))
}

fn long_state(a: &MailApp) -> &crate::reading::LongText {
    let mut all = a.ui.long_text.values();
    let v = all.next().expect("laid out by what's in view");
    assert!(all.next().is_none());
    v
}

/// A frame with `events`; what it copied (the last copy, which is what the clipboard keeps).
fn copied(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) -> Option<String> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    out.platform_output
        .commands
        .into_iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t),
            _ => None,
        })
        .next_back()
}

/// A click at `at`, with `modifiers` held.
fn click(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2, modifiers: Modifiers) {
    let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
    frame(ctx, a, vec![Event::PointerMoved(at), Event::ModifiersChanged(modifiers)]);
    frame(ctx, a, vec![button(true)]);
    frame(ctx, a, vec![button(false), Event::ModifiersChanged(Modifiers::NONE)]);
}

fn scroll_to_end(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2) {
    for _ in 0..60 {
        let wheel = Event::MouseWheel {
            unit: egui::MouseWheelUnit::Page,
            delta: egui::vec2(0.0, -3.0),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
        };
        frame(ctx, a, vec![Event::PointerMoved(at), wheel]);
    }
}

/// A 300,000-character message (127,000 took a 179 ms frame, laid out whole): the first frame
/// lays out at most 3 of its chunks, and scrolled to its end, the last one. A chunk's guessed
/// height is the one it's laid out at, so the text in view stays put as those above are measured.
#[test]
fn a_long_message_lays_out_what_is_in_view() {
    let (mut a, dir) = app("long-text");
    let ctx = egui::Context::default();
    let (text, first) = open_long(&mut a, &ctx, long_message(4400));
    assert!(text.len() > 300_000, "{}", text.len());
    let n = crate::reading::chunks(&text, crate::reading::CHUNK).len();
    assert!((1..=3).contains(&first), "{first} of {n} chunks laid out in the first frame");
    let v = long_state(&a);
    let measured: Vec<(f32, f32)> = v.heights().filter_map(|(g, m)| Some((g, m?))).collect();
    assert!(measured.len() > 3);
    // The last chunk ends the text, which ends with a line break of its own.
    for (g, m) in measured.iter().take(measured.len().min(n - 1)) {
        assert!((g - m).abs() < 3.0, "guessed {g}, laid out at {m}");
    }
    let at = v.shown.first().map(|(_, r)| egui::pos2(r.left() + 20.0, 400.0)).unwrap();
    scroll_to_end(&ctx, &mut a, at);
    let v = long_state(&a);
    assert!(v.shown.iter().any(|(i, _)| *i + 1 == n), "the last chunk, at the end: {:?}", v.shown);
    assert!(v.laid < n / 4, "{} of {n}", v.laid);
    let _ = std::fs::remove_dir_all(dir);
}

/// A selection from the first chunk to one far below (scrolled to, Shift+clicked) copies all the
/// text between, chunks passed over included; Ctrl+A in the text copies all of it.
#[test]
fn a_selection_over_chunks_passed_over_copies_them() {
    let (mut a, dir) = app("long-select");
    let ctx = egui::Context::default();
    let (text, _) = open_long(&mut a, &ctx, long_message(4400));
    let ranges = crate::reading::chunks(&text, crate::reading::CHUNK);
    let first = long_state(&a).shown.first().map(|(_, r)| *r).unwrap();
    click(&ctx, &mut a, first.left_top() + egui::vec2(2.0, 10.0), Modifiers::NONE);
    scroll_to_end(&ctx, &mut a, egui::pos2(first.left() + 20.0, 400.0));
    let (last, rect) = long_state(&a).shown.last().copied().unwrap();
    assert!(last > 10);
    // (Below the message's header, which stays put above it in the fork.)
    let at = egui::pos2(rect.left() + 2.0, rect.center().y.clamp(400.0, 700.0));
    assert!(rect.contains(at));
    click(&ctx, &mut a, at, Modifiers::SHIFT);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    let before = ranges.get(last - 1).map(|r| r.end).unwrap();
    assert!(got.len() > before, "from the start into the chunk clicked: {} of {before}", got.len());
    assert!(text.starts_with(&got), "the text as it is");
    // Ctrl+A, then a copy: the whole text, and the mail selection is left alone.
    click(&ctx, &mut a, at, Modifiers::NONE);
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    assert!(long_state(&a).all);
    assert_eq!(a.session.selection.len(), 1);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert!(got == text, "{} of {}", got.len(), text.len());
    let _ = std::fs::remove_dir_all(dir);
}

/// Formatted mail of thousands of lines is laid out in blocks of lines in the same way.
#[test]
fn a_long_formatted_message_lays_out_what_is_in_view() {
    let (mut a, dir) = app("long-rich");
    let ctx = egui::Context::default();
    let mut raw = b"From: Sam <sam@studio.example>\r\nSubject: Long\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body>".to_vec();
    for i in 0..3000 {
        raw.extend_from_slice(format!("<div>Line {i:05} of a long <b>formatted</b> message.</div>\r\n").as_bytes());
    }
    raw.extend_from_slice(b"</body></html>\r\n");
    let (_, first) = open_long(&mut a, &ctx, raw);
    let lines = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map_or(0, |p| p.rich.len());
    assert!(lines > 2_000 && a.session.prefs.show_html, "{lines} lines");
    let n = lines.div_ceil(64);
    assert!((1..=6).contains(&first), "{first} of {n} blocks laid out");
    let v = long_state(&a);
    let at = v.shown.first().map(|(_, r)| egui::pos2(r.left() + 20.0, 400.0)).unwrap();
    scroll_to_end(&ctx, &mut a, at);
    let v = long_state(&a);
    assert!(v.shown.iter().any(|(i, _)| *i + 1 == n), "the last block, at the end: {:?}", v.shown);
    let _ = std::fs::remove_dir_all(dir);
}

/// Ctrl+A in the long text, then Ctrl+E to the search box: Ctrl+A and a copy there no longer
/// copy the whole message (that copy came last, so it was the one kept). egui's own label
/// selection, still there, copies what it holds as it does for any message.
#[test]
fn ctrl_a_in_the_search_box_after_a_long_message_does_not_copy_it_whole() {
    let (mut a, dir) = app("long-select-all");
    let ctx = egui::Context::default();
    let (text, _) = open_long(&mut a, &ctx, long_message(4400));
    let first = long_state(&a).shown.first().map(|(_, r)| *r).unwrap();
    click(&ctx, &mut a, first.left_top() + egui::vec2(2.0, 10.0), Modifiers::NONE);
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    assert!(long_state(&a).all);
    frame(&ctx, &mut a, vec![key(Key::E, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.search_focused);
    frame(&ctx, &mut a, vec![Event::Text("zebra".into())]);
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    assert!(!long_state(&a).all);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert!(got.len() < text.len(), "{} of {}", got.len(), text.len());
    let _ = std::fs::remove_dir_all(dir);
}

/// The fork's plain text keeps its addresses as links in a long message too: a click on one
/// opens it (a mail address: a new message to it).
#[test]
fn addresses_in_a_long_plain_message_open_with_a_click() {
    let (mut a, dir) = app("long-links");
    let ctx = egui::Context::default();
    let mut body = String::from("Questions: support@example.com.\n");
    body.push_str(&(0..1000).map(|i| format!("Line {i:05} of a long message, with words enough to wrap now and then.\n")).collect::<String>());
    let d = quatrelle_engine::mime::Draft { subject: "Long".into(), body, ..Default::default() };
    let raw =
        quatrelle_engine::mime::build(&d, &quatrelle_engine::model::EmailAddress::new("Sam", "sam@studio.example"), NOW, "<long-links@x>").unwrap();
    let (text, _) = open_long(&mut a, &ctx, raw);
    assert!(text.len() > 20_000, "{}", text.len());
    let at: Vec<egui::Rect> = ctx.data(|d| d.get_temp(egui::Id::unique("long-text-links"))).unwrap_or_default();
    assert_eq!(at.len(), 1, "the mail address, in the chunk in view");
    let p = at[0].left_center() + egui::vec2(4.0, 0.0);
    let button = |pressed| Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(&ctx, &mut a, vec![Event::PointerMoved(p)]);
    frame(&ctx, &mut a, vec![button(true)]);
    frame(&ctx, &mut a, vec![button(false)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.compose.as_ref().map(|c| c.to.as_str()), Some("support@example.com"), "a new message to it");
    let _ = std::fs::remove_dir_all(dir);
}

/// The cell heights kept for the open message, by cell number.
fn cell_heights(a: &MailApp) -> Vec<(usize, f32)> {
    let key = a.session.reading.as_ref().unwrap().key.clone();
    (0..20_000).filter_map(|c| a.ui.cell_heights.get(&key, c).map(|h| (c, h))).collect()
}

fn wheel(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2, pages: f32) {
    let wheel = Event::MouseWheel {
        unit: egui::MouseWheelUnit::Page,
        delta: egui::vec2(0.0, pages),
        modifiers: Modifiers::NONE,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
    };
    frame(ctx, a, vec![Event::PointerMoved(at), wheel]);
}

/// A long newsletter laid out in columns (the fork's tables): its tables are laid out one by one
/// with the lines between them, and each keeps its cells' numbers whatever is laid out above it,
/// so a row aligned to its tallest cell finds its heights again when scrolled away and back.
#[test]
fn a_long_newsletter_with_tables_lays_out_what_is_in_view() {
    let (mut a, dir) = app("long-tables");
    let ctx = egui::Context::default();
    let table = |n: usize| {
        format!(
            "<table width=\"100%\"><tr><td valign=\"middle\" width=\"50%\">Column {n} on the left{}</td>\
             <td valign=\"middle\">Column {n} on the right</td></tr></table>\r\n",
            "<br>one line more".repeat(n % 7)
        )
    };
    let mut raw = b"From: Sam <sam@studio.example>\r\nSubject: Long\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body>".to_vec();
    for i in 0..2400 {
        if i % 400 == 0 {
            raw.extend_from_slice(table(i).as_bytes());
        }
        raw.extend_from_slice(format!("<div>Line {i:05} of a long <b>formatted</b> message.</div>\r\n").as_bytes());
    }
    raw.extend_from_slice(table(9999).as_bytes());
    raw.extend_from_slice(b"</body></html>\r\n");
    let (_, first) = open_long(&mut a, &ctx, raw);
    let layout = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map_or(0, |p| p.layout.len());
    assert!(layout > 7, "laid out in blocks and tables: {layout}");
    let n = long_state(&a).heights().count();
    assert!(first < n / 4, "{first} of {n} parts laid out");
    let top = cell_heights(&a);
    assert_eq!(top.len(), 14, "the seven tables' cells measured: {top:?}");
    let at = long_state(&a).shown.first().map(|(_, r)| egui::pos2(r.left() + 20.0, 400.0)).unwrap();
    scroll_to_end(&ctx, &mut a, at);
    let v = long_state(&a);
    assert!(v.shown.iter().any(|(i, _)| *i + 1 == n), "the last part, at the end: {:?}", v.shown);
    assert!(v.laid < n / 2, "{} of {n}", v.laid);
    // Each table measured at the top as the scroll bar settled: at the end, with the tables
    // above it not laid out, the last one's cells keep their numbers, and no other table's
    // number got another's height.
    assert_eq!(cell_heights(&a), top, "the same numbers, the same heights");
    for _ in 0..60 {
        wheel(&ctx, &mut a, at, 3.0);
    }
    assert_eq!(cell_heights(&a), top, "back at the top");
    let _ = std::fs::remove_dir_all(dir);
}
