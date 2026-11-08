// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The scam check: external mail that arrives in an Exchange account's Inbox is read by the
//! assistant's language model (an option in File › Options › Assistant, off until turned on), and
//! what it finds is kept as a verdict by Message-ID (`scam.json`).
//!
//! Only what could be a scam is sent: an unknown sender's mail, or a known sender's when the message
//! shows a local sign of trouble (`lookalike::signs`). Mail already there is never sent (only what
//! `note_new_mail` takes for new, within a day), nor internal mail, nor a message already judged.
//! One question to the model at a time, a daily cap, a short queue. The model only ever adds a
//! warning; its answer is hostile input (`quatrelle_model::scam::read_answer`).

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use quatrelle_model::lookalike::signs::{self, Sign};
use quatrelle_model::lookalike::{self};
use quatrelle_model::scam::{Facts, ScamDoc, Verdict, WARN_SCORE, read_answer, request};

use super::*;

mod trial;
pub use trial::{Miss as ScamMiss, ScamTest};

/// An account's document of verdicts and trusted senders.
pub const SCAM_DOC: &str = "scam.json";
/// The most messages waiting to be checked: past it, the oldest go.
pub const QUEUE_MAX: usize = 50;
/// How long the model has to answer, in seconds.
pub const ANSWER_SECS: i64 = 60;
/// The daily cap's default and its limits.
pub const DEFAULT_CAP: u32 = 200;
pub const MAX_CAP: u32 = 2000;
/// How many judged messages are remembered for this run (so one that failed is not asked again).
const ASKED_MAX: usize = 50_000;
/// The biggest source held in memory for a message waiting its turn.
const SOURCE_MAX: usize = 256 << 10;

/// A message waiting to be checked.
#[derive(Clone, Debug)]
struct Pending {
    key: MessageKey,
    message_id: String,
}

/// What the scam check did today, for Options ([`Session::scam_today`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScamToday {
    /// Messages the model judged.
    pub judged: usize,
    /// Of them, those it warns about.
    pub warned: usize,
}

/// The question on its way to the model.
#[derive(Clone, Debug)]
struct Inflight {
    id: u64,
    pending: Pending,
    since: i64,
}

/// The session's scam check: each account's document (read when first needed), what waits, and the
/// day's count.
#[derive(Debug, Default)]
pub(crate) struct ScamState {
    docs: HashMap<String, ScamDoc>,
    queue: VecDeque<Pending>,
    /// Messages whose source was asked for, to be checked when it comes.
    fetching: HashSet<MessageKey>,
    /// Sources that came for them (a source over [`SOURCE_MAX`] isn't held: the cache has it, and
    /// fifty waiting messages hold 12.5 MB at most).
    sources: HashMap<MessageKey, Vec<u8>>,
    /// Messages whose source was asked for once already: one that still isn't here after is
    /// given up on, never asked for again (no loop of downloads).
    fetched: HashSet<MessageKey>,
    inflight: Option<Inflight>,
    /// "Try it on examples": the run in progress or the last one's result.
    trial: Option<trial::Trial>,
    /// `account` and Message-ID of every message asked this run, answered or not.
    asked: HashSet<(String, String)>,
    /// The day (UTC, days since 1970) the count is for, and how many questions were asked in it.
    day: i64,
    count: u32,
}

impl Session {
    /// Whether the account is an Exchange one (EWS or Microsoft 365): the only kind checked.
    pub fn is_exchange(&self, account: &str) -> bool {
        self.account(account).is_some_and(|a| a.config.ews.is_some() || a.config.graph.is_some())
    }

