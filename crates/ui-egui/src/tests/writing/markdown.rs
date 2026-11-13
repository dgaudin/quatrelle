// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: Markdown pasted into a message being written goes in formatted, and Undo right
//! after gives back the text as it was pasted; a plain-text message, Keep text only, text that
//! isn't Markdown and a clipboard that holds HTML are pasted as before.

use super::*;
use quatrelle_engine::mime::rich::List;

const MD: &str = "# Plan\n\n**Bold** and *it*\n\n- one\n- two\n";

/// A new message being written in the reading pane, its text with the keyboard.
fn writing(name: &str) -> (MailApp, std::path::PathBuf, egui::Context) {
    let (mut a, dir) = app(name);
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ann@contoso.example", "subject": "Plan", "body": ""}));
    frame(&ctx, &mut a, vec![]);
    let id = crate::rich_edit::body_id(a.session.compose.as_ref().unwrap());
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&ctx, &mut a, vec![]);
    (a, dir, ctx)
}

fn compose(a: &MailApp) -> quatrelle_engine::Compose {
    a.session.compose.clone().unwrap()
}

/// The paragraph `line` starts, and its first character's index.
fn line_of(text: &str, line: &str) -> (usize, usize) {
    let mut at = 0;
    for (n, l) in text.split('\n').enumerate() {
        if l == line {
            return (n, at);
        }
        at += l.chars().count() + 1;
    }
    panic!("{line:?} isn't a line of {text:?}");
}

