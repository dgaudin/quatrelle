//! The session: the language model set in Options (an OpenAI-compatible server), asked about the
//! message being read; its answer is shown above it.

use super::*;

/// The most of a message's text sent to the model: its start, where mail says what it wants.
const MAX_TEXT: usize = 24_000;

/// How the model is asked for a summary.
const SUMMARY_PROMPT: &str = "You summarize an email for the person who received it. Write in the language the email is written in. \
Give the gist in at most five short bullet points starting with \"- \": who wants what, by when, and what is expected of the reader. \
No preamble, and nothing that isn't in the email.";

/// What the language model was asked, and its answer once it came.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assistant {
    pub id: u64,
    /// The message asked about.
    pub key: MessageKey,
    /// `None` while it's working; the text, or why there's none.
    pub answer: Option<Result<String, String>>,
}

impl Session {
    /// Summarize the message being read with the language model set in Options; the answer
    /// comes in as [`Session::assistant`].
    pub fn summarize(&mut self) -> CmdResult {
        let url = self.prefs.llm_url.trim().to_string();
        if url.is_empty() {
            return Err(CmdError::Disabled("set a language model in File › Options › Assistant first".into()));
        }
        let r = self.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        let p = r.parsed.as_ref().ok_or_else(|| CmdError::Disabled("the message isn't downloaded yet".into()))?;
        let from = if p.from.name.trim().is_empty() { p.from.email.clone() } else { format!("{} <{}>", p.from.name.trim(), p.from.email) };
        let text: String = p.text.chars().take(MAX_TEXT).collect();
        let mail = format!("From: {from}\nSubject: {}\n\n{text}", p.subject);
        let key = r.key.clone();
        let id = self.fresh_id();
        self.assistant = Some(Assistant { id, key, answer: None });
        self.jobs.push(Job::AskLlm {
            id,
            url,
            model: self.prefs.llm_model.trim().to_string(),
            messages: vec![("system".into(), SUMMARY_PROMPT.into()), ("user".into(), mail)],
        });
        Ok(serde_json::json!({ "asked": id }))
    }

    /// The language model's answer, for the question still on view.
    pub(in crate::session) fn llm_answered(&mut self, id: u64, text: String, error: Option<String>) {
        if let Some(a) = self.assistant.as_mut().filter(|a| a.id == id) {
            a.answer = Some(match error {
                Some(e) => Err(e),
                None => Ok(text),
            });
        }
    }

    /// Set the language model: an OpenAI-compatible server's address (`…/v1`; empty: none) and
    /// the model to ask; a key given goes to the keychain only.
    pub fn set_assistant(&mut self, url: &str, model: &str, key: Option<&str>) -> CmdResult {
        let url = url.trim();
        let lower = url.to_ascii_lowercase();
        let web = lower.starts_with("https://") || lower.starts_with("http://");
        if !url.is_empty() && !web {
            let shown: String = url.chars().take(80).collect();
            return Err(CmdError::Params(format!("`{shown}` isn't a web address (https://… or http://…)")));
        }
        self.prefs.llm_url = url.to_string();
        self.prefs.llm_model = model.trim().chars().take(200).collect();
        if let Some(k) = key {
            self.jobs.push(Job::SetLlmKey { key: quatrelle_model::Secret(k.trim().to_string()) });
        }
        if url.is_empty() {
            self.assistant = None;
        }
        Ok(serde_json::json!({ "url": self.prefs.llm_url, "model": self.prefs.llm_model }))
    }
}