    /// An account's verdicts and trusted senders, read from `scam.json` the first time (a file
    /// that can't be read gives an empty document and a line in the log, never an error).
    fn scam_doc(&mut self, account: &str) -> &mut ScamDoc {
        if !self.scam.docs.contains_key(account) {
            let doc = match self.store.as_ref().map(|s| s.load_account_doc::<ScamDoc>(account, SCAM_DOC)) {
                Some(Ok(doc)) => doc,
                Some(Err(e)) => {
                    log::warn!("{account}: the scam check's verdicts couldn't be read, starting empty: {e}");
                    ScamDoc::default()
                }
                None => ScamDoc::default(),
            };
            self.scam.docs.insert(account.to_string(), doc);
        }
        self.scam.docs.entry(account.to_string()).or_default()
    }

    fn save_scam(&mut self, account: &str) {
        let (Some(store), Some(doc)) = (&self.store, self.scam.docs.get(account)) else { return };
        if let Err(e) = store.save_account_doc(account, SCAM_DOC, doc) {
            log::warn!("{account}: the scam check's verdicts couldn't be kept: {e}");
        }
    }

    /// The verdict kept for a message, if any: `(score, reasons)`.
    pub fn scam_verdict(&mut self, key: &MessageKey) -> Option<(u8, Vec<String>)> {
        let id = self.summary(key)?.message_id.clone();
        if id.is_empty() {
            return None;
        }
        self.scam_doc(&key.account).verdicts.get(&id).map(|v| (v.score, v.reasons.clone()))
    }

    /// Read every account's document now (the window does it once a frame, cheap after the first), so
    /// that what it draws can look verdicts up without loading anything.
    pub fn scam_warm(&mut self) {
        let ids: Vec<String> = self.accounts.iter().map(|a| a.config.id.clone()).collect();
        for id in ids {
            if !self.scam.docs.contains_key(&id) {
                self.scam_doc(&id);
            }
        }
    }

    /// What the check did today (the same UTC day as the daily cap): the messages judged, and how
    /// many of them warn. Counted from the verdicts kept, so it holds across a restart; read once
    /// the documents are ([`Session::scam_warm`]).
    pub fn scam_today(&self) -> ScamToday {
        let start = self.clock.now.div_euclid(86_400).saturating_mul(86_400);
        let mut today = ScamToday::default();
        for v in self.scam.docs.values().flat_map(|d| d.verdicts.values()).filter(|v| v.at >= start) {
            today.judged += 1;
            if v.score >= quatrelle_model::scam::WARN_SCORE {
                today.warned += 1;
            }
        }
        today
    }

    /// The verdict a message carries when it is a warning (a score of [`WARN_SCORE`] or more) and its
    /// account's document is read ([`Session::scam_warm`]); `message_id` stands in for a message
    /// that isn't listed.
    pub fn scam_warning(&self, key: &MessageKey, message_id: &str) -> Option<(u8, &[String])> {
        let listed = self.summary(key).map(|m| m.message_id.as_str()).filter(|id| !id.is_empty());
        let v = self.scam.docs.get(&key.account)?.verdicts.get(listed.unwrap_or(message_id))?;
        (v.score >= WARN_SCORE).then_some((v.score, v.reasons.as_slice()))
    }

    /// The score of a listed message's warning, for the list's mark.
    pub fn scam_score(&self, m: &Summary) -> Option<u8> {
        if m.message_id.is_empty() {
            return None;
        }
        let v = self.scam.docs.get(&m.key.account)?.verdicts.get(&m.message_id)?;
        (v.score >= WARN_SCORE).then_some(v.score)
    }

    /// Every address taken as not a scam, in any account's document read so far, sorted.
    pub fn scam_trusted_all(&self) -> Vec<String> {
        let all: BTreeSet<&String> = self.scam.docs.values().flat_map(|d| d.trusted.iter()).collect();
        all.into_iter().cloned().collect()
    }

    /// The addresses an account's user said are not scams, sorted.
    pub fn scam_trusted(&mut self, account: &str) -> Vec<String> {
        self.scam_doc(account).trusted.iter().cloned().collect()
    }

