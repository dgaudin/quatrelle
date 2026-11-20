// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Teams meetings through the worker, driven by hand at the times the test makes, against the
//! fake Graph: made, changed and deleted one request at a time, in the order asked, each ask
//! answered once; a 429 waited out and sent again, three sends at most; a 5xx on a creation never
//! sent again; the permission missing (the chats go on) or the account's; a 401 after a renewal;
//! no network, two minutes at most; signed out, switched off and working offline; a full queue;
//! a link elsewhere, its meeting deleted; a panic on the way; and what the log says.

use std::collections::BTreeMap;

use quatrelle_model::pim::teams::{TeamsMeeting, TeamsMeetingError, TeamsMeetingFields};
use serde_json::Value;

use super::super::sync::{Run, STEP};
use super::*;
use crate::oauth::TEAMS_MEETINGS_SCOPE;
use crate::teams::meetings::{self, MEETING_QUEUE, MEETING_SENDS, MEETING_WAIT, error_of};
use crate::teams::worker::TURN;

/// 2026-11-20T09:00:00Z: when the meetings start.
const NINE: i64 = 1_795_165_200;
/// A meeting's id as Graph writes them (base64, with `/`, `+` and `=`), not one of the fake's.
const SOMEONE: &str = "MSpkYzE3/NzRj+LTgx=";

fn budget() -> TeamsMeetingFields {
    TeamsMeetingFields { title: "Budget".into(), start: NINE, end: NINE + 3600 }
}

/// [`budget`], begun half an hour later.
fn later() -> TeamsMeetingFields {
    TeamsMeetingFields { start: NINE + 1800, ..budget() }
}

fn created(ask: u64, error: TeamsMeetingError) -> model::Event {
    model::Event::MeetingCreated { ask, meeting: None, error: Some(error) }
}

fn updated(id: &str, error: TeamsMeetingError) -> model::Event {
    model::Event::MeetingUpdated { id: id.into(), meeting: None, error: Some(error) }
}

fn deleted(id: &str, error: Option<TeamsMeetingError>) -> model::Event {
    model::Event::MeetingDeleted { id: id.into(), error }
}

fn refused(status: u16, code: &str) -> TeamsMeetingError {
    TeamsMeetingError::Refused { status, code: code.into() }
}

/// What the worker answered of meetings, in order.
fn answers(seen: &[model::Event]) -> Vec<model::Event> {
    seen.iter()
        .filter(|e| matches!(e, model::Event::MeetingCreated { .. } | model::Event::MeetingUpdated { .. } | model::Event::MeetingDeleted { .. }))
        .cloned()
        .collect()
}

/// What the worker said of meetings being allowed, in order.
fn allowed(seen: &[model::Event]) -> Vec<bool> {
    seen.iter().filter_map(|e| if let model::Event::Meetings { allowed } = e { Some(*allowed) } else { None }).collect()
}

/// How many times each ask was answered, by ask.
fn per_ask(said: &[model::Event]) -> BTreeMap<u64, usize> {
    let mut n = BTreeMap::new();
    for e in said {
        if let model::Event::MeetingCreated { ask, .. } = e {
            *n.entry(*ask).or_default() += 1;
        }
    }
    n
}

/// A worker signed in by hand with the meetings' permission, its first pass over ([`Run`]), and
/// the sign-in service of the test.
struct Meet {
    r: Run,
    desk: Arc<Desk>,
}

fn signed(teams: FakeTeams) -> Meet {
    let server = serve_teams(teams);
    let (mut w, desk, seen, creds) = by_hand(&server, grant(Some(TENANT)));
    let t0 = Instant::now();
    w.job(Job::SignIn { client_id: CLIENT.into(), tenant: "organizations".into(), meetings: true });
    turn_until(&mut w, t0, "the account", |_| account(&teams_of(&seen)).is_some());
    let mut r = Run { w, server, seen, t0, at: Duration::ZERO, creds };
    r.turns(Duration::from_secs(5));
    assert!(r.w.settings.meetings && r.w.polling(), "signed in, meetings allowed");
    Meet { r, desk }
}

impl Meet {
    /// The test's clock.
    fn now(&self) -> Instant {
        self.r.t0 + self.r.at
    }

    fn create(&mut self, ask: u64, fields: TeamsMeetingFields) {
        let now = self.now();
        meetings::create(&mut self.r.w, ask, fields, now);
    }

