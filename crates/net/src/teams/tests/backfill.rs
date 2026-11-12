// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The first copy and the history against the fake Graph server, the worker driven by hand at the
//! times the test makes: every chat listed first, then each one's history backwards (recent chats
//! first, meetings last), every read with a range, resumed after a restart from the link kept (one
//! elsewhere dropped, one Graph refuses read again from the oldest change copied), the disk guard,
//! a 429, a chat whose read panics, and a chat new after the first copy.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use quatrelle_model::pim::teams::{self as model, Chat, ChatKind, ChatMessage, ChatRow, Job, MAX_CHATS, Status};
use quatrelle_store::Store;

use super::worker::{AtDesk, CLIENT, Desk, Kept, Seen, account, desk, emit_into, grant, said, settings, teams_of, turn_until};
use super::{CHAT, ME, MINUTE, T0, TENANT, chat, hits, me, serve_teams, teams_with};
use crate::fake_graph::{FakeChat, FakeMessage, FakeTeams, FakeUser, Server, rfc3339};
use crate::teams::backfill;
use crate::teams::limits::Cadence;
use crate::teams::store::{ChatRecord, Copy, Identity};
use crate::teams::worker::{REFRESH_KEY, Worker, keyring};
use crate::worker::Credentials;

const DAY: i64 = 24 * 60 * MINUTE;
/// The fake's clock: 40 days after the fixtures' messages (none of them recent).
const NOW: i64 = T0 + 40 * DAY;
const OTHER: &str = "19:c3d4@thread.v2";
/// The step the test's clock moves by between two turns.
const STEP: Duration = Duration::from_millis(100);
const LT: &str = "lastModifiedDateTime lt ";
const GT: &str = "lastModifiedDateTime gt ";
const GB: u64 = 1 << 30;

/// A worker driven by hand, the fake Graph it talks to, and what it said.
struct Run {
    w: Worker,
    server: Server,
    seen: Seen,
    t0: Instant,
    /// How far past `t0` the last turn was.
    at: Duration,
    _creds: Arc<Kept>,
}

/// A worker on `store`, talking to `server`, not started.
fn worker(server: &Server, store: &Store) -> (Worker, Arc<Desk>, Seen, Arc<Kept>) {
    let (desk, seen, creds) = (desk(grant(Some(TENANT))), Seen::default(), Arc::new(Kept::default()));
    let w = Worker::new(store.clone(), creds.clone(), emit_into(&seen), server.base.clone(), Box::new(AtDesk(desk.clone()))).unwrap();
    (w, desk, seen, creds)
}

/// A worker on `store`, made ready by `ready` (a test's hooks), then signed in by hand to a fake
/// Graph whose clock stands at [`NOW`]: nothing turned past the account.
fn signed_in(mut teams: FakeTeams, store: &Store, ready: impl FnOnce(&mut Worker)) -> Run {
    teams.clock.get_or_insert(NOW);
    let server = serve_teams(teams);
    let (mut w, _, seen, creds) = worker(&server, store);
    ready(&mut w);
    let t0 = Instant::now();
    w.job(Job::SignIn { client_id: CLIENT.into(), tenant: "organizations".into() });
    turn_until(&mut w, t0, "the account", |_| account(&teams_of(&seen)).is_some());
    Run { w, server, seen, t0, at: Duration::ZERO, _creds: creds }
}

fn run(teams: FakeTeams) -> Run {
    signed_in(teams, &Store::memory(), |_| {})
}

impl Run {
    /// Turns, a [`STEP`] apart, for `span` of the test's clock.
    fn turns(&mut self, span: Duration) {
        self.turns_every(span, STEP);
    }

    fn turns_every(&mut self, span: Duration, step: Duration) {
        let end = self.at + span;
        while self.at < end {
            self.turn(step);
        }
    }

    fn turn(&mut self, step: Duration) {
        self.at += step;
        self.w.turn(self.t0 + self.at);
    }