    /// "Not a scam": the message's verdict goes and its sender is trusted (never sent to the model again).
    pub fn not_a_scam(&mut self, key: &MessageKey) -> CmdResult {
        let (id, from) = {
            let m = self.summary(key).ok_or_else(|| CmdError::Params(format!("`{}` is not in the list", key.to_id())))?;
            (m.message_id.clone(), m.from.email.trim().to_string())
        };
        let doc = self.scam_doc(&key.account);
        doc.verdicts.remove(&id);
        let trusted = !from.is_empty() && doc.trust(&from);
        self.save_scam(&key.account);
        Ok(serde_json::json!({ "trusted": trusted, "address": from.to_lowercase() }))
    }

    /// Take a sender back out of the trusted ones, in every account.
    pub fn scam_untrust(&mut self, address: &str) -> CmdResult {
        let address = address.trim().to_lowercase();
        let ids: Vec<String> = self.accounts.iter().map(|a| a.config.id.clone()).collect();
        let mut removed = false;
        for id in ids {
            if self.scam_doc(&id).trusted.remove(&address) {
                removed = true;
                self.save_scam(&id);
            }
        }
        Ok(serde_json::json!({ "removed": removed }))
    }

    /// The option: `enabled`, the accounts it applies to (Exchange ones) and the daily cap.
    pub fn set_scam_check(&mut self, enabled: Option<bool>, accounts: Option<Vec<String>>, cap: Option<i64>) -> CmdResult {
        if enabled == Some(true) && (self.prefs.llm_url.trim().is_empty() || self.prefs.llm_model.trim().is_empty()) {
            return Err(CmdError::Disabled("set a language model's server and model in File › Options › Assistant first".into()));
        }
        if let Some(list) = &accounts {
            for a in list {
                if self.account(a).is_none() {
                    return Err(CmdError::Params(format!("no account `{a}`")));
                }
                if !self.is_exchange(a) {
                    return Err(CmdError::Params(format!("`{a}` isn't an Exchange account: the scam check is for those only")));
                }
            }
        }
        let cap = match cap {
            Some(c) if (1..=i64::from(MAX_CAP)).contains(&c) => Some(c as u32),
            Some(_) => return Err(CmdError::Params(format!("`dailyCap` is 1 to {MAX_CAP}"))),
            None => None,
        };
        if let Some(on) = enabled {
            self.prefs.scam_check = on;
        }
        if let Some(list) = accounts {
            self.prefs.scam_accounts = list.into_iter().collect();
        }
        if let Some(c) = cap {
            self.prefs.scam_daily_cap = c;
        }
        // Turned on with nothing chosen: every Exchange account.
        if enabled == Some(true) && self.prefs.scam_accounts.is_empty() {
            let all: Vec<String> = self.accounts.iter().map(|a| a.config.id.clone()).filter(|id| self.is_exchange(id)).collect();
            self.prefs.scam_accounts = all.into_iter().collect();
        }
        Ok(serde_json::json!({ "enabled": self.prefs.scam_check, "accounts": self.prefs.scam_accounts, "dailyCap": self.prefs.scam_daily_cap }))
    }

    /// Whether `email` is outside the organisation: the organisation as the sender check has it
    /// (the domains listed in Options, else the accounts'), **whatever the External tag says**.
    /// With no organisation to tell by, no one is: nothing is sent rather than everything.
    pub(crate) fn is_outside(&mut self, email: &str) -> bool {
        let org = self.org_domains();
        if org.is_empty() || email.trim().is_empty() {
            return false;
        }
        self.senders.as_ref().is_some_and(|s| !s.check.is_own(email) && lookalike::is_external(email, &org))
    }

