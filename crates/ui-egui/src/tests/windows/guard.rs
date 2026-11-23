// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the guard around each frame of the main window. A panic in what a frame draws or
//! does is said and Quatrelle goes on; frames that panic three in a row keep what's being written
//! in Drafts, and only then is Quatrelle let go. The frames here are run as `apps/quatrelle` runs
//! them (`logic`, then the drawing inside egui's pass, each under `catch_unwind`), and the panic
//! is made by a hook the tests set (`UiState::test_panic`), read at named places in the drawing.

use std::panic::{AssertUnwindSafe, catch_unwind};

use super::*;
use crate::app::PanicAt;

/// A frame as the app runs it, drawing the sample's window: `logic`, then `ui` unless the window is
/// hidden (`hidden`), each under the guard. Whether the guard said Quatrelle must end now.
pub(super) fn guarded_frame(ctx: &egui::Context, a: &mut MailApp, hidden: bool) -> bool {
    a.frame_part_starts(crate::FramePart::Logic);
    let ran = catch_unwind(AssertUnwindSafe(|| a.logic(ctx)));
    let ended = a.frame_part_ended(ctx, crate::FramePart::Logic, ran);
    if ended || hidden {
        return ended;
    }
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut ended = false;
    let mut out = ctx.run_ui(input, |ui| {
        a.frame_part_starts(crate::FramePart::Ui);
        let ran = catch_unwind(AssertUnwindSafe(|| a.ui(ui)));
        ended = a.frame_part_ended(ui.ctx(), crate::FramePart::Ui, ran);
    });
    // No renderer here: the texture uploads are dropped deliberately.
    out.textures_delta.clear();
    ended
}

/// The app with a message half written (not saved anywhere), and the context its frames run in.
fn writing(name: &str) -> (MailApp, std::path::PathBuf, egui::Context) {
    let (mut a, dir) = app(name);
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "Not lost", "body": "Written before the window failed."}));
    (a, dir, ctx)
}

/// How many messages called "Not lost" Drafts holds in the store (read there: opening the folder
/// would keep the message being written, as leaving it does).
fn kept(a: &MailApp) -> usize {
    let store = a.session.store.as_ref().expect("the sample has a store");
    let drafts = store.load_folder("sample", "Drafts").expect("Drafts reads");
    drafts.messages.iter().filter(|m| m.subject == "Not lost").count()
}

/// A dialog that panics in a frame (its button, say) no longer ends Quatrelle with the message
/// being written: the panic is said, the dialog is put away, the message is as it was, and the
/// next frame draws. Drawn without the guard, the panic ends the test.
#[test]
fn a_panic_in_a_frame_is_said_and_quatrelle_goes_on() {
    let (mut a, dir, ctx) = writing("frame-guard");
    a.ui.dialog = Some(Dialog::add_account_with("zoe@studio.example", false));
    a.ui.test_panic = Some(PanicAt::Dialog);
    assert!(!guarded_frame(&ctx, &mut a, false), "one panic doesn't end Quatrelle");
    assert!(a.ui.dialog.is_none(), "the dialog on top is put away");
    let said =
        a.session.notices.iter().filter(|n| n.error && n.text.starts_with("Something went wrong in the window, and Quatrelle went on")).count();
    assert_eq!(said, 1, "the panic is said, once");
    assert_eq!(a.session.all_composes().count(), 1, "the message being written is still open");
    assert_eq!(a.session.compose.as_ref().map(|c| c.subject.as_str()), Some("Not lost"));
    // The next frame draws as before.
    a.ui.test_panic = None;
    assert!(!guarded_frame(&ctx, &mut a, false));
    assert_eq!(a.session.compose.as_ref().map(|c| c.subject.as_str()), Some("Not lost"));
    let _ = std::fs::remove_dir_all(dir);
}

