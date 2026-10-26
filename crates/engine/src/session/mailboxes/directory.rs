//! The organisation's directory (Exchange's Global Address List), looked up on the server by
//! name or address: by Check Names (Ctrl+K), the address book's Global Address List and the From
//! button's other address. An account is asked one name at a time; a name being typed waits its
//! turn in place of the one typed before it, and the answers are kept while Quatrelle runs.

use std::collections::VecDeque;

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
    pub people: Vec<EmailAddress>,
    /// It has more than it gave.
    pub more: bool,
    /// Why it couldn't be asked.
    pub error: Option<String>,
    at: i64,
}

/// The directory's questions and answers, and what Check Names waits on.
#[derive(Default)]
pub(crate) struct Directory {
    /// (account, the name as asked) → the answer, and the order they came in.
    found: HashMap<(String, String), DirectoryAnswer>,
    order: VecDeque<(String, String)>,
    /// The name each account is being asked about.
    asked: HashMap<String, String>,
    /// What waits its turn, per account: (the name, typed rather than checked).
    waiting: HashMap<String, VecDeque<(String, bool)>>,
    /// Names Check Names waits on the directory for.
    pub(crate) checks: Vec<NameCheck>,
    /// Names several people fit, for the one meant to be chosen.
    pub(crate) choices: Vec<NameChoice>,
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
        let q = directory_key(query);
        let now = self.clock.now;
        let d = &mut self.contacts.directory;
        let known = d.found.get(&(account.to_string(), q.clone())).is_some_and(|f| f.error.is_none() || now - f.at < RETRY_AFTER);
        if q.is_empty() || known || d.asked.get(account) == Some(&q) {
            return;
        }
        let w = d.waiting.entry(account.to_string()).or_default();
        if typed {
            w.retain(|(_, t)| !t);
        }
        if !w.iter().any(|(x, _)| *x == q) {
            w.push_back((q, typed));
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

    /// The next name waiting, asked when the account isn't being asked another.
    fn ask_directory(&mut self, account: &str) {
        let d = &mut self.contacts.directory;
        if d.asked.contains_key(account) {
            return;
        }
        let Some((q, _)) = d.waiting.get_mut(account).and_then(VecDeque::pop_front) else { return };
        d.asked.insert(account.to_string(), q.clone());
        self.jobs.push(Job::SearchDirectory { account: account.to_string(), query: q });
    }

    /// The directory answered: kept, the next name asked, and Check Names told.
    pub(crate) fn apply_directory(&mut self, account: String, query: String, people: Vec<EmailAddress>, more: bool, error: Option<String>) {
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
        if d.found.insert(k.clone(), DirectoryAnswer { people, more, error, at }).is_none() {
            d.order.push_back(k);
        }
        while d.order.len() > KEPT {
            if let Some(old) = d.order.pop_front() {
                d.found.remove(&old);
            }
        }
        self.contacts.changed();
        self.ask_directory(&account);
        self.directory_checked(&account, &q);
    }
}
