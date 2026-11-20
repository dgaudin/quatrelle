// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! An appointment window's Teams meeting: asked for (one at a time, T5), taken in when Teams
//! answers or the refusal said, and deleted when the window leaves it unsaved, or Quatrelle closes
//! with the window open. What each of these does to the form is in `pim_dialogs/teams_meeting.rs`.

use crate::pim_dialogs::teams_meeting::{leaving, refused, take_meeting};
use crate::*;

impl MailApp {
    /// Window `w` asks for a meeting: it waits for the answer, or says why the engine refused. A
    /// window gone, or already waiting for one, asks nothing.
    pub(in crate::app) fn for_teams_meeting(&mut self, w: u64, p: Value) {
        let Some((_, f)) = self.ui.event_windows.iter_mut().find(|(x, f)| *x == w && f.teams_wait.is_none()) else { return };
        match self.session.run("event.teamsMeeting", &p) {
            Ok(v) => f.teams_wait = v.get("ask").and_then(Value::as_u64),
            Err(e) => f.error = Some(e.to_string().trim_start_matches("bad parameters: ").to_string()),
        }
    }

    /// Each window's answer from Teams, once it's in: the meeting put in the appointment, or why
    /// not said there (Teams signed out meanwhile opens Options › Teams).
    pub(in crate::app) fn teams_meeting_answers(&mut self) {
        let mut acts = Vec::new();
        for (_, f) in &mut self.ui.event_windows {
            let Some(answer) = f.teams_wait.and_then(|ask| self.session.teams_meeting_take_answer(ask)) else { continue };
            f.teams_wait = None;
            match answer {
                Ok(m) => take_meeting(f, &m),
                Err(e) => refused(f, &e, &mut acts),
            }
        }
        for a in acts {
            self.act(a);
        }
    }

    /// Window `w` is going without saving: nothing stays in Teams for it. The saved appointment's
    /// meeting stays as the appointment does.
    pub(in crate::app) fn teams_meeting_left(&mut self, w: u64) {
        let left = self.ui.event_windows.iter().find(|(x, _)| *x == w).and_then(|(_, f)| leaving(f));
        if let Some(p) = left {
            self.run("event.teamsMeeting", p);
        }
    }

    /// Quatrelle is closing: what the open windows leave unsaved goes from Teams, each window once
    /// (`session.teams_meetings.closing`). The windows are read, not taken: [`Self::webex_closing`]
    /// takes them after, which is why this is asked first. `true` while a delete is on its way, or a
    /// creation given up may bring a meeting to delete, for the caller to ask again shortly. With
    /// nothing on its way, `false` at once: closing waits for nothing, not a look at the workers,
    /// and a delete Teams can't take (signed out) has nothing to wait for.
    pub fn teams_meetings_closing(&mut self) -> bool {
        if !self.session.teams_meetings.closing {
            self.session.teams_meetings.closing = true;
            let left: Vec<Value> = self.ui.event_windows.iter().filter_map(|(_, f)| leaving(f)).collect();
            for p in left {
                // Refused, it leaves nothing to wait for; Teams unusable, the engine says so.
                let _ = self.session.run("event.teamsMeeting", &p);
            }
        }
        if !self.session.teams_meetings_deleting() {
            return false;
        }
        // In this order: an answer may bring one more meeting to delete.
        self.pump();
        self.flush_jobs();
        self.session.teams_meetings_deleting()
    }
}
