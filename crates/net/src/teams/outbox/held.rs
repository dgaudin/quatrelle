// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The outbox's records held back after a request of theirs got no answer at all (the connection
//! cut, nothing said), so that the worker goes on with the rest meanwhile: another chat's
//! messages, the reads. The network away, every request fails so; while it works, only that one
//! does (a proxy cutting it, a request timing out each time), and that is what is counted.
//!
//! * The first failure without an answer never counts as a try: the network may just have gone.
//!   The record is held back [`HOLD`], past the worker's own pause after a failure, so that
//!   another request has its turn first and says whether the network answers.
//! * A later one counts as a try when Graph answered a request, whatever its status, since the
//!   record's last such failure (`Worker::answered` grew). Each that counts doubles the hold,
//!   [`HOLD_MAX`] at most. One that doesn't (nothing answered: the network is away) holds it
//!   [`HOLD`] again, never more, so that it goes soon after the network's return.
//! * An answer of any kind forgets the record, and so does its leaving the outbox's heads.
//!
//! What is hostile here: nothing from outside (the outbox's own numbers). What it protects: the
//! worker, never held by one request; the user's change, never given up while the network is
//! away. At most [`OUTBOX_MAX`] records are held: past it, the one whose hold ends first is let
//! go.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use quatrelle_model::pim::teams::OUTBOX_MAX;

/// A record's hold after its first failure without an answer, or one that doesn't count.
const HOLD: Duration = Duration::from_secs(20);
/// The longest hold, however many tries counted.
const HOLD_MAX: Duration = Duration::from_secs(300);

/// The records whose last request got no answer at all, by number.
#[derive(Default)]
pub(super) struct Held {
    records: HashMap<u64, Unanswered>,
}

/// A record's last failure without an answer.
#[derive(Clone, Copy)]
struct Unanswered {
    /// The requests Graph had answered then.
    answered: u64,
    /// Its failures without an answer that counted as tries.
    counted: u32,
    /// It doesn't go again before.
    until: Instant,
}

impl Held {
    /// A failure without an answer of the record `seq` at `now`, Graph having answered `answered`
    /// requests so far: it is held back, and whether it counts as a try is said (the first never
    /// does; a later one when Graph answered another request since the last).
    pub(super) fn unanswered(&mut self, seq: u64, answered: u64, now: Instant) -> bool {
        let before = self.records.get(&seq).copied();
        let counts = before.is_some_and(|b| answered > b.answered);
        let counted = before.map_or(0, |b| b.counted).saturating_add(u32::from(counts));
        let hold = if counts { HOLD.saturating_mul(1u32.checked_shl(counted).unwrap_or(u32::MAX)).min(HOLD_MAX) } else { HOLD };
        if before.is_none()
            && self.records.len() >= OUTBOX_MAX
            && let Some(first) = self.records.iter().min_by_key(|(_, r)| r.until).map(|(seq, _)| *seq)
        {
            self.records.remove(&first);
        }
        self.records.insert(seq, Unanswered { answered, counted, until: now.checked_add(hold).unwrap_or(now) });
        counts
    }

    /// When the record `seq` may go again, if it is held.
    pub(super) fn until(&self, seq: u64) -> Option<Instant> {
        self.records.get(&seq).map(|r| r.until)
    }

    /// The record `seq` was answered: its next failure without an answer is a first again.
    pub(super) fn forget(&mut self, seq: u64) {
        self.records.remove(&seq);
    }

    /// Only the records `keep` says are kept.
    pub(super) fn retain(&mut self, keep: impl Fn(u64) -> bool) {
        self.records.retain(|seq, _| keep(*seq));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    #[test]
    fn the_first_failure_never_counts() {
        let (mut held, t) = (Held::default(), Instant::now());
        assert!(!held.unanswered(7, 3, t), "the network may just have gone");
        assert_eq!(held.until(7), Some(t + 20 * S), "held past the worker's 10 s pause");
        assert!(!held.unanswered(8, 9, t), "each record its own");
        // An answer forgets it: its next failure is a first again.
        held.forget(7);
        assert_eq!(held.until(7), None);
        assert!(!held.unanswered(7, 4, t + 30 * S));
        assert_eq!(held.until(8), Some(t + 20 * S));
    }

    #[test]
    fn a_failure_counts_once_another_request_was_answered() {
        let (mut held, t) = (Held::default(), Instant::now());
        assert!(!held.unanswered(7, 3, t));
        assert!(!held.unanswered(7, 3, t + 20 * S), "nothing answered since: the network is away");
        assert!(held.unanswered(7, 4, t + 60 * S), "answered since: the network works");
        assert!(!held.unanswered(7, 4, t + 100 * S), "weighed against its last failure, not its first");
    }

    #[test]
    fn the_wait_doubles_to_five_minutes() {
        let (mut held, t) = (Held::default(), Instant::now());
        let mut waits = Vec::new();
        // The first never counts; each after it counts, another request answered meanwhile.
        for answered in 0..6 {
            let at = t + 1_000 * S * u32::try_from(answered).unwrap();
            assert_eq!(held.unanswered(7, answered, at), answered > 0);
            waits.push(held.until(7).unwrap() - at);
        }
        assert_eq!(waits, [20, 40, 80, 160, 300, 300].map(Duration::from_secs));
    }

    /// Ruling P-16: while nothing answers (the network away), the wait never grows: the record
    /// goes again soon after the network's return.
    #[test]
    fn a_failure_that_doesnt_count_holds_twenty_seconds_never_more() {
        let (mut held, t) = (Held::default(), Instant::now());
        for i in 0..6 {
            let at = t + 20 * S * i;
            assert!(!held.unanswered(7, 3, at));
            assert_eq!(held.until(7), Some(at + 20 * S), "failure {i}");
        }
        // Counted twice, then the network away: 20 s again; back, the doubling goes on.
        let wait = |held: &mut Held, answered: u64, at: Instant| {
            let counts = held.unanswered(7, answered, at);
            (counts, held.until(7).unwrap() - at)
        };
        let at = t + 1_000 * S;
        assert_eq!(wait(&mut held, 4, at), (true, 40 * S));
        assert_eq!(wait(&mut held, 5, at + 40 * S), (true, 80 * S));
        assert_eq!(wait(&mut held, 5, at + 120 * S), (false, 20 * S));
        assert_eq!(wait(&mut held, 5, at + 140 * S), (false, 20 * S));
        assert_eq!(wait(&mut held, 6, at + 160 * S), (true, 160 * S));
    }

    #[test]
    fn at_most_the_outbox_s_records_are_held() {
        let (mut held, t) = (Held::default(), Instant::now());
        let max = u64::try_from(OUTBOX_MAX).unwrap();
        for seq in 0..max + 10 {
            held.unanswered(seq, 0, t + S * u32::try_from(seq).unwrap());
        }
        assert!((0..10).all(|seq| held.until(seq).is_none()), "those whose wait ends first let go");
        assert!((10..max + 10).all(|seq| held.until(seq).is_some()), "the last noted kept");
        // Gone from the outbox's heads: forgotten.
        held.retain(|seq| seq % 2 == 0);
        assert!((10..max + 10).all(|seq| held.until(seq).is_some() == (seq % 2 == 0)));
    }
}
