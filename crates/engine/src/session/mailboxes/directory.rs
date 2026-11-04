// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The organisation's directory (Exchange's Global Address List), looked up on the server by
//! name or address: by Check Names (Ctrl+K), the address book's Global Address List and the From
//! button's other address, and a stranger's name as their message opens (the sender check). An
//! account is asked one name at a time; a name being typed waits its turn in place of the one
//! typed before it, a sender's in place of the sender's before it, behind the user's own
//! questions; the answers are kept while Quatrelle runs.

use std::collections::VecDeque;

use quatrelle_model::Addressee;

use super::*;

/// Answers kept (the oldest go first).
const KEPT: usize = 300;

/// The longest name looked up.
const MAX_QUERY: usize = 100;

/// A failed answer is asked again after this long (seconds), not every time it's wanted.
const RETRY_AFTER: i64 = 60;

/// What the directory answered for a name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DirectoryAnswer {
    pub people: Vec<Addressee>,
    /// It has more than it gave.
    pub more: bool,
    /// Why it couldn't be asked.
    pub error: Option<String>,
    at: i64,
}

/// Who a name waiting on the directory is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Asked {
    /// Check Names, the From button: asked in turn.
    Checked,
    /// A name being typed (the Global Address List): in place of the one typed before it.
    Typed,
    /// An opened message's sender (the sender check): in place of the sender's before it, and
    /// after the user's own questions.
    Sender,
}

/// The directory's questions and answers (what fields wait on them is kept by field, in
/// `People`).
#[derive(Default)]
pub(crate) struct Directory {
    /// (account, the name as asked) → the answer, and the order they came in.
    found: HashMap<(String, String), DirectoryAnswer>,
    order: VecDeque<(String, String)>,
    /// The name each account is being asked about.
    asked: HashMap<String, String>,
    /// What waits its turn, per account, and who for.
    waiting: HashMap<String, VecDeque<(String, Asked)>>,
}

/// A name as it's asked: trimmed, in lower case, cut to a length.
pub(crate) fn directory_key(query: &str) -> String {
    query.trim().chars().take(MAX_QUERY).collect::<String>().to_lowercase()
}

impl Session {
    /// The Exchange account whose directory is searched: `preferred` when it's one, else the
    /// first.
    pub fn directory_account(&self, preferred: Option<&str>) -> Option<String> {
        let exchange = |a: &&AccountState| a.config.ews.is_some();
        preferred.and_then(|p| self.account(p)).filter(exchange).or_else(|| self.accounts.iter().find(exchange)).map(|a| a.config.id.clone())
    }

    /// Look `query` up in `account`'s directory; `typed`: it's being typed, and takes the place
    /// of a name typed before it that wasn't asked yet. Nothing when it's known or being asked.
    pub fn search_directory(&mut self, account: &str, query: &str, typed: bool) {
        self.wait_on_directory(account, query, if typed { Asked::Typed } else { Asked::Checked });
    }

    /// Look an opened message's sender's `name` up in `account`'s directory: in place of another
    /// sender's name still waiting (arrowing through the list asks for the last one), never of
    /// the user's own questions. Nothing when it's known or being asked.
    pub(crate) fn ask_directory_for_sender(&mut self, account: &str, name: &str) {
        self.wait_on_directory(account, name, Asked::Sender);
    }

    /// `query` waits its turn on `account`'s directory for `who`, in place of a name waiting for
    /// the same kind of question when that's a name typed or a sender's.
    fn wait_on_directory(&mut self, account: &str, query: &str, who: Asked) {
        let q = directory_key(query);
        let now = self.clock.now;
        let d = &mut self.contacts.directory;
        let known = d.found.get(&(account.to_string(), q.clone())).is_some_and(|f| f.error.is_none() || now.saturating_sub(f.at) < RETRY_AFTER);
        if q.is_empty() || known || d.asked.get(account) == Some(&q) {
            return;
        }
        let w = d.waiting.entry(account.to_string()).or_default();
        if who != Asked::Checked {
            w.retain(|(_, t)| *t != who);
        }
        match w.iter_mut().find(|(x, _)| *x == q) {
            // A sender's name the user asks about too is theirs now, and a name typed that Check
            // Names asks about is Check Names' own: neither can be displaced.
            Some((_, t)) if *t == Asked::Sender || who == Asked::Checked => *t = who,
            Some(_) => {}
            None => w.push_back((q, who)),
        }
        self.ask_directory(account);
    }

