// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
//! Typing right-to-left text into Quatrelle's boxes: egui 0.37 lays a Hebrew or Arabic run out
//! with more places than it has letters ("שלום", four letters, ended at seven: emilk/egui#8576),
//! and Ctrl+Backspace or Alt+Left from there ran a count under zero. The Craft apps' egui fork
//! counts right (the fix of emilk/egui#8698).

use super::*;

/// Hebrew in the search box, then End and Ctrl+Backspace: the last word goes, nothing panics
/// (it did in debug builds; release builds took the whole box).
#[test]
fn deleting_a_word_after_hebrew_takes_one_word() {
    let (mut a, dir) = app("rtl-search");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let word = if cfg!(target_os = "macos") { Modifiers::ALT } else { Modifiers::CTRL };
    for (typed, left) in [("hello world", "hello "), ("שלום עולם", "שלום "), ("مرحبا بكم", "مرحبا ")] {
        a.ui.search_text.clear();
        // The box takes the focus, the words are typed, End, then a word deleted and a word moved over.
        a.ui.focus_search = true;
        for _ in 0..2 {
            frame(&ctx, &mut a, vec![]);
        }
        frame(&ctx, &mut a, vec![Event::Text(typed.into())]);
        assert_eq!(a.ui.search_text, typed, "typed into the search box");
        frame(&ctx, &mut a, vec![key(Key::End, Modifiers::NONE)]);
        frame(&ctx, &mut a, vec![key(Key::Backspace, word)]);
        assert_eq!(a.ui.search_text, left, "one word deleted from {typed:?}");
        frame(&ctx, &mut a, vec![key(Key::End, Modifiers::NONE)]);
        frame(&ctx, &mut a, vec![key(Key::ArrowLeft, word)]);
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Gap 2: a message's Hebrew and Arabic text is selected with the pointer, as it reads, and copied
/// whole (with stock egui 0.37's count, a selection there copied nothing, so it couldn't be
/// selected at all), Latin names, times and dates inside it included. Dragged from the start of
/// the line on the right to its end on the left. (A line ending in a number or Latin word reads it
/// left to right at the far left, so its left edge is that run's start, as in any bidi layout.)
#[test]
fn right_to_left_text_is_selected_and_copied() {
    for text in ["שלום עולם, מה שלומך?", "مرحبا بكم في البريد", "שלום Riley, הפגישה ב-14:30 ביום שלישי"]
    {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 200.0));
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let run = |events: Vec<Event>| {
            let mut out = ctx.run_ui(RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    rect.set(crate::bidi::selectable_label(ui, text, egui::FontId::proportional(16.0), egui::Color32::BLACK).rect);
                });
            });
            out.textures_delta.clear();
            out.platform_output.commands
        };
        run(vec![]);
        run(vec![]);
        let rect = rect.get();
        let (start, end) = (egui::pos2(rect.right() - 1.0, rect.center().y), egui::pos2(rect.left() + 1.0, rect.center().y));
        let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        run(vec![Event::PointerMoved(start), button(start, true)]);
        run(vec![Event::PointerMoved(rect.center())]);
        run(vec![Event::PointerMoved(end)]);
        run(vec![button(end, false)]);
        let copied: Vec<String> = run(vec![Event::Copy])
            .into_iter()
            .filter_map(|c| match c {
                egui::OutputCommand::CopyText(t) => Some(t),
                _ => None,
            })
            .collect();
        assert_eq!(copied, [text.to_string()], "dragged across {text:?}");
    }
}

/// An all-day invitation's day is cut before it is translated: Korean writes `2026. 10. 14.` with
/// spaces, and cutting after kept only the weekday and `2026.`.
#[test]
fn an_all_day_invitations_day_is_whole_in_every_language() {
    let clock = Clock::new(NOW, 0);
    let wed = 1_791_936_000; // 2026-10-14 00:00 UTC
    let english = quatrelle_engine::model::dates::full_time(wed, &clock);
    assert!(english.contains("2026-10-14"), "{english}");
    for (code, day) in [("ko", "2026. 10. 14."), ("fr", "14/10/2026"), ("de", "14.10.2026"), ("en", "2026-10-14")] {
        crate::i18n::set_current(crate::i18n::Lang::from_code(code).unwrap());
        let label = crate::reading::all_day_label(&clock, wed);
        crate::i18n::set_current(crate::i18n::Lang::EN);
        assert!(label.ends_with(day), "{code}: {label}");
        assert!(!label.contains(':'), "{code}: no time of day in {label}");
    }
}

/// A message's header in Hebrew draws its date as one run, `DD.MM.YYYY`. A galley's text is the
/// logical order (egui lays right-to-left out when it draws), so the frame can't show the visual
/// order: it shows the run is whole, in order and with no mark inside it.
#[test]
fn a_hebrew_header_draws_its_date_as_one_run() {
    let (mut a, dir) = app("rtl-date");
    a.ui.language = "he".into();
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let key = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": key.to_id()}));
    let mut shapes = Vec::new();
    for _ in 0..4 {
        shapes = frame_shapes(&ctx, &mut a, vec![], egui::vec2(1280.0, 800.0));
    }
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let texts: Vec<String> = shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect();
    let run = |t: &str| {
        let b = t.as_bytes();
        (0..b.len()).any(|i| {
            b.get(i..i + 10).is_some_and(|w| w.iter().enumerate().all(|(k, c)| if matches!(k, 2 | 5) { *c == b'.' } else { c.is_ascii_digit() }))
        })
    };
    let header = texts.iter().find(|t| run(t) && t.contains(':')).unwrap_or_else(|| panic!("a Hebrew header date in {texts:?}"));
    assert!(header.chars().all(|c| !matches!(c, '\u{2066}'..='\u{2069}')), "no isolate inside the date line: {header}");
    let _ = std::fs::remove_dir_all(dir);
}
