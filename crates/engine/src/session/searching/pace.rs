// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The session: how much of a call a search's work may take.
//!
//! The window gave the search 500 messages a frame: a count, not a time, so a frame that met
//! long downloaded bodies took half a second. It gives the search a share of the frame now, by
//! its own clock (the engine reads none: std's clock panics on wasm32, and the window's runs
//! everywhere), and the search begins a unit of work (a message indexed, one looked at) only while
//! one as long as the longest of the call so far still fits in the share.

use std::time::Duration;

use super::*;

/// The window's clock and the share of a frame a search's work takes: [`Session::set_search_clock`].
pub struct SearchClock {
    /// The share of a call.
    pub budget: Duration,
    /// The time now, from any start the window likes (only differences are used).
    pub now: Box<dyn FnMut() -> Duration>,
}

/// One call's allowance: units at most, and the clock's share when the window gave one.
pub(crate) struct Pace {
    clock: Option<SearchClock>,
    /// Whether this call keeps to the clock's share (a whole search asked for at once doesn't).
    timed: bool,
    /// Units a call may take at most (`usize::MAX`: no limit), and what's left of them.
    pub(super) units: usize,
    left: usize,
    began: Duration,
    last: Duration,
    longest: Duration,
}

/// The most messages a call adds to the index when the window asked for no count (a scope picked
/// from a menu, a Search Folder opened): as many as it asks for when it does; the clock's share
/// keeps the call to a frame.
pub(super) const WINDOW_SLICE: usize = 4096;

/// The share of a call the index takes; the answer's looks have the rest (the first finds show
/// while the index is still being read).
const INDEX_SHARE: u32 = 3;
const SHARES: u32 = 4;

impl Default for Pace {
    fn default() -> Pace {
        Pace { clock: None, timed: false, units: usize::MAX, left: usize::MAX, began: Duration::ZERO, last: Duration::ZERO, longest: Duration::ZERO }
    }
}

impl Pace {
    /// A call begins, `units` at most; `timed`: kept to the clock's share too, when the window
    /// gave one (a search asked for whole, `usize::MAX` units, isn't).
    pub(super) fn begin(&mut self, units: usize) {
        self.timed = units != usize::MAX;
        self.units = units.max(1);
        self.left = self.units;
        self.longest = Duration::ZERO;
        let now = self.now();
        self.began = now;
        self.last = now;
    }

    /// The same allowance again, for work outside a search call (the list shown again).
    pub(super) fn again(&mut self) {
        self.begin(self.units);
    }

    fn now(&mut self) -> Duration {
        match self.clock.as_mut() {
            Some(c) if self.timed => (c.now)(),
            _ => Duration::ZERO,
        }
    }

    /// Whether the index may begin a unit more: units are left, and time ([`INDEX_SHARE`] of the
    /// call's).
    pub(super) fn index_more(&mut self) -> bool {
        self.left > 0 && self.more(INDEX_SHARE)
    }

    /// Whether the answer may look at a message more, by the time (the whole of the call's; it
    /// counts its looks itself).
    pub(super) fn look_more(&mut self) -> bool {
        self.more(SHARES)
    }

    /// By the window's clock, a unit more may begin: one as long as the longest of the call so
    /// far still fits in `shares` quarters of its share. The time since the last question is the
    /// unit just done.
    fn more(&mut self, shares: u32) -> bool {
        let Some(c) = self.clock.as_mut().filter(|_| self.timed) else { return true };
        let now = (c.now)();
        let budget = c.budget.saturating_mul(shares) / SHARES;
        self.longest = self.longest.max(now.saturating_sub(self.last));
        self.last = now;
        now.saturating_sub(self.began).saturating_add(self.longest) < budget
    }

    /// `n` units done.
    pub(super) fn spend(&mut self, n: usize) {
        self.left = self.left.saturating_sub(n);
    }

    /// How many units are left (`usize::MAX`: no limit).
    pub(super) fn left(&self) -> usize {
        self.left
    }

    pub(super) fn set_clock(&mut self, clock: Option<SearchClock>) {
        self.clock = clock;
    }

    /// Whether the window gave its clock: its searches are never done whole.
    pub(super) fn has_clock(&self) -> bool {
        self.clock.is_some()
    }
}

impl Session {
    /// The window's clock: a search's call then takes `budget` of it at most too, beginning a
    /// unit of work only while one as long as the longest of the call so far still fits. A search
    /// asked for whole is a slice too (4096 messages at most), carried on by
    /// `search.more`. `None`: counts only, and whole when asked, as the command line and the
    /// tests run it.
    pub fn set_search_clock(&mut self, clock: Option<SearchClock>) {
        self.search_pace.set_clock(clock);
    }
}