    fn update(&mut self, id: &str, before: TeamsMeetingFields, after: TeamsMeetingFields) {
        let now = self.now();
        meetings::update(&mut self.r.w, id.into(), before, after, now);
    }

    fn delete(&mut self, id: &str) {
        let now = self.now();
        meetings::delete(&mut self.r.w, id.into(), now);
    }

    /// One turn, `step` after the last.
    fn turn_after(&mut self, step: Duration) {
        self.r.at += step;
        let now = self.now();
        self.r.w.turn(now);
    }

    fn answers(&self) -> Vec<model::Event> {
        answers(&self.r.events())
    }

    /// Each meeting request the fake had: its route, the id it named, its body.
    fn writes(&self) -> Vec<(String, String, Value)> {
        self.r.fake(|t| t.meeting_writes.clone())
    }

    /// A meeting of `fields` made for `ask`, as the worker answered it.
    fn made(&mut self, ask: u64, fields: TeamsMeetingFields) -> TeamsMeeting {
        self.create(ask, fields);
        self.r.turns(Duration::from_secs(1));
        let said = self.answers();
        match said.last() {
            Some(model::Event::MeetingCreated { ask: a, meeting: Some(m), error: None }) if *a == ask => m.clone(),
            _ => panic!("the meeting wasn't made: {said:?}"),
        }
    }

    /// Whether the loop, looked at now, waits [`TURN`] at least.
    fn never_spins(&self) -> bool {
        let now = self.now();
        self.r.w.wake(now).is_some_and(|at| at >= now + TURN)
    }
}

#[test]
fn a_meeting_is_created_and_answered_to_its_ask() {
    let mut m = signed(teams_with(Vec::new()));
    m.create(7, budget());
    m.r.turns(Duration::from_secs(1));
    let said = m.answers();
    let [model::Event::MeetingCreated { ask: 7, meeting: Some(meeting), error: None }] = said.as_slice() else { panic!("{said:?}") };
    let kept = m.r.fake(|t| t.meetings.clone());
    assert_eq!((kept.len(), meeting.id.as_str()), (1, kept[0].id.as_str()));
    assert_eq!(meeting.join_url, kept[0].join_url);
    assert_eq!((meeting.meeting_id.as_str(), meeting.passcode.0.as_str()), ("1234567890", "aB3xY9"));
    assert_eq!(m.r.hits("createMeeting"), 1);
    let body = json!({"subject": "Budget", "startDateTime": "2026-11-20T09:00:00Z", "endDateTime": "2026-11-20T10:00:00Z"});
    assert_eq!(m.writes(), [("createMeeting".to_string(), String::new(), body)]);
    assert!(!meetings::waiting(&m.r.w));
}

#[test]
fn an_update_sends_only_what_changed() {
    let mut m = signed(teams_with(Vec::new()));
    let made = m.made(1, budget());
    m.update(&made.id, budget(), later());
    m.r.turns(Duration::from_secs(1));
    let writes = m.writes();
    assert_eq!(writes.last(), Some(&("updateMeeting".to_string(), made.id.clone(), json!({"startDateTime": "2026-11-20T09:30:00Z"}))));
    let said = m.answers();
    assert!(
        matches!(said.last(), Some(model::Event::MeetingUpdated { id, meeting: Some(now), error: None }) if *id == made.id && now.id == made.id),
        "{said:?}"
    );
    // The same fields: nothing asked, and answered all the same, in its turn.
    m.update(&made.id, later(), later());
    m.r.turns(Duration::from_secs(1));
    assert_eq!(m.r.hits("updateMeeting"), 1);
    assert_eq!(m.answers().last(), Some(&model::Event::MeetingUpdated { id: made.id.clone(), meeting: None, error: None }));
    assert_eq!(m.answers().len(), 3);
}

#[test]
fn a_delete_of_a_meeting_gone_counts_as_deleted() {
    let mut m = signed(teams_with(Vec::new()));
    let made = m.made(1, budget());
    m.r.fake(|t| t.not_found_next.insert("deleteMeeting".into(), 1));
    m.delete(&made.id);
    m.r.turns(Duration::from_secs(1));
    assert_eq!(m.answers().last(), Some(&deleted(&made.id, None)));
    assert_eq!(m.r.hits("deleteMeeting"), 1);
    assert!(!meetings::waiting(&m.r.w));
}

