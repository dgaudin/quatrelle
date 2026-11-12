// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The rows of the list and the open chat's window. A row is found by its id in an index kept
//! beside the rows, never by going through them: an event of 50 rows touches 50 entries, and the
//! list is gone through (sorted and indexed again) once for an event whose rows move. The window
//! holds at most [`WINDOW_MAX`] messages of the open chat; past that its far end goes.

use std::cmp::Reverse;

use quatrelle_model::pim::teams::{ChatMessage, ChatRow, MAX_CHATS, MAX_ID, WINDOW_MAX};

use super::*;

#[cfg(test)]
thread_local! {
    /// Passes over every row (a sort, the index made again), for the tests.
    static SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times this thread went through every row.
#[cfg(test)]
pub(crate) fn row_scans() -> usize {
    SCANS.with(std::cell::Cell::get)
}

/// A row's place in the list: newest activity first (none last), then by id.
fn order(r: &ChatRow) -> (Reverse<i64>, &str) {
    (Reverse(r.at.unwrap_or(i64::MIN)), r.id.as_str())
}

/// A message's place in the window: by creation, then by id.
fn place(m: &ChatMessage) -> (i64, &str) {
    (m.created, m.id.as_str())
}

fn usable(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID
}

impl TeamsState {
    /// Rows new or changed. Each is found by its id and changed where it is while it keeps its
    /// place; when some move (new activity) or come in among the others, the list is sorted and
    /// indexed again, once. Past [`MAX_CHATS`] the least recently active go (a new row older than
    /// all of them isn't taken).
    pub(super) fn upsert_rows(&mut self, rows: Vec<ChatRow>) {
        let mut moved = false;
        for row in rows {
            if !usable(&row.id) {
                continue;
            }
            match self.index.get(&row.id).copied() {
                Some(i) => {
                    let Some(slot) = self.rows.get_mut(i) else { continue };
                    *slot = row;
                    moved |= !self.in_place(i);
                }
                None => {
                    if self.rows.len() >= MAX_CHATS && self.rows.last().is_some_and(|last| order(&row) > order(last)) {
                        continue;
                    }
                    let i = self.rows.len();
                    self.index.insert(row.id.clone(), i);
                    self.rows.push(row);
                    moved |= !self.in_place(i);
                }
            }
        }
        if moved || self.rows.len() > MAX_CHATS {
            self.reorder();
        }
    }

    /// Whether the row at `i` is between its neighbours in the list's order.
    fn in_place(&self, i: usize) -> bool {
        let Some(r) = self.rows.get(i) else { return true };
        let after_previous = i.checked_sub(1).and_then(|p| self.rows.get(p)).is_none_or(|p| order(p) < order(r));
        let before_next = i.checked_add(1).and_then(|n| self.rows.get(n)).is_none_or(|n| order(r) < order(n));
        after_previous && before_next
    }

    /// The list sorted, cut at [`MAX_CHATS`], and indexed again.
    fn reorder(&mut self) {
        #[cfg(test)]
        SCANS.with(|s| s.set(s.get().saturating_add(1)));
        let Self { rows, index, .. } = self;
        rows.sort_by(|a, b| order(a).cmp(&order(b)));
        for gone in rows.drain(MAX_CHATS.min(rows.len())..) {
            index.remove(&gone.id);
        }
        for (i, r) in rows.iter().enumerate() {
            if let Some(slot) = index.get_mut(&r.id) {
                *slot = i;
            }
        }
    }

    /// The user is no longer in this chat (left, removed, or it was deleted): it stays listed, to
    /// read.
    pub(super) fn left(&mut self, chat: &str) {
        if let Some(r) = self.index.get(chat).copied().and_then(|i| self.rows.get_mut(i)) {
            r.left = true;
        }
    }

    /// A chat's messages: into the window when it's the open chat's, each in its place by
    /// creation (an edit replaces its message). Past [`WINDOW_MAX`] the far end goes: the newest
    /// when the user went back through the history, else the oldest.
    pub(super) fn take_messages(&mut self, chat: &str, messages: Vec<ChatMessage>) {
        if self.open.as_deref() != Some(chat) {
            return;
        }
        for m in messages {
            if m.chat != chat || !usable(&m.id) {
                continue;
            }
            if let Some(i) = self.window.iter().position(|w| w.id == m.id) {
                match self.window.get_mut(i) {
                    Some(w) if w.created == m.created => {
                        *w = m;
                        continue;
                    }
                    _ => {
                        self.window.remove(i);
                    }
                }
            } else if self.newer_left_out && self.window.back().is_some_and(|last| place(&m) > place(last)) {
                continue;
            }
            let at = self.window.partition_point(|w| place(w) < place(&m));
            self.window.insert(at, m);
        }
        while self.window.len() > WINDOW_MAX {
            if self.scrolled_back {
                self.window.pop_back();
                self.newer_left_out = true;
            } else {
                self.window.pop_front();
            }
        }
    }
}
