// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Teams meetings through Graph's `/me/onlineMeetings`: made, changed and deleted, and the
//! meeting Graph answers with read and checked.
//!
//! What is sent is the user's: a meeting's subject (the appointment's title on one line, at most
//! [`MEETING_SUBJECT_MAX`] characters) and its start and end, in UTC. What comes back is a
//! server's, and hostile: [`meeting_of`] keeps a meeting only with a plausible id
//! ([`valid_teams_meeting_id`]) and links that are Teams' own ([`teams_meeting_url`]), and each
//! of its values only when it is plain ([`teams_meeting_value`]). The invitation text Graph writes
//! (`joinInformation`) is Microsoft's own: never read. A meeting's id goes into an address only
//! when plausible, encoded as one segment (Graph's ids are base64: `/` is `%2F`); an implausible
//! one is refused before any request. Nothing here writes a log: an error says Graph's status and
//! code, never a title, a link, a passcode or an id.

use quatrelle_model::job::Secret;
use quatrelle_model::pim::teams::{TeamsMeeting, TeamsMeetingFields, is_bidi, teams_meeting_url, teams_meeting_value, valid_teams_meeting_id};

use super::*;

/// The longest subject sent, in characters.
pub const MEETING_SUBJECT_MAX: usize = 255;

/// What `/me/onlineMeetings` makes a meeting from: its subject (the title on one line, at most
/// [`MEETING_SUBJECT_MAX`] characters), and its start and end as UTC times
/// (`2026-11-20T09:00:00Z`).
pub fn meeting_body(f: &TeamsMeetingFields) -> Value {
    json!({"subject": subject(&f.title), "startDateTime": utc(f.start), "endDateTime": utc(f.end)})
}

/// What changed between the fields last sent and these, as a meeting's `PATCH` says it: the
/// subject when it is another, a start or an end that moved; `None` when nothing changed.
pub fn meeting_patch(before: &TeamsMeetingFields, after: &TeamsMeetingFields) -> Option<Value> {
    let mut patch = serde_json::Map::new();
    let title = subject(&after.title);
    if title != subject(&before.title) {
        patch.insert("subject".into(), Value::String(title));
    }
    if after.start != before.start {
        patch.insert("startDateTime".into(), Value::String(utc(after.start)));
    }
    if after.end != before.end {
        patch.insert("endDateTime".into(), Value::String(utc(after.end)));
    }
    (!patch.is_empty()).then_some(Value::Object(patch))
}

/// The `id` of Graph's answer about a meeting, when it is plausible ([`valid_teams_meeting_id`]).
pub fn meeting_id_of(v: &Value) -> Option<String> {
    v.get("id").and_then(Value::as_str).filter(|id| valid_teams_meeting_id(id)).map(str::to_string)
}

/// A meeting from Graph's answer: `None` unless its id is plausible ([`meeting_id_of`]) and its
/// `joinWebUrl` is Teams' own ([`teams_meeting_url`]), as is its `meetingOptionsWebUrl` when it
/// gives one (only the organiser gets it: none is empty). Its meeting ID and passcode
/// (`joinMeetingIdSettings`), its number and conference ID (`audioConferencing`) are kept when
/// plain ([`teams_meeting_value`]), else empty. A field of another JSON type than the one Graph
/// documents is none. `joinInformation` is never read.
pub fn meeting_of(v: &Value) -> Option<TeamsMeeting> {
    let id = meeting_id_of(v)?;
    let text = |at: &str| v.pointer(at).and_then(Value::as_str);
    let join_url = text("/joinWebUrl").filter(|u| teams_meeting_url(u))?.to_string();
    let options_url = match text("/meetingOptionsWebUrl") {
        None | Some("") => String::new(),
        Some(u) if teams_meeting_url(u) => u.to_string(),
        Some(_) => return None,
    };
    let value = |at: &str| text(at).and_then(teams_meeting_value).unwrap_or_default();
    Some(TeamsMeeting {
        id,
        join_url,
        meeting_id: value("/joinMeetingIdSettings/joinMeetingId"),
        passcode: Secret(value("/joinMeetingIdSettings/passcode")),
        toll_number: value("/audioConferencing/tollNumber"),
        conference_id: value("/audioConferencing/conferenceId"),
        options_url,
    })
}