#[test]
fn requests_go_in_the_order_asked() {
    let mut m = signed(teams_with(Vec::new()));
    let first = m.made(1, budget());
    let before = m.writes().len();
    m.create(2, TeamsMeetingFields { title: "Review".into(), ..budget() });
    m.update(&first.id, budget(), TeamsMeetingFields { end: NINE + 5400, ..budget() });
    m.delete(&first.id);
    for (n, route) in ["createMeeting", "updateMeeting", "deleteMeeting"].into_iter().enumerate() {
        m.turn_after(Duration::from_secs(1));
        let writes = m.writes();
        assert_eq!(writes.len(), before + n + 1, "one request a turn: {writes:?}");
        assert_eq!(writes.last().map(|w| w.0.as_str()), Some(route));
    }
    let said = m.answers();
    assert!(matches!(said.get(1), Some(model::Event::MeetingCreated { ask: 2, meeting: Some(_), error: None })), "{said:?}");
    assert!(matches!(said.get(2), Some(model::Event::MeetingUpdated { id, meeting: Some(_), error: None }) if *id == first.id), "{said:?}");
    assert_eq!(said.get(3), Some(&deleted(&first.id, None)));
    assert_eq!(said.len(), 4);
    assert_eq!(m.r.fake(|t| t.meetings.len()), 1, "the second is left");
}

#[test]
fn a_429_is_waited_out_then_sent_again() {
    let mut m = signed(teams_with(Vec::new()));
    m.r.fake(|t| t.throttle_once = Some("createMeeting"));
    m.create(1, budget());
    m.turn_after(STEP);
    assert_eq!(m.r.hits("createMeeting"), 1);
    // Nothing while its Retry-After (a second) runs, and the loop never spins meanwhile.
    for _ in 0..8 {
        m.turn_after(STEP);
        assert!(m.never_spins());
    }
    assert_eq!(m.r.hits("createMeeting"), 1);
    assert!(m.answers().is_empty());
    m.r.turns(Duration::from_secs(1));
    assert_eq!(m.r.hits("createMeeting"), 2);
    let said = m.answers();
    assert!(matches!(said.as_slice(), [model::Event::MeetingCreated { ask: 1, meeting: Some(_), error: None }]), "{said:?}");
}

#[test]
fn three_429s_end_as_refused() {
    let mut m = signed(teams_with(Vec::new()));
    m.r.fake(|t| t.meeting_refusal = Some((429, "TooManyRequests")));
    m.create(1, budget());
    m.r.turns(Duration::from_secs(6));
    assert_eq!(m.r.hits("createMeeting"), MEETING_SENDS);
    assert_eq!(m.answers(), [created(1, refused(429, "TooManyRequests"))]);
}

#[test]
fn a_500_on_a_creation_is_not_sent_again() {
    let mut m = signed(teams_with(Vec::new()));
    m.r.fake(|t| t.fail_next.insert("createMeeting".into(), 1));
    m.create(1, budget());
    m.r.turns(Duration::from_secs(3));
    assert_eq!(m.r.hits("createMeeting"), 1);
    assert_eq!(m.answers(), [created(1, refused(500, "InternalServerError"))]);
    // Nor a 503: the meeting may have been made, its answer lost.
    m.r.fake(|t| t.answer_lost.insert("createMeeting".into(), 1));
    m.create(2, budget());
    m.r.turns(Duration::from_secs(12));
    assert_eq!(m.r.hits("createMeeting"), 2);
    assert_eq!(m.answers().last(), Some(&created(2, refused(503, "ServiceUnavailable"))));
}

#[test]
fn a_missing_permission_clears_meetings_and_leaves_the_chats() {
    let mut m = signed(teams_with(vec![chat(CHAT, T0, 2)]));
    m.r.fake(|t| t.meeting_refusal = Some((403, "InsufficientPrivileges")));
    m.create(1, budget());
    m.r.turns(Duration::from_secs(1));
    // Given again (a new sign-in with it), then refused under Graph's other code.
    m.r.w.settings.meetings = true;
    m.r.fake(|t| t.meeting_refusal = Some((403, "Forbidden")));
    m.create(2, budget());
    m.r.turns(Duration::from_secs(1));
    assert_eq!(m.answers(), [created(1, TeamsMeetingError::Consent), created(2, TeamsMeetingError::Consent)]);
    assert_eq!(allowed(&m.r.events()), [true, false, false]);
    assert_eq!(m.r.hits("createMeeting"), 2);
    assert!(!m.r.w.settings.meetings && m.r.w.polling(), "the chats go on");
    // The list is read again, and the next renewal asks for the chats' permissions alone.
    let listed = m.r.hits("chats");
    m.r.w.tokens.as_mut().unwrap().expires_at = now_unix() - 1;
    m.r.turns(Duration::from_secs(11));
    assert!(m.r.hits("chats") > listed);
    assert_eq!(m.desk.asked.lock().unwrap().last(), Some(&("refresh", false)));
}

