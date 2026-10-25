//! The session: the language model set in Options (an OpenAI-compatible server), asked about the
//! message being read; its answer is shown above it.

use super::*;

/// The most of a message's text sent to the model: its start, where mail says what it wants.
const MAX_TEXT: usize = 24_000;

/// How the model is asked for a summary.
const SUMMARY_PROMPT: &str = "You summarize an email for the person who received it. Write in the language the email is written in. \
Give the gist in at most five short bullet points starting with \"- \": who wants what, by when, and what is expected of the reader. \
No preamble, and nothing that isn't in the email.";

/// How the model is asked for a reply; `{me}` is who it writes for.
const REPLY_PROMPT: &str = "You write a reply to an email on behalf of {me}, the person who received it. \
Write in the language the email is written in, in the same register (formal or informal). Answer what it asks, briefly and politely. \
Begin with a greeting and end with a closing formula, but write no signature: it is added after. \
Where you don't know a fact or a decision, put a placeholder in [square brackets]. Write only the reply's text.";

/// What the language model was asked, and its answer once it came.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assistant {
    pub id: u64,
    /// The message asked about.
    pub key: MessageKey,
    /// Asked for a reply to it (else a summary).
    pub reply: bool,
    /// `None` while it's working; the text, or why there's none.
    pub answer: Option<Result<String, String>>,
}

impl Session {
    /// Summarize the message being read with the language model set in Options; the answer
    /// comes in as [`Session::assistant`].
    pub fn summarize(&mut self) -> CmdResult {
        self.ask_about(false)
    }

    /// Ask the language model for a reply to the message being read, to send as it is or change.
    pub fn suggest_reply(&mut self) -> CmdResult {
        self.ask_about(true)
    }

    /// Reply to the message being read with the suggested reply at the top, above the signature
    /// and the message quoted.
    pub fn reply_with_suggestion(&mut self) -> CmdResult {
        let reading = self.reading.as_ref().map(|r| r.key.clone());
        let text = match &self.assistant {
            Some(Assistant { reply: true, answer: Some(Ok(text)), key, .. }) if Some(key) == reading.as_ref() => text.clone(),
            _ => return Err(CmdError::Disabled("ask for a suggested reply first".into())),
        };
        let out = self.reply_compose(ReplyMode::Reply)?;
        if let Some(c) = self.compose.as_mut() {
            let add = format!("{}\n", text.trim());
            let add_styles = quatrelle_mime::rich::Styles::plain(&add);
            quatrelle_mime::rich::splice(&mut c.body, &mut c.styles, 0..0, &add, &add_styles);
        }
        Ok(out)
    }

    fn ask_about(&mut self, reply: bool) -> CmdResult {
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
        let prompt = if reply {
            let me = self.account(&key.account).map(|a| if a.config.name.trim().is_empty() { a.config.email.clone() } else { a.config.name.clone() });
            REPLY_PROMPT.replace("{me}", me.as_deref().unwrap_or("the reader"))
        } else {
            SUMMARY_PROMPT.to_string()
        };
        let id = self.fresh_id();
        self.assistant = Some(Assistant { id, key, reply, answer: None });
        self.jobs.push(Job::AskLlm {
            id,
            url,
            model: self.prefs.llm_model.trim().to_string(),
            messages: vec![("system".into(), prompt), ("user".into(), mail)],
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
