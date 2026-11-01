//! The types the Webex scheduling tasks share: the fields of an event that a Webex meeting is
//! made from, the meeting Webex answers with, and what can go wrong.

use super::Secret;
use crate::ical::CalEvent;
use serde::{Deserialize, Serialize};

/// The port Quatrelle listens on for Webex's sign-in redirect, and the address it registers.
pub const WEBEX_REDIRECT_PORT: u16 = 53683;
pub const WEBEX_REDIRECT_URI: &str = "http://127.0.0.1:53683/";

/// What a Webex meeting is made from: an event's title, times, zone and repeat rule.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebexFields {
    pub title: String,
    /// Unix seconds.
    pub start: i64,
    pub end: i64,
    /// The IANA name of the zone (`UTC` when the event's isn't one).
    pub timezone: String,
    pub recurrence: Option<String>,
}

impl WebexFields {
    pub fn of(e: &CalEvent) -> WebexFields {
        let timezone = crate::zone::Zone::named(&e.tzid).and_then(|z| z.name()).unwrap_or("UTC").to_string();
        WebexFields { title: e.summary.clone(), start: e.start, end: e.end, timezone, recurrence: e.rrule.clone() }
    }
}

/// A meeting as Webex made it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebexMeeting {
    pub id: String,
    pub web_link: String,
    pub meeting_number: String,
    pub password: Secret,
    pub sip_address: String,
}

/// Why Webex didn't do what was asked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebexError {
    /// The sign-in is gone or was refused.
    SignIn,
    /// The account isn't allowed to schedule meetings.
    Forbidden,
    Refused {
        status: u16,
        message: String,
    },
    Failed(String),
}

impl std::fmt::Display for WebexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebexError::SignIn => f.write_str("Sign in to Webex again"),
            WebexError::Forbidden => f.write_str("Your Webex account can't schedule meetings on this site"),
            WebexError::Refused { status, message } => write!(f, "Webex answered {status}: {message}"),
            WebexError::Failed(text) => f.write_str(text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_take_the_zone_by_its_iana_name() {
        for (tzid, zone) in [("Romance Standard Time", "Europe/Paris"), ("Europe/Berlin", "Europe/Berlin"), ("", "UTC"), ("+0530", "UTC")] {
            let e =
                CalEvent { summary: "Sync".into(), start: 100, end: 200, tzid: tzid.into(), rrule: Some("FREQ=WEEKLY".into()), ..Default::default() };
            let f = WebexFields::of(&e);
            assert_eq!(f.timezone, zone, "{tzid:?}");
            assert_eq!((f.title.as_str(), f.start, f.end, f.recurrence.as_deref()), ("Sync", 100, 200, Some("FREQ=WEEKLY")));
        }
    }
}
