// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The limiter: Microsoft's per-second limits, the approved cadences, the slow-down after a 429
//! and its return, and a long gap (the computer slept) bringing one request per cadence, no burst.

use std::time::{Duration, Instant};

use crate::teams::limits::{Cadence, Limiter, Route};

const SECOND: Duration = Duration::from_secs(1);
const ALL: [Cadence; 5] = [Cadence::List, Cadence::Open, Cadence::Active, Cadence::Hourly, Cadence::Presence];

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn one_chat_list_a_second_and_sixteen_in_all() {
    let t0 = Instant::now();
    let mut l = Limiter::new(t0);
    assert_eq!(l.wait(Route::ChatList, t0), Duration::ZERO);
    l.took(Route::ChatList, t0);
    assert!(l.wait(Route::ChatList, t0 + ms(500)) > Duration::ZERO, "a second chat list within the second");
    assert_eq!(l.wait(Route::ChatList, t0 + SECOND), Duration::ZERO);

    // 20 reads of distinct chats within one second: the 17th request in the second waits.
    let t1 = t0 + Duration::from_secs(5);
    let mut waited = Vec::new();
    for n in 0..20u64 {
        let at = t1 + ms(n * 10);
        waited.push(l.wait(Route::Messages(n), at) > Duration::ZERO);
        l.took(Route::Messages(n), at);
    }
    assert!(waited.iter().take(16).all(|w| !w), "the first 16 go at once: {waited:?}");
    assert!(waited.iter().skip(16).all(|w| *w), "past 16 in the second, a wait: {waited:?}");
    // The wait ends when the oldest of the last 16 is a second old.
    let at = t1 + ms(200);
    assert_eq!(l.wait(Route::Messages(99), at), (t1 + ms(40) + SECOND) - at);
    assert_eq!(l.wait(Route::Presence, t1 + ms(190) + SECOND), Duration::ZERO);
}

#[test]
fn never_two_reads_of_one_chat_within_a_second() {
    let t0 = Instant::now();
    let mut l = Limiter::new(t0);
    l.took(Route::Messages(7), t0);
    assert_eq!(l.wait(Route::Messages(7), t0 + ms(300)), ms(700));
    assert_eq!(l.wait(Route::Messages(8), t0 + ms(300)), Duration::ZERO, "another chat isn't held");
    assert_eq!(l.wait(Route::Hosted(7), t0 + ms(300)), Duration::ZERO, "hosted contents count apart");
    assert_eq!(l.wait(Route::Messages(7), t0 + SECOND), Duration::ZERO);
    l.took(Route::Hosted(7), t0);
    assert_eq!(l.wait(Route::Hosted(7), t0 + ms(999)), ms(1));
    // Members, presence, photos and drive have only the ceiling.
    for r in [Route::Members, Route::Presence, Route::Photo, Route::Drive] {
        l.took(r, t0);
        assert_eq!(l.wait(r, t0), Duration::ZERO, "{r:?}");
    }
}

#[test]
fn more_chats_than_the_cap_keep_the_latest_spaced() {
    let t0 = Instant::now();
    let mut l = Limiter::new(t0);
    let n = quatrelle_model::pim::teams::MAX_CHATS as u64 + 50;
    for k in 0..n {
        l.took(Route::Messages(k), t0 + ms(k));
    }
    let now = t0 + ms(n);
    assert!(l.wait(Route::Messages(n - 1), now) > Duration::ZERO, "the latest read still holds its chat");
    // Only the ceiling holds the others back.
    assert_eq!(l.wait(Route::Messages(0), now), l.wait(Route::Messages(u64::MAX), now));
    assert_eq!(l.wait(Route::Messages(n - 1), now), ms(999));
}

