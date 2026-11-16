// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The reading pane's header as Outlook's: Reply | Reply all | Forward as one group, Assistant
//! and ⋯ beside it, at the sender's right in a wide pane; on a line of their own under the
//! header in a narrower one, their icons alone when even that is too narrow. Print and Save went
//! into ⋯, the assistant's three into Assistant.

use super::*;

/// The sample's first message open in a window `width` wide, with a language model set or not;
/// the sender's name as the header writes it.
fn opened(name: &str, width: f32, llm: bool) -> (MailApp, std::path::PathBuf, egui::Vec2, String) {
    let (mut a, dir) = app(name);
    if llm {
        a.run("assistant.settings", json!({"url": "http://localhost:11434/v1", "model": "llama3"}));
    }
    let k = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let sender = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.from.display().to_string()).unwrap_or_default();
    (a, dir, egui::vec2(width, 800.0), sender)
}

/// A name in Cc cut by the end of the line is underlined under the pointer where its words are,
/// on each of its lines: it was underlined under the whole width of the next line, and not at
/// all under its first part.
#[test]
fn a_name_cut_by_the_line_is_underlined_under_its_words_only() {
    const NAMES: [&str; 6] = [
        "Alexandra Montgomery-Fairweather",
        "Bartholomew Castellanos-Whitfield",
        "Constance Abernathy-Pemberton",
        "Dorothea Vanderbilt-Ashworth",
        "Evangeline Kowalczyk-Brightwater",
        "Fitzgerald Okonkwo-Harrington",
    ];
    let dir = std::env::temp_dir().join(format!("quatrelle-ui-cut-names-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    sample::seed_if_empty(&store, NOW).unwrap();
    let cc = NAMES.iter().enumerate().map(|(i, n)| format!("{n} <p{i}@studio.example>")).collect::<Vec<_>>().join(", ");
    let raw = format!(
        "From: Studio Shop <shop@studio.example>\r\nTo: Riley Moreno <riley@studio.example>\r\nCc: {cc}\r\nSubject: Cut names\r\n\
         Date: Wed, 22 Oct 2026 10:00:00 +0200\r\nMessage-ID: <cut-names@studio.example>\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\nHello\r\n"
    );
    let account = quatrelle_engine::sample::SAMPLE_ACCOUNT;
    let mut fs = store.load_folder(account, "INBOX").unwrap();
    let uid = fs.uid_next.max(1);
    let key = MessageKey { account: account.into(), folder: "INBOX".into(), validity: fs.validity.max(1), uid };
    store.write_body(&key, raw.as_bytes()).unwrap();
    let flags = quatrelle_engine::model::Flags { seen: true, ..Default::default() };
    fs.messages.push(quatrelle_engine::mime::summary_from_full(raw.as_bytes(), key.clone(), flags, raw.len() as u32, NOW));
    fs.uid_next = uid + 1;
    store.save_folder(account, "INBOX", &fs).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    let mut a = MailApp::new(s, Box::new(NullBackend), Box::new(NullServices(NOW)));
    a.run("list.open", json!({"key": key.to_id()}));
    let ctx = egui::Context::default();
    // A width where one of the names is cut by the end of its line.
    let cut = (0..80).map(|i| 1500.0 - 10.0 * i as f32).find_map(|w| {
        let size = egui::vec2(w, 800.0);
        settled_text_at(&ctx, &mut a, "Cut names", size)?;
        frame_shapes(&ctx, &mut a, vec![], size).into_iter().find_map(|s| match s {
            egui::Shape::Text(t) if NAMES.contains(&t.galley.text()) && t.galley.rows.iter().filter(|r| !r.glyphs.is_empty()).count() == 2 => {
                Some((size, t.pos, t.galley))
            }
            _ => None,
        })
    });
    let (size, pos, galley) = cut.expect("a width where a name is cut by the end of its line");
    let rows: Vec<egui::Rect> =
        galley.rows.iter().filter(|r| !r.glyphs.is_empty()).map(|r| r.rect_without_leading_space().translate(pos.to_vec2())).collect();
    let first = rows.first().copied().expect("the name's first line");
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(first.center())], size);
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    let (top, bottom) = (rows.iter().map(|r| r.top()).fold(f32::MAX, f32::min), rows.iter().map(|r| r.bottom()).fold(f32::MIN, f32::max));
    let (left, right) = (rows.iter().map(|r| r.left()).fold(f32::MAX, f32::min), rows.iter().map(|r| r.right()).fold(f32::MIN, f32::max));
    let lines: Vec<[egui::Pos2; 2]> = shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::LineSegment { points, .. }
                if (points[0].y - points[1].y).abs() < 0.5
                    && points[0].y > top
                    && points[0].y < bottom + 3.0
                    && points[0].x.min(points[1].x) < right
                    && points[0].x.max(points[1].x) > left =>
            {
                Some(*points)
            }
            _ => None,
        })
        .collect();
    assert!(!lines.is_empty(), "the name is underlined under the pointer");
    for [p, q] in &lines {
        let (l, r) = (p.x.min(q.x), p.x.max(q.x));
        assert!(
            rows.iter().any(|row| (row.bottom() - p.y).abs() < 3.0 && l >= row.left() - 1.0 && r <= row.right() + 1.0),
            "an underline from {l} to {r} at {} is under the name's words ({rows:?})",
            p.y
        );
    }
    for row in &rows {
        assert!(lines.iter().any(|[p, _]| (row.bottom() - p.y).abs() < 3.0), "each of the name's lines is underlined: {row:?} in {lines:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_reply_group_sits_at_the_senders_right_in_a_wide_pane() {
    let (mut a, dir, size, sender) = opened("head-wide", 2000.0, true);
    let ctx = egui::Context::default();
    let group = settled_text_at(&ctx, &mut a, "Reply all", size).expect("the reply group");
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    let from = text_at(&shapes, &sender).expect("the sender");
    assert!((group.y - from.y).abs() < 12.0 && group.x > from.x + 200.0, "at the sender's right: {group:?} {from:?}");
    let assistant = text_at(&shapes, "Assistant").expect("Assistant beside the group");
    assert!((assistant.y - group.y).abs() < 2.0 && assistant.x > group.x);
    for gone in ["Print", "Save", "Summarize"] {
        assert!(text_at(&shapes, gone).is_none(), "{gone} is in a menu now");
    }
    click_at(&ctx, &mut a, assistant, 1, size);
    assert!(settled_text_at(&ctx, &mut a, "Summarize", size).is_some(), "Assistant's menu");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_reply_group_goes_under_the_header_when_the_pane_is_narrower() {
    let (mut a, dir, size, sender) = opened("head-narrow", 1280.0, true);
    let ctx = egui::Context::default();
    let group = settled_text_at(&ctx, &mut a, "Reply all", size).expect("the reply group, with its words");
    let from = text_at(&frame_shapes(&ctx, &mut a, vec![], size), &sender).expect("the sender");
    assert!(group.y > from.y + 30.0, "under the header: {group:?} {from:?}");
    let _ = std::fs::remove_dir_all(dir);
}
