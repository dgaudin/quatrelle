//! Webex's answers. The engine asks and the worker's thread calls Webex; what comes back waits
//! here for the window that asked, and a meeting whose window has closed is deleted, as nothing
//! else will ever hold its id.

use quatrelle_model::{WebexError, WebexFields, WebexMeeting};

use super::*;

/// A creation on its way.
pub struct WebexAsk {
    pub fields: WebexFields,
    pub quiet: bool,
    /// Its window closed: nobody takes the answer.
    pub abandoned: bool,
}

/// Webex's sign-in and the meetings on their way.
#[derive(Default)]
pub struct WebexState {
    pub signing_in: bool,
    pub sign_in_error: Option<WebexError>,
    /// Creations on their way, by `ask`.
    pub asks: HashMap<u64, WebexAsk>,
    /// Their answers, until the window that asked takes them.
    pub answers: HashMap<u64, Result<WebexMeeting, WebexError>>,
    /// What each meeting made in this run was last sent with.
    pub sent: HashMap<String, WebexFields>,
}

/// The link in an appointment's notes or place that Webex's `link` replaces: a join page on the
/// same site. Every answer gives the link, most often the one the appointment has already, and
/// any other meeting link in it (a Zoom room typed in, a personal room) is the user's.
fn replaced_link(e: &CalEvent, link: &str) -> Option<String> {
    let site = ical::webex_join_host(link)?;
    let links = e.links();
    if links.iter().any(|l| l == link) {
        return None;
    }
    links.into_iter().find(|l| ical::webex_join_host(l).as_ref() == Some(&site) && (e.description.contains(l) || e.location.contains(l)))
}

impl Session {
    /// Whether this event's Webex meeting is the user's to change: it has one, it is the event
    /// itself and not an occurrence of it, and the user organizes it (or no one does, as for an
    /// appointment made here). A stranger's event may carry any id.
    fn webex_is_mine(&self, e: &CalEvent) -> bool {
        e.webex_id.is_some() && e.recurrence_id.is_none() && e.organizer.as_ref().is_none_or(|o| self.is_my_address(&o.email))
    }

    /// An appointment of the user's changed from `before` to `after`: its Webex meeting is sent
    /// what differs from what it was last sent, or else from what `before` said.
    pub(crate) fn webex_follow(&mut self, before: Option<&CalEvent>, after: &CalEvent) {
        let Some(id) = after.webex_id.as_deref().filter(|_| self.webex_is_mine(after)) else { return };
        let now = WebexFields::of(after);
        let last = match self.webex.sent.get(id) {
            Some(sent) => sent.clone(),
            None => match before.filter(|b| b.webex_id.as_deref() == Some(id)) {
                Some(b) => WebexFields::of(b),
                None => return,
            },
        };
        if now == last {
            return;
        }
        self.webex.sent.insert(id.to_string(), now.clone());
        self.jobs.push(Job::WebexUpdate {
            client_id: self.prefs.webex_client_id.clone(),
            id: id.to_string(),
            before: last,
            after: now,
            quiet: !after.attendees.is_empty(),
        });
    }

    /// An appointment of the user's is deleted: so is its Webex meeting. One occurrence of a
    /// series leaves the series' meeting alone.
    pub(crate) fn webex_gone(&mut self, e: &CalEvent, occurrence: Option<i64>) {
        let Some(id) = e.webex_id.as_deref().filter(|_| self.webex_is_mine(e)) else { return };
        if occurrence.is_some() && e.rrule.is_some() {
            return;
        }
        self.webex.sent.remove(id);
        self.jobs.push(Job::WebexDelete { client_id: self.prefs.webex_client_id.clone(), id: id.to_string(), quiet: !e.attendees.is_empty() });
    }

    /// The answer to a creation, once, for the window that asked.
    pub fn webex_take_answer(&mut self, ask: u64) -> Option<Result<WebexMeeting, WebexError>> {
        self.webex.answers.remove(&ask)
    }

    pub(in crate::session) fn webex_signed_in(&mut self, user: Option<String>, error: Option<WebexError>) {
        self.webex.signing_in = false;
        self.webex.sign_in_error = error;
        if self.webex.sign_in_error.is_none() {
            self.prefs.webex_user = user.unwrap_or_default();
            self.prefs.webex_signed_in = true;
        }
    }

    pub(in crate::session) fn webex_created(&mut self, ask: u64, meeting: Option<WebexMeeting>, error: Option<WebexError>) {
        let Some(asked) = self.webex.asks.remove(&ask) else { return };
        let answer = meeting.ok_or_else(|| error.unwrap_or_else(|| WebexError::Failed("Webex gave no answer".into())));
        if asked.abandoned {
            if let Ok(m) = answer {
                self.jobs.push(Job::WebexDelete { client_id: self.prefs.webex_client_id.clone(), id: m.id, quiet: asked.quiet });
            }
            return;
        }
        if let Ok(m) = &answer {
            self.webex.sent.insert(m.id.clone(), asked.fields);
        }
        self.webex.answers.insert(ask, answer);
    }

    pub(in crate::session) fn webex_updated(&mut self, id: &str, meeting: Option<WebexMeeting>, error: Option<WebexError>) {
        let done = match (meeting, error) {
            (_, Some(e)) => Err(e.to_string()),
            (Some(m), None) => self.webex_link_changed(id, &m.web_link).map_err(|e| e.to_string()),
            (None, None) => Ok(()),
        };
        if let Err(e) = done {
            self.notice_with("The Webex meeting couldn't be updated: {error}", &[("error", &e)], true);
        }
    }

    /// Webex gave a meeting another link: the appointment says the new one, and its guests hear
    /// of it.
    fn webex_link_changed(&mut self, id: &str, link: &str) -> Result<(), CmdError> {
        let found = self.pim.iter().find_map(|(account, p)| {
            let e = p.events.iter().map(|e| &e.event).find(|e| e.webex_id.as_deref() == Some(id) && self.webex_is_mine(e))?;
            Some((account.clone(), e.uid.clone(), replaced_link(e, link)?))
        });
        let Some((account, uid, old)) = found else { return Ok(()) };
        self.update_event(&uid, |e| {
            e.description = e.description.replace(&old, link);
            e.location = e.location.replace(&old, link);
        })?;
        self.tell_guests(&account, &uid, None, true)?;
        Ok(())
    }

    pub(in crate::session) fn webex_deleted(&mut self, error: Option<WebexError>) {
        if let Some(e) = error {
            self.notice_with("The Webex meeting couldn't be deleted: {error}", &[("error", &e.to_string())], true);
        }
    }
}