/// T7: the permission said missing, what waits after it, and what comes until it's given again, is
/// answered so at once: Graph isn't asked again only to say it once more.
#[test]
fn after_a_missing_permission_the_rest_is_answered_without_graph() {
    let mut m = signed(teams_with(Vec::new()));
    m.r.fake(|t| t.meeting_refusal = Some((403, "Forbidden")));
    m.create(1, budget());
    m.create(2, budget());
    m.update(SOMEONE, budget(), later());
    m.delete(SOMEONE);
    m.r.turns(Duration::from_secs(2));
    let consent = TeamsMeetingError::Consent;
    assert_eq!(
        m.answers(),
        [created(1, consent.clone()), created(2, consent.clone()), updated(SOMEONE, consent.clone()), deleted(SOMEONE, Some(consent.clone()))]
    );
    let asked = |m: &Meet| m.r.hits("createMeeting") + m.r.hits("updateMeeting") + m.r.hits("deleteMeeting");
    assert_eq!(asked(&m), 1, "the first alone went");
    assert_eq!(allowed(&m.r.events()), [true, false], "said once");
    m.create(3, budget());
    assert_eq!(m.answers().last(), Some(&created(3, consent)));
    m.r.turns(Duration::from_secs(1));
    assert_eq!(asked(&m), 1);
    assert!(!meetings::waiting(&m.r.w));
}

/// T7: a change or a deletion refused for the permission says so too, and lets it go; the deletion
/// of a meeting whose answer was refused (D7), refused so, is nobody's to hear of, and isn't tried
/// again.
#[test]
fn a_change_or_a_deletion_refused_for_the_permission_lets_it_go() {
    for change in [true, false] {
        let mut m = signed(teams_with(Vec::new()));
        let made = m.made(1, budget());
        m.r.fake(|t| t.meeting_refusal = Some((403, "InsufficientPrivileges")));
        if change {
            m.update(&made.id, budget(), later());
        } else {
            m.delete(&made.id);
        }
        m.r.turns(Duration::from_secs(1));
        let told = if change { updated(&made.id, TeamsMeetingError::Consent) } else { deleted(&made.id, Some(TeamsMeetingError::Consent)) };
        assert_eq!(m.answers().last(), Some(&told));
        assert_eq!(allowed(&m.r.events()), [true, false], "{change}");
        assert!(!m.r.w.settings.meetings && m.r.w.polling(), "the chats go on");
    }
    let mut m = signed(teams_with(Vec::new()));
    m.r.fake(|t| t.meeting_answer = Some(json!({"joinWebUrl": "https://teams.microsoft.com.evil.example/x"})));
    m.create(9, budget());
    m.turn_after(STEP);
    assert_eq!(m.answers(), [created(9, TeamsMeetingError::Untrusted)]);
    m.r.fake(|t| t.meeting_refusal = Some((403, "Forbidden")));
    m.r.turns(Duration::from_secs(1));
    assert_eq!(m.answers().len(), 1, "nobody hears of it");
    assert_eq!(m.r.hits("deleteMeeting"), 1, "tried once");
    assert!(!meetings::waiting(&m.r.w));
}

