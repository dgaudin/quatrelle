// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The pool: Teams' jobs. They go to the Teams worker (`teams/worker.rs`), a thread of its own
//! started with the first of them: nothing of Teams runs until asked. There is one worker for
//! the pool's life: switched off or signed out, it asks nothing and holds nothing, and takes the
//! jobs after in their order. A second one started while the first finished a stalled request
//! would be two writers of one copy, and the first's late sign-out would delete the sign-in made
//! since.

use quatrelle_model::pim::teams;

use super::*;
use crate::teams::worker::{self, Microsoft};

/// Tests: the Graph address and what makes who signs in, in place of Microsoft's; and how many
/// workers the pool started.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Test {
    pub(crate) at: Option<(String, Box<dyn Fn() -> Box<dyn worker::Authority> + Send>)>,
    pub(crate) started: u32,
    /// The system gives no thread for the worker.
    pub(crate) no_thread: bool,
}

impl Pool {
    pub(super) fn teams_job(&mut self, job: teams::Job) {
        if self.teams.is_none() {
            // Nothing runs: nothing to stop.
            if job == teams::Job::Stop {
                return;
            }
            let sink = self.sink.clone();
            let emit: worker::Emit = Arc::new(move |e| sink.emit(e));
            let (graph, authority): (String, Box<dyn worker::Authority>) = (crate::graph::DEFAULT_URL.to_string(), Box::new(Microsoft));
            #[cfg(test)]
            let (graph, authority) = match &self.teams_test.at {
                Some((graph, authority)) => (graph.clone(), authority()),
                None => (graph, authority),
            };
            #[cfg(test)]
            {
                self.teams_test.started += 1;
            }
            #[cfg(test)]
            let started = if self.teams_test.no_thread {
                Err(std::io::Error::other("no thread, as the test asked"))
            } else {
                worker::start(self.store.clone(), self.creds.clone(), emit, graph, authority)
            };
            #[cfg(not(test))]
            let started = worker::start(self.store.clone(), self.creds.clone(), emit, graph, authority);
            match started {
                Ok(handle) => {
                    // Started while working offline: it asks nothing until that's turned off.
                    if self.offline {
                        handle.work_offline(true);
                    }
                    self.teams = Some(handle);
                }
                Err(e) => {
                    // Said, so that nothing waits for an answer that won't come: a meeting's ask
                    // is answered too, its window waiting on it.
                    log::warn!("teams: the worker didn't start: {e}");
                    let status = teams::Status::Failed { why: "The sign-in stopped unexpectedly; try again".into() };
                    self.sink.emit(Event::Teams { event: teams::Event::Status { status } });
                    if let Some(event) = unanswered(&job) {
                        self.sink.emit(Event::Teams { event });
                    }
                    return;
                }
            }
        }
        if let Some(handle) = &self.teams {
            handle.submit(job);
        }
    }
}

/// The answer to a meeting's ask that no worker will make: Teams said nothing ("no connection").
/// The other jobs have no answer waited for.
fn unanswered(job: &teams::Job) -> Option<teams::Event> {
    let offline = Some(teams::TeamsMeetingError::Offline);
    match job {
        teams::Job::MeetingCreate { ask, .. } => Some(teams::Event::MeetingCreated { ask: *ask, meeting: None, error: offline }),
        teams::Job::MeetingUpdate { id, .. } => Some(teams::Event::MeetingUpdated { id: id.clone(), meeting: None, error: offline }),
        teams::Job::MeetingDelete { id } => Some(teams::Event::MeetingDeleted { id: id.clone(), error: offline }),
        _ => None,
    }
}