/// Three frames in a row that panic keep what's being written in Drafts (the way a lost GPU does,
/// at once, in the store), say so, and only then does the guard say Quatrelle must end.
#[test]
fn three_panics_running_keep_the_drafts_then_stop() {
    let (mut a, dir, ctx) = writing("frame-guard-three");
    a.ui.test_panic = Some(PanicAt::Dialog);
    for pass in 1..=2 {
        a.ui.dialog = Some(Dialog::add_account_with("zoe@studio.example", false));
        assert!(!guarded_frame(&ctx, &mut a, false), "panic {pass} doesn't end Quatrelle");
    }
    assert_eq!(kept(&a), 0, "nothing is filed while Quatrelle goes on");
    a.ui.dialog = Some(Dialog::add_account_with("zoe@studio.example", false));
    assert!(guarded_frame(&ctx, &mut a, false), "the third in a row ends Quatrelle");
    assert_eq!(kept(&a), 1, "what was being written is in Drafts by then");
    let closing = "Quatrelle has to close after an error it couldn't get past. What you were writing is kept in Drafts";
    assert!(a.session.notices.iter().any(|n| n.error && n.text == closing), "and it is said");
    // Let go, it stays let go, and the drafts aren't filed twice.
    assert!(guarded_frame(&ctx, &mut a, false));
    assert_eq!(kept(&a), 1);
    let _ = std::fs::remove_dir_all(dir);
}

/// A frame drawn whole between panics starts the count again: Quatrelle only ends for panics that
/// follow one another.
#[test]
fn a_frame_drawn_whole_starts_the_count_again() {
    let (mut a, dir, ctx) = writing("frame-guard-again");
    for _ in 0..3 {
        a.ui.test_panic = Some(PanicAt::Dialog);
        a.ui.dialog = Some(Dialog::add_account_with("zoe@studio.example", false));
        assert!(!guarded_frame(&ctx, &mut a, false));
        a.ui.test_panic = None;
        assert!(!guarded_frame(&ctx, &mut a, false), "a frame drawn whole");
    }
    assert_eq!(kept(&a), 0, "nothing was let go, nothing kept");
    let _ = std::fs::remove_dir_all(dir);
}

/// A hidden window runs `logic` alone: one that panics every time is let go too, with what's being
/// written kept first, and one that runs cleanly ends a run as a drawn frame does.
#[test]
fn a_hidden_windows_frames_count_too() {
    let (mut a, dir, ctx) = writing("frame-guard-hidden");
    a.ui.test_panic = Some(PanicAt::Logic);
    assert!(!guarded_frame(&ctx, &mut a, true) && !guarded_frame(&ctx, &mut a, true));
    a.ui.test_panic = None;
    assert!(!guarded_frame(&ctx, &mut a, true) && !guarded_frame(&ctx, &mut a, true), "clean frames end the run");
    a.ui.test_panic = Some(PanicAt::Logic);
    assert!(!guarded_frame(&ctx, &mut a, true) && !guarded_frame(&ctx, &mut a, true));
    assert_eq!(kept(&a), 0);
    assert!(guarded_frame(&ctx, &mut a, true), "the third in a row ends Quatrelle");
    assert_eq!(kept(&a), 1, "after what's being written was kept");
    let _ = std::fs::remove_dir_all(dir);
}

/// A window of its own that panics in its own pass hands the panic over (the first stays, and the
/// window gives nothing) and `window_drawn` counts it as the frame's: said once, and three frames
/// in a row keep the drafts and end Quatrelle, at the frame's own end.
#[test]
fn a_windows_own_panic_counts_as_the_frames() {
    let (mut a, dir, ctx) = writing("frame-guard-window");
    for pass in 1..=3 {
        a.frame_part_starts(crate::FramePart::Logic);
        a.frame_part_starts(crate::FramePart::Ui);
        let mut caught = None;
        let first = crate::app::guarded_window(&mut caught, || -> u8 {
            assert!(pass == 0, "the window's own panic");
            0
        });
        let second = crate::app::guarded_window(&mut caught, || 7u8);
        assert!(first.is_none() && second == Some(7) && caught.is_some());
        a.window_drawn(&ctx, caught);
        let ended = a.frame_part_ended(&ctx, crate::FramePart::Ui, Ok(()));
        assert_eq!(ended, pass == 3, "pass {pass}");
    }
    assert_eq!(kept(&a), 1);
    let _ = std::fs::remove_dir_all(dir);
}

