// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! "Try it on examples": the twenty messages written for Quatrelle (`quatrelle_model::scam::EXAMPLES`)
//! put to the language model one at a time, to see what it makes of them before the check is
//! turned on. Nothing of the user's is sent; nothing is kept; a real arrival is never mixed in.

use quatrelle_model::scam::{EXAMPLES, Facts, WARN_SCORE, read_answer, request};

use super::*;

/// What the test found so far, or in the end.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScamTest {
    /// Still asking.
    pub running: bool,
    /// Examples answered (or given up on) so far, and all of them.
    pub done: usize,
    pub total: usize,
    /// The scams among them, and how many the model scored as a warning.
    pub scams: usize,
    pub spotted: usize,
    /// The ordinary messages among them, and how many it left alone.
    pub normals: usize,
    pub left_alone: usize,
    /// Examples with no usable answer (server failed, nonsense, nothing within a minute).
    pub unanswered: usize,
    /// The examples the model got wrong or didn't answer about, in order.
    pub misses: Vec<Miss>,
    /// Seconds from the first question to the last answer.
    pub secs: i64,
}

/// An example the model got wrong (its score), or gave no usable answer about.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Miss {
    pub subject: String,
    /// The example is a scam (let through), or an ordinary message (taken for one).
    pub scam: bool,
    pub score: Option<u8>,
}

impl ScamTest {
    /// Seconds a message took, rounded.
    pub fn secs_per_message(&self) -> i64 {
        if self.done == 0 { 0 } else { (self.secs + self.done as i64 / 2) / self.done as i64 }
    }
}

/// The run: its result so far, the question on its way and when the run began.
#[derive(Debug)]
pub(super) struct Trial {
    pub(super) result: ScamTest,
    id: u64,
    since: i64,
    began: i64,
}

impl Session {
    /// The last test's result (or the one running), if one was started.
    pub fn scam_test(&self) -> Option<&ScamTest> {
        self.scam.trial.as_ref().map(|t| &t.result)
    }

    /// Start the test: the examples go to the model one at a time; the result is read with
    /// [`Session::scam_test`]. Starts over when one was run before.
    pub fn start_scam_test(&mut self) -> CmdResult {
        if self.prefs.llm_url.trim().is_empty() || self.prefs.llm_model.trim().is_empty() {
            return Err(CmdError::Disabled("set a language model's server and model in File › Options › Assistant first".into()));
        }
        if self.scam.trial.as_ref().is_some_and(|t| t.result.running) {
            return Ok(self.scam_test_json());
        }
        let result = ScamTest {
            running: true,
            total: EXAMPLES.len(),
            scams: EXAMPLES.iter().filter(|e| e.scam).count(),
            normals: EXAMPLES.iter().filter(|e| !e.scam).count(),
            ..Default::default()
        };
        self.scam.trial = Some(Trial { result, id: 0, since: self.clock.now, began: self.clock.now });
        self.trial_ask();
        Ok(self.scam_test_json())
    }

    /// The test's state as the command gives it.
    pub fn scam_test_json(&self) -> Value {
        match self.scam_test() {
            None => Value::Null,
            Some(t) => serde_json::json!({
                "running": t.running, "done": t.done, "total": t.total, "scams": t.scams, "spotted": t.spotted,
                "normals": t.normals, "leftAlone": t.left_alone, "unanswered": t.unanswered, "misses": t.misses,
                "secondsPerMessage": t.secs_per_message(),
            }),
        }
    }

    /// Ask the model about the next example, or end the run.
    fn trial_ask(&mut self) {
        let lang = self.answer_language();
        let (url, model) = (self.prefs.llm_url.trim().to_string(), self.prefs.llm_model.trim().to_string());
        let id = self.fresh_id();
        let now = self.clock.now;
        let Some(trial) = self.scam.trial.as_mut() else { return };
        let Some(e) = EXAMPLES.get(trial.result.done) else {
            trial.result.running = false;
            trial.result.secs = now - trial.began;
            return;
        };
        let facts = Facts {
            from_name: e.from_name,
            from: e.from,
            reply_to: None,
            subject: e.subject,
            date: now,
            known: false,
            signs: &[],
            link_targets: &[],
            attachment_names: &[],
            text: e.text,
        };
        trial.id = id;
        trial.since = now;
        let messages = request(&facts, &lang);
        self.jobs.push(Job::AskLlm { id, url, model, messages });
    }

    /// The model's answer `id` if it was the test's: counted. `false` when it wasn't.
    pub(super) fn trial_answered(&mut self, id: u64, text: &str, error: Option<&str>) -> bool {
        let now = self.clock.now;
        let Some(trial) = self.scam.trial.as_mut().filter(|t| t.result.running && t.id == id) else { return false };
        let verdict = if error.is_some() { None } else { read_answer(text) };
        trial.finish(verdict.map(|v| v.0), now);
        self.trial_ask();
        true
    }

    /// A question that got no answer in time is given up on (called with every tick).
    pub(super) fn trial_tick(&mut self) {
        let now = self.clock.now;
        if let Some(trial) = self.scam.trial.as_mut().filter(|t| t.result.running && now - t.since > ANSWER_SECS) {
            log::info!("scam test: no answer from the model within {ANSWER_SECS} s, skipped");
            trial.finish(None, now);
            self.trial_ask();
        }
    }
}

impl Trial {
    /// Count the example just asked about: its score, or none.
    fn finish(&mut self, score: Option<u8>, now: i64) {
        let Some(e) = EXAMPLES.get(self.result.done) else { return };
        let r = &mut self.result;
        r.done += 1;
        r.secs = now - self.began;
        match score {
            None => {
                r.unanswered += 1;
                r.misses.push(Miss { subject: e.subject.to_string(), scam: e.scam, score: None });
            }
            Some(s) => {
                let warns = s >= WARN_SCORE;
                match (e.scam, warns) {
                    (true, true) => r.spotted += 1,
                    (false, false) => r.left_alone += 1,
                    _ => r.misses.push(Miss { subject: e.subject.to_string(), scam: e.scam, score: Some(s) }),
                }
            }
        }
    }
}
