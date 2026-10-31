//! Sending once the names are known: a message's Send or a meeting's, or a dialog's OK, held
//! while its fields' names are resolved and its contact groups expanded
//! (`PeopleFields::gate_send`, `PeopleFields::gate_meeting`, `PeopleFields::gate_dialog`), then
//! run, or stopped with why in the window or dialog it was asked from; and the commands of an
//! appointment's or meeting's window.

use crate::dialogs::people_field::{
    DIALOG_WINDOWS, MEETING_FIELDS, MEETING_WINDOWS, PeopleFields, dialog_field, is_dialog_window, is_meeting_window, meeting_field, places,
};
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
        // A held dialog's field likewise.
        if self.ui.people.holding(DIALOG_WINDOWS)
            && let Some(d) = self.ui.dialog.as_mut()
        {
            places::prepare(&mut self.ui.people, &mut self.session, d);
        }
        for (id, done) in self.ui.people.send_when_resolved(&mut self.session, now) {
            if is_meeting_window(id) {
                self.meeting_resolved(id & !MEETING_WINDOWS, done, now);
            } else if is_dialog_window(id) {
                self.dialog_resolved(done);
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
    /// has nothing left to send. Its lines go through the check again first: a name typed in one
    /// while the others were checked is closed and checked in turn (held again while it's asked),
    /// never sent as typed.
    fn meeting_resolved(&mut self, wid: u64, done: Result<Value, String>, now: f64) {
        let Some((_, f)) = self.ui.event_windows.iter_mut().find(|(w, _)| *w == wid) else { return };
        match done {
            Ok(_) => {
                if check_and_take(f, &mut self.session, &mut self.ui.people, now) {
                    return;
                }
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
        if check_and_take(f, &mut self.session, people, now) {
            return;
        }
        let p = f.params();
        self.event_command(w, id, p);
    }

    /// The open dialog's OK (Add calendar, Open another mailbox): what's typed in its field closed
    /// and its names checked first. Add calendar waits only on the names being asked and does
    /// what it can with each entry; Open another mailbox stops on one it can't open. Held, it goes
    /// on from [`Self::send_when_resolved`]; another press meanwhile does nothing.
    pub(super) fn dialog_ok(&mut self) {
        let people = &mut self.ui.people;
        if people.holding(DIALOG_WINDOWS) {
            return;
        }
        let now = self.ui.close_checked.unwrap_or(0.0);
        let held = match self.ui.dialog.as_mut() {
            Some(Dialog::OpenSharedCalendar(f)) => {
                let account = self.session.directory_account(None);
                people.gate_dialog(&mut self.session, "calendar", account.as_deref(), &mut f.text, true, now)
            }
            Some(Dialog::OpenSharedMailbox { account, email, .. }) => {
                let account = account.clone();
                people.gate_dialog(&mut self.session, "mailbox", Some(&account), email, false, now)
            }
            _ => return,
        };
        if !held {
            self.dialog_resolved(Ok(Value::Null));
        }
    }

    /// What the open dialog's OK came to once its names were settled. Add calendar opens each
    /// person's calendar: one opened leaves the field, one that couldn't stays, red, with why; the
    /// dialog closes once nothing is left (a wait that gave up says so, under the field). Open
    /// another mailbox opens the one entry (closing on success, saying why otherwise), or says
    /// why it can't.
    fn dialog_resolved(&mut self, done: Result<Value, String>) {
        match self.ui.dialog.as_mut() {
            Some(Dialog::OpenSharedCalendar(f)) => {
                let field = dialog_field(0, "calendar");
                for who in self.ui.people.addressees(field) {
                    match self.session.run("calendar.openShared", &json!({"email": who.address})) {
                        Ok(_) => self.ui.people.opened(field, &who.address),
                        Err(e) => self.ui.people.failed(field, &who.address, e.to_string().trim_start_matches("bad parameters: ").to_string()),
                    }
                }
                f.error = done.err();
                if self.ui.people.pieces(field).is_empty() && self.ui.people.typing(field).trim().is_empty() {
                    self.ui.dialog = None;
                }
            }
            Some(Dialog::OpenSharedMailbox { account, calendar, error, .. }) => match done {
                Ok(_) => {
                    let Some(who) = self.ui.people.addressees(dialog_field(0, "mailbox")).into_iter().next() else { return };
                    let p = json!({"account": account, "email": who.address, "calendar": *calendar});
                    self.act(Action::Submit("account.openMailbox", p));
                }
                Err(why) => *error = Some(why),
            },
            _ => {}
        }
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

/// A meeting's Send checked ([`PeopleFields::gate_meeting`]) and, unless that holds it (`true`),
/// its lines taken into its form. A room the check took out of the guests that Rooms couldn't
/// book is back among them then, saying why: the send is held on it as it stands (not checked
/// again, which would take it out once more), and stops on it.
fn check_and_take(f: &mut EventForm, s: &mut Session, people: &mut PeopleFields, now: f64) -> bool {
    let account = f.account_id();
    let lines = [("required", &mut f.guests), ("optional", &mut f.optional), ("rooms", &mut f.rooms_line)];
    if people.gate_meeting(s, f.window, &account, lines, now) {
        return true;
    }
    take_lines(f, s, people);
    MEETING_FIELDS.iter().any(|&name| people.blocked(meeting_field(f.window, name))) && people.hold_meeting(s, f.window, now)
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