/// A meeting's subject: `title` on one line (each run of white space or control characters one
/// space, none at either end), without bidi characters, cut to [`MEETING_SUBJECT_MAX`]
/// characters (never inside one, never on a space).
fn subject(title: &str) -> String {
    let mut out = String::new();
    let (mut chars, mut space) = (0usize, false);
    for c in title.chars() {
        if c.is_whitespace() || c.is_control() {
            space = true;
            continue;
        }
        if is_bidi(c) {
            continue;
        }
        let room = if space && chars > 0 { 2 } else { 1 };
        if chars.saturating_add(room) > MEETING_SUBJECT_MAX {
            break;
        }
        if room == 2 {
            out.push(' ');
        }
        out.push(c);
        chars = chars.saturating_add(room);
        space = false;
    }
    out
}

impl TeamsClient {
    /// Make a meeting of these fields ([`meeting_body`]): Graph's answer, for [`meeting_of`]. A
    /// creation isn't repeated on a 503: one made though its answer was lost would be two.
    pub fn create_meeting(&self, f: &TeamsMeetingFields) -> Result<Value> {
        self.request("POST", &self.at("/me/onlineMeetings"), Some(&meeting_body(f)), false, false)
    }

    /// Change a meeting by a patch ([`meeting_patch`]): Graph's answer, the meeting as it is now.
    /// An id that isn't plausible is refused before any request.
    pub fn update_meeting(&self, id: &str, patch: &Value) -> Result<Value> {
        self.request("PATCH", &self.meeting_at(id)?, Some(patch), false, true)
    }

    /// Delete a meeting; one already gone (404) counts as deleted. An id that isn't plausible is
    /// refused before any request.
    pub fn delete_meeting(&self, id: &str) -> Result<()> {
        let resp = self.send(&self.prepared("DELETE", &self.meeting_at(id)?, None, false)?, true)?;
        if resp.status == 404 {
            return Ok(());
        }
        status_error(resp.status, &resp.body)
    }

