// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The Teams worker's events, taken a few a frame: a first copy sends thousands of rows and
//! messages, and applying them all in one frame would freeze the window. The mail's events are
//! applied at once, as before.
//!
//! What is hostile here: how many events the worker sends (it reports what Graph gave). The queue
//! holds at most [`PENDING_MAX`] (and what is said once); past it the oldest rows and messages go
//! (the worker sends them again on its next pass), and what says where the connection stands
//! stays.

use std::collections::VecDeque;

use quatrelle_engine::Session;
use quatrelle_engine::model::Event;
use quatrelle_engine::model::pim::teams;

use super::pictures::TeamsPictures;

/// The most Teams events applied in a frame.
pub(crate) const TEAMS_EVENTS_A_FRAME: usize = 4;
/// The most Teams events waiting.
const PENDING_MAX: usize = 10_000;

/// The Teams events waiting to be applied, oldest first, and the pictures they brought.
#[derive(Default)]
pub(crate) struct TeamsQueue {
    pending: VecDeque<Event>,
    pub(crate) pictures: TeamsPictures,
}

fn teams_event(e: &Event) -> Option<&teams::Event> {
    match e {
        Event::Teams { event } => Some(event),
        _ => None,
    }
}

/// Rows and messages: the worker sends them again.
fn sent_again(e: &Event) -> bool {
    matches!(teams_event(e), Some(teams::Event::Chats { .. } | teams::Event::Messages { .. }))
}

fn is_status(e: &Event) -> bool {
    matches!(teams_event(e), Some(teams::Event::Status { .. }))
}

/// Who signed in, a copy set aside, the open chat's members, a chat left: the worker doesn't
/// say them again, so they're never dropped.
fn said_once(e: &Event) -> bool {
    matches!(
        teams_event(e),
        Some(teams::Event::Account { .. } | teams::Event::SetAside { .. } | teams::Event::Members { .. } | teams::Event::Removed { .. })
    )
}

impl TeamsQueue {
    /// Every Teams event of `events` moved to the queue; the others stay, in their order.
    pub(crate) fn take(&mut self, events: &mut Vec<Event>) {
        if !events.iter().any(|e| teams_event(e).is_some()) {
            return;
        }
        let (teams, others): (Vec<Event>, Vec<Event>) = std::mem::take(events).into_iter().partition(|e| teams_event(e).is_some());
        *events = others;
        self.pending.extend(teams);
        self.bound();
    }

    /// At most `most` events applied, oldest first; whether some still wait (the frame is drawn
    /// again for them). Called every frame, whatever the module.
    pub(crate) fn apply(&mut self, s: &mut Session, most: usize) -> bool {
        for _ in 0..most {
            let Some(e) = self.pending.pop_front() else { break };
            s.apply(e);
        }
        // Pictures answered: taken to be decoded every frame, so the engine's queue of them
        // never fills.
        self.pictures.take(s);
        !self.pending.is_empty()
    }

    /// Past [`PENDING_MAX`]: the oldest rows and messages go first, then statuses a later one says
    /// again, then the oldest of the rest; the last status, and what is said once, always stay.
    fn bound(&mut self) {
        let mut over = self.pending.len().saturating_sub(PENDING_MAX);
        if over == 0 {
            return;
        }
        self.pending.retain(|e| {
            let drop = over > 0 && sent_again(e);
            over = over.saturating_sub(usize::from(drop));
            !drop
        });
        let last_status = self.pending.iter().rposition(is_status);
        let mut i = 0usize;
        self.pending.retain(|e| {
            let drop = over > 0 && is_status(e) && Some(i) != last_status;
            over = over.saturating_sub(usize::from(drop));
            i = i.saturating_add(1);
            !drop
        });
        let mut i = 0usize;
        let last_status = self.pending.iter().rposition(is_status);
        self.pending.retain(|e| {
            let drop = over > 0 && Some(i) != last_status && !said_once(e);
            over = over.saturating_sub(usize::from(drop));
            i = i.saturating_add(1);
            !drop
        });
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether a Teams event waiting is one `f` takes.
    #[cfg(test)]
    pub(crate) fn holds(&self, f: impl Fn(&teams::Event) -> bool) -> bool {
        self.pending.iter().filter_map(teams_event).any(f)
    }
}