/// T5, T7: the connection cut once Graph made the meeting, before its answer came. The client never
/// sends a creation again by itself; the worker waits for the network and sends it once more (the
/// first meeting may stay in Teams: as many are left as without sending it again), and answers with
/// the second.
#[test]
fn a_creation_cut_after_it_went_is_sent_again_once_the_network_is_back() {
    let mut m = signed(teams_with(Vec::new()));
    m.r.fake(|t| t.cut_after.insert("createMeeting".into(), 1));
    m.create(1, budget());
    m.turn_after(STEP);
    assert_eq!(m.r.hits("createMeeting"), 1, "not sent again by the client");
    assert!(m.answers().is_empty(), "it waits for the network");
    assert_eq!(m.r.fake(|t| t.meetings.len()), 1, "made, though its answer was lost");
    m.r.turns(Duration::from_secs(12));
    assert_eq!(m.r.hits("createMeeting"), 2);
    let said = m.answers();
    let [model::Event::MeetingCreated { ask: 1, meeting: Some(made), error: None }] = said.as_slice() else { panic!("{said:?}") };
    let kept = m.r.fake(|t| t.meetings.clone());
    assert_eq!(kept.len(), 2, "the first left in Teams");
    assert_eq!(made.id, kept[1].id, "answered with the second");
}

/// D4: the token says it carries the permission, so Teams refuses the account itself.
#[test]
fn a_403_with_the_permission_granted_is_the_accounts() {
    let mut m = signed(teams_with(Vec::new()));
    m.r.w.tokens.as_mut().unwrap().granted = format!("{TEAMS_SCOPE} {TEAMS_MEETINGS_SCOPE}");
    m.r.fake(|t| t.meeting_refusal = Some((403, "Forbidden")));
    m.create(1, budget());
    m.r.turns(Duration::from_secs(1));
    m.r.fake(|t| t.meeting_refusal = Some((403, "InsufficientPrivileges")));
    m.create(2, budget());
    m.r.turns(Duration::from_secs(1));
    assert_eq!(m.answers(), [created(1, TeamsMeetingError::NotAllowed), created(2, TeamsMeetingError::NotAllowed)]);
    assert!(m.r.w.settings.meetings, "the permission is kept");
    assert_eq!(allowed(&m.r.events()), [true]);
}

#[test]
fn another_403_is_the_accounts() {
    let mut m = signed(teams_with(Vec::new()));
    m.r.fake(|t| t.meeting_refusal = Some((403, "AccessDenied")));
    m.create(1, budget());
    m.r.turns(Duration::from_secs(1));
    assert_eq!(m.answers(), [created(1, TeamsMeetingError::NotAllowed)]);
    assert!(m.r.w.settings.meetings);
    assert_eq!(allowed(&m.r.events()), [true]);
}

#[test]
fn a_401_after_a_renewal_is_sign_in_again() {
    let mut m = signed(teams_with(Vec::new()));
    m.r.fake(|t| t.meeting_refusal = Some((401, "InvalidAuthenticationToken")));
    m.create(1, budget());
    m.create(2, budget());
    m.r.turns(Duration::from_secs(1));
    // The first sent twice, the token renewed between; the second never sent, as Teams stopped.
    assert_eq!(m.r.hits("createMeeting"), 2);
    assert_eq!(m.desk.asked.lock().unwrap().last(), Some(&("refresh", true)));
    let answered = m.answers();
    assert_eq!(per_ask(&answered), BTreeMap::from([(1, 1), (2, 1)]));
    assert!(answered.contains(&created(1, TeamsMeetingError::SignIn)), "{answered:?}");
    assert!(answered.contains(&created(2, TeamsMeetingError::NotSignedIn)), "{answered:?}");
    assert!(said(&m.r.events(), &Status::SignInAgain));
    assert!(!meetings::waiting(&m.r.w));
}

#[test]
fn no_network_is_offline() {
    let mut m = signed(teams_with(Vec::new()));
    m.r.fake(|t| t.cut_all = true);
    m.create(1, budget());
    m.r.turns_every(Duration::from_secs(119), Duration::from_secs(1));
    assert!(m.answers().is_empty(), "it waits for the network");
    assert!(said(&m.r.events(), &Status::Offline));
    assert!(m.r.hits("createMeeting") >= 2, "sent again while it waits");
    m.r.turns_every(Duration::from_secs(2), Duration::from_secs(1));
    assert_eq!(m.answers(), [created(1, TeamsMeetingError::Offline)]);
    assert!(!meetings::waiting(&m.r.w));
}

