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
