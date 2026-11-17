// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the user's edits and deletions going out, under their message. An edit draws its new
//! words dimmed in the bubble with "Editing…" (Cancel while it waits its turn), a deletion dims the
//! bubble with "Deleting…"; one not made says "Not edited" or "Not deleted", why and Teams' code,
//! with Retry and Cancel. A change whose message the window doesn't hold is drawn after the
//! messages going out, so it can still be let go; a reaction going out is never drawn as a message.
//! With `QUATRELLE_FRAME_PNG` set to a folder, they are painted there, light and dark.

use quatrelle_engine::model::pim::teams::{OutKind, OutState, Unsent};

use super::chat::*;
use super::reactions::{alpha_of, ana, change, click, mine, outbox, settle};
use super::writing::{SIZE, asked};
use super::*;

fn edit_of(message: &str, secs: i64) -> OutKind {
    OutKind::Edit { message: message.into(), created: secs * 1000 }
}

fn deletion_of(message: &str, secs: i64) -> OutKind {
    OutKind::Delete { message: message.into(), created: secs * 1000 }
}

fn refused() -> OutState {
    OutState::NotSent { reason: Unsent::Refused, code: Some("HTTP 403 Forbidden".into()) }
}

/// The user's "hello" edited to "hello there": waiting, the new words dimmed in the bubble and
/// "Editing…", with Cancel; being made, nothing offered; not made, "Not edited", why with Teams'
/// code, Retry and Cancel by its number.
#[test]
fn an_edit_going_out_shows_its_text_and_editing() {
    let (mut a, dir) = super::writing::writing_app("teams-change-edit", vec![ana("m1", NOW - 600, "Can you check?"), mine("m2", NOW - 300, "hello")]);
    let ctx = egui::Context::default();
    outbox(&mut a, vec![change(7, edit_of("m2", NOW - 300), "hello there", OutState::Waiting)]);
    let s = settle(&ctx, &mut a);
    assert!(has_text(&s.texts, "hello there") && !has_text(&s.texts, "hello"), "the new words in its place: {:?}", s.texts);
    assert!(alpha_of(&s.shapes, "hello there").is_some_and(|x| x < 200), "dimmed");
    assert_eq!(alpha_of(&s.shapes, "Can you check?"), Some(255), "the others not");
    assert!(has_text(&s.texts, "Editing…") && !has_text(&s.texts, "Sending…"), "{:?}", s.texts);
    let under = s.text("Editing…").unwrap();
    assert!(under.top() >= s.text("hello there").unwrap().bottom(), "under its message");
    let c = click(&ctx, &mut a, s.text("Cancel").expect("Cancel while it waits its turn").center());
    assert_eq!(asked(&c.acts, "teams.discard"), vec![json!({"seq": 7})]);
    outbox(&mut a, vec![change(7, edit_of("m2", NOW - 300), "hello there", OutState::Sending)]);
    let s = settle(&ctx, &mut a);
    assert!(has_text(&s.texts, "Editing…") && !has_text(&s.texts, "Cancel"), "being made: {:?}", s.texts);
    outbox(&mut a, vec![change(7, edit_of("m2", NOW - 300), "hello there", refused())]);
    let s = settle(&ctx, &mut a);
    for w in ["Not edited", "Teams refused this change (HTTP 403 Forbidden)", "Retry", "Cancel", "hello there"] {
        assert!(has_text(&s.texts, w), "{w}: {:?}", s.texts);
    }
    let c = click(&ctx, &mut a, s.text("Retry").unwrap().center());
    assert_eq!(asked(&c.acts, "teams.retry"), vec![json!({"seq": 7})]);
    let s = settle(&ctx, &mut a);
    let c = click(&ctx, &mut a, s.text("Cancel").unwrap().center());
    assert_eq!(asked(&c.acts, "teams.discard"), vec![json!({"seq": 7})]);
    let unconfirmed = OutState::NotSent { reason: Unsent::Unconfirmed, code: None };
    outbox(&mut a, vec![change(7, edit_of("m2", NOW - 300), "hello there", unconfirmed)]);
    let s = settle(&ctx, &mut a);
    assert!(has_text(&s.texts, "Quatrelle couldn't confirm the change was made"), "{:?}", s.texts);
    let _ = std::fs::remove_dir_all(dir);
}

