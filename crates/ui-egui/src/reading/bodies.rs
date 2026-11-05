// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The open message's body got ready on the reading lane: read from the cache and parsed there
//! (a big message took 45 ms, a frame the window couldn't draw), shown when it's back (from
//! upstream a75755e). Meanwhile the reading pane shows the message's subject where its header
//! goes, and a quiet spinner.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use quatrelle_engine::model::Summary;
use quatrelle_engine::{BodyWork, PreparedBody};

use super::*;
use crate::{Lane, MailApp, Work};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the window and the reading lane share.
#[derive(Default)]
pub(crate) struct Bodies {
    /// The latest work for the lane: work handed over before it ran finds the newest here (or
    /// nothing), so a body the pane moved on from is never read.
    next: Arc<Mutex<Option<BodyWork>>>,
    /// What the lane made of it, for the window to show (the latest).
    done: Arc<Mutex<Option<PreparedBody>>>,
}

impl MailApp {
    /// From now on (after the commands asked for at start and the files handed over, which want
    /// their answers at once), the open message's body is got ready on the reading lane, where
    /// there is one.
    pub fn defer_bodies(&mut self) {
        let on = self.services.has_background();
        self.session.defer_bodies(on);
    }

    /// A body the lane got ready is shown (its source given to the pictures, which needn't read
    /// it again).
    pub(crate) fn take_body(&mut self) {
        let done = lock(&self.ui.bodies.done).take();
        if let Some(p) = done {
            if let Some(raw) = p.raw().filter(|_| self.session.body_waiting(p.key())) {
                self.ui.pictures.give_source(p.key(), raw.clone());
            }
            self.session.finish_body(p);
        }
    }

    /// The body the open message waits for goes to the reading lane (once a frame, the latest
    /// only), or is got ready here when no lane takes it.
    pub(crate) fn send_body_work(&mut self, ctx: &egui::Context) {
        let Some(work) = self.session.take_body_work() else { return };
        // Work handed over before that hasn't run yet takes this one instead.
        if lock(&self.ui.bodies.next).replace(work).is_some() {
            return;
        }
        let (next, done, ctx) = (self.ui.bodies.next.clone(), self.ui.bodies.done.clone(), ctx.clone());
        let run = Box::new(move || {
            let work = lock(&next).take();
            if let Some(work) = work {
                let prepared = work.run();
                *lock(&done) = Some(prepared);
                ctx.request_repaint();
            }
        });
        if let Err(work) = self.services.background(Lane::Reading, Work { run, cancelled: Arc::new(AtomicBool::new(false)), first: false }) {
            (work.run)();
            self.take_body();
        }
    }
}

/// The reading pane while the open message is got ready on the reading lane (a frame or a few):
/// its subject, where the header puts it, and a quiet spinner.
pub(super) fn getting_ready(t: &Tokens, ui: &mut Ui, summary: Option<&Summary>) {
    if let Some(m) = summary {
        let subject = if m.subject.is_empty() { tl!("(no subject)") } else { m.subject.as_str() };
        crate::bidi::label(ui, subject, theme::bold(21.0), t.text);
        ui.add_space(10.0);
    }
    ui.add(egui::Spinner::new().size(14.0).color(t.faint));
}
