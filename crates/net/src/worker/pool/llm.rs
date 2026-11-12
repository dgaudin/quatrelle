// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The pool: the language model's jobs. A question and the list of models each go on a thread of
//! their own, where the key is read too: a keychain slow to answer (or waiting to be unlocked)
//! mustn't hold the window.

use super::*;

impl Pool {
    pub(super) fn llm(&mut self, job: Job) {
        match job {
            Job::AskLlm { id, url, model, messages } => {
                let (sink, creds) = (self.sink.clone(), self.creds.clone());
                let spawned = std::thread::Builder::new().name("language-model".into()).spawn(move || {
                    let key = creds.get_secret(&crate::llm::keyring(), crate::llm::KEY);
                    let done = match crate::llm::ask(&url, &model, key.as_deref(), &messages) {
                        Ok(text) => Event::LlmAnswered { id, text, error: None },
                        Err(e) => Event::LlmAnswered { id, text: String::new(), error: Some(e.to_string()) },
                    };
                    sink.emit(done);
                });
                if let Err(e) = spawned {
                    log::warn!("language model: {e}");
                }
            }
            Job::ListLlmModels { url } => {
                let (sink, creds) = (self.sink.clone(), self.creds.clone());
                let spawned = std::thread::Builder::new().name("language-models".into()).spawn(move || {
                    let key = creds.get_secret(&crate::llm::keyring(), crate::llm::KEY);
                    let done = match crate::llm::models(&url, key.as_deref()) {
                        Ok(models) => Event::LlmModels { url, models, error: None },
                        Err(e) => Event::LlmModels { url, models: Vec::new(), error: Some(e.to_string()) },
                    };
                    sink.emit(done);
                });
                if let Err(e) = spawned {
                    log::warn!("language model: {e}");
                }
            }
            Job::SetLlmKey { key } => {
                if key.0.trim().is_empty() {
                    self.creds.delete_secret(&crate::llm::keyring(), crate::llm::KEY);
                } else if let Err(e) = self.creds.set_secret(&crate::llm::keyring(), crate::llm::KEY, key.0.trim()) {
                    self.sink.emit(Event::Notice { account: String::new(), message: format!("the language model's key couldn't be kept: {e}") });
                }
            }
            _ => {}
        }
    }
}
