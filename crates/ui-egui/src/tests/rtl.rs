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
