// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Teams meetings made from the calendar, as the session keeps them. The engine asks and the
//! Teams worker calls Graph; what comes back waits here for the window that asked, a meeting whose
//! window has closed is deleted (nothing else will ever hold its id), a change Teams didn't take
//! is said and left for the next save to send again, and a meeting given another link has it
//! written where the old one was. A meeting follows its appointment: a save sends Teams what its
//! title or times changed, deleting the appointment (or taking the meeting out of it) deletes the
//! meeting, and Teams unusable leaves it waiting for the next save. Meetings may be made while
//! Teams is on, signed in, and the permission to make them was given, as the worker says
//! ([`teams::Event::Meetings`]), and only in a calendar of the user's own ([`TeamsMeetingPath`]).
//! This state is kept apart from [`TeamsState`]: signing out doesn't forget what was sent.
//!
//! What is hostile here: Graph's answers, read and checked by the worker, bounded again here. An
//! answer to an ask nobody made is dropped (the asks are at most [`TEAMS_MEETING_ASKS_MAX`]). A new
//! link is written only when it is a Teams meeting's join link ([`ical::is_teams_link`] and
//! [`teams_meeting_url`]), only into an appointment the user organizes, and only in place of a
//! Teams meeting's link its notes, place or conference held. An event the user doesn't organize
//! may carry any id: it is never acted on (T3); only the ids kept on the user's own appointments,
//! or made here, are sent (T6).

use quatrelle_model::pim::teams::{TeamsMeeting, TeamsMeetingError, TeamsMeetingFields, teams_meeting_url};

use super::*;

/// The most creations on their way at once.
pub const TEAMS_MEETING_ASKS_MAX: usize = 16;

/// A creation on its way.
pub struct TeamsMeetingAsk {
    pub fields: TeamsMeetingFields,
    /// Its window closed: nobody takes the answer.
    pub abandoned: bool,
}

/// The Teams meetings on their way, and what each was last sent with.
#[derive(Default)]
pub struct TeamsMeetingsState {
    /// Creations on their way, by `ask`.
    pub asks: HashMap<u64, TeamsMeetingAsk>,
    /// Their answers, until the window that asked takes them.
    pub answers: HashMap<u64, Result<TeamsMeeting, TeamsMeetingError>>,
    /// What each meeting made or changed in this run was last sent with.
    pub sent: HashMap<String, TeamsMeetingFields>,
    /// What each update on its way started from, oldest first: what Teams had before it, put back
    /// in `sent` when the update fails.
    pub unacknowledged: HashMap<String, VecDeque<TeamsMeetingFields>>,
    /// Meetings being deleted: asked of Teams, not answered yet.
    pub deleting: usize,
    /// Quatrelle is closing: the open windows were gone through once.
    pub closing: bool,
}

/// How a Teams meeting is added to an appointment, by the calendar it is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TeamsMeetingPath {
    /// Quatrelle asks Teams for the meeting, and the meeting follows the appointment.
    Standalone,
    /// A Microsoft 365 mailbox's calendar: Exchange Online makes the meeting with the appointment.
    Mailbox,
    /// A calendar Quatrelle doesn't know, or someone else's: no Teams meeting is added there.
    Unavailable,
}

/// What an appointment's window says of it, for the option to make every meeting with guests a
/// Teams meeting ([`Session::teams_meeting_auto`]). Made by the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TeamsAutoAsk {
    /// How its calendar takes a meeting.
    pub path: TeamsMeetingPath,
    /// A meeting made for it would be the user's to keep: an appointment they organize.
    pub can_hold: bool,
    /// Its guests, required and optional (not its rooms).
    pub guests: usize,
    /// It has a Teams meeting, or one is on its way.
    pub has_teams: bool,
    /// It has a Webex meeting or room.
    pub has_webex: bool,
    /// The user removed a Teams meeting in this window (a calendar that couldn't hold one taking
    /// it out isn't the user's).
    pub removed: bool,
    /// The option already added one in this window, or said why it couldn't.
    pub tried: bool,
}

/// What the option does for the window now.
#[derive(Debug, PartialEq, Eq)]
pub enum TeamsAuto {
    /// Nothing.
    No,
    /// A Teams meeting is added, as Add a Teams meeting does.
    Add,
    /// None can be made (Teams signed out, switched off, or the permission missing): the window
    /// says so, once.
    SayOnce,
}

/// Whether a link joins a meeting Teams made: a link Teams joins by, on one of the hosts its
/// meetings are on.
fn teams_meeting_link(url: &str) -> bool {
    ical::is_teams_link(url) && teams_meeting_url(url)
}

