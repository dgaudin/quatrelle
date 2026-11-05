// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a message's source in a window of its own (View message source, issue #27).

use super::*;

/// From the reading pane's More menu's command, a window with the source; asked for again it
/// stays one window, brought to the front; Escape closes it.
#[test]
fn a_source_window_opens_once_and_closes() {
    let (mut a, dir) = app("source-window");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list[0].key.clone();
    a.run("list.open", json!({"key": k.to_id()}));
    a.act(crate::Action::Run("message.source", json!({})));
    assert_eq!(a.ui.raise_source.as_ref(), Some(&k));
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.raise_source.is_none(), "brought to the front once");
    a.act(crate::Action::Run("message.source", json!({"key": k.to_id()})));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.sources.len(), 1);
    assert!(!a.session.sources[0].pending && !a.session.sources[0].lines.is_empty());
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.sources.is_empty(), "Escape closes it");
    // Nothing chosen: said, not a window.
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.act(crate::Action::Run("message.source", json!({})));
    assert!(a.session.sources.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// A source of 30 MB: its window draws quickly, scrolled through and its header alone, without
/// laying out all of it.
#[test]
fn a_huge_source_draws_quickly() {
    let (mut a, dir) = app("source-huge");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list[0].key.clone();
    let mut raw = b"Subject: big\r\nContent-Type: application/octet-stream\r\nContent-Transfer-Encoding: base64\r\n\r\n".to_vec();
    let line = [b'Q'; 76];
    while raw.len() < 30 << 20 {
        raw.extend_from_slice(&line);
        raw.extend_from_slice(b"\r\n");
    }
    // And a line of 2 MB with nothing to break it.
    raw.extend(std::iter::repeat_n(b'x', 2 << 20));
    a.session.store.as_ref().unwrap().write_body(&k, &raw).unwrap();
    let began = std::time::Instant::now();
    a.act(crate::Action::Run("message.source", json!({"key": k.to_id()})));
    let opened = began.elapsed();
    assert!(a.session.sources[0].truncated);
    let began = std::time::Instant::now();
    for i in 0..20 {
        let at = egui::pos2(640.0, 400.0);
        frame(
            &ctx,
            &mut a,
            vec![
                Event::PointerMoved(at),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Page,
                    delta: egui::vec2(0.0, -3.0),
                    modifiers: Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                },
            ],
        );
        if i == 10 {
            a.ui.source_headers.insert(k.clone());
        }
    }
    let drawn = began.elapsed();
    eprintln!("30 MB source: prepared in {opened:?}, 20 frames in {drawn:?}");
    assert!(opened.as_secs() < 5 && drawn.as_secs() < 20, "{opened:?} {drawn:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// What a screen reader is told of: each node's label and where it is.
fn nodes(ctx: &egui::Context, a: &mut MailApp) -> Vec<(String, egui::Rect)> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let tree = out.platform_output.accesskit_update.expect("AccessKit is on");
    tree.nodes
        .iter()
        .filter_map(|(_, n)| {
            let b = n.bounds()?;
            let label = n.label().or(n.value()).unwrap_or_default().to_string();
            Some((label, egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32))))
        })
        .collect()
}

fn at(ctx: &egui::Context, a: &mut MailApp, label: &str) -> egui::Pos2 {
    let found = nodes(ctx, a);
    found
        .iter()
        .find(|(l, _)| l == label)
        .map(|(_, r)| r.center())
        .unwrap_or_else(|| panic!("{label}: {:?}", found.iter().map(|(l, _)| l).collect::<Vec<_>>()))
}

fn press(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2) {
    let b = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(ctx, a, vec![Event::PointerMoved(at)]);
    frame(ctx, a, vec![b(true)]);
    frame(ctx, a, vec![b(false)]);
    frame(ctx, a, vec![]);
}