#[test]
fn a_429_waits_retry_after_then_doubles_up_to_sixteen() {
    let t0 = Instant::now();
    let mut l = Limiter::new(t0);
    assert_eq!(l.interval(Cadence::List), Duration::from_secs(10));
    l.ran(Cadence::List, t0);
    l.throttled(Cadence::List, Some(5), t0 + SECOND);
    assert!(!l.due(Cadence::List, t0 + SECOND));
    assert!(!l.due(Cadence::List, t0 + ms(5_999)));
    assert!(l.due(Cadence::List, t0 + Duration::from_secs(6)), "due once Retry-After has passed");
    assert_eq!(l.interval(Cadence::List), Duration::from_secs(20));
    // Each new 429 doubles it again, never past 16 times.
    let mut at = t0 + Duration::from_secs(10);
    for _ in 0..5 {
        l.throttled(Cadence::List, Some(5), at);
        at += Duration::from_secs(30);
    }
    assert_eq!(l.interval(Cadence::List), Duration::from_secs(160));
    l.ran(Cadence::List, at);
    assert!(!l.due(Cadence::List, at + Duration::from_secs(159)));
    assert!(l.due(Cadence::List, at + Duration::from_secs(160)));
    // The other cadences keep their pace.
    assert_eq!(l.interval(Cadence::Open), Duration::from_secs(3));
    // No Retry-After: 10 s.
    l.throttled(Cadence::Open, None, at);
    assert!(!l.due(Cadence::Open, at + ms(9_999)));
    assert!(l.due(Cadence::Open, at + Duration::from_secs(10)));
    assert_eq!(l.interval(Cadence::Open), Duration::from_secs(6));
    // An hour's Retry-After is waited whole, a longer one held to the hour.
    l.throttled(Cadence::Presence, Some(3600), at);
    assert!(!l.due(Cadence::Presence, at + Duration::from_secs(3599)));
    l.throttled(Cadence::Active, Some(u64::MAX), at);
    assert!(l.due(Cadence::Active, at + Duration::from_secs(3600)));
}

#[test]
fn an_hour_clean_brings_the_cadence_back() {
    let t0 = Instant::now();
    let mut l = Limiter::new(t0);
    l.throttled(Cadence::Active, Some(1), t0);
    l.throttled(Cadence::Active, Some(1), t0 + Duration::from_secs(2));
    assert_eq!(l.interval(Cadence::Active), Duration::from_secs(240));
    l.ran(Cadence::Active, t0 + Duration::from_secs(10));
    l.ran(Cadence::Active, t0 + Duration::from_secs(1000));
    assert_eq!(l.interval(Cadence::Active), Duration::from_secs(240), "not yet an hour clean");
    // An hour after the last 429, it is due at the base pace and runs at it.
    let clean = t0 + Duration::from_secs(2 + 3600);
    l.ran(Cadence::Active, clean - Duration::from_secs(100));
    assert!(l.due(Cadence::Active, clean), "the base interval counts once the hour is clean");
    l.ran(Cadence::Active, clean);
    assert_eq!(l.interval(Cadence::Active), Duration::from_secs(60));
    // A 429 after that starts again from twice the base, not from where it was.
    l.throttled(Cadence::Active, None, clean + SECOND);
    assert_eq!(l.interval(Cadence::Active), Duration::from_secs(120));
}

#[test]
fn a_long_gap_gives_one_request_per_cadence() {
    let t0 = Instant::now();
    let mut l = Limiter::new(t0);
    for c in ALL {
        l.ran(c, t0);
    }
    let woke = t0 + Duration::from_secs(10 * 3600);
    for c in ALL {
        assert!(l.due(c, woke), "{c:?} due after the gap");
        l.ran(c, woke);
        assert!(!l.due(c, woke), "{c:?} due only once");
        assert!(!l.due(c, woke + l.interval(c) - ms(1)), "{c:?} not before its interval");
        assert!(l.due(c, woke + l.interval(c)), "{c:?} due again at its interval");
    }
    // A throttled cadence asleep through the gap is due once too, at the base pace (clean hour).
    let mut l = Limiter::new(t0);
    l.throttled(Cadence::List, Some(30), t0);
    let woke = t0 + Duration::from_secs(10 * 3600);
    assert!(l.due(Cadence::List, woke));
    l.ran(Cadence::List, woke);
    assert!(!l.due(Cadence::List, woke + ms(9_999)));
    assert_eq!(l.interval(Cadence::List), Duration::from_secs(10));
}

#[test]
fn the_deadline_is_never_more_than_a_second_away() {
    let t0 = Instant::now();
    let mut l = Limiter::new(t0);
    // Everything due at start: the deadline is now.
    assert_eq!(l.next_deadline(t0), t0);
    for c in ALL {
        l.ran(c, t0);
    }
    // Nothing due before 3 s, yet the worker wakes every second to hear a stop.
    for s in [0, 1, 2] {
        let now = t0 + Duration::from_secs(s) + ms(100);
        let d = l.next_deadline(now);
        assert!(d > now && d <= now + SECOND, "{s}: {:?}", d - now);
    }
    // A cadence due in less than a second sets the deadline.
    assert_eq!(l.next_deadline(t0 + ms(2_500)), t0 + Duration::from_secs(3));
    // A long Retry-After never stretches the wait past a second.
    l.throttled(Cadence::Hourly, Some(3600), t0);
    let now = t0 + Duration::from_secs(20);
    assert!(l.next_deadline(now) <= now + SECOND);
    // Overdue is now, never in the past.
    assert_eq!(l.next_deadline(t0 + Duration::from_secs(4000)), t0 + Duration::from_secs(4000));
}
