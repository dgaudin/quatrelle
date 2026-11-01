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

impl Session {
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
    /// of it. Where its notes and location don't hold the old one (a server's own join address),
    /// there's nothing to tell them.
    fn webex_link_changed(&mut self, id: &str, link: &str) -> Result<(), CmdError> {
        let found = self.pim.iter().find_map(|(account, p)| {
            let e = p.events.iter().map(|e| &e.event).find(|e| e.webex_id.as_deref() == Some(id) && e.recurrence_id.is_none())?;
            Some((
                account.clone(),
                e.uid.clone(),
                e.join_link().filter(|old| old != link && (e.description.contains(old) || e.location.contains(old)))?,
            ))
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
