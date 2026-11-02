//! An appointment window's Webex meeting: asked for, taken in when Webex answers, and deleted
//! when the window leaves it unsaved. What each of these does to the form is in
//! `pim_dialogs/webex.rs`.

use crate::pim_dialogs::webex::{error_text, leaving, take_meeting};
use crate::*;

impl MailApp {
    /// Window `w` asks for a meeting: it waits for the answer, or says why the engine refused. A
    /// window gone, or already waiting for one, asks nothing.
    pub(super) fn for_webex(&mut self, w: u64, p: Value) {
        let Some((_, f)) = self.ui.event_windows.iter_mut().find(|(x, f)| *x == w && f.webex_wait.is_none()) else { return };
        match self.session.run("webex.createMeeting", &p) {
            Ok(v) => f.webex_wait = v.get("ask").and_then(Value::as_u64),
            Err(e) => f.error = Some(e.to_string().trim_start_matches("bad parameters: ").to_string()),
        }
    }

    /// Each window's answer from Webex, once it's in.
    pub(super) fn webex_answers(&mut self) {
        for (_, f) in &mut self.ui.event_windows {
            let Some(answer) = f.webex_wait.and_then(|ask| self.session.webex_take_answer(ask)) else { continue };
            f.webex_wait = None;
            match answer {
                Ok(m) => take_meeting(f, &m),
                Err(e) => f.error = Some(error_text(&e)),
            }
        }
    }

    /// Quatrelle is closing: what the open windows leave unsaved goes from Webex's site, each
    /// window once. `true` while that's on its way, for the caller to ask again shortly. With
    /// nothing of theirs to delete, `false` at once: closing waits for nothing, and a delete
    /// refused (signed out) has nothing to wait for.
    pub fn webex_closing(&mut self) -> bool {
        for (_, f) in std::mem::take(&mut self.ui.event_windows) {
            if let Some(p) = leaving(&f) {
                self.session.webex.closing |= self.session.run("webex.deleteMeeting", &p).is_ok();
            }
        }
        if !self.session.webex.closing {
            return false;
        }
        // In this order: an answer may bring one more meeting to delete.
        self.pump();
        self.flush_jobs();
        self.session.webex_deleting()
    }

    /// Window `w` is going without saving: nothing stays on Webex's site for it.
    pub(super) fn webex_left(&mut self, w: u64) {
        let left = self.ui.event_windows.iter().find(|(x, _)| *x == w).and_then(|(_, f)| leaving(f));
        if let Some(p) = left {
            self.run("webex.deleteMeeting", p);
        }
    }
}