/// Where Outlook has it: the reading pane's ⋯ › View › View message source. Its window has the
/// fork's window bar (Whole message | Header only, Copy all with its icon) over the source's lines,
/// and Header only leaves the body out.
#[test]
fn the_source_from_more_actions_view_in_the_forks_window() {
    let (mut a, dir) = app("source-from-more");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let k = a.session.list[0].key.clone();
    a.run("list.open", json!({"key": k.to_id()}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let more = at(&ctx, &mut a, "More actions");
    press(&ctx, &mut a, more);
    // The menu's View, not the ribbon's tab (the one further right).
    let views: Vec<egui::Pos2> = nodes(&ctx, &mut a).into_iter().filter(|(l, _)| l == "View").map(|(_, r)| r.center()).collect();
    let view = views.into_iter().max_by(|a, b| a.x.total_cmp(&b.x)).expect("the menu's View");
    press(&ctx, &mut a, view);
    let item = at(&ctx, &mut a, "View message source");
    press(&ctx, &mut a, item);
    assert_eq!(a.session.sources.len(), 1, "the window is open");
    assert_eq!(a.session.sources[0].key, k);
    let size = egui::vec2(1280.0, 800.0);
    let mut shapes = Vec::new();
    for _ in 0..10 {
        shapes = frame_shapes(&ctx, &mut a, vec![], size);
    }
    let whole = text_at(&shapes, "Whole message").expect("the bar's choice");
    let header = text_at(&shapes, "Header only").expect("the bar's choice");
    let copy = text_at(&shapes, "Copy all").expect("Copy all");
    assert!((whole.y - copy.y).abs() < 3.0 && header.x > whole.x && copy.x > header.x, "one bar: {whole:?} {header:?} {copy:?}");
    let lines = &a.session.sources[0].lines;
    let subject = lines.iter().find(|l| l.starts_with("Subject:")).cloned().expect("a subject line");
    let line = text_at(&shapes, &subject).expect("the source's lines are drawn");
    assert!(line.y > copy.y, "under the bar");
    let body = lines.get(a.session.sources[0].header_lines + 1..).and_then(|b| b.iter().find(|l| l.trim().len() > 3)).cloned().expect("a body line");
    // Header only: the body goes.
    let header_at = header;
    press(&ctx, &mut a, header_at);
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    assert!(a.ui.source_headers.contains(&k));
    assert!(text_at(&shapes, &subject).is_some() && text_at(&shapes, &body).is_none(), "{body}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The list's right-click menu has it too, under View, for the message right-clicked.
#[test]
fn the_source_from_the_lists_menu() {
    let (mut a, dir) = app("source-from-list");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let k = a.session.row_keys()[1].clone();
    let subject = a.session.summary(&k).map(|m| m.subject.clone()).unwrap();
    let row = nodes(&ctx, &mut a).into_iter().find(|(l, _)| l.contains(&format!(", {subject}, "))).map(|(_, r)| r.center()).expect("its row");
    let b = |pressed| Event::PointerButton { pos: row, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
    frame(&ctx, &mut a, vec![Event::PointerMoved(row)]);
    frame(&ctx, &mut a, vec![b(true)]);
    frame(&ctx, &mut a, vec![b(false)]);
    frame(&ctx, &mut a, vec![]);
    // The menu's View, not the ribbon's tab (the lowest of them).
    let views: Vec<egui::Rect> = nodes(&ctx, &mut a).into_iter().filter(|(l, _)| l == "View").map(|(_, r)| r).collect();
    assert!(views.len() > 1, "the menu is open: {views:?}");
    let view = views.iter().max_by(|x, y| x.min.y.total_cmp(&y.min.y)).map(|r| r.center()).unwrap();
    press(&ctx, &mut a, view);
    let item = at(&ctx, &mut a, "View message source");
    press(&ctx, &mut a, item);
    assert_eq!(a.session.sources.iter().map(|v| v.key.clone()).collect::<Vec<_>>(), vec![k]);
    let _ = std::fs::remove_dir_all(dir);
}

/// The window is filled with lines to its foot: the rows laid out are counted at the height
/// they're drawn at (they were counted with the spacing under each, which the lines don't
/// have, and the window's lower third stayed empty).
#[test]
fn a_source_fills_its_window() {
    let v =
        quatrelle_engine::SourceView { lines: (0..400).map(|i| format!("Line {i}")).collect(), header_lines: 3, size: 4000, ..Default::default() };
    let t = crate::theme::Tokens::light();
    let ctx = egui::Context::default();
    let height = 600.0;
    let mut shown = 0..0;
    let mut row = 0.0;
    for _ in 0..2 {
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, height))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| {
                row = ui.fonts_mut(|f| f.row_height(&egui::FontId::monospace(12.5)));
                shown = crate::app::source::source_text(ui, &t, &v, false);
            });
        });
        out.textures_delta.clear();
    }
    assert_eq!(shown.start, 0);
    assert!(shown.len() as f32 * row >= height - row, "{} lines of {row} in {height}", shown.len());
}
