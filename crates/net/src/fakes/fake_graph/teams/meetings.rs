// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The fake Graph server's online meetings, as `/me/onlineMeetings` answers them: `POST` makes one
//! (201; `subject`, `startDateTime` and `endDateTime` wanted, the times in UTC, `…Z`, else 400
//! `BadRequest`), `PATCH /{id}` changes it (200) and `DELETE /{id}` deletes it (204; an unknown
//! one 404 `NotFound`). The id is base64, as Graph's is, and carries `/`, `+` and `=`: the fake
//! finds the meeting by the id of the decoded path, and keeps the raw path in `meeting_paths`.

use super::*;

/// A UTC time (`…Z`) the fake accepts: a real RFC 3339 date and time, written with a `Z`.
fn utc_time(v: Option<&Value>) -> Option<String> {
    let t = v?.as_str()?;
    if !t.ends_with('Z') {
        return None;
    }
    // A month or a day out of range moves the time: what comes back must read as it was written.
    let back = rfc3339(ms_of(t)?);
    (back.get(..19) == t.get(..19)).then(|| t.to_string())
}

impl FakeTeams {
    /// The meeting routes (`createMeeting`, `updateMeeting`, `deleteMeeting`); `None` for others.
    pub(super) fn meeting_route(&mut self, name: &str, p: &[&str], input: &Value) -> Option<Reply> {
        let id = p.get(2).copied().unwrap_or_default();
        let mut reply = match name {
            "createMeeting" => self.create_meeting(input),
            "updateMeeting" => self.update_meeting(id, input),
            "deleteMeeting" => {
                let before = self.meetings.len();
                self.meetings.retain(|m| m.id != id);
                if self.meetings.len() == before {
                    return Some(Reply::error(404, "NotFound", "No such meeting."));
                }
                return Some(Reply::empty(204));
            }
            _ => return None,
        };
        if reply.status < 300
            && let Some(Value::Object(over)) = self.meeting_answer.take()
            && let Ok(Value::Object(mut v)) = serde_json::from_slice::<Value>(&reply.body)
        {
            v.extend(over);
            reply = Reply::json(reply.status, &Value::Object(v));
        }
        Some(reply)
    }

    fn create_meeting(&mut self, input: &Value) -> Reply {
        let bad = |why: &str| Reply::error(400, "BadRequest", why);
        let Some(subject) = input.get("subject").and_then(Value::as_str) else { return bad("subject is missing.") };
        let (Some(start), Some(end)) = (utc_time(input.get("startDateTime")), utc_time(input.get("endDateTime"))) else {
            return bad("startDateTime and endDateTime are wanted, in UTC (…Z).");
        };
        let n = self.hits.get("createMeeting").copied().unwrap_or(1);
        // Base64 of bytes that spell `/`, `+` and `=`, as Graph's ids may.
        let mut raw = vec![0xfb, 0xff, 0xff];
        raw.extend(format!("1*{n}*0**19:meeting_{n}@thread.v2").bytes());
        while raw.len() % 3 == 0 {
            raw.push(b'!');
        }
        let meeting = FakeMeeting {
            id: crate::imap::base64(&raw),
            subject: subject.into(),
            start,
            end,
            join_url: format!(
                "https://teams.microsoft.com/l/meetup-join/19%3ameeting_{n}%40thread.v2/0?context=%7b%22Tid%22%3a%22{}%22%7d",
                self.tenant
            ),
        };
        let made = self.meeting_json(&meeting, n);
        self.meetings.push(meeting);
        Reply::json(201, &made)
    }

    fn update_meeting(&mut self, id: &str, input: &Value) -> Reply {
        let bad = |why: &str| Reply::error(400, "BadRequest", why);
        let Some(at) = self.meetings.iter().position(|m| m.id == id) else { return Reply::error(404, "NotFound", "No such meeting.") };
        let mut m = self.meetings[at].clone();
        if let Some(s) = input.get("subject") {
            let Some(s) = s.as_str() else { return bad("subject must be text.") };
            m.subject = s.into();
        }
        for (key, slot) in [("startDateTime", &mut m.start), ("endDateTime", &mut m.end)] {
            if let Some(v) = input.get(key) {
                let Some(t) = utc_time(Some(v)) else { return bad("The times are wanted in UTC (…Z).") };
                *slot = t;
            }
        }
        if self.meeting_moves {
            m.join_url = m.join_url.replacen("/0?", "/1?", 1);
        }
        // Its thread's number is the one its join link was made with.
        let n = m.join_url.split("meeting_").nth(1).and_then(|r| r.split("%40").next()).and_then(|n| n.parse().ok()).unwrap_or(0);
        let answer = self.meeting_json(&m, n);
        self.meetings[at] = m;
        Reply::json(200, &answer)
    }

    /// A meeting as Graph answers it; `n` numbers its thread.
    fn meeting_json(&self, m: &FakeMeeting, n: u32) -> Value {
        let audio = if self.audio_conferencing {
            json!({"tollNumber": "+1 323-555-0166", "tollFreeNumber": null, "conferenceId": "9999999",
                "dialinUrl": "https://dialin.teams.microsoft.com/usp/pstnconferencing"})
        } else {
            Value::Null
        };
        json!({
            "id": m.id, "subject": m.subject, "startDateTime": m.start, "endDateTime": m.end, "joinWebUrl": m.join_url,
            "joinMeetingIdSettings": {"joinMeetingId": "1234567890", "passcode": "aB3xY9", "isPasscodeRequired": true},
            "audioConferencing": audio,
            "meetingOptionsWebUrl": format!("https://teams.microsoft.com/meetingOptions/?organizerId={}&threadId=19_meeting_{n}@thread.v2", self.me.id),
            "joinInformation": {"content": "<div>Microsoft's own invitation text</div>"},
            "chatInfo": {"threadId": format!("19:meeting_{n}@thread.v2"), "messageId": "0", "replyChainMessageId": null},
        })
    }
}
