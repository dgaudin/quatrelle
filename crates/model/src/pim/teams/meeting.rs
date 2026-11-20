// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Teams meetings made from the calendar: what one is made from (an event's title and times),
//! the meeting Microsoft Graph answers with, what can go wrong, and what a link and a value from
//! Graph must be for Quatrelle to write them into an appointment.
//!
//! What is hostile here: all of Graph's answer. A link is kept only when it is Teams' own
//! ([`teams_meeting_url`]); a meeting ID, passcode, phone number or conference ID only when it is
//! plain ([`teams_meeting_value`]); the meeting's id only when it is plausible
//! ([`valid_teams_meeting_id`]), as it goes into the address of the requests that change or
//! delete the meeting.

use serde::{Deserialize, Serialize};

use super::is_bidi;
use crate::ical::CalEvent;
use crate::job::Secret;

/// The hosts a Teams meeting's links may be on (the commercial cloud, the US government's), and
/// whether their subdomains may be too (`emea.teams.microsoft.com`).
pub const TEAMS_MEETING_HOSTS: [(&str, bool); 4] =
    [("teams.microsoft.com", true), ("teams.microsoft.us", false), ("gov.teams.microsoft.us", false), ("dod.teams.microsoft.us", false)];
/// The longest link kept, in bytes.
pub const TEAMS_MEETING_URL_MAX: usize = 2048;
/// The longest meeting ID, passcode, phone number or conference ID kept, in characters.
pub const TEAMS_MEETING_VALUE_MAX: usize = 64;
/// The longest meeting id kept.
pub const TEAMS_MEETING_ID_MAX: usize = 512;
/// What a Teams meeting's conference is labelled in an event's file (`CONFERENCE;…;LABEL=`).
pub const TEAMS_CONFERENCE_LABEL: &str = "Microsoft Teams";
/// How long a meeting made from an event without a length lasts, in seconds.
const NO_LENGTH: i64 = 30 * 60;

/// Whether `host` (already in lower case) is one of [`TEAMS_MEETING_HOSTS`], or a subdomain of
/// one that admits them, each of its labels letters, digits and `-`.
pub fn teams_meeting_host(host: &str) -> bool {
    let label = |l: &str| !l.is_empty() && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    TEAMS_MEETING_HOSTS.iter().any(|&(name, subdomains)| {
        host == name || (subdomains && host.strip_suffix(name).and_then(|h| h.strip_suffix('.')).is_some_and(|sub| sub.split('.').all(label)))
    })
}

/// Whether a link Graph gave (to join a meeting, or to its options) is Teams' own: at most
/// [`TEAMS_MEETING_URL_MAX`] bytes, without whitespace, control or bidi characters or `\`; over
/// `https`, with no user information (`@`) before the path and no port but 443, on a host
/// [`teams_meeting_host`] takes.
pub fn teams_meeting_url(url: &str) -> bool {
    if url.len() > TEAMS_MEETING_URL_MAX || url.chars().any(|c| c.is_whitespace() || c.is_control() || is_bidi(c) || c == '\\') {
        return false;
    }
    let Some(rest) = url.get(..8).filter(|s| s.eq_ignore_ascii_case("https://")).and_then(|_| url.get(8..)) else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return false;
    }
    let (host, port) = authority.split_once(':').map_or((authority, None), |(h, p)| (h, Some(p)));
    port.is_none_or(|p| p == "443") && teams_meeting_host(&host.to_ascii_lowercase())
}

/// A meeting ID, passcode, phone number or conference ID from Graph as it may be written in an
/// appointment: trimmed, when it is then 1 to [`TEAMS_MEETING_VALUE_MAX`] ASCII letters, digits,
/// spaces, `+`, `-`, `(` and `)`; else none.
pub fn teams_meeting_value(s: &str) -> Option<String> {
    let v = s.trim();
    let plain = (1..=TEAMS_MEETING_VALUE_MAX).contains(&v.len())
        && v.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b' ' | b'+' | b'-' | b'(' | b')'));
    plain.then(|| v.to_string())
}

/// Whether a meeting's id is plausible: 1 to [`TEAMS_MEETING_ID_MAX`] ASCII letters, digits, `-`,
/// `_`, `=`, `+` and `/` (Graph's ids are base64, so an address carries one encoded as a segment).
pub fn valid_teams_meeting_id(id: &str) -> bool {
    (1..=TEAMS_MEETING_ID_MAX).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'=' | b'+' | b'/'))
}

/// What a Teams meeting is made from: an event's title and times. A meeting has no zone of its
/// own: its start and end are instants.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamsMeetingFields {
    pub title: String,
    /// Unix seconds (UTC).
    pub start: i64,
    pub end: i64,
}