#[test]
fn a_request_never_waits_past_two_minutes() {
    let mut m = signed(teams_with(Vec::new()));
    m.r.fake(|t| t.cut_all = true);
    for ask in 1..=MEETING_QUEUE as u64 {
        m.create(ask, budget());
    }
    m.r.turns_every(Duration::from_secs(121), Duration::from_secs(1));
    let said = m.answers();
    assert_eq!(per_ask(&said), (1..=MEETING_QUEUE as u64).map(|ask| (ask, 1)).collect::<BTreeMap<_, _>>());
    assert!(said.iter().all(|e| matches!(e, model::Event::MeetingCreated { meeting: None, error: Some(TeamsMeetingError::Offline), .. })));
    // Graph asking to wait longer than that: answered at two minutes all the same, the loop
    // woken for it, nothing asked meanwhile.
    m.r.fake(|t| {
        t.cut_all = false;
        t.answer_lost.insert("updateMeeting".into(), 1);
        t.lost_retry_after = Some(600);
    });
    m.r.turns(Duration::from_secs(11));
    let asked = m.now();
    m.update(SOMEONE, budget(), later());
    m.turn_after(STEP);
    m.turn_after(STEP);
    assert_eq!(m.r.hits("updateMeeting"), 1);
    assert_eq!(m.r.w.wake(m.now()), Some(asked + MEETING_WAIT));
    m.r.turns_every(Duration::from_secs(120), Duration::from_secs(1));
    assert_eq!(m.answers().last(), Some(&updated(SOMEONE, TeamsMeetingError::Offline)));
    assert_eq!(m.r.hits("updateMeeting"), 1);
}

#[test]
fn signed_out_or_stopped_answers_at_once() {
    // Never signed in.
    let server = serve_teams(teams_with(Vec::new()));
    let (mut w, _, seen, _) = by_hand(&server, grant(Some(TENANT)));
    let now = Instant::now();
    meetings::create(&mut w, 1, budget(), now);
    meetings::update(&mut w, SOMEONE.into(), budget(), later(), now);
    meetings::delete(&mut w, SOMEONE.into(), now);
    let not_signed_in = TeamsMeetingError::NotSignedIn;
    assert_eq!(
        answers(&teams_of(&seen)),
        [created(1, not_signed_in.clone()), updated(SOMEONE, not_signed_in.clone()), deleted(SOMEONE, Some(not_signed_in.clone()))]
    );
    assert!(!meetings::waiting(&w));
    assert_eq!(hits(&server, "createMeeting") + hits(&server, "updateMeeting") + hits(&server, "deleteMeeting"), 0);
    // Switched off, and signed out: what waited is answered, and what comes after at once.
    for job in [Job::Stop, Job::SignOut { delete_copy: false }] {
        let mut m = signed(teams_with(Vec::new()));
        m.create(1, budget());
        m.r.w.job(job);
        m.create(2, budget());
        assert_eq!(m.answers(), [created(1, not_signed_in.clone()), created(2, not_signed_in.clone())]);
        m.r.turns(Duration::from_secs(1));
        assert_eq!((m.r.hits("createMeeting"), m.answers().len()), (0, 2));
    }
}

#[test]
fn working_offline_answers_offline() {
    let mut m = signed(teams_with(Vec::new()));
    m.r.w.work_offline(true);
    m.create(1, budget());
    m.update(SOMEONE, budget(), later());
    m.delete(SOMEONE);
    let offline = TeamsMeetingError::Offline;
    assert_eq!(m.answers(), [created(1, offline.clone()), updated(SOMEONE, offline.clone()), deleted(SOMEONE, Some(offline))]);
    m.r.turns(Duration::from_secs(2));
    assert!(m.writes().is_empty());
}

#[test]
fn going_offline_answers_what_waits() {
    let mut m = signed(teams_with(Vec::new()));
    m.r.fake(|t| t.cut_all = true);
    m.create(1, budget());
    m.update(SOMEONE, budget(), later());
    m.delete(SOMEONE);
    m.turn_after(STEP);
    assert!(meetings::waiting(&m.r.w));
    m.r.w.work_offline(true);
    let offline = TeamsMeetingError::Offline;
    assert_eq!(m.answers(), [created(1, offline.clone()), updated(SOMEONE, offline.clone()), deleted(SOMEONE, Some(offline))]);
    assert!(!meetings::waiting(&m.r.w));
    // Nothing more is said: not at two minutes, nor back online.
    m.r.turns_every(Duration::from_secs(121), Duration::from_secs(1));
    m.r.w.work_offline(false);
    m.r.fake(|t| {
        t.cut_all = false;
        t.throttle_once = Some("createMeeting");
    });
    m.r.turns(Duration::from_secs(11));
    assert_eq!(m.answers().len(), 3);
    // While a Retry-After runs, the loop waits a turn at least, each time it looks.
    m.create(4, budget());
    m.turn_after(STEP);
    assert_eq!(m.r.hits("createMeeting"), 2, "the one cut, and this one");
    for _ in 0..8 {
        assert!(m.never_spins());
        m.turn_after(STEP);
    }
    m.r.turns(Duration::from_secs(1));
    assert!(matches!(m.answers().last(), Some(model::Event::MeetingCreated { ask: 4, meeting: Some(_), error: None })));
}

