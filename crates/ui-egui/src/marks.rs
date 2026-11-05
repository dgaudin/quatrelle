// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A local account's marks written on the writing lane: a message marked read in a local account
//! or a data file wrote its mark and its folder's count on the window's thread, 25 ms at each
//! Down held. The session queues them; once a frame, the work writing them goes to the lane
//! (one at a time), or is done here when no lane takes it.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::{Lane, MailApp, Work};

impl MailApp {
    /// From now on (after the commands asked for at start, which want their answers at once),
    /// local accounts' marks are written on the writing lane, where there is one.
    pub fn defer_local_marks(&mut self) {
        let on = self.services.has_background();
        self.session.defer_local_marks(on);
    }

    /// The marks waiting go to the writing lane, or are written here when no lane takes them.
    pub(crate) fn send_mark_work(&mut self) {
        let Some(work) = self.session.take_mark_work() else { return };
        let run = Box::new(move || work.run());
        if let Err(work) = self.services.background(Lane::Writing, Work { run, cancelled: Arc::new(AtomicBool::new(false)), first: false }) {
            (work.run)();
        }
    }
}