    /// Mail that just arrived in an account's Inbox (what blocked senders and ignored conversations
    /// didn't take, and `note_new_mail` took for new): the ones worth the model's reading are queued.
    pub(crate) fn scam_arrived(&mut self, account: &str, arrived: &[Summary]) {
        if !self.prefs.scam_check
            || arrived.is_empty()
            || !self.prefs.scam_accounts.contains(account)
            || !self.is_exchange(account)
            || self.prefs.llm_url.trim().is_empty()
            || self.prefs.llm_model.trim().is_empty()
        {
            return;
        }
        // Oldest first (the folder lists the newest first), so the queue's oldest go first.
        let mut batch: Vec<&Summary> = arrived.iter().collect();
        batch.sort_by_key(|m| (m.received_at(), m.key.uid));
        for m in batch {
            let email = m.from.email.trim().to_string();
            if m.message_id.is_empty()
                || !self.is_outside(&email)
                || self.sender_safe(&email)
                || self.scam_doc(account).trusts(&email)
                || self.scam_doc(account).verdicts.contains_key(&m.message_id)
            {
                continue;
            }
            self.scam.queue.push_back(Pending { key: m.key.clone(), message_id: m.message_id.clone() });
        }
        // The newest are kept.
        while self.scam.queue.len() > QUEUE_MAX {
            if let Some(dropped) = self.scam.queue.pop_front() {
                self.scam.fetching.remove(&dropped.key);
                self.scam.sources.remove(&dropped.key);
            }
        }
        self.scam_pump();
    }

    /// A message's source came (or failed to): what waited for it can go on.
    pub(crate) fn scam_body(&mut self, key: &MessageKey, raw: Option<&[u8]>) {
        if !self.scam.fetching.remove(key) {
            return;
        }
        match raw {
            Some(raw) if raw.len() <= SOURCE_MAX => {
                self.scam.sources.insert(key.clone(), raw.to_vec());
            }
            Some(_) => {}
            None => self.scam.queue.retain(|p| &p.key != key),
        }
        self.scam_pump();
    }

    /// Whether the language model's answer `id` was the scam check's: kept (or logged) if so.
    pub(crate) fn scam_answered(&mut self, id: u64, text: &str, error: Option<&str>) -> bool {
        if self.trial_answered(id, text, error) {
            return true;
        }
        let Some(sent) = self.scam.inflight.take_if(|f| f.id == id) else { return false };
        match (error, read_answer(text)) {
            (Some(e), _) => log::info!("scam check: the model's server failed: {e}"),
            (None, None) => log::info!("scam check: the model's answer had no verdict"),
            (None, Some((score, reasons))) => {
                let v = Verdict { score, reasons, model: self.prefs.llm_model.trim().to_string(), at: self.clock.now };
                self.scam_doc(&sent.pending.key.account).keep(&sent.pending.message_id, v);
                self.save_scam(&sent.pending.key.account);
            }
        }
        self.scam_pump();
        true
    }

    /// An answer that didn't come in time is given up on (called with every tick).
    pub(crate) fn scam_tick(&mut self) {
        self.trial_tick();
        if self.scam.inflight.as_ref().is_some_and(|f| self.clock.now - f.since > ANSWER_SECS) {
            log::info!("scam check: no answer from the model within {ANSWER_SECS} s, skipped");
            self.scam.inflight = None;
        }
        self.scam_pump();
    }

    /// Ask for the next message that can be checked, when no question is on its way.
    fn scam_pump(&mut self) {
        if self.scam.inflight.is_some() {
            return;
        }
        while let Some(at) = self.scam.queue.iter().position(|p| !self.scam.fetching.contains(&p.key)) {
            let Some(p) = self.scam.queue.remove(at) else { return };
            if self.scam_ask(&p) {
                return;
            }
        }
        // What is left waits for its source: asked for now.
        let missing: Vec<MessageKey> = self.scam.queue.iter().map(|p| p.key.clone()).filter(|k| !self.scam.fetching.contains(k)).collect();
        for key in missing {
            if self.scam.fetched.len() >= ASKED_MAX {
                self.scam.fetched.clear();
            }
            self.scam.fetched.insert(key.clone());
            self.scam.fetching.insert(key.clone());
            self.jobs.push(Job::FetchBody { key });
        }
    }

