// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The Teams worker's meetings: the Teams meetings of the calendar's appointments, made, changed
//! and deleted at Graph's `/me/onlineMeetings`, each the engine's ask, answered with an event.
//!
//! * The asks wait in a queue, in the order they came, [`MEETING_QUEUE`] at most: one more is
//!   answered "no connection" at once (Teams said nothing). The queue is served before the rest
//!   of the worker's steps, one request a turn, through the limiter ([`Route::Meeting`]).
//! * Every ask is answered once: the meeting, or why not ([`TeamsMeetingError`]). Signed out or
//!   switched off, "signed out", at once, and so is what waited when Teams stops; working
//!   offline, "no connection", and so is what waited when "Work offline" comes on. Nothing waits
//!   past [`MEETING_WAIT`]: it is "no connection" then.
//! * Graph asking to wait (a 429; a 503 to a change or a deletion, which can be made twice) is
//!   waited out, and the request sent again, [`MEETING_SENDS`] sends in all. A creation is never
//!   sent again after a server's error: one made though its answer was lost would be two. The
//!   network away, the request waits for it, within its two minutes.
//! * A change that changes nothing asks nothing; it is answered in its turn, after what was asked
//!   before it. A meeting already gone counts as deleted.
//! * The meetings' permission said missing (a 403 `Forbidden` or `InsufficientPrivileges`, the
//!   token not saying it carries it) is let go ([`Worker::meetings_lost`]): the renewals ask for
//!   the chats' permissions alone, and the chats go on. While it's missing, an ask is answered so
//!   without a request, those waiting as those that come: Graph would only say it again.
//! * Graph's answer is read while its ask is still the one under way: a bug met reading it is
//!   given up with an answer ([`given_up`]), never without one.
//!
//! What is hostile here: Graph's answers. A meeting is taken only as [`meeting_of`] reads it. A
//! creation whose answer is refused (a link that isn't Teams' own) is answered "untrusted", and
//! the meeting it made deleted at once when its id is plausible; nobody hears of that deletion.
//! What it protects: the appointments (no link but Teams' own), the user's Teams (a creation
//! never sent again after a server's error, an untrusted meeting not left behind), the worker (a
//! bounded queue, a bounded wait) and Graph's limits. The log says statuses, Graph's codes,
//! counts and keyed hashes ([`Worker::hashed`]): never a title, a link, a passcode or an id.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use quatrelle_model::pim::teams::{Event, TeamsMeeting, TeamsMeetingError, TeamsMeetingFields, valid_teams_meeting_id};
use serde_json::Value;

use super::graph::{meeting_id_of, meeting_of, meeting_patch};
use super::limits::Route;
use super::worker::{Worker, busy, kind, meetings_granted, of_the_network};
use crate::{NetError, Result};

/// The most asks that wait; one more is answered "no connection" at once.
pub(crate) const MEETING_QUEUE: usize = 32;
/// The most sends of a request Graph asks to wait for (429).
pub(crate) const MEETING_SENDS: u32 = 3;
/// The longest an ask waits for Teams: it is "no connection" then.
pub(crate) const MEETING_WAIT: Duration = Duration::from_secs(120);
/// The longest error code of Graph's told, as the client keeps them.
const CODE_MAX: usize = 64;

/// The asks waiting, in their order, and the one whose request is under way.
#[derive(Default)]
pub(crate) struct Meetings {
    queue: VecDeque<Waiting>,
    /// Out of the queue while its request goes: answered by the step that sent it (whatever
    /// [`stop`] says of the others meanwhile), or by [`given_up`] when a bug met on the way ended
    /// that step.
    sending: Option<Waiting>,
}

/// An ask waiting: what it is, when it came, and how many times its request went.
struct Waiting {
    what: What,
    asked: Instant,
    sends: u32,
}

enum What {
    Create {
        ask: u64,
        fields: TeamsMeetingFields,
    },
    /// `patch`: what changed ([`meeting_patch`]); none, nothing to ask.
    Update {
        id: String,
        patch: Option<Value>,
    },
    Delete {
        id: String,
    },
    /// A meeting made whose answer was refused: deleted, nobody told.
    Discard {
        id: String,
    },
}

/// The request an ask makes, its own copy: the ask waits aside while it goes.
enum Request {
    Create(TeamsMeetingFields),
    Update(String, Value),
    Delete(String),
}

impl What {
    /// Its request; none for a change that changes nothing.
    fn request(&self) -> Option<Request> {
        match self {
            What::Create { fields, .. } => Some(Request::Create(fields.clone())),
            What::Update { id, patch } => patch.clone().map(|p| Request::Update(id.clone(), p)),
            What::Delete { id } | What::Discard { id } => Some(Request::Delete(id.clone())),
        }
    }