/// A context that gives the windows of their own a window each (egui draws them inside the page by
/// default), the way the app on a desktop does: a message written in a window is no longer inline.
fn desktop_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    ctx
}

/// How many times the panic is said.
fn said(a: &MailApp) -> usize {
    a.session.notices.iter().filter(|n| n.error && n.text.starts_with("Something went wrong in the window, and Quatrelle went on")).count()
}

/// The app with a message half written in a window of its own, drawn once.
fn writing_in_a_window(name: &str) -> (MailApp, std::path::PathBuf, egui::Context) {
    let (mut a, dir) = app(name);
    a.session.prefs.new_mail_window = true;
    let ctx = desktop_ctx();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "Not lost", "body": "Written before the window failed."}));
    assert!(!guarded_frame(&ctx, &mut a, false));
    assert!(a.session.compose.as_ref().is_some_and(|c| c.window), "it is in a window of its own");
    (a, dir, ctx)
}

/// The message being written, as it stands: its subject and its text.
fn written(a: &MailApp) -> (Option<String>, Option<String>) {
    (a.session.compose.as_ref().map(|c| c.subject.clone()), a.session.compose.as_ref().map(|c| c.body.clone()))
}

/// A panic in a dialog of a message's window, then in its form, leaves the message open with what
/// was written in it (a "close" that threw it away is what a panic used to do to it).
#[test]
fn a_panic_in_a_message_window_keeps_it_open_and_unsaved_text() {
    let (mut a, dir, ctx) = writing_in_a_window("guard-compose-window");
    let id = a.session.compose.as_ref().map(|c| c.id).expect("a message");
    let text = written(&a);
    for at in [PanicAt::ComposeDialog, PanicAt::Form] {
        a.ui.dialog = Some(Dialog::SaveChanges);
        a.ui.dialog_for = Some(id);
        a.ui.test_panic = Some(at);
        assert!(!guarded_frame(&ctx, &mut a, false), "{at:?} doesn't end Quatrelle");
        assert_eq!(a.session.all_composes().count(), 1, "{at:?}: the message is still open");
        assert_eq!(written(&a), text, "{at:?}: with its text");
        assert!(a.session.compose.as_ref().is_some_and(|c| c.window), "{at:?}: in its window");
    }
    assert_eq!(said(&a), 2, "each panic is said");
    assert_eq!(kept(&a), 0, "nothing was filed: the message is still being written");
    // The next frame draws it as before.
    a.ui.test_panic = None;
    assert!(!guarded_frame(&ctx, &mut a, false));
    assert_eq!(written(&a), text);
    let _ = std::fs::remove_dir_all(dir);
}

/// A panic between taking the message out of the session and putting it back leaves it in the
/// session, with its editors: the message in the reading pane, here.
#[test]
fn a_panic_in_the_form_puts_the_message_back() {
    let (mut a, dir, ctx) = writing("guard-form");
    let id = a.session.compose.as_ref().map(|c| c.id).expect("a message");
    let text = written(&a);
    a.ui.test_panic = Some(PanicAt::Form);
    assert!(!guarded_frame(&ctx, &mut a, false));
    assert_eq!(said(&a), 1, "the panic is said");
    assert_eq!(a.session.compose.as_ref().map(|c| c.id), Some(id), "the message is back in the session");
    assert_eq!(written(&a), text);
    assert_eq!(a.ui.editor.as_ref().map(|e| e.id), Some(id), "and its editor");
    // Frames that go on panicking keep it in Drafts first, as any window's panic does.
    assert!(!guarded_frame(&ctx, &mut a, false));
    assert!(guarded_frame(&ctx, &mut a, false), "the third in a row");
    assert_eq!(kept(&a), 1, "it was found to be kept");
    let _ = std::fs::remove_dir_all(dir);
}

