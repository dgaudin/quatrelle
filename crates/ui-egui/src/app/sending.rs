//! Sending once the names are known: a message's Send or a meeting's, held while its fields'
//! names are resolved and its contact groups expanded (`PeopleFields::gate_send`,
//! `PeopleFields::gate_meeting`), then run, or stopped with why in the window it was asked from;
//! and the commands of an appointment's or meeting's window.

use crate::dialogs::people_field::{MEETING_FIELDS, MEETING_WINDOWS, PeopleFields, is_meeting_window, meeting_field};
use crate::pim_dialogs::{self, EventForm};
use crate::*;

impl MailApp {
    /// A send held for the names its fields are still resolving: run once they're settled, or
    /// said where it was asked when one can't be sent to (a name not found, the directory not
    /// answering within its timeout): over the message, or in the meeting's window.
    pub(super) fn send_when_resolved(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        // A held meeting's lines take what the engine settled since its window last drew (this
        // runs before the windows do): a name answered but not yet taken would read as one
        // nothing is being asked for.
        for (wid, f) in &mut self.ui.event_windows {
            if self.ui.people.holding(MEETING_WINDOWS | *wid) {
                pim_dialogs::prepare(f, &mut self.session, &mut self.ui.people);
            }
        }
        for (id, done) in self.ui.people.send_when_resolved(&mut self.session, now) {
            if is_meeting_window(id) {
                self.meeting_resolved(id & !MEETING_WINDOWS, done);
            } else {
                match done {
                    Ok(params) => {
                        if self.use_compose(id) {
                            self.run_now("compose.send", params);
                        }
                    }
                    Err(why) => self.ui.compose_alert = Some(why),
                }
            }
            ctx.request_repaint();
        }
        if let Some(left) = self.ui.people.send_wait_left(now) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(left.clamp(0.05, 1.0)));
        }
    }

    /// Meeting window `wid`'s held Send concluded: sent as its lines hold them now (created, or
    /// updated when it was sent before), or why not said in its window. A window closed meanwhile
    /// has nothing left to send.
    fn meeting_resolved(&mut self, wid: u64, done: Result<Value, String>) {
        let Some((_, f)) = self.ui.event_windows.iter_mut().find(|(w, _)| *w == wid) else { return };
        match done {
            Ok(_) => {
                take_lines(f, &mut self.session, &mut self.ui.people);
                let id = if f.uid.is_some() { "event.update" } else { "event.create" };
                let p = f.params();
                self.event_command(wid, id, p);
            }
            Err(why) => f.error = Some(why),
        }
    }

    /// A command from appointment or meeting window `w`. Saving or sending one that can invite
    /// first checks the names left in its lines and expands its contact groups: with any still to
    /// settle it waits ([`Self::send_when_resolved`] sends it then, from the form as it is), and
    /// another press meanwhile does nothing. With none, it goes at once, its parameters taken from
    /// the form as the check left it. Its other commands (delete, respond, move) never wait.
    pub(crate) fn for_event(&mut self, w: u64, id: &'static str, p: Value) {
        let invites = matches!(id, "event.create" | "event.update");
        let Some((_, f)) = self.ui.event_windows.iter_mut().find(|(x, _)| *x == w).filter(|e| invites && e.1.can_invite) else {
            return self.event_command(w, id, p);
        };
        let people = &mut self.ui.people;
        if people.holding(MEETING_WINDOWS | w) {
            return;
        }
        let now = self.ui.close_checked.unwrap_or(0.0);
        let account = f.account_id();
        let lines = [("required", &mut f.guests), ("optional", &mut f.optional), ("rooms", &mut f.rooms_line)];
        if people.gate_meeting(&mut self.session, w, &account, lines, now) {
            return;
        }
        take_lines(f, &mut self.session, people);
        // A room the check took out of the guests that Rooms couldn't book is back among them,
        // saying why: checked again, the send stops on it.
        if MEETING_FIELDS.iter().any(|&name| people.blocked(meeting_field(w, name))) {
            let lines = [("required", &mut f.guests), ("optional", &mut f.optional), ("rooms", &mut f.rooms_line)];
            if people.gate_meeting(&mut self.session, w, &account, lines, now) {
                return;
            }
        }
        let p = f.params();
        self.event_command(w, id, p);
    }

    /// Run a command of window `w`'s: done, the window closes; refused, it says why.
    pub(super) fn event_command(&mut self, w: u64, id: &'static str, p: Value) {
        match self.session.run(id, &p) {
            Ok(v) => {
                if id == "event.move" {
                    self.moved_event(&v);
                }
                self.ui.event_windows.retain(|(x, _)| *x != w);
            }
            Err(e) => {
                if let Some((_, f)) = self.ui.event_windows.iter_mut().find(|(x, _)| *x == w) {
                    f.error = Some(e.to_string().trim_start_matches("bad parameters: ").to_string());
                }
            }
        }
    }
}

/// A meeting's lines as its fields hold them now, into its form: what was settled since its
/// window last drew taken (a room among the guests booked, or kept there), each line's text as
/// its pieces spell it (the Scheduling Assistant draws no line, so the form's text may lag
/// behind), and the rooms those its Rooms line books.
fn take_lines(f: &mut EventForm, s: &mut Session, people: &mut PeopleFields) {
    pim_dialogs::prepare(f, s, people);
    for (name, line) in [("required", &mut f.guests), ("optional", &mut f.optional)] {
        people.write_line(meeting_field(f.window, name), line);
    }
    let rooms = meeting_field(f.window, "rooms");
    let mut line = f.rooms_line.clone();
    people.write_line(rooms, &mut line);
    f.rooms_from_field(&people.addressees(rooms), line);
}
