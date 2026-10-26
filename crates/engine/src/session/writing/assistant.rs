//! The session: the language model set in Options (an OpenAI-compatible server), asked about the
//! message being read; its answer is shown above it.

use super::*;

/// The most of a message's text sent to the model: its start, where mail says what it wants.
const MAX_TEXT: usize = 24_000;

/// How the model is asked for a summary.
const SUMMARY_PROMPT: &str = "You summarize an email for the person who received it. Write in the language the email is written in. \
Give the gist in at most five short bullet points starting with \"- \": who wants what, by when, and what is expected of the reader. \
No preamble, and nothing that isn't in the email.";

/// How the model is asked for a conversation's summary.
const THREAD_PROMPT: &str = "You summarize an email conversation for one of the people in it. Write in the language the emails are written in. \
The messages come oldest first. Give, in at most seven short bullet points starting with \"- \": what it's about, what was decided or answered, \
and what is still open or expected of whom, by when. No preamble, and nothing that isn't in the emails.";

/// The most of a conversation's messages sent, the newest kept, and the most of each one's text.
const THREAD_MAX: usize = 15;
const MESSAGE_MAX: usize = 4_000;

/// What a message adds to its conversation: its text up to where it quotes the one before
/// (`>` lines, or the header block mail programs put above a quoted message).
fn own_part(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let l = line.trim_start();
        let quoting = l.starts_with('>')
            || l.starts_with("-----Original Message-----")
            || l.starts_with("-----Message d'origine-----")
            || ((l.starts_with("From: ") || l.starts_with("De : ") || l.starts_with("De: ")) && !out.trim().is_empty());
        if quoting {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().chars().take(MESSAGE_MAX).collect()
}

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

/// How the model is asked where a message belongs; the choices follow.
const FILING_PROMPT: &str = "You file an email. Choose the one folder from the list below where it belongs, and at most one category \
from the list below that fits it. Answer with JSON only, as {\"folder\": \"…\", \"category\": \"…\"}, each name written exactly \
as listed, and \"\" when none fits.";

/// The most folders offered to the model.
const FOLDERS_MAX: usize = 300;

/// What the language model is asked about a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistKind {
    Summary,
    Reply,
    /// Where it belongs: a folder and a category of the account's.
    Filing,
}

/// Where the model says a message belongs, among the account's own: a folder (its path, and
/// its name as shown) and a category.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filing {
    pub folder: Option<(String, String)>,
    pub category: Option<String>,
}

/// What the language model was asked, and its answer once it came.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assistant {
    pub id: u64,
    /// The message asked about.
    pub key: MessageKey,
    pub kind: AssistKind,
    /// For [`AssistKind::Filing`], the answer read: what it names among the account's own.
    pub filing: Option<Filing>,
    /// `None` while it's working; the text, or why there's none.
    pub answer: Option<Result<String, String>>,
}

/// The models the language model's server offers, to pick from in Options (one can still be
/// typed): asked for when its address or key changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LlmModels {
    /// The server asked.
    pub url: String,
    /// Still being asked.
    pub asking: bool,
    pub names: Vec<String>,
    /// Why the last answer gave none.
    pub error: Option<String>,
}

impl Session {
    /// Summarize the message being read with the language model set in Options; the answer
    /// comes in as [`Session::assistant`].
    pub fn summarize(&mut self) -> CmdResult {
        self.ask_about(AssistKind::Summary)
    }

    /// Ask the language model where the message being read belongs: one of the account's
    /// folders and categories, to move it there and tag it with a click.
    pub fn suggest_filing(&mut self) -> CmdResult {
        self.ask_about(AssistKind::Filing)
    }

    /// The folders a message can be filed in, as shown (the path's names, `/` between them) with
    /// their paths: the account's own, not Drafts, Sent Items, Deleted Items nor the Outbox.
    fn filing_folders(&self, account: &str) -> Vec<(String, String)> {
        let Some(a) = self.account(account) else { return Vec::new() };
        a.folders
            .iter()
            .filter(|f| f.selectable && f.mailbox.is_none() && !matches!(f.role, FolderRole::Drafts | FolderRole::Sent | FolderRole::Trash))
            .filter(|f| f.path != OUTBOX)
            .take(FOLDERS_MAX)
            .map(|f| {
                let decoded = quatrelle_model::utf7::decode(&f.path);
                let mut names: Vec<String> = match f.delimiter {
                    Some(d) => decoded.split(d).map(str::to_string).collect(),
                    None => vec![decoded],
                };
                if let Some(last) = names.last_mut() {
                    *last = f.display_name();
                }
                (names.join("/"), f.path.clone())
            })
            .collect()
    }