    /// What it is, as the log says it.
    fn name(&self) -> &'static str {
        match self {
            What::Create { .. } => "making a meeting",
            What::Update { .. } => "changing a meeting",
            What::Delete { .. } => "deleting a meeting",
            What::Discard { .. } => "deleting a meeting whose answer was refused",
        }
    }
}

/// [`Job::MeetingCreate`]: a meeting of `fields`, for the window that asked (`ask`).
///
/// [`Job::MeetingCreate`]: quatrelle_model::pim::teams::Job::MeetingCreate
pub(crate) fn create(w: &mut Worker, ask: u64, fields: TeamsMeetingFields, now: Instant) {
    queue(w, What::Create { ask, fields }, now);
}

/// [`Job::MeetingUpdate`]: the meeting `id` changed from `before` (what was sent last) to
/// `after`; only what changed is sent.
///
/// [`Job::MeetingUpdate`]: quatrelle_model::pim::teams::Job::MeetingUpdate
pub(crate) fn update(w: &mut Worker, id: String, before: TeamsMeetingFields, after: TeamsMeetingFields, now: Instant) {
    let patch = meeting_patch(&before, &after);
    queue(w, What::Update { id, patch }, now);
}

/// [`Job::MeetingDelete`]: the meeting `id` deleted.
///
/// [`Job::MeetingDelete`]: quatrelle_model::pim::teams::Job::MeetingDelete
pub(crate) fn delete(w: &mut Worker, id: String, now: Instant) {
    queue(w, What::Delete { id }, now);
}

/// An ask in the queue, or answered at once: nothing can be asked of Teams (signed out or
/// switched off; working offline), the meeting's id can't be one (no request could name it), or
/// the queue is full.
fn queue(w: &mut Worker, what: What, now: Instant) {
    let unasked = if !w.polling() {
        Some(TeamsMeetingError::NotSignedIn)
    } else if w.offline() {
        Some(TeamsMeetingError::Offline)
    } else if without_permission(w, &what) {
        Some(TeamsMeetingError::Consent)
    } else {
        None
    };
    if let Some(error) = unasked {
        return answer(w, what, error);
    }
    if let What::Update { id, .. } | What::Delete { id } = &what
        && !valid_teams_meeting_id(id)
    {
        log::warn!("teams: a meeting's id that can't be one was refused");
        return answer(w, what, TeamsMeetingError::Untrusted);
    }
    if w.meetings.queue.len() >= MEETING_QUEUE {
        log::warn!("teams: {MEETING_QUEUE} meeting requests wait already; one more was turned away");
        return answer(w, what, TeamsMeetingError::Offline);
    }
    w.meetings.queue.push_back(Waiting { what, asked: now, sends: 0 });
}

/// Whether `what` would only hear again that the meetings' permission is missing: it is, and
/// `what` is the user's ask (a meeting of Quatrelle's own to delete is still tried, nobody told).
fn without_permission(w: &Worker, what: &What) -> bool {
    !w.settings.meetings && !matches!(what, What::Discard { .. })
}

/// Whether an ask waits for its request.
pub(crate) fn waiting(w: &Worker) -> bool {
    !w.meetings.queue.is_empty()
}

/// When the first ask waiting is out of time ([`MEETING_WAIT`]): the loop looks then, whatever
/// else holds it.
pub(crate) fn deadline(w: &Worker) -> Option<Instant> {
    w.meetings.queue.iter().filter_map(|m| m.asked.checked_add(MEETING_WAIT)).min()
}

/// One step, a request at most: the first ask waiting, when the limiter lets it go. A change
/// that changes nothing, found first, is answered on the way, without one. Whether a request
/// went.
pub(crate) fn step(w: &mut Worker, now: Instant) -> bool {
    loop {
        // The permission went while it waited: answered so, without a request.
        if w.meetings.queue.front().is_some_and(|m| without_permission(w, &m.what)) {
            if let Some(m) = w.meetings.queue.pop_front() {
                answer(w, m.what, TeamsMeetingError::Consent);
            }
            continue;
        }
        let Some(request) = w.meetings.queue.front().map(|m| m.what.request()) else { return false };
        let Some(request) = request else {
            if let Some(Waiting { what: What::Update { id, .. }, .. }) = w.meetings.queue.pop_front() {
                log::info!("teams: meeting {} had nothing to change", w.hashed(&id));
                w.say(Event::MeetingUpdated { id, meeting: None, error: None });
            }
            continue;
        };
        if !w.may(Route::Meeting, now) {
            return false;
        }
        let Some(mut m) = w.meetings.queue.pop_front() else { return false };
        m.sends = m.sends.saturating_add(1);
        w.meetings.sending = Some(m);
        let result = send(w, &request, now);
        // Read while its ask is still the one under way ([`given_up`] answers it then).
        let result = result.map(|v| {
            let what = w.meetings.sending.as_ref().map(|m| &m.what);
            #[cfg(test)]
            w.trip_meeting();
            what.map(|what| read(what, &v)).unwrap_or_default()
        });
        if let Some(m) = w.meetings.sending.take() {
            match result {
                Ok(read) => done(w, m, read, now),
                Err(e) => failed(w, m, &e, now),
            }
        }
        return true;
    }
}