/// Two appointment windows, each with a meeting made in it and not yet saved (Webex, then Teams).
fn two_appointments(name: &str) -> (MailApp, std::path::PathBuf, egui::Context) {
    let (mut a, dir) = app(name);
    let ctx = desktop_ctx();
    frame(&ctx, &mut a, vec![]);
    for _ in 0..2 {
        a.ui.dialog = Some(Dialog::new_event(&a.session, Some(NOW + 7200)));
        assert!(!guarded_frame(&ctx, &mut a, false));
    }
    assert_eq!(a.ui.event_windows.len(), 2, "two windows of their own");
    for (n, (_, f)) in a.ui.event_windows.iter_mut().enumerate() {
        f.summary = format!("Review {n}");
        if n == 0 {
            f.webex_id = Some("fresh-webex".into());
            f.webex_fresh = true;
        } else {
            f.teams_meeting_id = Some("fresh-teams".into());
            f.teams_fresh = true;
        }
    }
    (a, dir, ctx)
}

/// A panic in an appointment window (its form, or what it is made ready with) leaves the windows
/// open with their meetings: closed by the panic, the window's fresh Webex or Teams meeting would
/// have been deleted from the site, and all the windows were lost with the one that panicked.
#[test]
fn a_panic_in_an_appointment_window_keeps_it_and_its_meeting() {
    let (mut a, dir, ctx) = two_appointments("guard-event-window");
    for at in [PanicAt::Meeting, PanicAt::Prepare] {
        a.ui.test_panic = Some(at);
        let before = a.session.notices.len();
        assert!(!guarded_frame(&ctx, &mut a, false), "{at:?} doesn't end Quatrelle");
        assert_eq!(a.ui.event_windows.len(), 2, "{at:?}: both windows are still open");
        let (webex, teams) = (&a.ui.event_windows[0].1, &a.ui.event_windows[1].1);
        assert!(webex.webex_fresh && webex.webex_id.as_deref() == Some("fresh-webex"), "{at:?}: the Webex meeting is kept");
        assert!(teams.teams_fresh && teams.teams_meeting_id.as_deref() == Some("fresh-teams"), "{at:?}: the Teams meeting is kept");
        let fresh: Vec<&str> = a.session.notices.iter().skip(before).map(|n| n.text.as_str()).collect();
        assert_eq!(fresh.len(), 1, "{at:?}: the panic alone is said: {fresh:?}");
        assert_eq!(a.ui.event_windows[0].1.summary, "Review 0");
    }
    // The meetings go when the windows are closed, not before: a clean frame, then a close.
    a.ui.test_panic = None;
    assert!(!guarded_frame(&ctx, &mut a, false));
    assert_eq!(a.ui.event_windows.len(), 2);
    let _ = std::fs::remove_dir_all(dir);
}

/// A window of its own that panics on every pass: the first is said, the third keeps what is being
/// written in Drafts, and Quatrelle ends after that, whichever window it is.
fn three_passes_keep_the_drafts_first(a: &mut MailApp, ctx: &egui::Context) {
    assert!(!guarded_frame(ctx, a, false));
    assert_eq!(said(a), 1, "said from the first pass");
    assert_eq!(kept(a), 0, "nothing is filed while Quatrelle goes on");
    assert!(!guarded_frame(ctx, a, false));
    assert_eq!(said(a), 2, "said at each pass, once for the pass");
    assert!(guarded_frame(ctx, a, false), "the third in a row ends Quatrelle");
    assert_eq!(kept(a), 1, "the message being written is in Drafts by then");
    let closing = "Quatrelle has to close after an error it couldn't get past. What you were writing is kept in Drafts";
    assert!(a.session.notices.iter().any(|n| n.error && n.text == closing), "and it is said");
}

