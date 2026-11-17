// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: editing one's own message from the box. ↑ in an empty box loads the user's last
//! message (its bold kept) under "Editing message"; Enter sends the edit, Esc or Cancel lets it
//! go, and the draft set aside comes back either way. A last message the box can't hold is said,
//! and nothing older is loaded; an edit that changes nothing sends nothing, one to nothing says to
//! delete it; one the engine refuses comes back in the box, still an edit. Esc closes the offer
//! first, then cancels the edit, and leaves the conversations' search as it is. With
//! `QUATRELLE_FRAME_PNG` set to a folder, the box editing is painted there, light and dark.

use quatrelle_engine::model::pim::teams::{Attachment, ChatMessage, Run};
use serde_json::Value;

use super::chat::*;
use super::writing::{SIZE, asked, enter, focus_box, in_box, one_frame, sends, typed, writing_app};
use super::*;
use crate::Action;

fn text(s: &str) -> Event {
    Event::Text(s.into())
}

fn up() -> Event {
    key(Key::ArrowUp, Modifiers::NONE)
}

fn esc() -> Event {
    key(Key::Escape, Modifiers::NONE)
}

/// Ana's question.
fn from_ana(id: &str, secs: i64) -> ChatMessage {
    said(id, user("u-ana", "Ana Lima"), secs, para(vec![words("Can you check the figures?")]))
}

/// The user's "hello", its "hel" bold.
fn hello() -> ChatMessage {
    said("m2", user(ME, "Riley Moreno"), NOW - 300, para(vec![Run { text: "hel".into(), bold: true, ..Default::default() }, words("lo")]))
}

/// The edits the box asked for.
fn edits(acts: &[Action]) -> Vec<Value> {
    asked(acts, "teams.edit")
}

fn editing(a: &MailApp) -> Option<String> {
    a.ui.conversations.drafts.get(CHAT).and_then(|d| d.editing().map(str::to_string))
}

fn said_in_box(a: &MailApp) -> Option<&'static str> {
    a.ui.conversations.drafts.get(CHAT).and_then(|d| d.said())
}