#[test]
fn markdown_pasted_goes_in_formatted_and_undo_gives_its_text_back() {
    let (mut a, dir, ctx) = writing("md-paste");
    frame(&ctx, &mut a, vec![Event::Paste(MD.into())]);
    let c = compose(&a);
    let (_, at) = line_of(&c.body, "Plan");
    assert!(c.styles.style(at).bold && c.styles.style(at).size == Some(20), "a heading: {:?}", c.styles.style(at));
    let (_, at) = line_of(&c.body, "Bold and it");
    assert!(c.styles.style(at).bold && c.styles.style(at + 9).italic);
    let (n, _) = line_of(&c.body, "one");
    assert_eq!(c.styles.para(n).list, List::Bullet);
    // Ctrl+Z right after: the text as it was pasted; once more, as it was before.
    frame(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    assert_eq!(compose(&a).body, MD);
    assert!(compose(&a).styles.is_plain());
    frame(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    assert_eq!(compose(&a).body, "");
    // Ctrl+Y: formatted again.
    frame(&ctx, &mut a, vec![key(Key::Y, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![key(Key::Y, Modifiers::COMMAND)]);
    assert!(compose(&a).body.starts_with("Plan\n"), "{:?}", compose(&a).body);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn markdown_stays_text_where_it_should() {
    // Text that isn't Markdown: as it is.
    let (mut a, dir, ctx) = writing("md-paste-plain");
    frame(&ctx, &mut a, vec![Event::Paste("Call me - tomorrow at 10.".into())]);
    assert_eq!(compose(&a).body, "Call me - tomorrow at 10.");
    let _ = std::fs::remove_dir_all(dir);

    // A plain-text message takes no formatting.
    let (mut a, dir, ctx) = writing("md-paste-plain-message");
    a.run("compose.format", json!({"plain": true}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Paste(MD.into())]);
    assert_eq!(compose(&a).body, MD);
    let _ = std::fs::remove_dir_all(dir);

    // Paste › Keep text only.
    let (mut a, dir, ctx) = writing("md-paste-text-only");
    a.ui.editor.as_mut().expect("the message is drawn").paste_text_only(0..0);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Paste(MD.into())]);
    assert_eq!(compose(&a).body, MD);
    let _ = std::fs::remove_dir_all(dir);
}

thread_local! {
    /// The HTML this test thread's clipboard holds.
    static CLIPBOARD_HTML: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn clipboard_html() -> Option<String> {
    CLIPBOARD_HTML.with(|c| c.borrow().clone())
}

/// With HTML on the clipboard (copied from a web page, with its text), the HTML is pasted, as
/// before: its text being Markdown changes nothing.
#[test]
fn an_html_clipboard_is_pasted_as_before() {
    crate::set_clipboard_html(clipboard_html);
    let (mut a, dir, ctx) = writing("md-paste-html");
    CLIPBOARD_HTML.with(|c| *c.borrow_mut() = Some("<p><b>From</b> the page</p>".into()));
    frame(&ctx, &mut a, vec![Event::Paste(MD.into())]);
    CLIPBOARD_HTML.with(|c| *c.borrow_mut() = None);
    let c = compose(&a);
    assert_eq!(c.body, "From the page");
    assert!(c.styles.style(0).bold && !c.styles.style(5).bold);
    let _ = std::fs::remove_dir_all(dir);
}

/// Markdown copied from where it shows as a code block comes with HTML that is only that block
/// (a web page's `<pre><code>`, an editor's coloured lines): it is pasted formatted, from the
/// clipboard's text. A code block that isn't Markdown keeps the HTML's look, as before (HTML
/// with formatting of its own too: above).
#[test]
fn markdown_copied_as_a_code_block_is_pasted_formatted() {
    crate::set_clipboard_html(clipboard_html);
    let text = "# Titre\n- **a**\n> b";
    let browser = "<meta charset='utf-8'><pre><code># Titre\n- **a**\n&gt; b</code></pre>";
    let editor = "<meta charset='utf-8'><div style=\"color: #d4d4d4;white-space: pre;\"><div><span style=\"color: #569cd6;\"># Titre</span></div><div><span style=\"color: #6796e6;\">-</span> <span style=\"color: #569cd6;\">**a**</span></div><div><span>&gt; b</span></div></div>";
    for html in [browser, editor] {
        let (mut a, dir, ctx) = writing("md-paste-code");
        CLIPBOARD_HTML.with(|c| *c.borrow_mut() = Some(html.into()));
        frame(&ctx, &mut a, vec![Event::Paste(text.into())]);
        CLIPBOARD_HTML.with(|c| *c.borrow_mut() = None);
        let c = compose(&a);
        let (_, at) = line_of(&c.body, "Titre");
        assert!(c.styles.style(at).bold && c.styles.style(at).size == Some(20), "a heading from {html}: {:?}", c.body);
        let (n, at) = line_of(&c.body, "a");
        assert!(c.styles.para(n).list == List::Bullet && c.styles.style(at).bold, "a bold item");
        let (n, _) = line_of(&c.body, "b");
        assert_eq!(c.styles.para(n).indent, 1, "a quote");
        let _ = std::fs::remove_dir_all(dir);
    }
    let (mut a, dir, ctx) = writing("md-paste-code-plain");
    CLIPBOARD_HTML.with(|c| *c.borrow_mut() = Some("<pre><code>let x = 1;</code></pre>".into()));
    frame(&ctx, &mut a, vec![Event::Paste("let x = 1;".into())]);
    CLIPBOARD_HTML.with(|c| *c.borrow_mut() = None);
    let c = compose(&a);
    assert!(c.body == "let x = 1;" && c.styles.style(0).mono, "the code block as before: {:?}", c.body);
    let _ = std::fs::remove_dir_all(dir);
}

/// Pasted with the mouse, Markdown goes in formatted as with Ctrl+V: from the text's
/// right-click menu (Paste asks the system for the clipboard once the text has the keyboard
/// back, and the paste that comes goes through the same hook), and with a middle click (the
/// X11/Wayland selection, which egui puts where the click was).
#[test]
fn markdown_pasted_with_the_mouse_goes_in_formatted() {
    // The right-click menu's Paste.
    let (mut a, dir, ctx) = writing("md-paste-menu");
    a.ui.editor.as_mut().expect("the message is drawn").paste_here(0..0);
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let out = ctx.run_ui(input, |ui| a.ui(ui));
    let asked = out.viewport_output.values().any(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::RequestPaste)));
    assert!(asked, "the menu's Paste asks for the clipboard");
    frame(&ctx, &mut a, vec![Event::Paste(MD.into())]);
    let c = compose(&a);
    let (n, _) = line_of(&c.body, "one");
    assert_eq!(c.styles.para(n).list, List::Bullet, "{:?}", c.body);
    let _ = std::fs::remove_dir_all(dir);

    // A middle click on the text.
    let (mut a, dir, ctx) = writing("md-paste-middle");
    let at = a.ui.editor.as_ref().and_then(|e| e.drawn.as_ref()).map(|d| d.rect.left_top() + egui::vec2(20.0, 8.0)).expect("drawn");
    let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Middle, pressed, modifiers: Modifiers::NONE };
    frame(&ctx, &mut a, vec![Event::PointerMoved(at)]);
    frame(&ctx, &mut a, vec![press(true), Event::MiddleClickPaste { pos: at, text: MD.into() }]);
    frame(&ctx, &mut a, vec![press(false)]);
    let c = compose(&a);
    let (_, start) = line_of(&c.body, "Plan");
    assert!(c.styles.style(start).bold && c.styles.style(start).size == Some(20), "formatted: {:?}", c.body);
    frame(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    assert_eq!(compose(&a).body, MD, "Undo gives back the text as it was pasted");
    // Text that isn't Markdown goes in where the click was, as egui puts it.
    let (mut a, dir2, ctx) = writing("md-paste-middle-plain");
    frame(&ctx, &mut a, vec![Event::PointerMoved(at)]);
    frame(&ctx, &mut a, vec![press(true), Event::MiddleClickPaste { pos: at, text: "just words".into() }]);
    frame(&ctx, &mut a, vec![press(false)]);
    assert_eq!(compose(&a).body, "just words");
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
}

/// This repository's own page on Gmail, pasted whole into a message written in French, in the
/// light and the dark look: each item of its lists keeps its bullet or number (drawn), the
/// lines an item is written over join with one space, nothing of the conversion's marks is
/// left, and each paragraph's indent is laid out once, at its start. (Its English words are
/// misspelt in French: underlined, the indent came again before each one, and left the start,
/// where the text then covered the bullet or the number.)
#[test]
fn a_whole_page_pasted_keeps_its_lists_in_either_look() {
    assert!(crate::spellcheck::load_for_test(), "the dictionaries are built in");
    crate::spellcheck::set_on(true);
    let md = include_str!("../../../../../docs/gmail.md");
    for dark in [false, true] {
        let (mut a, dir, ctx) = writing("md-paste-page");
        a.session.compose.as_mut().expect("written").language = "fr".into();
        a.ui.dark = Some(dark);
        frame(&ctx, &mut a, vec![Event::Paste(md.into())]);
        frame(&ctx, &mut a, vec![]);
        let c = compose(&a);
        assert!(!c.body.chars().any(|ch| ('\u{E000}'..='\u{F8FF}').contains(&ch)), "no marks left: {:?}", c.body);
        assert!(!c.body.contains("  "), "no run of spaces: {:?}", c.body);
        let items: Vec<(usize, List)> =
            (0..c.body.split('\n').count()).map(|n| (n, c.styles.para(n).list)).filter(|(_, l)| *l != List::None).collect();
        assert_eq!(items.iter().filter(|(_, l)| *l == List::Bullet).count(), 8, "{:?}", c.body);
        assert_eq!(items.iter().filter(|(_, l)| *l == List::Number).count(), 2, "{:?}", c.body);
        let (_, at) = line_of(
            &c.body,
            "Sign in with Google, through the browser (OAuth 2.0), in a copy of Quatrelle built with a Google client (oauth.md). The published builds carry none, so they don't offer it.",
        );
        assert!(c.styles.style(at).bold);
        // Drawn: the numbers and the bullets beside their items.
        let shapes = frame_shapes(&ctx, &mut a, vec![], egui::vec2(1280.0, 2400.0));
        let drawn = |s: &str| shapes.iter().filter(|x| matches!(x, egui::Shape::Text(t) if t.galley.text() == s)).count();
        assert!(drawn("1.") >= 1 && drawn("2.") >= 1, "the numbers are drawn");
        assert!(drawn("•") >= 8, "the bullets are drawn: {}", drawn("•"));
        // Laid out: an indent only where a paragraph starts, one for each indented paragraph.
        let galley = a.ui.editor.as_ref().and_then(|e| e.drawn.as_ref()).map(|d| d.galley.clone()).expect("drawn");
        let starts: Vec<usize> = std::iter::once(0).chain(c.body.match_indices('\n').map(|(i, _)| i + 1)).collect();
        let indented = c
            .body
            .split('\n')
            .enumerate()
            .filter(|(n, l)| !l.is_empty() && (c.styles.para(*n).list != List::None || c.styles.para(*n).indent > 0))
            .count();
        let leads: Vec<usize> = galley.job.sections.iter().filter(|s| s.leading_space > 0.0).map(|s| s.byte_range.start.0).collect();
        assert!(leads.iter().all(|at| starts.contains(at)), "an indent inside a paragraph: {leads:?}");
        assert_eq!(leads.len(), indented, "each indented paragraph's indent, once");
        let _ = std::fs::remove_dir_all(dir);
    }
}