/// The link in an appointment's notes or place that Teams' `link` replaces: the meeting's own,
/// its conference's when that is a Teams meeting's link (and only it, wherever it still is: any
/// other Teams link there is the user's); else, without one, the first Teams meeting's link found.
/// Every answer gives the link, most often the one the appointment has already.
fn replaced_link(e: &CalEvent, link: &str) -> Option<String> {
    let links = e.links();
    if links.iter().any(|l| l == link) {
        return None;
    }
    let held = |l: &str| e.description.contains(l) || e.location.contains(l);
    if let Some(c) = e.conference.as_ref().filter(|c| teams_meeting_link(&c.url)) {
        return held(&c.url).then(|| c.url.clone());
    }
    links.into_iter().find(|l| teams_meeting_link(l) && held(l))
}

/// Whether a conference's link gives way to Teams' `link`: another Teams meeting's link (the
/// meeting's link before).
fn moved_conference(url: &str, link: &str) -> bool {
    url != link && teams_meeting_link(url)
}

impl Session {
    /// Whether a Teams meeting can be asked for now: Teams on and signed in, and the permission to
    /// make meetings given.
    pub fn teams_meetings_usable(&self) -> Result<(), TeamsMeetingError> {
        let t = &self.prefs.teams;
        if !(t.on && t.signed_in) {
            Err(TeamsMeetingError::NotSignedIn)
        } else if !t.meetings {
            Err(TeamsMeetingError::Consent)
        } else {
            Ok(())
        }
    }

    /// Whether a Teams meeting of this event's would be the user's to change: the rule a Webex
    /// meeting's follows ([`Session::webex_can_hold`]).
    pub fn teams_meeting_can_hold(&self, e: &CalEvent) -> bool {
        self.webex_can_hold(e)
    }

    /// Whether this event has a Teams meeting that is the user's to change. A stranger's event may
    /// carry any id.
    fn teams_meeting_is_mine(&self, e: &CalEvent) -> bool {
        e.teams_meeting_id.is_some() && self.teams_meeting_can_hold(e)
    }

    /// How a Teams meeting is added in `calendar` (its id, or its name, as `event.create` finds
    /// it) of `account`: never in a calendar Quatrelle doesn't know, nor in one the account lists
    /// that isn't the user's (a colleague's shared one, a public folder's, one subscribed to by
    /// its link), whatever the account; by Exchange Online in a Microsoft 365 mailbox's calendar
    /// (all but the one on this computer); otherwise by Quatrelle.
    pub fn teams_meeting_path(&self, account: &str, calendar: &str) -> TeamsMeetingPath {
        let found = self.pim.get(account).and_then(|p| p.calendars.iter().find(|c| c.id == calendar || c.name.eq_ignore_ascii_case(calendar)));
        let Some(c) = found.filter(|c| !c.not_own && own_calendar(&c.id)) else { return TeamsMeetingPath::Unavailable };
        if c.id != LOCAL && self.account(account).is_some_and(|a| a.config.graph.is_some()) {
            TeamsMeetingPath::Mailbox
        } else {
            TeamsMeetingPath::Standalone
        }
    }

    /// What File › Options › Calendar's option to make every meeting with guests a Teams meeting
    /// does for an appointment's window: with the option on, an appointment of the user's own that
    /// has a guest, no Teams meeting, no Webex, and no Teams meeting the user removed in this
    /// window, gets one, once per window, in a calendar Quatrelle makes the meeting for; Teams
    /// can't make it, the window says so instead, once. Never in someone else's calendar, nor yet
    /// in a Microsoft 365 mailbox's (D13: Exchange Online is to make its own there).
    pub fn teams_meeting_auto(&self, a: &TeamsAutoAsk) -> TeamsAuto {
        if !self.prefs.teams_meeting_for_guests || !a.can_hold || a.guests == 0 || a.has_teams || a.has_webex || a.removed || a.tried {
            return TeamsAuto::No;
        }
        match a.path {
            TeamsMeetingPath::Unavailable | TeamsMeetingPath::Mailbox => TeamsAuto::No,
            TeamsMeetingPath::Standalone if self.teams_meetings_usable().is_ok() => TeamsAuto::Add,
            TeamsMeetingPath::Standalone => TeamsAuto::SayOnce,
        }
    }