/// The user's "hello" being deleted: its bubble dimmed and "Deleting…"; not made, "Not deleted",
/// why, Retry and Cancel by its number.
#[test]
fn a_deletion_going_out_dims_the_message() {
    let (mut a, dir) =
        super::writing::writing_app("teams-change-delete", vec![ana("m1", NOW - 600, "Can you check?"), mine("m2", NOW - 300, "hello")]);
    let ctx = egui::Context::default();
    outbox(&mut a, vec![change(8, deletion_of("m2", NOW - 300), "", OutState::Waiting)]);
    let s = settle(&ctx, &mut a);
    assert!(has_text(&s.texts, "Deleting…") && has_text(&s.texts, "hello"), "{:?}", s.texts);
    assert!(alpha_of(&s.shapes, "hello").is_some_and(|x| x < 200), "the bubble dimmed");
    assert_eq!(alpha_of(&s.shapes, "Can you check?"), Some(255));
    outbox(&mut a, vec![change(8, deletion_of("m2", NOW - 300), "", refused())]);
    let s = settle(&ctx, &mut a);
    for w in ["Not deleted", "Teams refused this change (HTTP 403 Forbidden)", "Retry", "Cancel"] {
        assert!(has_text(&s.texts, w), "{w}: {:?}", s.texts);
    }
    let c = click(&ctx, &mut a, s.text("Retry").unwrap().center());
    assert_eq!(asked(&c.acts, "teams.retry"), vec![json!({"seq": 8})]);
    let s = settle(&ctx, &mut a);
    let c = click(&ctx, &mut a, s.text("Cancel").unwrap().center());
    assert_eq!(asked(&c.acts, "teams.discard"), vec![json!({"seq": 8})]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Changes still counted in the conversation's share whose message the window doesn't hold (the
/// window went back to older messages, a refusal kept for the conversation): drawn after the
/// messages going out, an edit with its words, each with Retry and Cancel.
#[test]
fn a_change_of_a_message_not_in_the_window_is_drawn_after_the_sends() {
    let (mut a, dir) = super::writing::writing_app("teams-change-elsewhere", vec![ana("m1", NOW - 600, "Can you check?")]);
    let ctx = egui::Context::default();
    let send = change(4, OutKind::Send, "On its way", OutState::Waiting);
    outbox(
        &mut a,
        vec![send, change(5, edit_of("m-old", NOW - 9000), "Older words", refused()), change(6, deletion_of("m-older", NOW - 9500), "", refused())],
    );
    let s = settle(&ctx, &mut a);
    for w in ["On its way", "Sending…", "Older words", "Not edited", "Not deleted"] {
        assert!(has_text(&s.texts, w), "{w}: {:?}", s.texts);
    }
    let (sent, edited) = (s.text("On its way").unwrap(), s.text("Older words").unwrap());
    assert!(edited.top() > sent.bottom(), "after the messages going out: {sent:?} {edited:?}");
    let cancels: Vec<egui::Rect> = s.texts.iter().filter(|(t, _)| t == "Cancel").map(|(_, r)| *r).collect();
    assert_eq!(cancels.len(), 2, "{:?}", s.texts);
    let mut discarded = Vec::new();
    for at in cancels {
        discarded.extend(asked(&click(&ctx, &mut a, at.center()).acts, "teams.discard"));
        settle(&ctx, &mut a);
    }
    discarded.sort_by_key(|v| v.to_string());
    assert_eq!(discarded, vec![json!({"seq": 5}), json!({"seq": 6})]);
    let _ = std::fs::remove_dir_all(dir);
}

/// A reaction going out shows on its chip, never as a message going out ("Sending…").
#[test]
fn a_reaction_going_out_is_no_message_going_out() {
    let (mut a, dir) = super::writing::writing_app("teams-change-react", vec![ana("m1", NOW - 600, "Lunch at noon")]);
    let ctx = egui::Context::default();
    a.run("teams.react", json!({"chat": CHAT, "message": "m1", "emoji": "\u{1F44D}"}));
    let s = settle(&ctx, &mut a);
    assert!(has_text(&s.texts, "\u{1F44D} 1"), "{:?}", s.texts);
    assert!(!has_text(&s.texts, "Sending…"), "{:?}", s.texts);
    let _ = std::fs::remove_dir_all(dir);
}

/// An edit not made ("Not edited", why, Retry and Cancel) and a deletion going out ("Deleting…",
/// its bubble dimmed), light and dark.
#[test]
fn changes_going_out_in_light_and_dark() {
    let messages = vec![ana("m1", NOW - 900, "Can you check the figures?"), mine("m2", NOW - 600, "hello"), mine("m3", NOW - 300, "See you at ten")];
    let (mut a, dir) = super::writing::writing_app("teams-change-frames", messages);
    outbox(
        &mut a,
        vec![change(7, edit_of("m2", NOW - 600), "hello there", refused()), change(8, deletion_of("m3", NOW - 300), "", OutState::Waiting)],
    );
    let ctx = egui::Context::default();
    let pictures_dir = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let mut tex = Textures::default();
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        a.ui.dark = Some(dark);
        for _ in 0..8 {
            frame_with_size(&ctx, &mut a, &mut tex, vec![], None, SIZE);
        }
        let png = pictures_dir.as_ref().map(|d| d.join(format!("teams-changes-{theme}.png")));
        let (_, shapes) = frame_with_size(&ctx, &mut a, &mut tex, vec![], png, SIZE);
        let texts = texts_of(&shapes);
        for w in ["hello there", "Not edited", "Teams refused this change (HTTP 403 Forbidden)", "Retry", "Cancel", "See you at ten", "Deleting…"] {
            assert!(has_text(&texts, w), "{theme}: {w}: {texts:?}");
        }
    }
    a.ui.dark = Some(false);
    let _ = std::fs::remove_dir_all(dir);
}
