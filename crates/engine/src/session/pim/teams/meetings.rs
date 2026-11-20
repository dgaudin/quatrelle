// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Teams meetings made from the calendar, as the session keeps them. The engine asks and the
//! Teams worker calls Graph; what comes back waits here for the window that asked, a meeting whose
//! window has closed is deleted (nothing else will ever hold its id), a change Teams didn't take
//! is said and left for the next save to send again, and a meeting given another link has it
//! written where the old one was. Meetings may be made while Teams is on, signed in, and the
//! permission to make them was given, as the worker says ([`teams::Event::Meetings`]). This
//! state is kept apart from [`TeamsState`]: signing out doesn't forget what was sent.
//!
//! What is hostile here: Graph's answers, read and checked by the worker, bounded again here. An
//! answer to an ask nobody made is dropped (the asks are at most [`TEAMS_MEETING_ASKS_MAX`]). A new
//! link is written only when it is a Teams meeting's join link ([`ical::is_teams_link`] and
//! [`teams_meeting_url`]), only into an appointment the user organizes, and only in place of a
//! Teams meeting's link its notes, place or conference held.

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

/// Whether a link joins a meeting Teams made: a link Teams joins by, on one of the hosts its
/// meetings are on.
fn teams_meeting_link(url: &str) -> bool {
    ical::is_teams_link(url) && teams_meeting_url(url)
}

/// The link in an appointment's notes or place that Teams' `link` replaces: a Teams meeting's,
/// the conference's first (the meeting's own), else the first found. Every answer gives the link,
/// most often the one the appointment has already, and any other link in it is the user's.
fn replaced_link(e: &CalEvent, link: &str) -> Option<String> {
    let links = e.links();
    if links.iter().any(|l| l == link) {
        return None;
    }
    let held = |l: &str| teams_meeting_link(l) && (e.description.contains(l) || e.location.contains(l));
    let own = e.conference.as_ref().map(|c| c.url.clone()).filter(|u| held(u));
    own.or_else(|| links.into_iter().find(|l| held(l)))
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
    /// `sent`, for the next save to send the difference again.
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
        let done = match (meeting, error) {
            (_, Some(e)) => Err(e.to_string()),
            (Some(made), None) => self.teams_meeting_link_changed(id, &made.join_url).map_err(|e| e.to_string()),
            (None, None) => Ok(()),
        };
        if let Err(e) = done {
            self.notice_with("The Teams meeting couldn't be updated: {error}", &[("error", &e)], true);
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
            self.notice_with("The Teams meeting couldn't be deleted: {error}", &[("error", &e.to_string())], true);
        }
    }
}
