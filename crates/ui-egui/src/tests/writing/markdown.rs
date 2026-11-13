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
