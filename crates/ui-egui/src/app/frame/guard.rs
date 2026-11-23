// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The last guard around each frame of the main window: a panic in what a frame does or draws (a
//! dialog's button, a pane, a message laid out) used to end Quatrelle, with the messages being
//! written in it. The app (`apps/quatrelle`) runs each part of a frame under `catch_unwind` and
//! tells the window how it went ([`MailApp::frame_part_ended`]): a panic is said, what was on top
//! (a dialog) is put away, and Quatrelle goes on. Frames that panic three in a row mean nothing
//! in the window can be used: what's being written is kept in Drafts first, and only then is the
//! app told to end. The text of a panic goes to the log (the app's panic hook), never to the
//! notice, as it can quote a message.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::*;

/// What a panic carries, as `catch_unwind` gives it.
pub(crate) type Caught = Box<dyn Any + Send>;

/// Frames in a row that may panic before Quatrelle is let go.
const PANICKING_FRAMES: u32 = 3;

/// The part of a frame run under the guard: eframe runs `logic` every frame, and `ui` after it
/// only while the window is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FramePart {
    Logic,
    Ui,
}

/// Where the drawing is made to panic by the tests that try the guard.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanicAt {
    /// Drawing the dialog on top, when there is one.
    Dialog,
    /// The start of `logic`.
    Logic,
}

/// How the frames under the guard have gone.
#[derive(Default)]
pub(crate) struct FrameGuard {
    /// Frames in a row that panicked.
    panicking: u32,
    /// This frame panicked (counted once, whichever part or window it came from).
    panicked: bool,
    /// The last frame ran `logic` with no `ui` after it (a minimized or hidden window).
    logic_alone: bool,
    /// What's being written was kept and the app is to end: set after the keeping, never before.
    letting_go: bool,
}

/// Run `draw`, a window of its own's drawing (`show_viewport_immediate` draws it in a pass of its
/// own inside the main window's): a panic in it is kept in `caught` (the first one stays) and
/// `None` stands for what it would have given, for [`MailApp::window_drawn`] once that pass is
/// over. Let out of the window's pass, the panic would leave egui unable to draw it again.
// Used by the windows of their own (a dialog's, a message's…) as they're put under the guard.
#[allow(dead_code)]
pub(crate) fn guarded_window<T>(caught: &mut Option<Caught>, draw: impl FnOnce() -> T) -> Option<T> {
    match catch_unwind(AssertUnwindSafe(draw)) {
        Ok(v) => Some(v),
        Err(payload) => {
            caught.get_or_insert(payload);
            None
        }
    }
}

impl MailApp {
    /// Before a part of a frame runs under the guard.
    pub fn frame_part_starts(&mut self, part: FramePart) {
        let g = &mut self.ui.caught;
        match part {
            FramePart::Logic => {
                // A hidden window's frames are `logic` alone: one that ran cleanly ends the run.
                if g.logic_alone && !g.panicked {
                    g.panicking = 0;
                }
                g.panicked = false;
                g.logic_alone = true;
            }
            FramePart::Ui => g.logic_alone = false,
        }
    }

    /// After a part of a frame ran under the guard: `ran` is what `catch_unwind` gave for it.
    /// True: Quatrelle must end now (what's being written is kept already). After a panic the
    /// dialog on top is put away and the panic is said; a frame drawn whole ends a run of
    /// panicking ones; the third frame in a row that panics keeps what's being written as drafts,
    /// says so, and only then lets go. A window of its own that panicked counts as the frame's
    /// ([`MailApp::window_drawn`]), so this tells it at the end of the frame's `ui`.
    pub fn frame_part_ended(&mut self, ctx: &egui::Context, part: FramePart, ran: std::thread::Result<()>) -> bool {
        match ran {
            Ok(()) => {
                if part == FramePart::Ui && !self.ui.caught.panicked {
                    self.ui.caught.panicking = 0;
                }
            }
            Err(payload) => {
                self.frame_panicked(part, payload);
                ctx.request_repaint();
            }
        }
        self.ui.caught.letting_go
    }

    /// After a window of its own was drawn under [`guarded_window`]: what it caught, if anything,
    /// as a panic of the frame's drawing. That Quatrelle must end is told by the frame's own
    /// [`MailApp::frame_part_ended`] after.
    #[allow(dead_code)]
    pub(crate) fn window_drawn(&mut self, ctx: &egui::Context, caught: Option<Caught>) {
        if let Some(payload) = caught {
            self.frame_part_ended(ctx, FramePart::Ui, Err(payload));
        }
    }

    /// A part of a frame panicked.
    fn frame_panicked(&mut self, part: FramePart, payload: Caught) {
        // What the panic carried is in the log already (the app's hook); dropped here, off the
        // guard's own stack, as dropping it can panic.
        let _ = catch_unwind(AssertUnwindSafe(|| drop(payload)));
        let g = &mut self.ui.caught;
        if g.letting_go {
            return;
        }
        let first = !g.panicked;
        if first {
            g.panicked = true;
            g.panicking = g.panicking.saturating_add(1);
        }
        let panicking = g.panicking;
        self.ui.dialog = None;
        if !first {
            return;
        }
        if panicking >= PANICKING_FRAMES {
            log::error!("the window's {part:?} panicked in {panicking} frames in a row: keeping what's being written as drafts, and closing");
            // The way a lost GPU does it: at once, in the store, never as a save to the server.
            match catch_unwind(AssertUnwindSafe(|| self.keep_written_now())) {
                Ok(Ok(kept)) => log::info!("{kept} message(s) being written kept as drafts"),
                Ok(Err(e)) => log::error!("the messages being written couldn't be kept as drafts: {e}"),
                Err(_) => log::error!("keeping what's being written as drafts panicked too"),
            }
            self.session.notice(tl!("Quatrelle has to close after an error it couldn't get past. What you were writing is kept in Drafts"), true);
            // Only now: set before the keeping, the app would end with nothing kept.
            self.ui.caught.letting_go = true;
            return;
        }
        log::error!("the window's {part:?} panicked and was stopped; Quatrelle goes on");
        self.session.notice(tl!("Something went wrong in the window, and Quatrelle went on. What you were doing may need doing again"), true);
    }

    /// In tests, a panic where `at` says the drawing is to make one.
    #[cfg(test)]
    pub(crate) fn panic_at(&self, at: PanicAt) {
        assert!(self.ui.test_panic != Some(at), "a panic the guard's tests asked for");
    }
}