    /// An appointment saved, from `before` (none: just made) to `after`: its Teams meeting follows
    /// it, when the meeting is the user's. One the appointment had and no longer has (taken out,
    /// or another put in its place) is deleted; the one it has is sent what differs from what
    /// Teams was last sent, or else from what `before` said (D16: its title, and its start and end
    /// as instants). Teams can't be used: nothing goes, a notice says so, and `sent` keeps what
    /// Teams still has, for the next save to send the whole change.
    pub(crate) fn teams_meeting_follow(&mut self, before: Option<&CalEvent>, after: &CalEvent) {
        let old = before.filter(|b| self.teams_meeting_can_hold(b)).and_then(|b| b.teams_meeting_id.clone());
        let new = after.teams_meeting_id.clone().filter(|_| self.teams_meeting_can_hold(after));
        if let Some(old) = old.clone().filter(|o| new.as_ref() != Some(o)) {
            self.teams_meeting_drop(old);
        }
        let Some(id) = new else { return };
        let last = match self.teams_meetings.sent.get(&id) {
            Some(sent) => sent.clone(),
            None => match before.filter(|_| old.as_ref() == Some(&id)) {
                Some(b) => TeamsMeetingFields::of(b),
                None => return,
            },
        };
        let now = TeamsMeetingFields::of(after);
        if now == last {
            return;
        }
        if let Err(e) = self.teams_meetings_usable() {
            // A meeting of an earlier session goes in with what it had before this change: the
            // next save sends this change with its own.
            self.teams_meetings.sent.entry(id).or_insert(last);
            return self.teams_meeting_failed(false, &e);
        }
        self.teams_meetings.sent.insert(id.clone(), now.clone());
        self.teams_meetings.unacknowledged.entry(id.clone()).or_default().push_back(last.clone());
        self.jobs.push(Job::Teams { job: teams::Job::MeetingUpdate { id, before: last, after: now } });
    }

    /// An appointment of the user's deleted (or cancelled): so is its Teams meeting. One
    /// occurrence of a series leaves the series' meeting alone.
    pub(crate) fn teams_meeting_gone(&mut self, e: &CalEvent, occurrence: Option<i64>) {
        let Some(id) = e.teams_meeting_id.clone().filter(|_| self.teams_meeting_can_hold(e)) else { return };
        if occurrence.is_some() && e.rrule.is_some() {
            return;
        }
        self.teams_meeting_drop(id);
    }

    /// A meeting nothing holds any more: forgotten, and deleted in Teams. Teams can't be used: a
    /// notice says the meeting wasn't deleted (it stays in Teams, where it expires).
    pub(crate) fn teams_meeting_drop(&mut self, id: String) {
        self.teams_meetings.sent.remove(&id);
        self.teams_meetings.unacknowledged.remove(&id);
        match self.teams_meetings_usable() {
            Ok(()) => self.teams_meeting_delete(id),
            Err(e) => self.teams_meeting_failed(true, &e),
        }
    }

    /// The answer to a creation, once, for the window that asked.
    pub fn teams_meeting_take_answer(&mut self, ask: u64) -> Option<Result<TeamsMeeting, TeamsMeetingError>> {
        self.teams_meetings.answers.remove(&ask)
    }

    /// Whether a meeting is still being deleted, or a creation given up may still bring one.
    pub fn teams_meetings_deleting(&self) -> bool {
        self.teams_meetings.deleting > 0 || self.teams_meetings.asks.values().any(|a| a.abandoned)
    }

    /// Ask Teams to delete a meeting: it counts as being deleted until the answer is in.
    pub(crate) fn teams_meeting_delete(&mut self, id: String) {
        self.teams_meetings.deleting = self.teams_meetings.deleting.saturating_add(1);
        self.jobs.push(Job::Teams { job: teams::Job::MeetingDelete { id } });
    }

    /// Teams switched off or signed out: nothing on its way will be answered. A window waiting
    /// hears that Teams is signed out, a creation given up is forgotten (nothing can delete its
    /// meeting now), and the updates out count as not taken: what Teams surely has goes back in
    /// `sent`, for the next save to send the difference again. What this leaves: an update Teams
    /// did take before the stop, then the appointment given back exactly the fields it had before
    /// that update — that save sends nothing, and Teams keeps the update's values.
    pub(crate) fn teams_meetings_stopped(&mut self) {
        let m = &mut self.teams_meetings;
        for (ask, asked) in m.asks.drain() {
            if !asked.abandoned {
                m.answers.insert(ask, Err(TeamsMeetingError::NotSignedIn));
            }
        }
        for (id, started_from) in m.unacknowledged.drain() {
            if let Some(from) = started_from.into_iter().next() {
                m.sent.insert(id, from);
            }
        }
        m.deleting = 0;
    }