#[test]
fn the_queue_is_bounded() {
    let mut m = signed(teams_with(Vec::new()));
    let full = MEETING_QUEUE as u64;
    for ask in 1..=full + 1 {
        m.create(ask, budget());
    }
    // Teams said nothing: not a refusal.
    assert_eq!(m.answers(), [created(full + 1, TeamsMeetingError::Offline)]);
    assert_eq!(m.r.hits("createMeeting"), 0);
    // None is left unanswered, nor answered twice.
    m.r.w.job(Job::Stop);
    let said = m.answers();
    assert_eq!(per_ask(&said), (1..=full + 1).map(|ask| (ask, 1)).collect::<BTreeMap<_, _>>());
    assert_eq!(said.iter().filter(|e| **e == created(1, TeamsMeetingError::NotSignedIn)).count(), 1);
}

/// D7: Teams made the meeting, and answered with a link that isn't its own.
#[test]
fn an_answer_with_a_link_elsewhere_is_refused_and_its_meeting_deleted() {
    let mut m = signed(teams_with(Vec::new()));
    m.r.fake(|t| t.meeting_answer = Some(json!({"joinWebUrl": "https://teams.microsoft.com.evil.example/x"})));
    m.create(9, budget());
    m.turn_after(STEP);
    assert_eq!(m.answers(), [created(9, TeamsMeetingError::Untrusted)]);
    let id = m.r.fake(|t| t.meetings[0].id.clone());
    m.turn_after(STEP);
    assert_eq!(m.writes().last(), Some(&("deleteMeeting".to_string(), id, Value::Null)));
    assert_eq!(m.r.fake(|t| t.meetings.len()), 0);
    m.r.turns(Duration::from_secs(1));
    assert_eq!(m.answers().len(), 1, "the deletion is nobody's to hear of");
    assert_eq!(m.r.hits("deleteMeeting"), 1);
    // An answer whose id can't be one: refused all the same, and nothing to delete.
    m.r.fake(|t| t.meeting_answer = Some(json!({"id": "a?b", "joinWebUrl": "https://evil.example/x"})));
    m.create(10, budget());
    m.r.turns(Duration::from_secs(1));
    assert_eq!(m.answers().last(), Some(&created(10, TeamsMeetingError::Untrusted)));
    assert_eq!(m.r.hits("deleteMeeting"), 1);
}

#[test]
fn the_log_never_says_a_meeting() {
    let mut m = signed(teams_with(Vec::new()));
    let tap = crate::fakes::logs::tap(|l| l.starts_with("teams: "));
    let quokka = TeamsMeetingFields { title: "Quokka offsite".into(), ..budget() };
    let made = m.made(1, quokka.clone());
    m.update(&made.id, quokka.clone(), TeamsMeetingFields { title: "Quokka offsite, moved".into(), ..later() });
    m.r.turns(Duration::from_secs(1));
    m.delete(&made.id);
    m.r.turns(Duration::from_secs(1));
    m.r.fake(|t| t.meeting_refusal = Some((400, "BadRequest")));
    m.create(2, quokka.clone());
    m.r.turns(Duration::from_secs(1));
    m.r.fake(|t| {
        t.meeting_refusal = None;
        t.meeting_answer = Some(json!({"joinWebUrl": "https://teams.microsoft.com.evil.example/x"}));
    });
    m.create(3, quokka);
    m.r.turns(Duration::from_secs(1));
    assert_eq!(m.answers().len(), 5);
    let lines = tap.lines();
    assert!(lines.iter().any(|l| l.contains("meeting") && l.contains("HTTP 400 BadRequest")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("a meeting was made")), "{lines:?}");
    for line in &lines {
        let lower = line.to_lowercase();
        for private in [
            made.id.as_str(),
            made.join_url.as_str(),
            "quokka",
            "offsite",
            "meetup-join",
            "thread.v2",
            "evil.example",
            "ab3xy9",
            "1234567890",
            "9999999",
        ] {
            assert!(!lower.contains(&private.to_lowercase()), "{private:?} in the log: {line}");
        }
    }
}

