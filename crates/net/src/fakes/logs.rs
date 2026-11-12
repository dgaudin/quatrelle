// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The log as the tests read it. A program has one logger, so the test binary's is this one,
//! whoever asks first: it hands each line to the tests listening ([`tap`]), from whichever
//! thread it comes, and drops it when none is.

use std::sync::{Arc, Mutex, Once, PoisonError};

/// Whether a tap keeps a line.
type Keep = Box<dyn Fn(&str) -> bool + Send>;

/// The tests listening: each one's number, what it keeps, and where.
static TAPS: Mutex<Vec<(u64, Keep, Arc<Mutex<Vec<String>>>)>> = Mutex::new(Vec::new());
static NEXT: Mutex<u64> = Mutex::new(0);

struct Lines;

impl log::Log for Lines {
    fn enabled(&self, _: &log::Metadata) -> bool {
        !TAPS.lock().unwrap_or_else(PoisonError::into_inner).is_empty()
    }

    fn log(&self, record: &log::Record) {
        let line = record.args().to_string();
        for (_, keep, lines) in TAPS.lock().unwrap_or_else(PoisonError::into_inner).iter() {
            if keep(&line) {
                lines.lock().unwrap_or_else(PoisonError::into_inner).push(line.clone());
            }
        }
    }

    fn flush(&self) {}
}

/// A test listening to the log, until it's dropped.
pub(crate) struct Tap {
    id: u64,
    lines: Arc<Mutex<Vec<String>>>,
}

impl Tap {
    /// The lines kept so far.
    pub(crate) fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        TAPS.lock().unwrap_or_else(PoisonError::into_inner).retain(|(id, _, _)| *id != self.id);
    }
}

/// Listen to the log from now on, at every level: the lines `keep` accepts, whichever test's
/// threads write them (tests run side by side: keep what only this test's lines carry, or look
/// only for what only they could). `keep` must not log.
pub(crate) fn tap(keep: impl Fn(&str) -> bool + Send + 'static) -> Tap {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = log::set_logger(&Lines);
        log::set_max_level(log::LevelFilter::Trace);
    });
    let id = {
        let mut next = NEXT.lock().unwrap_or_else(PoisonError::into_inner);
        *next += 1;
        *next
    };
    let lines = Arc::new(Mutex::new(Vec::new()));
    TAPS.lock().unwrap_or_else(PoisonError::into_inner).push((id, Box::new(keep), lines.clone()));
    Tap { id, lines }
}