    /// Turns until `done`, for at most `span` of the test's clock.
    fn until(&mut self, what: &str, span: Duration, done: impl Fn(&Run) -> bool) {
        let end = self.at + span;
        while !done(self) {
            assert!(self.at < end, "{what} never came; said: {:?}", self.statuses());
            self.turn(STEP);
        }
    }

    fn fake<T>(&self, change: impl FnOnce(&mut FakeTeams) -> T) -> T {
        change(self.server.data.lock().unwrap().teams.as_mut().unwrap())
    }

    /// The history reads of a chat the fake had (their `$filter`), in order.
    fn history(&self, chat: &str) -> Vec<String> {
        self.fake(|t| t.message_reads.iter().filter(|(c, f)| c == chat && f.starts_with(LT)).map(|(_, f)| f.clone()).collect())
    }

    /// Every history read the fake had: the chats, in order.
    fn history_chats(&self) -> Vec<String> {
        self.fake(|t| t.message_reads.iter().filter(|(_, f)| f.starts_with(LT)).map(|(c, _)| c.clone()).collect())
    }

    fn all_hits(&self) -> u32 {
        self.fake(|t| t.hits.values().sum())
    }

    fn events(&self) -> Vec<model::Event> {
        teams_of(&self.seen)
    }

    fn statuses(&self) -> Vec<Status> {
        self.events().into_iter().filter_map(|e| if let model::Event::Status { status } = e { Some(status) } else { None }).collect()
    }

    fn up_to_date(&self) -> bool {
        said(&self.events(), &Status::UpToDate)
    }

    fn row(&self, chat: &str) -> ChatRow {
        rows(&self.events()).into_iter().rev().find(|r| r.id == chat).unwrap_or_else(|| panic!("no row for {chat}"))
    }

    /// The messages said of a chat after event number `from`: (messages, fresh).
    fn messages(&self, chat: &str, from: usize) -> Vec<(Vec<ChatMessage>, bool)> {
        self.events()
            .into_iter()
            .skip(from)
            .filter_map(|e| match e {
                model::Event::Messages { chat: c, messages, fresh, .. } if c == chat => Some((messages, fresh)),
                _ => None,
            })
            .collect()
    }

    fn record(&self, chat: &str) -> ChatRecord {
        self.w.copy.as_ref().unwrap().chat(chat).unwrap()
    }

    /// The ids of a chat's messages in the copy, in creation order.
    fn copied(&self, chat: &str) -> Vec<String> {
        assert!(self.record(chat).pages.len() <= 5, "a window's worth of pages");
        self.w.copy.as_ref().unwrap().last_pages(chat, 5).into_iter().map(|m| m.id).collect()
    }

    /// The ids of a chat's messages on the fake, in creation order.
    fn served(&self, chat: &str) -> Vec<String> {
        self.fake(|t| t.chat(chat).unwrap().messages.iter().map(|m| m.id.clone()).collect())
    }

    /// A message of Ana's added to a chat of the fake at `at`, the fake's clock moved past it.
    fn post(&self, chat: &str, at: i64, text: &str) {
        self.fake(|t| {
            t.clock = Some(t.clock.unwrap_or(NOW).max(at + 1000));
            t.chat_mut(chat).unwrap().messages.push(FakeMessage::new(&at.to_string(), Some("u-ana"), &rfc3339(at), &format!("<p>{text}</p>")));
        });
    }
}

/// Every row said, in order.
fn rows(events: &[model::Event]) -> Vec<ChatRow> {
    events.iter().filter_map(|e| if let model::Event::Chats { rows } = e { Some(rows.clone()) } else { None }).flatten().collect()
}

/// The first copy's progress said: (done, of), in order.
fn copying(statuses: &[Status]) -> Vec<(u32, u32)> {
    statuses.iter().filter_map(|s| if let Status::Copying { done, of } = s { Some((*done, *of)) } else { None }).collect()
}