#[test]
fn an_error_is_told_by_graphs_status_and_code() {
    let http = |m: &str| NetError::Protocol(m.into());
    let granted = format!("{TEAMS_SCOPE} {TEAMS_MEETINGS_SCOPE}");
    // D4: the permission missing, unless the token says it carries it.
    assert_eq!(error_of(&http("HTTP 403 Forbidden"), ""), TeamsMeetingError::Consent);
    assert_eq!(error_of(&http("HTTP 403 InsufficientPrivileges"), " \t"), TeamsMeetingError::Consent);
    assert_eq!(error_of(&http("HTTP 403 Forbidden"), TEAMS_SCOPE), TeamsMeetingError::Consent);
    assert_eq!(error_of(&http("HTTP 403 Forbidden"), &granted), TeamsMeetingError::NotAllowed);
    assert_eq!(error_of(&http("HTTP 403 InsufficientPrivileges"), &granted), TeamsMeetingError::NotAllowed);
    assert_eq!(error_of(&http("HTTP 403 AccessDenied"), ""), TeamsMeetingError::NotAllowed);
    assert_eq!(error_of(&http("HTTP 403"), ""), TeamsMeetingError::NotAllowed);
    // Any other status, with Graph's code when it gave one.
    assert_eq!(error_of(&http("HTTP 400 BadRequest"), ""), refused(400, "BadRequest"));
    assert_eq!(error_of(&http("HTTP 404 NotFound"), &granted), refused(404, "NotFound"));
    assert_eq!(error_of(&NetError::Closed("HTTP 503".into()), ""), refused(503, ""));
    assert_eq!(error_of(&NetError::Closed("HTTP 429 TooManyRequests".into()), ""), refused(429, "TooManyRequests"));
    // The sign-in, and the network.
    assert_eq!(error_of(&NetError::Auth("refused".into()), ""), TeamsMeetingError::SignIn);
    assert_eq!(error_of(&NetError::Io(std::io::ErrorKind::ConnectionReset.into()), ""), TeamsMeetingError::Offline);
    assert_eq!(error_of(&NetError::Tls("handshake".into()), ""), TeamsMeetingError::Offline);
    assert_eq!(error_of(&NetError::Closed("before the answer".into()), ""), TeamsMeetingError::Offline);
    // An answer that isn't one Quatrelle reads, never in Graph's own words.
    assert_eq!(error_of(&http("Graph: expected value at line 1 column 1"), ""), TeamsMeetingError::Untrusted);
    assert_eq!(error_of(&http("HTTP 4031 Forbidden"), ""), TeamsMeetingError::Untrusted);
    assert_eq!(error_of(&http("HTTP +03 Forbidden"), ""), TeamsMeetingError::Untrusted);
}

/// A bug met while a request is answered costs that answer, never the ask: it is said Offline,
/// and the next goes as before.
#[test]
fn a_panic_on_the_way_still_answers() {
    let hooks = Hooks { trip_meeting: Arc::new(AtomicU32::new(1)), panic_wait: Some(Duration::from_millis(10)), ..Hooks::default() };
    let r = rig_with(teams_with(Vec::new()), grant(Some(TENANT)), Store::memory(), Arc::new(Kept::default()), hooks);
    r.handle.submit(Job::SignIn { client_id: CLIENT.into(), tenant: "organizations".into(), meetings: true });
    r.wait("the first list", listed);
    r.handle.submit(Job::MeetingCreate { ask: 5, fields: budget() });
    let seen = r.wait("the answer", |seen| !answers(seen).is_empty());
    assert_eq!(answers(&seen), [created(5, TeamsMeetingError::Offline)]);
    r.handle.submit(Job::MeetingCreate { ask: 6, fields: budget() });
    let seen = r.wait("the next answer", |seen| answers(seen).len() == 2);
    assert!(matches!(answers(&seen).last(), Some(model::Event::MeetingCreated { ask: 6, meeting: Some(_), error: None })), "{seen:?}");
    assert_eq!((r.hits("createMeeting"), per_ask(&answers(&seen)).len()), (2, 2));
}