    /// What a filing answer names, among the account's folders and categories (a folder may be
    /// named by its last name alone when no other has it).
    fn read_filing(&self, account: &str, text: &str) -> Filing {
        let json = match (text.find('{'), text.rfind('}')) {
            (Some(a), Some(b)) if a < b => text.get(a..=b).unwrap_or_default(),
            _ => "",
        };
        let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
        let said = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::trim).filter(|x| !x.is_empty()).map(str::to_lowercase);
        let folders = self.filing_folders(account);
        let folder = said("folder").and_then(|want| {
            let exact = folders.iter().find(|(shown, _)| shown.to_lowercase() == want);
            let leaf: Vec<&(String, String)> =
                folders.iter().filter(|(shown, _)| shown.rsplit('/').next().is_some_and(|l| l.to_lowercase() == want)).collect();
            exact.or(if leaf.len() == 1 { leaf.first().copied() } else { None }).map(|(shown, path)| (path.clone(), shown.clone()))
        });
        let category = said("category").and_then(|want| self.prefs.categories.iter().find(|c| c.name.to_lowercase() == want).map(|c| c.name.clone()));
        Filing { folder, category }
    }

    /// Ask the language model for a reply to the message being read, to send as it is or change.
    pub fn suggest_reply(&mut self) -> CmdResult {
        self.ask_about(AssistKind::Reply)
    }

    /// Reply to the message being read with the suggested reply at the top, above the signature
    /// and the message quoted.
    pub fn reply_with_suggestion(&mut self) -> CmdResult {
        let reading = self.reading.as_ref().map(|r| r.key.clone());
        let text = match &self.assistant {
            Some(Assistant { kind: AssistKind::Reply, answer: Some(Ok(text)), key, .. }) if Some(key) == reading.as_ref() => text.clone(),
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

    fn ask_about(&mut self, kind: AssistKind) -> CmdResult {
        let url = self.prefs.llm_url.trim().to_string();
        if url.is_empty() {
            return Err(CmdError::Disabled("set a language model in File › Options › Assistant first".into()));
        }
        let r = self.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        let p = r.parsed.as_ref().ok_or_else(|| CmdError::Disabled("the message isn't downloaded yet".into()))?;
        let who = |p: &quatrelle_mime::Parsed| {
            if p.from.name.trim().is_empty() { p.from.email.clone() } else { format!("{} <{}>", p.from.name.trim(), p.from.email) }
        };
        let from = who(p);
        let text: String = p.text.chars().take(MAX_TEXT).collect();
        let mut mail = format!("From: {from}\nSubject: {}\n\n{text}", p.subject);
        let key = r.key.clone();
        // A summary of a message in a conversation is the conversation's: each message's own
        // part, oldest first (those downloaded).
        let mut thread = false;
        let others = self.conversation_of(&key);
        if kind == AssistKind::Summary && !others.is_empty() {
            let mut all: Vec<(i64, String, String)> = others
                .iter()
                .rev()
                .take(THREAD_MAX)
                .filter_map(|k| self.store.as_ref()?.read_body(k))
                .filter_map(|raw| quatrelle_mime::parse(&raw))
                .map(|q| (q.date, who(&q), own_part(&q.text)))
                .collect();
            if !all.is_empty() {
                all.push((p.date, from.clone(), own_part(&p.text)));
                all.sort_by_key(|(date, _, _)| *date);
                let mut joined = format!("Subject: {}\n", p.subject);
                for (date, from, text) in &all {
                    joined.push_str(&format!("\n--- {} — {from}\n{text}\n", quatrelle_model::dates::full_time(*date, &self.clock)));
                }
                // Over the limit: the newest kept.
                let n = joined.chars().count();
                mail = joined.chars().skip(n.saturating_sub(MAX_TEXT)).collect();
                thread = true;
            }
        }
        let prompt = match kind {
            AssistKind::Reply => {
                let me =
                    self.account(&key.account).map(|a| if a.config.name.trim().is_empty() { a.config.email.clone() } else { a.config.name.clone() });
                REPLY_PROMPT.replace("{me}", me.as_deref().unwrap_or("the reader"))
            }
            AssistKind::Filing => {
                let folders = self.filing_folders(&key.account);
                if folders.is_empty() {
                    return Err(CmdError::Disabled("this account has no folders to file it in".into()));
                }
                let mut p = format!("{FILING_PROMPT}\n\nFolders:\n");
                for (shown, _) in &folders {
                    p.push_str(&format!("- {shown}\n"));
                }
                p.push_str("\nCategories:\n");
                for c in self.prefs.categories.iter().take(FOLDERS_MAX) {
                    p.push_str(&format!("- {}\n", c.name));
                }
                p
            }
            AssistKind::Summary if thread => THREAD_PROMPT.to_string(),
            AssistKind::Summary => SUMMARY_PROMPT.to_string(),
        };
        let id = self.fresh_id();
        self.assistant = Some(Assistant { id, key, kind, filing: None, answer: None });
        self.jobs.push(Job::AskLlm {
            id,
            url,
            model: self.prefs.llm_model.trim().to_string(),
            messages: vec![("system".into(), prompt), ("user".into(), mail)],
        });
        Ok(serde_json::json!({ "asked": id }))
    }

    /// The other messages of a message's conversation in the list, oldest first, whether the
    /// list shows conversations or not.
    fn conversation_of(&self, key: &MessageKey) -> Vec<MessageKey> {
        let Some(i) = self.list.iter().position(|m| &m.key == key) else { return Vec::new() };
        let mut keys: Vec<(i64, MessageKey)> = threads(&self.list)
            .iter()
            .find(|t| t.messages.contains(&i))
            .map(|t| t.messages.iter().filter(|&&j| j != i).filter_map(|&j| self.list.get(j).map(|m| (m.date, m.key.clone()))).collect())
            .unwrap_or_default();
        keys.sort_by_key(|(date, _)| *date);
        keys.into_iter().map(|(_, k)| k).collect()
    }

    /// The language model's answer, for the question still on view (about the message being
    /// read, or the text of one being written).
    pub(in crate::session) fn llm_answered(&mut self, id: u64, text: String, error: Option<String>) {
        let answer = match error {
            Some(e) => Err(e),
            None => Ok(text),
        };
        if let Some(a) = self.assistant.as_ref().filter(|a| a.id == id) {
            let filing = match (&answer, a.kind) {
                (Ok(text), AssistKind::Filing) => Some(self.read_filing(&a.key.account, text)),
                _ => None,
            };
            if let Some(a) = self.assistant.as_mut() {
                a.answer = Some(answer);
                a.filing = filing;
            }
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
        let moved = url != self.prefs.llm_url;
        self.prefs.llm_url = url.to_string();
        self.prefs.llm_model = model.trim().chars().take(200).collect();
        if let Some(k) = key {
            self.jobs.push(Job::SetLlmKey { key: quatrelle_model::Secret(k.trim().to_string()) });
        }
        if url.is_empty() {
            self.assistant = None;
            self.llm_models = None;
        } else if moved || key.is_some() {
            // Another server, or the key it may have wanted: its models again (after the key's
            // kept, the jobs going in order).
            self.list_llm_models()?;
        }
        Ok(serde_json::json!({ "url": self.prefs.llm_url, "model": self.prefs.llm_model }))
    }

    /// Ask the language model's server which models it offers; they come in as
    /// [`Session::llm_models`].
    pub fn list_llm_models(&mut self) -> CmdResult {
        let url = self.prefs.llm_url.trim().to_string();
        if url.is_empty() {
            return Err(CmdError::Disabled("set the language model's server first".into()));
        }
        let names = self.llm_models.as_ref().filter(|m| m.url == url).map(|m| m.names.clone()).unwrap_or_default();
        self.llm_models = Some(LlmModels { url: url.clone(), asking: true, names, error: None });
        self.jobs.push(Job::ListLlmModels { url });
        Ok(serde_json::json!({ "asking": true }))
    }

    /// The server's models, for the server still set.
    pub(in crate::session) fn llm_models_answered(&mut self, url: String, names: Vec<String>, error: Option<String>) {
        if let Some(m) = self.llm_models.as_mut().filter(|m| m.url == url) {
            m.asking = false;
            if error.is_none() {
                m.names = names;
            }
            m.error = error;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_adds_what_it_says_not_what_it_quotes() {
        assert_eq!(own_part("Works for me.\n\n> Could you bring the frames?\n> Zoe"), "Works for me.");
        assert_eq!(own_part("OK pour jeudi.\n\nDe : Zoe Park\nEnvoyé : lundi\n\nBonjour"), "OK pour jeudi.");
        assert_eq!(own_part("From: the team\nWe ship Friday."), "From: the team\nWe ship Friday.", "a first line isn't a quote");
    }
}