    /// What the directory answered for `query`, when it has.
    pub fn directory_found(&self, account: &str, query: &str) -> Option<&DirectoryAnswer> {
        self.contacts.directory.found.get(&(account.to_string(), directory_key(query)))
    }

    /// Whether `query` is being asked, or waits its turn.
    pub fn directory_searching(&self, account: &str, query: &str) -> bool {
        let (d, q) = (&self.contacts.directory, directory_key(query));
        d.asked.get(account) == Some(&q) || d.waiting.get(account).is_some_and(|w| w.iter().any(|(x, _)| *x == q))
    }

    /// The next name waiting, asked when the account isn't being asked another: the user's own
    /// questions before a sender's.
    fn ask_directory(&mut self, account: &str) {
        let d = &mut self.contacts.directory;
        if d.asked.contains_key(account) {
            return;
        }
        let Some(w) = d.waiting.get_mut(account) else { return };
        let next = w.iter().position(|(_, who)| *who != Asked::Sender).unwrap_or(0);
        let Some((q, _)) = w.remove(next) else { return };
        d.asked.insert(account.to_string(), q.clone());
        self.jobs.push(Job::SearchDirectory { account: account.to_string(), query: q });
    }

    /// The directory answered: kept, the next name asked, Check Names told and the sender check
    /// gathered again.
    pub(crate) fn apply_directory(&mut self, account: String, query: String, people: Vec<Addressee>, more: bool, error: Option<String>) {
        let q = directory_key(&query);
        if let Some(e) = &error {
            log::info!("{account}: the directory: {e}");
        }
        let at = self.clock.now;
        let d = &mut self.contacts.directory;
        if d.asked.get(&account) == Some(&q) {
            d.asked.remove(&account);
        }
        let k = (account.clone(), q.clone());
        let indexed = people.clone();
        if d.found.insert(k.clone(), DirectoryAnswer { people, more, error, at }).is_none() {
            d.order.push_back(k);
        }
        while d.order.len() > KEPT {
            if let Some(old) = d.order.pop_front() {
                d.found.remove(&old);
            }
        }
        // The index keeps its own order (`people.rs`): an answer cut here stays in it.
        self.index_directory(&indexed);
        self.contacts.changed();
        self.ask_directory(&account);
        self.directory_checked(&account, &q);
        // The people it named are known now: what's open says so.
        self.keep_senders_quietly();
    }
}

/// The entries of a directory answer that have an internet address, as addresses: those Check
/// Names and the address book take (a contact group, which has none, is left to the contacts
/// field).
pub fn directory_people(people: &[Addressee]) -> Vec<EmailAddress> {
    people.iter().filter(|p| !p.address.is_empty()).map(Addressee::email).collect()
}

/// Whether an address has no real name: none, the address itself, or its user part, as some
/// directories and senders give (issue #9).
pub(crate) fn bare_name(a: &EmailAddress) -> bool {
    let name = a.name.trim();
    let user = a.email.split('@').next().unwrap_or("");
    name.is_empty() || name.eq_ignore_ascii_case(&a.email) || name.eq_ignore_ascii_case(user)
}

/// A known address with no real name ([`bare_name`]) takes the directory's full name for it.
pub fn fill_name(known: &mut EmailAddress, directory: &EmailAddress) {
    if bare_name(known) && !directory.name.trim().is_empty() {
        known.name = directory.name.clone();
    }
}