/// A meeting's chat with one message of Ana's at `at`.
fn meeting(id: &str, at: i64) -> FakeChat {
    let mut c = FakeChat::new(id, "meeting", Some("Weekly review"), vec![me(), FakeUser::new("u-ana", "Ana Lima")]);
    c.messages.push(FakeMessage::new(&at.to_string(), Some("u-ana"), &rfc3339(at), "<p>Agenda</p>"));
    c
}

/// A chat as a record holds it, nothing of it known but its id.
fn bare(id: &str) -> Chat {
    Chat {
        id: id.into(),
        kind: ChatKind::OneOnOne,
        topic: None,
        members: Vec::new(),
        last_read: None,
        hidden: false,
        left: false,
        unreadable: None,
        preview: None,
        last_activity: None,
        updated: None,
        web_url: None,
        tenant_id: None,
    }
}

#[test]
fn the_first_copy_lists_everything_then_reads_recent_chats_first_meetings_last() {
    // 58 chats of one message, a minute apart, and two meetings: the newest activity of all, and
    // one among the oldest. Sixty chats: two pages of the list.
    let mut chats: Vec<FakeChat> = (0..58).map(|i| chat(&format!("19:c{i:02}@thread.v2"), T0 + i * MINUTE, 1)).collect();
    chats.push(meeting("19:m-new@thread.v2", T0 + 100 * MINUTE));
    chats.push(meeting("19:m-old@thread.v2", T0 + 30_000));
    let mut r = run(teams_with(chats));
    r.until("the first history read", Duration::from_secs(60), |r| !r.history_chats().is_empty());
    assert_eq!(r.w.copy.as_ref().unwrap().chats().len(), 60, "every chat is listed before any history is read");
    assert!(hits(&r.server, "chats") >= 2, "both pages of the list");
    r.until("the first copy's end", Duration::from_secs(120), Run::up_to_date);
    let mut order: Vec<String> = Vec::new();
    for c in r.history_chats() {
        if !order.contains(&c) {
            order.push(c);
        }
    }
    let mut expected: Vec<String> = (0..58).rev().map(|i| format!("19:c{i:02}@thread.v2")).collect();
    expected.extend(["19:m-new@thread.v2".to_string(), "19:m-old@thread.v2".to_string()]);
    assert_eq!(order, expected, "recent chats first, meetings last");
    // The progress: said from the list's end, a chat after the other, then up to date.
    let statuses = r.statuses();
    let progress = copying(&statuses);
    assert_eq!(progress.first(), Some(&(0, 60)), "{statuses:?}");
    assert!(progress.windows(2).all(|p| p[0].0 < p[1].0) && progress.iter().all(|p| p.1 == 60), "{progress:?}");
    let first = statuses.iter().position(|s| matches!(s, Status::Copying { .. })).unwrap();
    assert!(!statuses[..first].contains(&Status::UpToDate), "not up to date before the copy: {statuses:?}");
    assert_eq!(statuses.last(), Some(&Status::UpToDate));
}

#[test]
fn every_history_read_carries_a_range() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 120), chat(OTHER, T0 + DAY, 3)]));
    let mut pages: Vec<Duration> = Vec::new();
    while !r.up_to_date() {
        assert!(r.at < Duration::from_secs(120), "the first copy never ended");
        let before = r.history(CHAT).len();
        r.turn(STEP);
        if r.history(CHAT).len() > before {
            pages.push(r.at);
        }
    }
    let reads = r.fake(|t| t.message_reads.clone());
    assert!(r.history(CHAT).len() >= 3, "three pages of history: {reads:?}");
    assert!(pages.windows(2).all(|p| p[1] - p[0] >= Duration::from_secs(1)), "a chat's pages a second apart: {pages:?}");
    for (_, filter) in &reads {
        assert!(filter.starts_with(LT) || filter.starts_with(GT), "a read without a range: {filter:?}");
    }
    assert_eq!(r.copied(CHAT), r.served(CHAT));
}