    pub(super) fn teams_meeting_created(&mut self, ask: u64, meeting: Option<TeamsMeeting>, error: Option<TeamsMeetingError>) {
        let Some(asked) = self.teams_meetings.asks.remove(&ask) else {
            return log::info!("teams: the answer to a meeting nobody asked for was passed over");
        };
        let answer = meeting.ok_or_else(|| error.unwrap_or(TeamsMeetingError::Offline));
        if asked.abandoned {
            if let Ok(m) = answer {
                self.teams_meeting_delete(m.id);
            }
            return;
        }
        if let Ok(m) = &answer {
            self.teams_meetings.sent.insert(m.id.clone(), asked.fields);
        }
        self.teams_meetings.answers.insert(ask, answer);
    }

    pub(super) fn teams_meeting_updated(&mut self, id: &str, meeting: Option<TeamsMeeting>, error: Option<TeamsMeetingError>) {
        let m = &mut self.teams_meetings;
        let started_from = m.unacknowledged.get_mut(id).and_then(|q| q.pop_front());
        if m.unacknowledged.get(id).is_some_and(|q| q.is_empty()) {
            m.unacknowledged.remove(id);
        }
        if let (Some(_), Some(from)) = (&error, started_from) {
            // Teams still has what it had: the next save sends the difference again. The updates
            // still out started from something Teams now lacks too: a later failure can't move
            // `sent` forward again.
            if let Some(q) = m.unacknowledged.get_mut(id) {
                q.iter_mut().for_each(|f| f.clone_from(&from));
            }
            m.sent.insert(id.to_string(), from);
        }
        match (meeting, error) {
            (_, Some(e)) => self.teams_meeting_failed(false, &e),
            (Some(made), None) => {
                if let Err(e) = self.teams_meeting_link_changed(id, &made.join_url) {
                    self.notice_with("The Teams meeting couldn't be updated: {error}", &[("error", &e.to_string())], true);
                }
            }
            (None, None) => {}
        }
    }

    /// Teams didn't update (or delete) a meeting: a notice the window says in its language. Graph's
    /// refusal has a wording of its own, its status and code put in as they came (the code is
    /// Graph's word, never translated); any other error is Quatrelle's words, put in as `{error}`.
    fn teams_meeting_failed(&mut self, deleting: bool, error: &TeamsMeetingError) {
        let TeamsMeetingError::Refused { status, code } = error else {
            let error = error.to_string();
            if deleting {
                self.notice_with("The Teams meeting couldn't be deleted: {error}", &[("error", &error)], true);
            } else {
                self.notice_with("The Teams meeting couldn't be updated: {error}", &[("error", &error)], true);
            }
            return;
        };
        let status = status.to_string();
        let (status, code) = (("status", status.as_str()), ("code", code.as_str()));
        match (deleting, code.1.is_empty()) {
            (false, false) => self.notice_with("The Teams meeting couldn't be updated: Teams answered {status}: {code}", &[status, code], true),
            (false, true) => self.notice_with("The Teams meeting couldn't be updated: Teams answered {status}", &[status], true),
            (true, false) => self.notice_with("The Teams meeting couldn't be deleted: Teams answered {status}: {code}", &[status, code], true),
            (true, true) => self.notice_with("The Teams meeting couldn't be deleted: Teams answered {status}", &[status], true),
        }
    }

    /// Teams gave a meeting another link: the appointment says the new one in its notes and place,
    /// and as its conference when that was a Teams meeting's link, and its guests hear of it. A
    /// link that isn't a Teams meeting's changes nothing.
    fn teams_meeting_link_changed(&mut self, id: &str, link: &str) -> Result<(), CmdError> {
        if !teams_meeting_link(link) {
            return Ok(());
        }
        let found = self.pim.iter().find_map(|(account, p)| {
            let e = p.events.iter().map(|e| &e.event).find(|e| e.teams_meeting_id.as_deref() == Some(id) && self.teams_meeting_is_mine(e))?;
            let old = replaced_link(e, link);
            let conference = e.conference.as_ref().is_some_and(|c| moved_conference(&c.url, link));
            (old.is_some() || conference).then(|| (account.clone(), e.uid.clone(), old))
        });
        let Some((account, uid, old)) = found else { return Ok(()) };
        self.update_event(&uid, |e| {
            if let Some(old) = &old {
                e.description = e.description.replace(old.as_str(), link);
                e.location = e.location.replace(old.as_str(), link);
            }
            if let Some(c) = e.conference.as_mut().filter(|c| moved_conference(&c.url, link)) {
                c.url = link.to_string();
            }
        })?;
        self.tell_guests(&account, &uid, None, true)?;
        Ok(())
    }

    pub(super) fn teams_meeting_deleted(&mut self, error: Option<TeamsMeetingError>) {
        self.teams_meetings.deleting = self.teams_meetings.deleting.saturating_sub(1);
        if let Some(e) = error {
            self.teams_meeting_failed(true, &e);
        }
    }
}