fn send(w: &mut Worker, request: &Request, now: Instant) -> Result<Value> {
    match request {
        Request::Create(fields) => w.request(Route::Meeting, now, |c| c.create_meeting(fields)),
        Request::Update(id, patch) => w.request(Route::Meeting, now, |c| c.update_meeting(id, patch)),
        Request::Delete(id) => w.request(Route::Meeting, now, |c| c.delete_meeting(id)).map(|()| Value::Null),
    }
}

/// What Graph's answer to a request says, as [`read`] takes it.
#[derive(Default)]
struct Answer {
    /// The meeting, as [`meeting_of`] reads it: for a change, only when it is that meeting.
    meeting: Option<TeamsMeeting>,
    /// A creation's answer refused: the id of the meeting it made, when plausible.
    made: Option<String>,
    /// The answer is empty (a change made, said so).
    empty: bool,
}

/// Graph's answer to the request of `what`, read: the hostile part, done while its ask is still
/// the one under way.
fn read(what: &What, v: &Value) -> Answer {
    match what {
        What::Create { .. } => {
            let meeting = meeting_of(v);
            let made = if meeting.is_none() { meeting_id_of(v) } else { None };
            Answer { meeting, made, empty: false }
        }
        What::Update { id, .. } => Answer { meeting: meeting_of(v).filter(|meeting| meeting.id == *id), made: None, empty: v.is_null() },
        What::Delete { .. } | What::Discard { .. } => Answer::default(),
    }
}

/// Graph's answer to a request ([`read`]): told to its ask. A creation's that is refused is
/// "untrusted", and the meeting it made is deleted next, when its id is plausible. A change's
/// answer that isn't that meeting, or isn't read, is "untrusted" too (nothing of it is taken); an
/// empty one says the change was made.
fn done(w: &mut Worker, m: Waiting, a: Answer, now: Instant) {
    match m.what {
        What::Create { ask, .. } => {
            if let Some(meeting) = a.meeting {
                log::info!("teams: a meeting was made: {}", w.hashed(&meeting.id));
                return w.say(Event::MeetingCreated { ask, meeting: Some(meeting), error: None });
            }
            let id = a.made.filter(|_| w.polling());
            log::warn!("teams: a meeting was made, but Graph's answer was refused{}", if id.is_some() { "; it is deleted" } else { "" });
            w.say(Event::MeetingCreated { ask, meeting: None, error: Some(TeamsMeetingError::Untrusted) });
            if let Some(id) = id {
                w.meetings.queue.push_front(Waiting { what: What::Discard { id }, asked: now, sends: 0 });
            }
        }
        What::Update { id, .. } => {
            let meeting = a.meeting;
            let error = (meeting.is_none() && !a.empty).then_some(TeamsMeetingError::Untrusted);
            if error.is_some() {
                log::warn!("teams: meeting {} was changed, but Graph's answer was refused", w.hashed(&id));
            } else {
                log::info!("teams: meeting {} was changed", w.hashed(&id));
            }
            w.say(Event::MeetingUpdated { id, meeting, error });
        }
        What::Delete { id } => {
            log::info!("teams: meeting {} was deleted", w.hashed(&id));
            w.say(Event::MeetingDeleted { id, error: None });
        }
        What::Discard { id } => log::info!("teams: meeting {}, whose answer was refused, was deleted", w.hashed(&id)),
    }
}

/// A request that failed. Graph asking to wait (a 429; a 503 but to a creation): waited out, and
/// sent again then, [`MEETING_SENDS`] sends in all. The network away: it waits for it, within its
/// two minutes ([`expire`]). Anything else is the answer ([`error_of`]); the meetings' permission
/// missing is let go, and the chats go on.
fn failed(w: &mut Worker, m: Waiting, e: &NetError, now: Instant) {
    log::warn!("teams: {} failed: {}", m.what.name(), kind(e));
    // Teams stopped on the way (the renewed token refused too): what waited was answered, and
    // this one is told why.
    let going = w.polling() && !w.offline();
    if going && busy(e) {
        w.busy_wait(now);
        let throttled = matches!(e, NetError::Closed(said) if said.starts_with("HTTP 429"));
        if m.sends < MEETING_SENDS && (throttled || !matches!(m.what, What::Create { .. })) {
            w.meetings.queue.push_front(m);
            return;
        }
    } else if going && of_the_network(e) {
        w.pause(e, now);
        w.meetings.queue.push_front(m);
        return;
    }
    let error = error_of(e, w.tokens.as_ref().map_or("", |t| t.granted.as_str()));
    if error == TeamsMeetingError::Consent {
        log::warn!("teams: the meetings' permission is missing; the chats go on without it");
        w.meetings_lost();
    }
    answer(w, m.what, error);
}

