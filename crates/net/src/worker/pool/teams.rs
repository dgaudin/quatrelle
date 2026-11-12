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
            match worker::start(self.store.clone(), self.creds.clone(), emit, graph, authority) {
                Ok(handle) => self.teams = Some(handle),
                Err(e) => {
                    // Said, so that nothing waits for an answer that won't come.
                    log::warn!("teams: the worker didn't start: {e}");
                    let status = teams::Status::Failed { why: "Teams couldn't be started".into() };
                    return self.sink.emit(Event::Teams { event: teams::Event::Status { status } });
                }
            }
        }
        if let Some(handle) = &self.teams {
            handle.submit(job);
        }
    }
}
