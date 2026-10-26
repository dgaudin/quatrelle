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

/// How the model is asked to rewrite the text written, by the choice offered.
fn rewrite_prompt(how: &str) -> Option<&'static str> {
    Some(match how {
        "fix" => "Correct the spelling, grammar and punctuation of this text, changing nothing else, in its own language.",
        "shorter" => "Make this text shorter and clearer, keeping everything it says, in its own language.",
        "formal" => "Rewrite this text in a more formal, professional tone, in its own language.",
        "friendly" => "Rewrite this text in a warmer, friendlier tone, in its own language.",
        "english" => "Translate this text into English.",
        _ => return None,
    })
}

/// The language model's rewrite of the text written in a message, once it came.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rewrite {
    pub id: u64,
    /// `None` while it's working; the new text, or why there's none.
    pub answer: Option<Result<String, String>>,
}

/// Where the text the user wrote ends in a message being written: before its signature, or for
/// a reply or a forward without one, before the message quoted (its `From:` line).
pub fn own_text_end(c: &Compose) -> usize {
    let body = c.body.as_str();
    let sig = Some(c.signature.trim()).filter(|s| !s.is_empty()).and_then(|s| body.find(s));
    let quote = c.mode.and_then(|_| body.find("\nFrom: ").map(|i| i + 1));
    let end = match (sig, quote) {
        (Some(a), Some(b)) => a.min(b),
        (a, b) => a.or(b).unwrap_or(body.len()),
    };
    // Up to its last word: the blank lines before the signature stay.
    let end = body.get(..end).map_or(end, |t| t.trim_end().len());
    body.get(..end).map_or(0, |t| t.chars().count())
}

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

    /// The language model's answer, for the question still on view (about the message being
    /// read, or the text of one being written).
    pub(in crate::session) fn llm_answered(&mut self, id: u64, text: String, error: Option<String>) {
        let answer = match error {
            Some(e) => Err(e),
            None => Ok(text),
        };
        if let Some(a) = self.assistant.as_mut().filter(|a| a.id == id) {
            a.answer = Some(answer);
        } else if let Some(r) = self.compose.iter_mut().chain(self.composes.iter_mut()).filter_map(|c| c.rewrite.as_mut()).find(|r| r.id == id) {
            r.answer = Some(answer);
        }
    }

    /// Ask the language model to rewrite the text written in the message being written (up to
    /// its signature or the message quoted): `how` is `fix`, `shorter`, `formal`, `friendly` or
    /// `english`. The new text is offered beside it, to put in place or not.
    pub fn rewrite(&mut self, how: &str) -> CmdResult {
        let url = self.prefs.llm_url.trim().to_string();
        if url.is_empty() {
            return Err(CmdError::Disabled("set a language model in File › Options › Assistant first".into()));
        }
        let prompt = rewrite_prompt(how).ok_or_else(|| CmdError::Params(format!("no rewrite `{how}`")))?;
        let c = self.compose.as_ref().ok_or_else(|| CmdError::Disabled("write a message first".into()))?;
        let text: String = c.body.chars().take(own_text_end(c)).collect();
        if text.trim().is_empty() {
            return Err(CmdError::Params("write something first".into()));
        }
        let prompt =
            format!("{prompt} It is part of an email: keep its greeting and closing if it has them. Write only the new text, with no comment.");
        let id = self.fresh_id();
        if let Some(c) = self.compose.as_mut() {
            c.rewrite = Some(Rewrite { id, answer: None });
        }
        self.jobs.push(Job::AskLlm {
            id,
            url,
            model: self.prefs.llm_model.trim().to_string(),
            messages: vec![("system".into(), prompt), ("user".into(), text)],
        });
        Ok(serde_json::json!({ "asked": id }))
    }

    /// Put the rewritten text in place of the text written (`use`), or let it go.
    pub fn end_rewrite(&mut self, use_it: bool) -> CmdResult {
        let c = self.compose.as_mut().ok_or_else(|| CmdError::Disabled("write a message first".into()))?;
        let text = match c.rewrite.take() {
            Some(Rewrite { answer: Some(Ok(text)), .. }) => text,
            _ if !use_it => return Ok(serde_json::json!({ "dropped": true })),
            _ => return Err(CmdError::Disabled("the rewritten text hasn't come yet".into())),
        };
        if !use_it {
            return Ok(serde_json::json!({ "dropped": true }));
        }
        let end = own_text_end(c);
        let add = text.trim().to_string();
        let add_styles = quatrelle_mime::rich::Styles::plain(&add);
        quatrelle_mime::rich::splice(&mut c.body, &mut c.styles, 0..end, &add, &add_styles);
        Ok(serde_json::json!({ "replaced": end }))
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