#[test]
fn backfill_resumes_after_a_restart_without_duplicates() {
    // Seven pages of history (320 messages, 50 a page), and another chat.
    let store = Store::memory();
    let mut r = signed_in(teams_with(vec![chat(CHAT, T0, 320), chat(OTHER, T0 - DAY, 2)]), &store, |_| {});
    r.until("three pages of history", Duration::from_secs(60), |r| r.history(CHAT).len() >= 3);
    assert_eq!(r.history(CHAT).len(), 3);
    assert!(r.record(CHAT).next_link.is_some(), "where the history stopped is kept");
    // Quatrelle closes there, and opens again offline.
    let Run { w, server, .. } = r;
    drop(w);
    let (mut w, desk, seen, creds) = worker(&server, &store);
    creds.set_secret(&keyring(), REFRESH_KEY, "a-sign-in-kept-0002").unwrap();
    desk.offline.store(true, Ordering::SeqCst);
    let t0 = Instant::now();
    w.job(Job::Start { settings: settings(ME) });
    w.turn(t0);
    let at_once = teams_of(&seen);
    assert!(said(&at_once, &Status::Offline), "{at_once:?}");
    let listed: HashSet<String> = rows(&at_once).into_iter().map(|r| r.id).collect();
    assert_eq!(listed, HashSet::from([CHAT.to_string(), OTHER.to_string()]), "the list comes from the copy at once");
    // The network back, the history goes on from its fourth page.
    desk.offline.store(false, Ordering::SeqCst);
    let mut r = Run { w, server, seen, t0, at: Duration::ZERO, _creds: creds };
    r.until("the history's end", Duration::from_secs(120), |r| r.record(CHAT).done);
    let history = r.history(CHAT);
    assert_eq!(history.len(), 7, "pages 4 to 7 read after the restart, none of the first three again");
    assert!(history.iter().all(|f| *f == history[0]), "from the link kept (it carries the first range): {history:?}");
    let copied = r.copied(CHAT);
    assert_eq!(copied.iter().collect::<HashSet<_>>().len(), copied.len(), "each message once");
    assert_eq!(copied, r.served(CHAT), "exactly the server's messages");
}

#[test]
fn a_kept_next_link_elsewhere_is_dropped() {
    let store = Store::memory();
    let (copy, _) = Copy::open(store.clone(), &Identity { tenant_id: TENANT.into(), object_id: ME.into() }).unwrap();
    let mut kept = ChatRecord::new(bare(CHAT));
    kept.history_from = Some(NOW);
    kept.watermark = Some(NOW);
    kept.next_link = Some("https://evil.example/v1.0/chats/x/messages?$skiptoken=2".into());
    copy.put_chat(&kept).unwrap();
    drop(copy);
    let mut r = signed_in(teams_with(vec![chat(CHAT, T0, 3)]), &store, |_| {});
    r.until("the history's end", Duration::from_secs(60), |r| r.record(CHAT).done);
    // (A whole second goes without its milliseconds.)
    let from_s = format!("{LT}{}", rfc3339(NOW).replace(".000Z", "Z"));
    assert_eq!(r.history(CHAT), vec![from_s], "read again from where its copy began, the link never followed");
    assert_eq!(r.record(CHAT).next_link, None);
    assert_eq!(r.copied(CHAT), r.served(CHAT));
}

#[test]
fn a_refused_next_link_restarts_from_the_oldest_copied() {
    let mut teams = teams_with(vec![chat(CHAT, T0, 120)]);
    teams.refuse_links = 1;
    let mut r = run(teams);
    r.until("the history's end", Duration::from_secs(60), |r| r.record(CHAT).done);
    let history = r.history(CHAT);
    // The first page brought messages 119 down to 70; its link refused, the read starts again from
    // the oldest change copied (a millisecond after it: one changed in the same millisecond isn't
    // missed).
    assert_eq!(history.get(2), Some(&format!("{LT}{}", rfc3339(T0 + 70 * MINUTE + 1))), "{history:?}");
    assert_eq!(r.copied(CHAT), r.served(CHAT));
}

