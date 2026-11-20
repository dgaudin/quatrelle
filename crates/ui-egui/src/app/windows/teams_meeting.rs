// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! An appointment window's Teams meeting: asked for (one at a time, T5), taken in when Teams
//! answers or the refusal said, and deleted when the window leaves it unsaved, or Quatrelle closes
//! with the window open. A save or send waits for the meeting File › Options › Calendar's option
//! asks for as it goes, and goes once it's in. What each of these does to the form is in
//! `pim_dialogs/teams_meeting.rs`.

#[cfg(not(target_arch = "wasm32"))]
use crate::pim_dialogs::teams_meeting::auto;
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
    /// not said there (Teams signed out meanwhile opens Options › Teams). A save or send that
    /// waited for it goes now, with it; refused, it doesn't go, and the window says why.
    pub(in crate::app) fn teams_meeting_answers(&mut self) {
        let mut acts = Vec::new();
        let mut sends = Vec::new();
        for (w, f) in &mut self.ui.event_windows {
            let Some(answer) = f.teams_wait.and_then(|ask| self.session.teams_meeting_take_answer(ask)) else { continue };
            f.teams_wait = None;
            let held = f.teams_send_held.take();
            match answer {
                Ok(m) => {
                    take_meeting(f, &m);
                    sends.extend(held.map(|id| (*w, id, f.params())));
                }
                Err(e) => refused(f, &e, &mut acts),
            }
        }
        for a in acts {
            self.act(a);
        }
        for (w, id, p) in sends {
            self.for_event(w, id, p);
        }
    }

    /// Window `w`'s save or send, `id`, about to go: File › Options › Calendar's Teams meeting for
    /// guests asked for now holds it until Teams answers ([`Self::teams_meeting_answers`] sends it
    /// then); one that can't be made stops it, for the window to say why (a second press sends it
    /// without). `true`: it doesn't go now.
    #[cfg(not(target_arch = "wasm32"))]
    pub(in crate::app) fn teams_auto_holds(&mut self, w: u64, id: &'static str, now: f64) -> bool {
        let mut acts = Vec::new();
        let Some((_, f)) = self.ui.event_windows.iter_mut().find(|(x, _)| *x == w) else { return false };
        if !auto(f, &self.session, now, true, &mut acts) {
            return false;
        }
        for a in acts {
            self.act(a);
        }
        // Asked for: the send waits for its answer. Refused by the engine at once, it said why.
        if let Some((_, f)) = self.ui.event_windows.iter_mut().find(|(x, f)| *x == w && f.teams_wait.is_some()) {
            f.teams_send_held = Some(id);
        }
        true
    }

    /// In the browser, where no Teams meeting is made, nothing holds a save or send.
    #[cfg(target_arch = "wasm32")]
    pub(in crate::app) fn teams_auto_holds(&mut self, _w: u64, _id: &'static str, _now: f64) -> bool {
        false
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