/// A note's window that panics each pass: told at once, the drafts kept at the third, and the
/// note's own text is still in its window.
#[test]
fn a_note_window_that_panics_three_passes_running_keeps_the_drafts_first() {
    let (mut a, dir) = app("guard-note-window");
    a.session.prefs.new_mail_window = false;
    let ctx = desktop_ctx();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "Not lost", "body": "Written elsewhere."}));
    let uid = a.session.run("note.create", &json!({"text": "Water the plants"})).expect("a note")["uid"].as_str().unwrap_or_default().to_string();
    let n = a.session.note(&uid).cloned().expect("its note");
    crate::notes::open(&mut a.ui, &n);
    assert!(!guarded_frame(&ctx, &mut a, false));
    assert_eq!(a.ui.note_windows.len(), 1, "its window is open");
    a.ui.test_panic = Some(PanicAt::Note);
    three_passes_keep_the_drafts_first(&mut a, &ctx);
    assert_eq!(a.ui.note_windows.len(), 1, "the note's window is kept, with its text");
    assert_eq!(a.ui.note_windows[0].editor.text, "Water the plants");
    let _ = std::fs::remove_dir_all(dir);
}

/// The Reminders window that panics each pass: the same, and the reminders aren't taken for
/// dismissed by its closing.
#[test]
fn a_reminders_window_that_panics_three_passes_running_keeps_the_drafts_first() {
    let (mut a, dir) = app("guard-reminders-window");
    a.session.prefs.new_mail_window = false;
    let ctx = desktop_ctx();
    frame(&ctx, &mut a, vec![]);
    a.run("view.remindersOnTop", json!({"on": true}));
    // An appointment whose 15 minutes' reminder is due now.
    let mut d = Dialog::new_event(&a.session, Some(NOW + 600));
    let params = match &mut d {
        Dialog::Event(f) => {
            f.summary = "Colour review".into();
            f.params()
        }
        _ => serde_json::Value::Null,
    };
    a.act(crate::Action::Submit("event.create", params));
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "Not lost", "body": "Written elsewhere."}));
    assert!(!guarded_frame(&ctx, &mut a, false));
    assert!(!guarded_frame(&ctx, &mut a, false));
    assert!(crate::reminders::showing(&a.ui.reminders), "the reminder is due, its window up");
    let due = a.ui.reminders.due.len();
    a.ui.reminders.test_panic = true;
    three_passes_keep_the_drafts_first(&mut a, &ctx);
    assert!(crate::reminders::showing(&a.ui.reminders), "its window is still up");
    assert_eq!(a.ui.reminders.due.len(), due, "nothing was taken for answered");
    let _ = std::fs::remove_dir_all(dir);
}

/// Two windows of message sources, the first of which panics: it closes, and the second, drawn
/// without a panic after it, doesn't hide it: the panic is said, once.
#[test]
fn a_source_window_that_panics_closes_and_the_next_one_doesnt_hide_it() {
    let (mut a, dir) = app("guard-source-window");
    let ctx = desktop_ctx();
    frame(&ctx, &mut a, vec![]);
    let keys: Vec<_> = a.session.list.iter().take(2).map(|m| m.key.clone()).collect();
    assert_eq!(keys.len(), 2);
    for k in &keys {
        a.act(crate::Action::Run("message.source", json!({"key": k.to_id()})));
    }
    a.ui.test_panic = Some(PanicAt::Source);
    assert!(!guarded_frame(&ctx, &mut a, false));
    assert_eq!(said(&a), 1, "the first window's panic is said, though the second drew whole");
    assert_eq!(a.session.sources.len(), 1, "the first closed");
    assert_eq!(a.session.sources.first().map(|v| v.key.clone()), keys.get(1).cloned(), "the second stays");
    let _ = std::fs::remove_dir_all(dir);
}

/// A message opened in a window of its own that panics isn't closed by it.
#[test]
fn a_panic_in_a_read_message_window_leaves_it_open() {
    let (mut a, dir) = app("guard-read-window");
    let ctx = desktop_ctx();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list[0].key.clone();
    a.run("message.popOut", json!({"key": k.to_id()}));
    assert!(!guarded_frame(&ctx, &mut a, false));
    assert_eq!(a.session.windows.len(), 1, "its window is open");
    a.ui.test_panic = Some(PanicAt::ReadMessage);
    assert!(!guarded_frame(&ctx, &mut a, false));
    assert_eq!(said(&a), 1);
    assert_eq!(a.session.windows.len(), 1, "and stays open");
    let _ = std::fs::remove_dir_all(dir);
}