/// What waited since [`MEETING_WAIT`] or longer is answered "no connection"; the rest keeps its
/// order.
pub(crate) fn expire(w: &mut Worker, now: Instant) {
    let late = |m: &Waiting| now.saturating_duration_since(m.asked) >= MEETING_WAIT;
    if !w.meetings.queue.iter().any(late) {
        return;
    }
    let (out, kept): (VecDeque<Waiting>, VecDeque<Waiting>) = std::mem::take(&mut w.meetings.queue).into_iter().partition(late);
    w.meetings.queue = kept;
    log::warn!("teams: {} meeting requests waited two minutes, and were let go", out.len());
    for m in out {
        answer(w, m.what, TeamsMeetingError::Offline);
    }
}

/// Nothing more is asked of Teams: every ask waiting is answered `error`, and the queue emptied.
/// The request under way, if any, is its step's to answer.
pub(crate) fn stop(w: &mut Worker, error: TeamsMeetingError) {
    let queue = std::mem::take(&mut w.meetings.queue);
    if !queue.is_empty() {
        log::info!("teams: {} meeting requests waiting were answered at once", queue.len());
    }
    for m in queue {
        answer(w, m.what, error.clone());
    }
}

/// A bug met while a request went ended its step: its ask is answered "no connection" (Teams'
/// answer, if it came, is lost), never left without one.
pub(crate) fn given_up(w: &mut Worker) {
    if let Some(m) = w.meetings.sending.take() {
        log::warn!("teams: {} was given up", m.what.name());
        answer(w, m.what, TeamsMeetingError::Offline);
    }
}

/// An ask told why Teams didn't do it; a deletion of Quatrelle's own, no one.
fn answer(w: &Worker, what: What, error: TeamsMeetingError) {
    match what {
        What::Create { ask, .. } => w.say(Event::MeetingCreated { ask, meeting: None, error: Some(error) }),
        What::Update { id, .. } => w.say(Event::MeetingUpdated { id, meeting: None, error: Some(error) }),
        What::Delete { id } => w.say(Event::MeetingDeleted { id, error: Some(error) }),
        What::Discard { .. } => log::warn!("teams: a meeting whose answer was refused is left in Teams"),
    }
}

/// What a failure is to its ask. The sign-in refused (once renewed) is "sign in again", the
/// network "no connection". Graph's 403 `Forbidden` or `InsufficientPrivileges` is the meetings'
/// permission missing, unless `granted` (the permissions the token says it carries) names it:
/// then, as any other 403, the account may not make meetings. Any other status is Graph's
/// refusal, with its code; anything else an answer Quatrelle doesn't take. Never an error's own
/// words.
pub(crate) fn error_of(e: &NetError, granted: &str) -> TeamsMeetingError {
    if matches!(e, NetError::Auth(_)) {
        return TeamsMeetingError::SignIn;
    }
    if of_the_network(e) {
        return TeamsMeetingError::Offline;
    }
    match status_of(e) {
        Some((403, code)) if ["Forbidden", "InsufficientPrivileges"].iter().any(|c| code.eq_ignore_ascii_case(c)) => {
            // An empty `granted` says nothing of the permission (`meetings_granted` takes it as given).
            if !granted.trim().is_empty() && meetings_granted(granted) { TeamsMeetingError::NotAllowed } else { TeamsMeetingError::Consent }
        }
        Some((403, _)) => TeamsMeetingError::NotAllowed,
        Some((status, code)) => TeamsMeetingError::Refused { status, code },
        None => TeamsMeetingError::Untrusted,
    }
}

/// Graph's status and error code, as the client writes a refusal (`HTTP 403 Forbidden`; a 429 or
/// a 5xx as the connection's, `HTTP 503`). A code that isn't plain is none.
fn status_of(e: &NetError) -> Option<(u16, String)> {
    let (NetError::Protocol(said) | NetError::Closed(said)) = e else { return None };
    let rest = said.strip_prefix("HTTP ")?;
    let (digits, code) = rest.split_once(' ').unwrap_or((rest, ""));
    if digits.len() != 3 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let status = digits.parse().ok()?;
    let plain = code.len() <= CODE_MAX && code.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
    Some((status, if plain { code.to_string() } else { String::new() }))
}