/// Final review I2 (d): a kept or followed link answering 404 says nothing of the chat: dropped,
/// the read going on from the oldest change copied, never the chat taken for left.
#[test]
fn a_history_link_answering_404_is_dropped_not_taken_for_left() {
    let mut teams = teams_with(vec![chat(CHAT, T0, 120)]);
    teams.refuse_links = 1;
    teams.links_not_found = true;
    let mut r = run(teams);
    r.until("the history's end", Duration::from_secs(60), |r| r.record(CHAT).done);
    assert!(!r.record(CHAT).chat.left && !r.row(CHAT).left, "{:?}", r.row(CHAT));
    assert!(!r.events().iter().any(|e| matches!(e, model::Event::Removed { .. })));
    assert_eq!(r.copied(CHAT), r.served(CHAT));
}

#[test]
fn short_of_disk_history_pauses_and_new_messages_still_come() {
    let tap = crate::fakes::logs::tap(|l| l.starts_with("teams: the disk"));
    let mut r = signed_in(teams_with(vec![chat(CHAT, T0, 120)]), &Store::memory(), |w| w.backfill.probe = Some((GB, 100 * GB)));
    r.turns(Duration::from_secs(30));
    assert!(r.history(CHAT).is_empty(), "no history while the disk is short");
    // A new message comes all the same, as news.
    let from = r.events().len();
    r.post(CHAT, NOW + 1000, "Still here");
    r.turns(Duration::from_secs(15));
    let said_now = r.messages(CHAT, from);
    assert!(said_now.iter().any(|(m, fresh)| *fresh && m.iter().any(|m| m.id == (NOW + 1000).to_string())), "{said_now:?}");
    // Room again: looked at ten minutes after the disk was found short, not before.
    r.w.backfill.probe = Some((50 * GB, 100 * GB));
    r.turns(Duration::from_secs(30));
    assert!(r.history(CHAT).is_empty(), "not looked at again before ten minutes");
    r.turns_every(Duration::from_secs(600), Duration::from_secs(1));
    assert!(!r.history(CHAT).is_empty(), "the history goes on");
    assert_eq!(tap.lines().len(), 1, "told once: {:?}", tap.lines());
}

#[test]
fn a_chat_whose_read_panics_three_times_is_set_aside_for_a_day() {
    let mut r = signed_in(teams_with(vec![chat(CHAT, T0 + DAY, 2), chat(OTHER, T0, 2)]), &Store::memory(), |w| {
        w.hooks.trip_history = Some(CHAT.into());
    });
    r.turns_every(Duration::from_secs(600), Duration::from_millis(200));
    let kept = r.record(CHAT);
    assert_eq!(kept.set_aside_until, Some(NOW + DAY), "set aside for a day, by the server's clock");
    assert!(kept.chat.unreadable.is_some() && r.row(CHAT).unreadable, "and said it couldn't be read");
    assert_eq!(r.history(CHAT).len(), 3, "three reads that panicked, then none until the day is over");
    assert!(r.record(OTHER).done, "the others go on");
    assert_eq!(r.statuses().last(), Some(&Status::UpToDate));
}