/// The user's last message "hello" (Ana wrote after it): ↑ in the empty box loads it, its bold
/// kept, under "Editing message"; " there" and Enter send the edit; the box is empty after.
#[test]
fn up_in_an_empty_box_edits_the_last_message() {
    let (mut a, dir) = writing_app("teams-edit-up", vec![from_ana("m1", NOW - 600), hello(), from_ana("m3", NOW - 60)]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    typed(&ctx, &mut a, vec![up()]);
    assert_eq!(in_box(&a), "hello");
    assert_eq!(editing(&a).as_deref(), Some("m2"));
    let d = a.ui.conversations.drafts.get(CHAT).expect("the box's draft");
    let bold: Vec<bool> = (0..5).map(|i| d.styles.style(i).bold).collect();
    assert_eq!(bold, [true, true, true, false, false], "its bold kept");
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    let banner = rect_of(&texts, |t| t == "Editing message").expect("Editing message");
    // The message's bubble says "hello" too: the box's is the lowest.
    let written = texts.iter().filter(|(t, _)| t == "hello").map(|(_, r)| *r).max_by(|x, y| x.top().total_cmp(&y.top())).expect("the box's text");
    assert!(banner.bottom() <= written.top(), "over the box: {banner:?} {written:?}");
    let acts = typed(&ctx, &mut a, vec![text(" there"), enter()]);
    assert!(sends(&acts).is_empty(), "an edit, not a message");
    assert_eq!(edits(&acts), vec![json!({"chat": CHAT, "message": "m2", "text": "hello there", "bold": [[0, 3]], "italic": [], "mentions": []})]);
    assert_eq!(in_box(&a), "");
    assert_eq!(editing(&a), None);
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    assert!(!has_text(&texts, "Editing message"), "{texts:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Review Focus 5: the user's last message carries a file, so the box can't hold it: ↑ edits
/// nothing (not the older "hello" either) and says so under the box.
#[test]
fn up_when_the_last_cant_be_edited_says_so() {
    let mut with_file = said("m4", user(ME, "Riley Moreno"), NOW - 30, para(vec![words("Here it is")]));
    with_file.attachments = vec![Attachment::File { id: "f1".into(), name: "figures.xlsx".into(), url: String::new(), size: Some(2048) }];
    let (mut a, dir) = writing_app("teams-edit-up-not", vec![hello(), with_file]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    typed(&ctx, &mut a, vec![up()]);
    assert_eq!(editing(&a), None, "nothing edited");
    assert_eq!(in_box(&a), "", "the older message isn't loaded");
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    assert!(has_text(&texts, "Your last message can't be edited here"), "{texts:?}");
    assert!(!has_text(&texts, "Editing message"), "{texts:?}");
    // Said until the text changes.
    typed(&ctx, &mut a, vec![text("ok")]);
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    assert!(!has_text(&texts, "Your last message can't be edited here"), "{texts:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// What was in the box is set aside while a message is edited (asked for from the message:
/// `edit_asked`), and comes back when the edit is cancelled with Esc, or sent.
#[test]
fn the_draft_comes_back_after_an_edit() {
    let (mut a, dir) = writing_app("teams-edit-draft", vec![hello()]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    typed(&ctx, &mut a, vec![text("unsent words")]);
    a.ui.conversations.edit_asked = Some("m2".into());
    typed(&ctx, &mut a, vec![]);
    assert_eq!(in_box(&a), "hello");
    assert_eq!(editing(&a).as_deref(), Some("m2"));
    assert_eq!(a.ui.conversations.edit_asked, None, "taken");
    typed(&ctx, &mut a, vec![esc()]);
    assert_eq!(in_box(&a), "unsent words");
    assert_eq!(editing(&a), None);
    // Sent, the draft comes back too, and typing goes on at its end.
    a.ui.conversations.edit_asked = Some("m2".into());
    typed(&ctx, &mut a, vec![]);
    let acts = typed(&ctx, &mut a, vec![text("!"), enter()]);
    assert_eq!(edits(&acts).len(), 1);
    typed(&ctx, &mut a, vec![text(" more")]);
    assert_eq!(in_box(&a), "unsent words more");
    // A message that can't be edited here, asked for: nothing changes.
    a.ui.conversations.edit_asked = Some("m1".into());
    typed(&ctx, &mut a, vec![]);
    assert_eq!((in_box(&a).as_str(), editing(&a)), ("unsent words more", None));
    let _ = std::fs::remove_dir_all(dir);
}

/// Edit on the message the box is editing already keeps what was typed in it, and typing goes on;
/// Edit on another of the user's messages edits that one instead (what was typed in the first is
/// let go), and the draft set aside still comes back after.
#[test]
fn edit_on_the_message_being_edited_keeps_what_was_typed() {
    let see_you = said("m3", user(ME, "Riley Moreno"), NOW - 60, para(vec![words("see you")]));
    let (mut a, dir) = writing_app("teams-edit-again", vec![hello(), see_you]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    typed(&ctx, &mut a, vec![text("draft")]);
    a.ui.conversations.edit_asked = Some("m2".into());
    typed(&ctx, &mut a, vec![]);
    typed(&ctx, &mut a, vec![text(" there")]);
    assert_eq!(in_box(&a), "hello there");
    a.ui.conversations.edit_asked = Some("m2".into());
    typed(&ctx, &mut a, vec![]);
    assert_eq!((in_box(&a).as_str(), editing(&a).as_deref()), ("hello there", Some("m2")), "what was typed is kept");
    typed(&ctx, &mut a, vec![text("!")]);
    assert_eq!(in_box(&a), "hello there!", "typing goes on at its end");
    a.ui.conversations.edit_asked = Some("m3".into());
    typed(&ctx, &mut a, vec![]);
    assert_eq!((in_box(&a).as_str(), editing(&a).as_deref()), ("see you", Some("m3")), "the other one edited");
    typed(&ctx, &mut a, vec![esc()]);
    assert_eq!((in_box(&a).as_str(), editing(&a)), ("draft", None), "the draft back");
    let _ = std::fs::remove_dir_all(dir);
}

/// An edit that changes nothing sends nothing and lets go of the edit, also when the words typed
/// were taken back; an edit to nothing isn't sent: the box says to delete the message, still
/// editing it.
#[test]
fn an_unchanged_edit_sends_nothing_and_an_empty_one_says_delete_it() {
    let (mut a, dir) = writing_app("teams-edit-unchanged", vec![hello()]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    typed(&ctx, &mut a, vec![up()]);
    let acts = typed(&ctx, &mut a, vec![enter()]);
    assert!(edits(&acts).is_empty() && sends(&acts).is_empty(), "nothing sent");
    assert_eq!((in_box(&a).as_str(), editing(&a)), ("", None));
    let back = key(Key::Backspace, Modifiers::NONE);
    typed(&ctx, &mut a, vec![up(), text(" x"), back.clone(), back.clone()]);
    assert_eq!(in_box(&a), "hello");
    let acts = typed(&ctx, &mut a, vec![enter()]);
    assert!(edits(&acts).is_empty(), "the same words: nothing sent");
    assert_eq!(editing(&a), None);
    typed(&ctx, &mut a, vec![up(), key(Key::A, Modifiers::COMMAND), back]);
    assert_eq!(in_box(&a), "");
    let acts = typed(&ctx, &mut a, vec![enter()]);
    assert!(edits(&acts).is_empty(), "nothing left: not sent");
    assert_eq!(editing(&a).as_deref(), Some("m2"), "still editing");
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    assert!(has_text(&texts, "To remove a message, delete it"), "{texts:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Decision 17: an edit the engine refuses comes back in the box, still an edit of its message,
/// the caret at the text's end, and why is said; the draft set aside stays set aside. Nothing
/// left of it, as the engine says, is said in the box's words.
#[test]
fn a_refused_edit_comes_back_in_the_box() {
    let (mut a, dir) = writing_app("teams-edit-refused", vec![hello()]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    typed(&ctx, &mut a, vec![text("draft")]);
    a.act(Action::Run("teams.edit", json!({"chat": CHAT, "message": "m2", "text": "  "})));
    assert_eq!((in_box(&a).as_str(), editing(&a).as_deref()), ("  ", Some("m2")));
    assert_eq!(said_in_box(&a), Some("To remove a message, delete it"));
    // A frame for the box to take its keys as an edit's, then Esc lets it go.
    typed(&ctx, &mut a, vec![]);
    typed(&ctx, &mut a, vec![esc()]);
    assert_eq!((in_box(&a).as_str(), editing(&a)), ("draft", None));
    a.ui.conversations.edit_asked = Some("m2".into());
    typed(&ctx, &mut a, vec![text(" there")]);
    assert_eq!(in_box(&a), "hello there");
    // Deleted meanwhile (from the message): the engine no longer takes an edit of it. The window
    // runs what the box asked for at the end of the same frame.
    a.run("teams.delete", json!({"chat": CHAT, "message": "m2"}));
    let acts = pane(&ctx, &mut a, vec![enter()], SIZE).0;
    assert_eq!(edits(&acts).len(), 1);
    for x in acts {
        a.act(x);
    }
    assert_eq!(in_box(&a), "hello there");
    assert_eq!(editing(&a).as_deref(), Some("m2"), "still an edit");
    assert_eq!(said_in_box(&a), Some("This message couldn't be edited"));
    typed(&ctx, &mut a, vec![text("!")]);
    assert_eq!(in_box(&a), "hello there!", "the caret at the text's end");
    // Still judged against the message's own words: put back as they were, nothing is sent, and
    // the draft set aside comes back.
    let back = key(Key::Backspace, Modifiers::NONE);
    let acts = typed(&ctx, &mut a, std::iter::repeat_n(back, 7).chain([enter()]).collect());
    assert!(edits(&acts).is_empty(), "{:?}", edits(&acts));
    assert_eq!((in_box(&a).as_str(), editing(&a)), ("draft", None), "the draft set aside comes back");
    let _ = std::fs::remove_dir_all(dir);
}

/// Review Focus 5: the conversations' search "agenda" set, a message edited with "@a" typed: Esc
/// closes the offer (still editing), Esc cancels the edit (the draft back), the search kept each
/// time; Esc once more empties it, as before.
#[test]
fn esc_closes_the_offer_then_cancels_the_edit_and_keeps_the_search() {
    let (mut a, dir) = writing_app("teams-edit-esc", vec![hello()]);
    a.ui.conversations.query = "agenda".into();
    a.run("teams.search", json!({"query": "agenda"}));
    let ctx = egui::Context::default();
    let size = egui::vec2(SIZE.0 as f32, SIZE.1 as f32);
    let hint = settled_text_at(&ctx, &mut a, "Write a message", size).expect("the box");
    click_at(&ctx, &mut a, hint, 1, size);
    let step = |a: &mut MailApp, e: Event| {
        frame_sized(&ctx, a, vec![e], size);
        frame_sized(&ctx, a, vec![], size);
    };
    step(&mut a, up());
    assert_eq!(editing(&a).as_deref(), Some("m2"));
    for e in [text(" "), text("@"), text("a")] {
        step(&mut a, e);
    }
    let texts = texts_of(&frame_shapes(&ctx, &mut a, vec![], size));
    assert!(has_text(&texts, "Ana Lima"), "the offer: {texts:?}");
    step(&mut a, esc());
    let texts = texts_of(&frame_shapes(&ctx, &mut a, vec![], size));
    assert!(!has_text(&texts, "Ana Lima"), "the offer closed: {texts:?}");
    assert_eq!(editing(&a).as_deref(), Some("m2"), "still editing");
    assert_eq!(in_box(&a), "hello @a");
    assert_eq!(a.ui.conversations.query, "agenda");
    step(&mut a, esc());
    assert_eq!((in_box(&a).as_str(), editing(&a)), ("", None), "the edit cancelled, the draft back");
    assert_eq!(a.ui.conversations.query, "agenda");
    step(&mut a, esc());
    assert_eq!(a.ui.conversations.query, "", "not taken by the box: Esc empties the search");
    let _ = std::fs::remove_dir_all(dir);
}

/// An Esc right after Enter in the same frame (the T10 note): the edit goes with Enter, and the
/// Esc, replayed at the next frame and not the box's any more, lets go of the keyboard as an Esc
/// does.
#[test]
fn an_esc_after_enter_in_the_same_frame_lets_go_of_the_keyboard() {
    let (mut a, dir) = writing_app("teams-edit-esc-replayed", vec![hello()]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    typed(&ctx, &mut a, vec![up(), text("!")]);
    let acts = one_frame(&ctx, &mut a, vec![enter(), esc()], 3);
    assert_eq!(edits(&acts).len(), 1);
    typed(&ctx, &mut a, vec![text("x")]);
    assert_eq!(in_box(&a), "", "the box let go of the keyboard");
    let _ = std::fs::remove_dir_all(dir);
}

/// The box editing a message, light and dark: "Editing message" and Cancel over it, the text's
/// bold drawn; Cancel lets go of the edit.
#[test]
fn the_edit_in_light_and_dark() {
    let (mut a, dir) = writing_app("teams-edit-frames", vec![from_ana("m1", NOW - 600), hello()]);
    let ctx = egui::Context::default();
    let size = egui::vec2(SIZE.0 as f32, SIZE.1 as f32);
    let hint = settled_text_at(&ctx, &mut a, "Write a message", size).expect("the box");
    click_at(&ctx, &mut a, hint, 1, size);
    frame_sized(&ctx, &mut a, vec![up()], size);
    frame_sized(&ctx, &mut a, vec![text(" there")], size);
    let pictures_dir = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let mut tex = Textures::default();
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        a.ui.dark = Some(dark);
        for _ in 0..6 {
            frame_with_size(&ctx, &mut a, &mut tex, vec![], None, SIZE);
        }
        let png = pictures_dir.as_ref().map(|d| d.join(format!("teams-box-editing-{theme}.png")));
        let (_, shapes) = frame_with_size(&ctx, &mut a, &mut tex, vec![], png, SIZE);
        let texts = texts_of(&shapes);
        let banner = rect_of(&texts, |t| t == "Editing message").expect("Editing message");
        let cancel = rect_of(&texts, |t| t == "Cancel").expect("Cancel");
        let written = rect_of(&texts, |t| t == "hello there").expect("the box's text");
        let bubble = rect_of(&texts, |t| t == "hello").expect("the message itself");
        assert!(bubble.bottom() <= banner.top(), "{theme}: {bubble:?} {banner:?}");
        assert!(banner.bottom() <= written.top() && cancel.bottom() <= written.top(), "{theme}: {banner:?} {cancel:?} {written:?}");
        assert!((cancel.center().y - banner.center().y).abs() < 6.0, "{theme}: on one line: {banner:?} {cancel:?}");
    }
    a.ui.dark = Some(false);
    let texts = texts_of(&frame_shapes(&ctx, &mut a, vec![], size));
    let cancel = rect_of(&texts, |t| t == "Cancel").expect("Cancel");
    click_at(&ctx, &mut a, cancel.center(), 1, size);
    assert_eq!((in_box(&a).as_str(), editing(&a)), ("", None), "Cancel lets go of the edit");
    let _ = std::fs::remove_dir_all(dir);
}
