// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! An HTML message shown as Markdown: offered beside the way back to its formatting, drawn as
//! plain text in the monospace font (in the light and the dark look), its links cleaned as the
//! formatted view shows them, and copied as the Markdown it shows.

use super::*;

const HTML: &str = "<h1>Title</h1><p>Some <b>bold</b> and <a href=\"https://shop.contoso.example/sale?utm_source=news&amp;id=7\">a sale</a>.</p><ul><li>one</li><li>two</li></ul>";

/// The text shapes drawn in the reading pane, by their text.
fn drawn(ctx: &egui::Context, a: &mut MailApp) -> Vec<(String, egui::FontFamily)> {
    frame_shapes(ctx, a, vec![], SCREEN)
        .into_iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.galley.job.sections.first()?.format.font_id.family.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn an_html_message_shows_as_markdown_and_copies_it() {
    let (mut a, dir) = app_with_html("markdown-view", "Markdown view", HTML);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    tall(&ctx, &mut a, vec![]);
    let key = a.session.list.iter().find(|m| m.subject == "Markdown view").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": key.to_id()}));
    for _ in 0..3 {
        tall(&ctx, &mut a, vec![]);
    }
    // Formatted, More actions offers Markdown; shown as text, the line above it does, beside the
    // way back to the formatting.
    click_named(&ctx, &mut a, "More actions");
    assert!(shows(&ctx, &mut a, "Show as Markdown"), "in More actions");
    tall(&ctx, &mut a, vec![key_event(Key::Escape, Modifiers::NONE)]);
    a.act(crate::Action::ShowFormatted(key.clone(), false));
    tall(&ctx, &mut a, vec![]);
    assert!(shows(&ctx, &mut a, "Shown as text.") && shows(&ctx, &mut a, "Show formatted"));
    click_named(&ctx, &mut a, "Show as Markdown");
    tall(&ctx, &mut a, vec![]);
    assert!(shows(&ctx, &mut a, "Shown as Markdown."), "shown as Markdown");
    assert!(shows(&ctx, &mut a, "Show as plain text") && shows(&ctx, &mut a, "Show formatted"));
    let md = "# Title\n\nSome **bold** and [a sale](https://shop.contoso.example/sale?id=7).\n- one\n- two";
    // In the monospace font, in either look.
    for dark in [false, true] {
        a.ui.dark = Some(dark);
        tall(&ctx, &mut a, vec![]);
        let shapes = drawn(&ctx, &mut a);
        let shown = shapes.iter().find(|(t, _)| t.contains("# Title"));
        assert!(shown.is_some_and(|(t, f)| t == md && *f == egui::FontFamily::Monospace), "{shown:?}");
    }
    // Selected whole and copied: the Markdown.
    let at = text_at(&frame_shapes(&ctx, &mut a, vec![], SCREEN), md).expect("drawn");
    press(&ctx, &mut a, at);
    tall(&ctx, &mut a, vec![key_event(Key::A, Modifiers::COMMAND)]);
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)), events: vec![Event::Copy], ..Default::default() };
    let out = ctx.run_ui(input, |ui| a.ui(ui));
    let copied = out.platform_output.commands.into_iter().rev().find_map(|c| match c {
        egui::OutputCommand::CopyText(t) => Some(t),
        _ => None,
    });
    assert_eq!(copied.as_deref(), Some(md));
    // Show formatted goes back, Markdown let go.
    click_named(&ctx, &mut a, "Show formatted");
    tall(&ctx, &mut a, vec![]);
    assert!(!shows(&ctx, &mut a, "Shown as Markdown.") && !shows(&ctx, &mut a, "Shown as text."));
    let _ = std::fs::remove_dir_all(dir);
}

fn key_event(k: Key, modifiers: Modifiers) -> Event {
    Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers }
}