impl TeamsMeetingFields {
    /// An event's title, start and end; an event that ends where it starts (or before) gives a
    /// meeting of half an hour.
    pub fn of(e: &CalEvent) -> TeamsMeetingFields {
        let end = if e.end > e.start { e.end } else { e.start.saturating_add(NO_LENGTH) };
        TeamsMeetingFields { title: e.summary.clone(), start: e.start, end }
    }
}

/// A meeting as Teams made it. A value Graph didn't give, or gave in a form not kept, is empty.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamsMeeting {
    /// Graph's id of the meeting, to change or delete it ([`valid_teams_meeting_id`]).
    pub id: String,
    /// The link to join it ([`teams_meeting_url`]).
    pub join_url: String,
    /// The meeting ID and passcode to join it by typing them ([`teams_meeting_value`]).
    pub meeting_id: String,
    pub passcode: Secret,
    /// The number to call, and the conference ID to give it ([`teams_meeting_value`]).
    pub toll_number: String,
    pub conference_id: String,
    /// The link to the meeting's options ([`teams_meeting_url`]).
    pub options_url: String,
}

/// Why Teams didn't do what was asked of a meeting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TeamsMeetingError {
    /// Teams is off, or signed out.
    NotSignedIn,
    /// The permission to make Teams meetings wasn't given, or no longer is.
    Consent,
    /// The sign-in is gone or was refused.
    SignIn,
    /// The account may not make meetings (its licence, the organisation's policy).
    NotAllowed,
    /// No answer: no network, working offline, or waited too long.
    Offline,
    /// Graph answered with a link that isn't Teams' own ([`teams_meeting_url`]).
    Untrusted,
    /// Any other refusal: Graph's status, and its error code when it gave one (never its message,
    /// which can quote what was sent).
    Refused { status: u16, code: String },
}

