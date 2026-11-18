// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the clipboard in a message being written. A paste is never lost (what can't be
//! pasted with a look goes in as the text pasted); Shift+Insert pastes and Ctrl+Insert copies
//! as Ctrl+V and Ctrl+C do, on every system (egui maps them on Windows alone); a Ctrl+V that
//! brings nothing is noticed (and told in the log).

use super::markdown::{CLIPBOARD_HTML, HTML_READS, clipboard_html, compose, writing};
use super::*;
use quatrelle_engine::mime::rich::List;

const MD: &str = "# Plan\n\n**Bold** and *it*\n\n- one\n- two\n";

/// The editor of the message being written.
fn editor(a: &MailApp) -> &crate::rich_edit::Editor {
    a.ui.editor.as_ref().expect("the message is drawn")
}

/// HTML on the clipboard that gives nothing to paste (no text, a picture from the web or one the
/// message hasn't, and no picture to read off the clipboard): the text pasted goes in as it is,
/// at once or the frame after (once the clipboard is found without a picture).
#[test]
fn a_paste_with_nothing_to_show_goes_in_as_its_text() {
    crate::set_clipboard_html(clipboard_html);
    for html in
        ["<div><img src=\"https://contoso.example/a.png\"></div>", "<p> </p>", "<p><img src=\"cid:not-here\"></p>", "<html><body></body></html>"]
    {
        let (mut a, dir, ctx) = writing("paste-nothing");
        CLIPBOARD_HTML.with(|c| *c.borrow_mut() = Some(html.into()));
        frame(&ctx, &mut a, vec![Event::Paste("Call me at 10".into())]);
        CLIPBOARD_HTML.with(|c| *c.borrow_mut() = None);
        frame(&ctx, &mut a, vec![]);
        assert_eq!(compose(&a).body, "Call me at 10", "{html}");
        frame(&ctx, &mut a, vec![]);
        assert_eq!(compose(&a).body, "Call me at 10", "{html}: once");
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// What the HTML or Markdown path makes of a paste, taken in place of the text box's paste: when
/// it is empty, the text pasted instead, in the look typed at the cursor.
#[test]
fn nothing_made_of_a_paste_gives_the_text_pasted() {
    let typing = quatrelle_engine::mime::rich::Style { bold: true, ..Default::default() };
    let made = ("# Plan".to_string(), quatrelle_engine::mime::rich::Styles::plain("# Plan"));
    let (text, _) = crate::rich_edit::or_as_pasted(made.clone(), "raw", &typing);
    assert_eq!(text, "# Plan", "something made: kept");
    for empty in ["", " \n ", "\u{FFFC}"] {
        let (text, styles) = crate::rich_edit::or_as_pasted((empty.into(), quatrelle_engine::mime::rich::Styles::plain(empty)), "Call me", &typing);
        assert_eq!(text, "Call me", "{empty:?}");
        assert!(styles.style(0).bold && styles.style(6).bold, "in the look typed at the cursor");
    }
}

/// Shift+Insert pastes as Ctrl+V does, on every system: it asks the system for the clipboard
/// (as the menu's Paste does), and what comes goes in formatted when it is Markdown.
#[test]
fn shift_insert_pastes_as_ctrl_v_does() {
    let (mut a, dir, ctx) = writing("paste-shift-insert");
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
    let input = RawInput { screen_rect: Some(screen), events: vec![key(Key::Insert, Modifiers::SHIFT)], ..Default::default() };
    let out = ctx.run_ui(input, |ui| a.ui(ui));
    let asked = out.viewport_output.values().any(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::RequestPaste)));
    assert!(asked, "Shift+Insert asks for the clipboard");
    frame(&ctx, &mut a, vec![Event::Paste(MD.into())]);
    let c = compose(&a);
    let n = c.body.split('\n').position(|l| l == "one").expect("the item");
    assert_eq!(c.styles.para(n).list, List::Bullet, "{:?}", c.body);
    let _ = std::fs::remove_dir_all(dir);
}