#[test]
fn a_429_during_history_waits_its_retry_after_and_slows_history() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 300)]));
    r.until("the first page of history", Duration::from_secs(60), |r| r.history(CHAT).len() == 1);
    r.fake(|t| t.throttle_once = Some("messages"));
    let (hits_before, pages_before) = (hits(&r.server, "messages"), r.history(CHAT).len());
    r.until("the 429", Duration::from_secs(10), |r| hits(&r.server, "messages") > hits_before);
    assert_eq!(r.history(CHAT).len(), pages_before, "the history read was the one answered 429");
    let (at_429, all) = (r.at, r.all_hits());
    // Nothing at all asked during its Retry-After (a second, the fake's), not even the list, due
    // meanwhile.
    r.w.limiter.prompt(Cadence::List);
    while r.at < at_429 + Duration::from_millis(900) {
        r.turn(STEP);
    }
    assert_eq!(r.all_hits(), all, "no request during the Retry-After");
    // Then a page every two seconds at most, where it was one a second.
    let mut pages: Vec<Duration> = Vec::new();
    let mut count = r.history(CHAT).len();
    while pages.len() < 4 {
        assert!(r.at < at_429 + Duration::from_secs(60), "the history never went on");
        r.turn(STEP);
        let now = r.history(CHAT).len();
        if now > count {
            pages.push(r.at);
            count = now;
        }
    }
    assert!(pages[0] >= at_429 + Duration::from_secs(1), "{pages:?}");
    assert!(pages.windows(2).all(|p| p[1] - p[0] >= Duration::from_secs(2)), "history slowed: {pages:?}");
}

#[test]
fn a_chat_new_after_the_first_copy_has_its_first_message_announced() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 2)]));
    r.until("the first copy's end", Duration::from_secs(60), Run::up_to_date);
    // A chat made since, its first message 25 s before the list sees it.
    let from = r.events().len();
    r.fake(|t| {
        t.chats.push(chat(OTHER, NOW + 5_000, 1));
        t.clock = Some(NOW + 30_000);
    });
    r.turns(Duration::from_secs(15));
    let said_now = r.messages(OTHER, from);
    assert!(said_now.iter().any(|(m, fresh)| *fresh && m.iter().any(|m| m.id == (NOW + 5_000).to_string())), "announced: {said_now:?}");
    assert!(r.record(OTHER).history_from.is_some_and(|s| s <= NOW + 5_000), "its history is what came before it");
}

#[test]
fn the_progress_counts_at_most_max_chats() {
    assert_eq!(backfill::copying(3, 7), Status::Copying { done: 3, of: 7 });
    let max = u32::try_from(MAX_CHATS).unwrap();
    assert_eq!(backfill::copying(MAX_CHATS + 5, MAX_CHATS + 9), Status::Copying { done: max, of: max });
}

#[test]
fn a_history_going_round_or_bringing_nothing_new_ends() {
    // Every link points at the second page; then every link brings the first page again.
    for (what, change) in [("going round", 0), ("nothing new", 1)] {
        let mut teams = teams_with(vec![chat(CHAT, T0, 150)]);
        if change == 0 {
            teams.loop_next = Some("messages");
        } else {
            teams.repeat_page = Some("messages");
        }
        let mut r = run(teams);
        r.until("the first copy's end", Duration::from_secs(60), Run::up_to_date);
        let pages = r.history(CHAT).len();
        assert!(pages <= 3, "{what}: {pages} pages of history");
        assert!(r.record(CHAT).done && r.record(CHAT).next_link.is_none(), "{what}: its history ends");
    }
}

#[test]
fn a_chat_whose_history_keeps_failing_doesnt_hold_the_first_copy() {
    // One chat's messages answer 500, whatever is asked: the first copy ends all the same, and
    // its history goes on in the background, once the server serves it.
    let mut teams = teams_with(vec![chat(CHAT, T0 + DAY, 2), chat(OTHER, T0, 2)]);
    teams.fail_reads.push(CHAT.into());
    let mut r = run(teams);
    r.until("the first copy's end", Duration::from_secs(300), Run::up_to_date);
    assert!(r.record(OTHER).done && !r.record(CHAT).done, "the failing chat still to read");
    r.fake(|t| t.fail_reads.clear());
    r.turns_every(Duration::from_secs(1200), Duration::from_secs(1));
    assert!(r.record(CHAT).done, "read in the background");
    let copying_again = r.statuses().into_iter().rev().take_while(|s| *s != Status::UpToDate).any(|s| matches!(s, Status::Copying { .. }));
    assert!(!copying_again, "the first copy isn't said again: {:?}", r.statuses());
}