impl std::fmt::Display for TeamsMeetingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TeamsMeetingError::NotSignedIn => f.write_str("Teams is signed out"),
            TeamsMeetingError::Consent => f.write_str("Teams meetings need your permission"),
            TeamsMeetingError::SignIn => f.write_str("Sign in to Teams again"),
            TeamsMeetingError::NotAllowed => f.write_str("Your Teams account can't create meetings"),
            TeamsMeetingError::Offline => f.write_str("no connection"),
            TeamsMeetingError::Untrusted => f.write_str("Teams gave a link Quatrelle doesn't trust"),
            TeamsMeetingError::Refused { status, code } if code.is_empty() => write!(f, "Teams answered {status}"),
            TeamsMeetingError::Refused { status, code } => write!(f, "Teams answered {status}: {code}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_join_link_is_taken_only_from_teams_own_hosts() {
        for ok in [
            "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0?context=%7b%22Tid%22%3a%22t%22%7d",
            "https://teams.microsoft.com:443/meet/123?p=x",
            "https://emea.teams.microsoft.com/meetingOptions/?organizerId=o&threadId=19_m@thread.v2",
            "https://teams.microsoft.us/meet/1",
            "https://gov.teams.microsoft.us/meet/1",
            "https://dod.teams.microsoft.us/meet/1",
        ] {
            assert!(teams_meeting_url(ok), "{ok}");
        }
        let long = format!("https://teams.microsoft.com/meet/{}", "a".repeat(TEAMS_MEETING_URL_MAX));
        for bad in [
            "https://teams.microsoft.com.evil.example/meet/1",
            "https://evilteams.microsoft.com/meet/1",
            "http://teams.microsoft.com/meet/1",
            "javascript:alert(1)",
            "https://teams.microsoft.com:8443/meet/1",
            "https://user@teams.microsoft.com/meet/1",
            "https://teams.microsoft.com@evil.example/meet/1",
            "https://teams.microsoft.com\\@evil.example/meet/1",
            "https://teams.microsoft.com/meet/1 2",
            "https://teams.microsoft.com/meet/\u{7}",
            "https://teams.microsoft.com/meet/\u{202E}1",
            "https://x.gov.teams.microsoft.us/meet/1",
            "https://sub.teams.microsoft.us/meet/1",
            "",
            long.as_str(),
        ] {
            assert!(!teams_meeting_url(bad), "{bad:?}");
        }
        // At the bounds: a link of 2,048 bytes is taken, one more byte isn't; the scheme and the
        // host in capitals are the same link; an empty port, or an empty label, makes no host.
        let start = "https://teams.microsoft.com/meet/";
        let at_most = format!("{start}{}", "a".repeat(TEAMS_MEETING_URL_MAX - start.len()));
        assert_eq!(at_most.len(), TEAMS_MEETING_URL_MAX);
        assert!(teams_meeting_url(&at_most));
        assert!(!teams_meeting_url(&format!("{at_most}a")));
        assert!(teams_meeting_url("HTTPS://TEAMS.MICROSOFT.COM/meet/1"));
        assert!(teams_meeting_url("https://EMEA.Teams.Microsoft.com/meet/1"));
        for bad in ["https://teams.microsoft.com:/meet/1", "https://.teams.microsoft.com/meet/1", "https://a..teams.microsoft.com/meet/1"] {
            assert!(!teams_meeting_url(bad), "{bad}");
        }
    }

    #[test]
    fn a_value_is_kept_only_when_it_is_plain() {
        for ok in ["1234567890", "123 456 789 0", "aB3xY9", "+1 323-555-0166", "(555) 010-0100"] {
            assert_eq!(teams_meeting_value(ok).as_deref(), Some(ok));
        }
        assert_eq!(teams_meeting_value("  42 ").as_deref(), Some("42"));
        let longest = "1".repeat(TEAMS_MEETING_VALUE_MAX);
        assert_eq!(teams_meeting_value(&longest).as_deref(), Some(longest.as_str()), "64 at most");
        for bad in ["", "   ", &"1".repeat(65), "12<b>3", "12\n3", "１２３", "a_b", "é", "1\u{202E}2"] {
            assert_eq!(teams_meeting_value(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn an_id_is_plausible_or_dropped() {
        for ok in ["MSpkYzE3Njc0Yy04MWQ5LTRhZGItYmZiMi04ZdFpHRTNaR1F6WGhyZWFkLnYy", "abc_123-X=", "a/b+c=", &"a".repeat(512)] {
            assert!(valid_teams_meeting_id(ok), "{ok}");
        }
        for bad in ["", &"a".repeat(513), "a b", "a\nb", "é", "..", "a?b", "a#b", "a%2Fb"] {
            assert!(!valid_teams_meeting_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn fields_are_the_events_title_and_times() {
        let e = CalEvent {
            summary: "Budget".into(),
            start: 1_795_165_200,
            end: 1_795_168_800,
            tzid: "Romance Standard Time".into(),
            ..Default::default()
        };
        assert_eq!(TeamsMeetingFields::of(&e), TeamsMeetingFields { title: "Budget".into(), start: 1_795_165_200, end: 1_795_168_800 });
        // A length of nothing: Teams is given half an hour.
        let none = CalEvent { start: 100, end: 100, ..Default::default() };
        assert_eq!(TeamsMeetingFields::of(&none).end, 1900);
    }

    #[test]
    fn the_passcode_never_shows_in_debug() {
        let m = TeamsMeeting {
            id: "MSpkYzE3Njc0Yy04MWQ5".into(),
            join_url: "https://teams.microsoft.com/meet/1".into(),
            meeting_id: "1234567890".into(),
            passcode: Secret("aB3xY9".into()),
            toll_number: "+1 323-555-0166".into(),
            conference_id: "123 456 789 0".into(),
            options_url: String::new(),
        };
        let shown = format!("{m:?}");
        assert!(!shown.contains("aB3xY9"), "{shown}");
        let written = serde_json::to_string(&m).unwrap();
        assert!(!written.contains("aB3xY9"), "{written}");
    }

    #[test]
    fn a_gov_cloud_link_joins_too() {
        assert!(crate::ical::is_teams_link("https://gov.teams.microsoft.us/l/meetup-join/x"));
        assert!(!crate::ical::is_teams_link("https://evilteams.microsoft.com/l/meetup-join/x"));
    }

    #[test]
    fn each_error_says_what_went_wrong() {
        for (error, said) in [
            (TeamsMeetingError::NotSignedIn, "Teams is signed out"),
            (TeamsMeetingError::Consent, "Teams meetings need your permission"),
            (TeamsMeetingError::SignIn, "Sign in to Teams again"),
            (TeamsMeetingError::NotAllowed, "Your Teams account can't create meetings"),
            (TeamsMeetingError::Offline, "no connection"),
            (TeamsMeetingError::Untrusted, "Teams gave a link Quatrelle doesn't trust"),
            (TeamsMeetingError::Refused { status: 400, code: "BadRequest".into() }, "Teams answered 400: BadRequest"),
            (TeamsMeetingError::Refused { status: 503, code: String::new() }, "Teams answered 503"),
        ] {
            assert_eq!(error.to_string(), said);
        }
    }
}