/// Ctrl+Insert copies as Ctrl+C does, on every system: the selection, by the editor's own copy
/// (with the look as HTML where the app can put it; its text here).
#[test]
fn ctrl_insert_copies_as_ctrl_c_does() {
    let (mut a, dir, ctx) = writing("copy-ctrl-insert");
    a.run("compose.update", json!({"body": "Plan for Monday"}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
    let mut copied = |events: Vec<Event>| -> Vec<String> {
        let mut out = ctx.run_ui(RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| a.ui(ui));
        out.textures_delta.clear();
        out.platform_output
            .commands
            .into_iter()
            .filter_map(|c| match c {
                egui::OutputCommand::CopyText(t) => Some(t),
                _ => None,
            })
            .collect()
    };
    let by_ctrl_c = copied(vec![Event::Copy]);
    assert_eq!(by_ctrl_c, ["Plan for Monday"]);
    assert_eq!(copied(vec![key(Key::Insert, Modifiers::CTRL)]), by_ctrl_c, "as Ctrl+C");
    assert_eq!(compose(&a).body, "Plan for Monday", "nothing pasted or cut");
    let _ = std::fs::remove_dir_all(dir);
}

/// A Ctrl+V that brings no paste (the system's clipboard couldn't be read) is noticed: egui takes
/// the key's press for itself and sends only its release. One that brings its paste, and a `v`
/// typed, aren't.
#[test]
fn a_ctrl_v_that_brings_no_paste_is_noticed() {
    let release = |modifiers| Event::Key { key: Key::V, physical_key: None, pressed: false, repeat: false, modifiers };
    let (mut a, dir, ctx) = writing("paste-missed");
    frame(&ctx, &mut a, vec![release(Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(editor(&a).pastes.missed, 1, "a Ctrl+V with nothing");
    // Its paste came: as egui sends it, the paste on the press, the release after.
    frame(&ctx, &mut a, vec![Event::Paste("Call me".into())]);
    frame(&ctx, &mut a, vec![release(Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(editor(&a).pastes.missed, 1, "a Ctrl+V with its paste");
    // A `v` typed: its press and its release.
    frame(&ctx, &mut a, vec![key(Key::V, Modifiers::NONE), Event::Text("v".into())]);
    frame(&ctx, &mut a, vec![release(Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(editor(&a).pastes.missed, 1, "a v typed: {:?}", compose(&a).body);
    let _ = std::fs::remove_dir_all(dir);
}

/// The clipboard's HTML is read once a paste (a read can wait on the app that copied), whoever
/// asks for it: the check for a picture alone, the paste, the reading of the picture after the
/// frame and the text that goes in instead the frame after. The next paste reads it again.
#[test]
fn a_paste_reads_the_clipboards_html_once() {
    crate::set_clipboard_html(clipboard_html);
    for html in ["<p><b>From</b> the page</p>", "<div><img src=\"https://contoso.example/a.png\"></div>"] {
        let (mut a, dir, ctx) = writing("paste-html-once");
        CLIPBOARD_HTML.with(|c| *c.borrow_mut() = Some(html.into()));
        HTML_READS.with(|n| n.set(0));
        for paste in ["Call me", "Again"] {
            frame(&ctx, &mut a, vec![Event::Paste(paste.into())]);
            frame(&ctx, &mut a, vec![]);
            frame(&ctx, &mut a, vec![]);
        }
        CLIPBOARD_HTML.with(|c| *c.borrow_mut() = None);
        assert_eq!(HTML_READS.with(std::cell::Cell::get), 2, "once a paste: {html}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Found in review: HTML pasted with thousands of pictures the message hasn't (`cid:` addresses
/// of another message) froze the window, each taken out, and each styled, in a pass over the
/// whole text. The paste goes in with its text and none of them (what it costs is told in
/// `mime`'s `thousands_of_pictures_pasted_with_much_text_cost_a_few_passes`).
#[test]
fn unknown_pictures_pasted_go() {
    crate::set_clipboard_html(clipboard_html);
    let pictures: String = (0..10_000).map(|n| format!("<img src=\"cid:n{n}\">")).collect();
    let words = "Plan for Monday, and the week after. ".repeat(500);
    let (mut a, dir, ctx) = writing("paste-unknown-pictures");
    CLIPBOARD_HTML.with(|c| *c.borrow_mut() = Some(format!("<p>{pictures}</p><p>{words}</p>")));
    frame(&ctx, &mut a, vec![Event::Paste("Plan".into())]);
    CLIPBOARD_HTML.with(|c| *c.borrow_mut() = None);
    let body = compose(&a).body;
    assert!(!body.contains(quatrelle_engine::mime::rich::PICTURE) && body.contains("Plan for Monday, and the week after."), "{:?}", body.get(..80));
    let _ = std::fs::remove_dir_all(dir);
}

/// Found in review: the clipboard's HTML was read whole, however big, by each that looks at it
/// (the check for a picture alone, the paste): it's read no further than a message's HTML.
#[test]
fn the_clipboards_html_is_read_no_further_than_a_messages() {
    crate::set_clipboard_html(clipboard_html);
    let most = quatrelle_engine::mime::HTML_MAX;
    CLIPBOARD_HTML.with(|c| *c.borrow_mut() = Some(format!("<p>{}</p>", "é".repeat(most))));
    crate::rich_edit::forget_clipboard_html();
    let (html, _) = crate::rich_edit::clipboard_html_once();
    CLIPBOARD_HTML.with(|c| *c.borrow_mut() = None);
    crate::rich_edit::forget_clipboard_html();
    let html = html.unwrap_or_default();
    assert_eq!(html.chars().count(), most, "cut where a message's is");
    assert!(html.starts_with("<p>éé"));
}
