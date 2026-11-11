// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The check's hour: the approved polling cadences, run for a while to measure the 429 rate
//! they meet, the access token renewed on the way.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::*;

/// The approved cadences: the chat list, the open chat, the chats active lately.
const LIST_EVERY: Duration = Duration::from_secs(10);
const OPEN_EVERY: Duration = Duration::from_secs(3);
const ACTIVE_EVERY: Duration = Duration::from_secs(60);
/// What "active lately" is (milliseconds), and how many such chats are read at most.
const ACTIVE_FOR: i64 = 7 * 86_400_000;
const ACTIVE_MAX: usize = 20;
/// How often the hour says how it is going.
const REPORT_EVERY: Duration = Duration::from_secs(600);

/// 6. The approved cadences, for `d`: the 429 rate they meet.
///
/// The open chat is the newest one read; the chats Graph refused to read are left out, as
/// Quatrelle would.
impl Check<'_> {
    pub(super) fn hour(&mut self, d: Duration, newest: Option<String>, watermark: Option<i64>, refused: &HashSet<String>) {
        let before = self.client.stats();
        let start = Instant::now();
        // No request starts past the end: only one already sent (its waits on a 429) can overrun.
        let end = start.checked_add(d).unwrap_or(start);
        let from = self.now();
        // Each chat's watermark (milliseconds): what its incremental reads start after.
        let mut marks: HashMap<String, i64> = HashMap::new();
        if let (Some(chat), Some(at)) = (&newest, watermark.or(from)) {
            marks.insert(chat.clone(), at);
        }
        // The list's chats, with when their last message came.
        let mut list: Vec<(String, Option<i64>)> = Vec::new();
        let every = report_every();
        let (mut at_list, mut at_open, mut at_active, mut at_report) = (Duration::ZERO, Duration::ZERO, Duration::ZERO, every);
        let mut h = Hourly::default();
        loop {
            let t = start.elapsed();
            if t >= d {
                break;
            }
            if t >= at_list {
                at_list = t.saturating_add(LIST_EVERY);
                let mut r = self.client.chats(None);
                if self.renewed(&r, &mut h) {
                    r = self.client.chats(None);
                }
                let r = r.map(|p| {
                    list = p
                        .items
                        .iter()
                        .filter_map(|c| Some((id_of(c)?, c.pointer("/lastMessagePreview/createdDateTime")?.as_str().and_then(ms_of))))
                        .collect();
                    true
                });
                let newest_at = list.iter().filter_map(|(_, at)| *at).max();
                self.saw(newest_at);
                if !h.settle(r) {
                    break;
                }
            }
            if t >= at_open {
                at_open = t.saturating_add(OPEN_EVERY);
                if let Some(chat) = &newest {
                    let r = self.incremental_renewing(chat, &mut marks, from, end, &mut h);
                    if !h.settle(r) {
                        break;
                    }
                }
            }
            if t >= at_active {
                at_active = t.saturating_add(ACTIVE_EVERY);
                let since = self.now().map(|n| n.saturating_sub(ACTIVE_FOR));
                let active: Vec<String> = list
                    .iter()
                    .filter(|(id, at)| Some(id) != newest.as_ref() && !refused.contains(id) && since.is_some_and(|s| at.is_some_and(|a| a >= s)))
                    .take(ACTIVE_MAX)
                    .map(|(id, _)| id.clone())
                    .collect();
                // The end of the hour, or a token refused for good, stops the pass; a chat that
                // fails otherwise doesn't.
                for chat in &active {
                    let r = self.incremental_renewing(chat, &mut marks, from, end, &mut h);
                    if !h.settle(r) {
                        break;
                    }
                }
            }
            if h.over() {
                break;
            }
            if t >= at_report {
                at_report = at_report.saturating_add(every);
                let s = self.client.stats();
                let (requests, throttled) = (s.requests.saturating_sub(before.requests), s.throttled.saturating_sub(before.throttled));
                self.say(format!("hour, after {} min: requests {requests}, throttled {throttled}, failed {}", t.as_secs() / 60, h.failed));
            }
            let wake = at_list.min(at_open).min(at_active).min(at_report).min(d);
            let wait = wake.saturating_sub(start.elapsed());
            if !wait.is_zero() {
                (self.opts.sleep)(wait);
            }
        }
        let s = self.client.stats();
        let (requests, throttled) = (s.requests.saturating_sub(before.requests), s.throttled.saturating_sub(before.throttled));
        let rate = if requests == 0 { 0.0 } else { f64::from(throttled) * 100.0 / f64::from(requests) };
        let cut = if h.cut { ", cut short at the end" } else { "" };
        let renewed = if h.renewed > 0 { format!(", sign-in renewed {}", h.renewed) } else { String::new() };
        self.say(format!(
            "hour, {} s: requests {requests}, throttled {throttled}, failed {}, 429 rate {rate:.1} %{}{cut}{renewed}",
            start.elapsed().as_secs(),
            h.failed,
            h.stopped
        ));
    }

    /// When `r` is a refused token and [`CheckOptions::renew`] gives a new one: the client takes
    /// it, and the request is to be made once more. Once only: a second refusal stops the hour.
    fn renewed<T>(&self, r: &Result<T>, h: &mut Hourly) -> bool {
        if !matches!(r, Err(NetError::Auth(_))) {
            return false;
        }
        let Some(token) = self.opts.renew.as_ref().and_then(|renew| renew()) else { return false };
        self.client.set_token(&token);
        h.renewed = h.renewed.saturating_add(1);
        true
    }

    /// [`Self::incremental`], made once more after a refused token is renewed.
    fn incremental_renewing(
        &mut self,
        chat: &str,
        marks: &mut HashMap<String, i64>,
        from: Option<i64>,
        end: Instant,
        h: &mut Hourly,
    ) -> Result<bool> {
        let r = self.incremental(chat, marks, from, end);
        if self.renewed(&r, h) { self.incremental(chat, marks, from, end) } else { r }
    }

    /// A chat's messages changed after its watermark, every page; the watermark moved to the
    /// newest change seen. `false` when `end` came first (no page is asked for past it).
    fn incremental(&mut self, chat: &str, marks: &mut HashMap<String, i64>, from: Option<i64>, end: Instant) -> Result<bool> {
        let Some(mark) = marks.get(chat).copied().or(from) else { return Ok(true) };
        let after = rfc3339(mark);
        let mut newest = mark;
        let mut next: Option<String> = None;
        for _ in 0..PAGES {
            if Instant::now() >= end {
                return Ok(false);
            }
            let page = self.client.messages(chat, Range::After(&after), next.as_deref())?;
            for m in &page.items {
                newest = newest.max(m.get("lastModifiedDateTime").and_then(Value::as_str).and_then(ms_of).unwrap_or(newest));
            }
            match page.next {
                Some(n) => next = Some(n),
                None => break,
            }
        }
        marks.insert(chat.to_string(), newest);
        Ok(true)
    }
}

/// How the hour's requests went.
#[derive(Default)]
struct Hourly {
    failed: u32,
    /// The hour's end came in the middle of a pass.
    cut: bool,
    /// Why the hour stopped before its end (`, stopped (401)`), or empty.
    stopped: String,
    /// Tokens renewed.
    renewed: u32,
}

impl Hourly {
    /// Note how a request went: whether the hour goes on.
    fn settle(&mut self, r: Result<bool>) -> bool {
        match r {
            Ok(done) => self.cut |= !done,
            Err(e) => {
                self.failed = self.failed.saturating_add(1);
                if matches!(e, NetError::Auth(_)) {
                    self.stopped = format!(", stopped ({})", why(&e));
                }
            }
        }
        !self.over()
    }

    /// Whether the hour is over before its time: cut at the end, or the token refused for good.
    fn over(&self) -> bool {
        self.cut || !self.stopped.is_empty()
    }
}

/// How often the hour says how it is going.
fn report_every() -> Duration {
    #[cfg(test)]
    if let Some(d) = REPORT_EVERY_IN_TESTS.with(std::cell::Cell::get) {
        return d;
    }
    REPORT_EVERY
}