    /// A meeting's address: its id, when plausible, encoded as one segment.
    fn meeting_at(&self, id: &str) -> Result<String> {
        if !valid_teams_meeting_id(id) {
            return Err(NetError::Refused("not a meeting's id".into()));
        }
        Ok(self.at(&format!("/me/onlineMeetings/{}", seg(id))))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A meeting as the fake Graph answers one, audio conferencing on.
    fn answer() -> Value {
        json!({
            "id": "+///MSoxKjAqKjE5Om1lZXRpbmdfMUB0aHJlYWQudjI=",
            "subject": "Budget", "startDateTime": "2026-11-20T09:00:00Z", "endDateTime": "2026-11-20T10:00:00Z",
            "joinWebUrl": "https://teams.microsoft.com/l/meetup-join/19%3ameeting_1%40thread.v2/0?context=%7b%22Tid%22%3a%22tenant-1%22%7d",
            "joinMeetingIdSettings": {"joinMeetingId": "1234567890", "passcode": "aB3xY9", "isPasscodeRequired": true},
            "audioConferencing": {"tollNumber": "+1 323-555-0166", "tollFreeNumber": null, "conferenceId": "9999999",
                "dialinUrl": "https://dialin.teams.microsoft.com/usp/pstnconferencing"},
            "meetingOptionsWebUrl": "https://teams.microsoft.com/meetingOptions/?organizerId=u-me&threadId=19_meeting_1@thread.v2",
            "joinInformation": {"content": "<div>Microsoft's own invitation text</div>"},
            "chatInfo": {"threadId": "19:meeting_1@thread.v2", "messageId": "0", "replyChainMessageId": null},
        })
    }

    /// [`answer`] with `key` set to `value` (`Null` too), or taken out when `None`.
    fn with(key: &str, value: Option<Value>) -> Value {
        let mut v = answer();
        if let Value::Object(o) = &mut v {
            match value {
                Some(x) => {
                    o.insert(key.into(), x);
                }
                None => {
                    o.remove(key);
                }
            }
        }
        v
    }

    fn budget() -> TeamsMeetingFields {
        TeamsMeetingFields { title: "Budget".into(), start: 1_795_165_200, end: 1_795_168_800 }
    }

    #[test]
    fn the_body_says_the_title_and_utc_times() {
        let f = TeamsMeetingFields { title: "Budget".into(), start: 1_795_165_200, end: 1_795_168_800 };
        assert_eq!(meeting_body(&f), json!({"subject": "Budget", "startDateTime": "2026-11-20T09:00:00Z", "endDateTime": "2026-11-20T10:00:00Z"}));
    }

    #[test]
    fn the_subject_is_one_bounded_line() {
        let subject = |title: &str| {
            let f = TeamsMeetingFields { title: title.into(), ..budget() };
            meeting_body(&f)["subject"].as_str().unwrap().to_string()
        };
        let long = subject(&format!("A\nB\u{202E}{}", "x".repeat(300)));
        assert_eq!(long, format!("A B{}", "x".repeat(252)));
        assert_eq!(long.chars().count(), MEETING_SUBJECT_MAX);
        assert!(!long.chars().any(|c| c.is_control() || is_bidi(c)), "{long:?}");
        // Line breaks and controls are a space, a run of them one, none at either end.
        assert_eq!(subject("  Budget\t\r\n\u{7} review \u{2066}2027\u{2069}\n"), "Budget review 2027");
        // Cut on a character, never inside one, and never on a space.
        assert_eq!(subject(&"é".repeat(300)), "é".repeat(MEETING_SUBJECT_MAX));
        assert_eq!(subject(&format!("{} y", "x".repeat(MEETING_SUBJECT_MAX - 1))), "x".repeat(MEETING_SUBJECT_MAX - 1));
        assert_eq!(subject(""), "");
    }

    #[test]
    fn a_patch_says_only_what_changed() {
        let before = budget();
        let later = TeamsMeetingFields { start: before.start + 3600, ..before.clone() };
        assert_eq!(meeting_patch(&before, &later), Some(json!({"startDateTime": "2026-11-20T10:00:00Z"})));
        assert_eq!(meeting_patch(&before, &before), None);
        let longer = TeamsMeetingFields { end: before.end + 1800, ..before.clone() };
        assert_eq!(meeting_patch(&before, &longer), Some(json!({"endDateTime": "2026-11-20T10:30:00Z"})));
        let renamed = TeamsMeetingFields { title: "Budget 2027".into(), ..before.clone() };
        assert_eq!(meeting_patch(&before, &renamed), Some(json!({"subject": "Budget 2027"})));
        // A title that sends the same subject sends nothing.
        let spaced = TeamsMeetingFields { title: " Budget\n".into(), ..before.clone() };
        assert_eq!(meeting_patch(&before, &spaced), None);
        let all = TeamsMeetingFields { title: "B".into(), start: 0, end: 60 };
        assert_eq!(
            meeting_patch(&before, &all),
            Some(json!({"subject": "B", "startDateTime": "1970-01-01T00:00:00Z", "endDateTime": "1970-01-01T00:01:00Z"}))
        );
    }

    #[test]
    fn an_answer_is_read_whole() {
        let m = meeting_of(&answer()).unwrap();
        assert_eq!(
            m,
            TeamsMeeting {
                id: "+///MSoxKjAqKjE5Om1lZXRpbmdfMUB0aHJlYWQudjI=".into(),
                join_url: "https://teams.microsoft.com/l/meetup-join/19%3ameeting_1%40thread.v2/0?context=%7b%22Tid%22%3a%22tenant-1%22%7d".into(),
                meeting_id: "1234567890".into(),
                passcode: Secret("aB3xY9".into()),
                toll_number: "+1 323-555-0166".into(),
                conference_id: "9999999".into(),
                options_url: "https://teams.microsoft.com/meetingOptions/?organizerId=u-me&threadId=19_meeting_1@thread.v2".into(),
            }
        );
        assert_eq!(meeting_id_of(&answer()).as_deref(), Some("+///MSoxKjAqKjE5Om1lZXRpbmdfMUB0aHJlYWQudjI="));
        // Without audio conferencing (Graph's `null`), no number.
        let m = meeting_of(&with("audioConferencing", Some(Value::Null))).unwrap();
        assert_eq!((m.toll_number.as_str(), m.conference_id.as_str(), m.meeting_id.as_str()), ("", "", "1234567890"));
    }

    #[test]
    fn an_id_that_cannot_be_one_refuses_the_answer() {
        for id in [json!("a?b"), json!(""), json!("a".repeat(513)), json!(7), Value::Null] {
            let v = with("id", Some(id.clone()));
            assert_eq!(meeting_id_of(&v), None, "{id}");
            assert_eq!(meeting_of(&v), None, "{id}");
        }
        assert_eq!(meeting_of(&with("id", None)), None);
        assert_eq!(meeting_of(&json!([answer()])), None);
        assert_eq!(meeting_of(&json!("x")), None);
    }

    #[test]
    fn a_link_elsewhere_refuses_the_answer() {
        let long = format!("https://teams.microsoft.com/l/meetup-join/{}", "a".repeat(8 << 10));
        for join in ["https://teams.microsoft.com.evil.example/x", "javascript:alert(1)", long.as_str()] {
            assert_eq!(meeting_of(&with("joinWebUrl", Some(json!(join)))), None, "{join}");
        }
        // A meeting has a link to join it, as text.
        for join in [None, Some(Value::Null), Some(json!("")), Some(json!(7))] {
            assert_eq!(meeting_of(&with("joinWebUrl", join.clone())), None, "{join:?}");
        }
        for options in ["https://evil.example/meetingOptions/", "https://teams.microsoft.com/o\u{202E}x", " "] {
            assert_eq!(meeting_of(&with("meetingOptionsWebUrl", Some(json!(options)))), None, "{options:?}");
        }
        // Options Graph doesn't give (only their organiser gets them) leave the meeting without.
        for options in [None, Some(Value::Null), Some(json!("")), Some(json!(["https://evil.example/"]))] {
            let m = meeting_of(&with("meetingOptionsWebUrl", options.clone()));
            assert_eq!(m.map(|m| m.options_url), Some(String::new()), "{options:?}");
        }
    }

    #[test]
    fn values_that_are_not_plain_stay_out() {
        let mut v = answer();
        v["joinMeetingIdSettings"]["passcode"] = json!("<b>x</b>");
        v["audioConferencing"]["tollNumber"] = json!("1".repeat(65));
        v["audioConferencing"]["conferenceId"] = json!("1\n2");
        let m = meeting_of(&v).unwrap();
        assert_eq!((m.passcode.0.as_str(), m.toll_number.as_str(), m.conference_id.as_str()), ("", "", ""));
        assert_eq!(m.meeting_id, "1234567890");
        assert!(m.join_url.starts_with("https://teams.microsoft.com/l/meetup-join/") && m.options_url.starts_with("https://teams.microsoft.com/"));
        // A value that isn't text is none; one with spaces around is trimmed.
        v["joinMeetingIdSettings"]["joinMeetingId"] = json!(1_234_567_890);
        v["joinMeetingIdSettings"]["passcode"] = json!(" aB3xY9 ");
        let m = meeting_of(&v).unwrap();
        assert_eq!((m.meeting_id.as_str(), m.passcode.0.as_str()), ("", "aB3xY9"));
        let m = meeting_of(&with("joinMeetingIdSettings", Some(json!(["1234567890"])))).unwrap();
        assert_eq!((m.meeting_id.as_str(), m.passcode.0.as_str()), ("", ""));
    }

    #[test]
    fn microsofts_invitation_text_is_never_read() {
        let m = meeting_of(&answer()).unwrap();
        let fields = [&m.id, &m.join_url, &m.meeting_id, &m.passcode.0, &m.toll_number, &m.conference_id, &m.options_url];
        assert!(!fields.iter().any(|f| f.contains("invitation") || f.contains("<div>")), "{fields:?}");
        // Its links are never taken either, even when Graph gives no other.
        let mut v = with("joinWebUrl", None);
        v["joinInformation"]["content"] = json!("https://teams.microsoft.com/l/meetup-join/19%3ameeting_1%40thread.v2/0");
        assert_eq!(meeting_of(&v), None);
        let mut v = with("meetingOptionsWebUrl", None);
        v["joinInformation"]["content"] = json!("https://teams.microsoft.com/meetingOptions/");
        assert_eq!(meeting_of(&v).map(|m| m.options_url), Some(String::new()));
    }
}
