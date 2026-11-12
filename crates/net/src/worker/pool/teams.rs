// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The pool: Teams' jobs. They go to the Teams worker (`teams/worker.rs`), a thread of its own
//! started with the first of them: nothing of Teams runs until asked. It is let go once Teams
//! is switched off or signed out, and nothing of it runs after.

use quatrelle_model::pim::teams;

use super::*;
use crate::teams::worker::{self, Microsoft};

impl Pool {
    pub(super) fn teams_job(&mut self, job: teams::Job) {
        if self.teams.is_none() {
            // Nothing runs: nothing to stop.
            if job == teams::Job::Stop {
                return;
            }
            let sink = self.sink.clone();
            let emit: worker::Emit = Arc::new(move |e| sink.emit(e));
            match worker::start(self.store.clone(), self.creds.clone(), emit, crate::graph::DEFAULT_URL.to_string(), Box::new(Microsoft)) {
                Ok(handle) => self.teams = Some(handle),
                Err(e) => {
                    // Said, so that nothing waits for an answer that won't come.
                    log::warn!("teams: the worker didn't start: {e}");
                    let status = teams::Status::Failed { why: "Teams couldn't be started".into() };
                    return self.sink.emit(Event::Teams { event: teams::Event::Status { status } });
                }
            }
        }
        // The worker ends on a stop, and with its channel once it has signed out.
        let last = matches!(job, teams::Job::Stop | teams::Job::SignOut { .. });
        if let Some(handle) = &self.teams {
            handle.submit(job);
        }
        if last {
            self.teams = None;
        }
    }
}
