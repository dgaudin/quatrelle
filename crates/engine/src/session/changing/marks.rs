// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The session: a local account's marks written away from the window's thread. A message marked
//! read in a local account or a data file wrote its marks, then the account's `folders.json` for
//! its count of unread mail, on the window's thread: two commits, 25 ms at each Down held. Where
//! the window has a thread for them ([`Session::defer_local_marks`]), marks wait here in order,
//! and a piece of work ([`MarkWork`]) writes them there, each in one transaction
//! ([`Store::apply_marks`]). Anything else that reads or writes a local account's lists first
//! finishes the marks waiting ([`Session::finish_local_marks`]): the command that does, a list
//! read from the cache, another change. The CLI, `--cmd` and the tests write at once, as before.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::*;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A mark to write: the account, the change, and the counts of the folders it touched (path,
/// unread, total) as they were once it was made.
struct MarkWrite {
    account: String,
    op: Op,
    counts: Vec<(String, u32, u32)>,
}

#[derive(Default)]
struct Queue {
    waiting: VecDeque<MarkWrite>,
    /// A piece of work was handed out and hasn't found the queue empty yet.
    handed: bool,
    /// What couldn't be written, for the session to tell.
    errors: Vec<String>,
}

#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    /// Held while a mark is written: whoever finishes the marks waits for the one being written,
    /// and the marks go to the cache in the order they were made.
    writing: Mutex<()>,
}

impl Shared {
    /// The marks waiting, written one by one in order; what couldn't be is noted. `handed`: the
    /// work handed out ends here (the queue empty).
    fn write_waiting(&self, store: &Store, handed: bool) -> usize {
        let mut written = 0;
        loop {
            let _writing = lock(&self.writing);
            let next = {
                let mut q = lock(&self.queue);
                match q.waiting.pop_front() {
                    Some(m) => m,
                    None => {
                        if handed {
                            q.handed = false;
                        }
                        return written;
                    }
                }
            };
            if let Err(e) = store.apply_marks(&next.account, &next.op, &next.counts) {
                log::warn!("{}: a mark couldn't be kept: {e}", next.account);
                lock(&self.queue).errors.push(e.to_string());
            }
            written += 1;
        }
    }
}

/// The marks of local accounts waiting to be written.
#[derive(Default)]
pub(crate) struct LocalMarks {
    /// Written away from the window's thread ([`Session::defer_local_marks`]).
    defer: bool,
    store: Option<Store>,
    shared: Arc<Shared>,
    /// Marks written on the session's own thread ([`Session::local_marks_written_here`]).
    written_here: u64,
}

impl LocalMarks {
    pub(crate) fn new(store: Option<Store>) -> LocalMarks {
        LocalMarks { defer: false, store, shared: Arc::default(), written_here: 0 }
    }

    /// The marks waiting written here, after the one being written elsewhere; what couldn't be.
    fn finish(&mut self) -> Vec<String> {
        let Some(store) = &self.store else { return Vec::new() };
        let n = self.shared.write_waiting(store, false);
        self.written_here = self.written_here.wrapping_add(n as u64);
        std::mem::take(&mut lock(&self.shared.queue).errors)
    }
}

impl Drop for LocalMarks {
    /// The session ends: nothing made is left unwritten (a lane closing drops the work waiting).
    fn drop(&mut self) {
        for e in self.finish() {
            log::warn!("a mark wasn't kept: {e}");
        }
    }
}

/// The marks waiting, to write on another thread: in order, until there are none.
pub struct MarkWork {
    store: Store,
    shared: Arc<Shared>,
}

impl MarkWork {
    pub fn run(self) {
        self.shared.write_waiting(&self.store, true);
    }
}

impl Session {
    /// Write local accounts' marks away from the window's thread from now on (`on`: it has a
    /// thread for them, which runs [`Session::take_mark_work`]), or at once again.
    pub fn defer_local_marks(&mut self, on: bool) {
        self.marks.defer = on;
        if !on {
            self.finish_local_marks();
        }
    }

    /// Work writing the marks waiting, to run on another thread; asked once a frame (none while
    /// one handed out still runs). What couldn't be written is told.
    pub fn take_mark_work(&mut self) -> Option<MarkWork> {
        let (errors, work) = {
            let mut q = lock(&self.marks.shared.queue);
            let errors = std::mem::take(&mut q.errors);
            let work = match &self.marks.store {
                Some(store) if !q.handed && !q.waiting.is_empty() => {
                    q.handed = true;
                    Some(MarkWork { store: store.clone(), shared: self.marks.shared.clone() })
                }
                _ => None,
            };
            (errors, work)
        };
        for e in errors {
            self.notice(e, true);
        }
        work
    }

    /// The marks waiting, written here (after the one being written elsewhere, waited for):
    /// what comes next reads and writes the lists as they are.
    pub fn finish_local_marks(&mut self) {
        for e in self.marks.finish() {
            self.notice(e, true);
        }
    }

    /// How many local marks were written on the session's own thread (the window's, where
    /// they're deferred: those finished before something else).
    pub fn local_marks_written_here(&self) -> u64 {
        self.marks.written_here
    }

    /// A flag or category on a local account's messages whose folders are all in memory (they
    /// have it already, `apply_op_memory`): their counts taken from the lists, and the mark
    /// queued with them; written at once unless deferred.
    pub(super) fn queue_local_mark(&mut self, account: &str, op: &Op, touched: &[String]) {
        let mut counts = Vec::with_capacity(touched.len());
        for f in touched {
            if let Some(st) = self.cache.get(&(account.to_string(), f.clone())) {
                counts.push((f.clone(), st.unread(), u32::try_from(st.messages.len()).unwrap_or(u32::MAX)));
            }
        }
        if let Some(a) = self.account_mut(account) {
            for (p, unread, total) in &counts {
                if let Some(f) = a.folders.iter_mut().find(|f| &f.path == p) {
                    f.unread = *unread;
                    f.total = *total;
                }
            }
        }
        lock(&self.marks.shared.queue).waiting.push_back(MarkWrite { account: account.to_string(), op: op.clone(), counts });
        if !self.marks.defer {
            self.finish_local_marks();
        }
    }
}