    /// Start checking one message; `false` when it can't be (not here yet: asked for, or nothing
    /// to send) and the next may be tried. A message whose source isn't kept goes back in the queue
    /// until its source comes.
    fn scam_ask(&mut self, p: &Pending) -> bool {
        let held = self.scam.sources.remove(&p.key);
        let Some(raw) = held.or_else(|| self.store.as_ref().and_then(|s| s.read_body(&p.key))) else {
            if !self.scam.fetched.insert(p.key.clone()) {
                log::info!("scam check: a message's source never came, skipped");
                return false;
            }
            self.scam.fetching.insert(p.key.clone());
            self.jobs.push(Job::FetchBody { key: p.key.clone() });
            self.scam.queue.push_front(p.clone());
            return false;
        };
        let account = p.key.account.clone();
        let asked_key = (account.clone(), p.message_id.clone());
        if self.scam.asked.contains(&asked_key) || self.scam_doc(&account).verdicts.contains_key(&p.message_id) {
            return false;
        }
        let Some(parsed) = quatrelle_mime::parse(&raw) else { return false };
        let email = parsed.from.email.trim().to_string();
        if !self.prefs.scam_check || self.sender_safe(&email) || self.scam_doc(&account).trusts(&email) || !self.is_outside(&email) {
            return false;
        }
        self.keep_senders();
        let known = self.senders.as_ref().is_some_and(|s| s.check.knows(&email));
        let mut found: Vec<Sign> = Vec::new();
        if let Some(key) = Some(&p.key)
            && self.warning_now(key, &parsed).is_some()
        {
            found.push(Sign::SenderWarning);
        }
        let html = quatrelle_mime::html_body(&raw).unwrap_or_default();
        let pairs = quatrelle_mime::html_link_pairs(&html);
        let names: Vec<String> = parsed.attachments.iter().filter(|a| !a.inline).map(|a| a.name.clone()).collect();
        found.extend(signs::misleading_links(&pairs));
        found.extend(signs::risky_attachments(&names));
        found.extend(signs::auth_failures(&header_text(&raw)));
        if known && found.is_empty() {
            return false;
        }
        let today = self.clock.now.div_euclid(86_400);
        if self.scam.day != today {
            self.scam.day = today;
            self.scam.count = 0;
        }
        if self.scam.count >= self.prefs.scam_daily_cap {
            log::info!("scam check: the day's cap of {} is reached, a message isn't checked", self.prefs.scam_daily_cap);
            return false;
        }
        let mut targets: Vec<String> = quatrelle_mime::html_links(&html);
        targets.extend(pairs.iter().map(|(_, t)| t.clone()));
        let reply_to = parsed.reply_to.first().map(|r| r.email.trim().to_string()).filter(|r| !r.is_empty() && !r.eq_ignore_ascii_case(&email));
        let lang = self.answer_language();
        let facts = Facts {
            from_name: &parsed.from.name,
            from: &email,
            reply_to: reply_to.as_deref(),
            subject: &parsed.subject,
            date: parsed.date,
            known,
            signs: &found,
            link_targets: &targets,
            attachment_names: &names,
            text: &parsed.text,
        };
        let messages = request(&facts, &lang);
        if self.scam.asked.len() >= ASKED_MAX {
            self.scam.asked.clear();
        }
        self.scam.asked.insert(asked_key);
        self.scam.count += 1;
        let id = self.fresh_id();
        self.scam.inflight = Some(Inflight { id, pending: p.clone(), since: self.clock.now });
        self.jobs.push(Job::AskLlm { id, url: self.prefs.llm_url.trim().to_string(), model: self.prefs.llm_model.trim().to_string(), messages });
        true
    }
}

/// A message source's header: everything before the first blank line.
fn header_text(raw: &[u8]) -> String {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").or_else(|| raw.windows(2).position(|w| w == b"\n\n")).unwrap_or(raw.len());
    String::from_utf8_lossy(raw.get(..end.min(16 * 1024)).unwrap_or(raw)).into_owned()
}
