//! Check Names (Ctrl+K), as Outlook's: each name typed in To, Cc or Bcc is looked up among the
//! contacts and the people written to or heard from, and in the organisation's directory
//! (Exchange's). One person found is written out with their address, several are offered to
//! choose from, and none is said.

use super::*;

/// What Check Names says when a name is left as it was typed.
pub const NAMES_NOT_FOUND: &str = "Some names weren't found in your contacts or among the people you've written to";

/// The most names one Check Names looks up.
const MAX_NAMES: usize = 500;

/// A name Check Names waits on the directory for, in the message being written `compose`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NameCheck {
    compose: u64,
    account: String,
    token: String,
}

/// Several people fit a name typed in the message being written `compose`: the one meant is
/// chosen ([`Session::choose_name`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NameChoice {
    pub compose: u64,
    pub token: String,
    pub people: Vec<EmailAddress>,
}

/// The names and addresses in a recipient field, as typed.
fn tokens(field: &str) -> impl Iterator<Item = &str> {
    field.split([';', ',']).map(str::trim).filter(|t| !t.is_empty()).take(MAX_NAMES)
}

/// `field` with the name `token` written out as `with`; `None` when it isn't there any more.
fn replaced(field: &str, token: &str, with: &EmailAddress) -> Option<String> {
    let mut done = false;
    let parts: Vec<String> = tokens(field)
        .map(|t| {
            if !done && t == token {
                done = true;
                with.to_string()
            } else {
                t.to_string()
            }
        })
        .collect();
    done.then(|| format!("{}; ", parts.join("; ")))
}

impl Session {
    /// The people here a name fits: by name, or the start of an address, among the contacts,
    /// the people written to or heard from, and the user's own addresses.
    fn local_names(&mut self, token: &str) -> Vec<EmailAddress> {
        let lower = token.to_lowercase();
        let fits = |a: &EmailAddress| a.name.to_lowercase().contains(&lower) || a.email.to_lowercase().starts_with(&lower);
        let mut out: Vec<EmailAddress> = self.suggest_contacts(token, 10).into_iter().map(|c| c.address).filter(|a| fits(a)).collect();
        for a in self.own_addresses() {
            if fits(&a) && !out.iter().any(|x| x.email.eq_ignore_ascii_case(&a.email)) {
                out.push(a);
            }
        }
        out
    }

    /// Outlook's Check Names on the message being written. The names it can't settle here wait
    /// on the directory (answered as `searching`); those left as typed are said.
    pub fn check_names(&mut self) -> CmdResult {
        let c = self.compose.as_ref().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
        let (id, account) = (c.id, c.account.clone());
        let fields = [c.to.clone(), c.cc.clone(), c.bcc.clone()];
        let directory = self.directory_account(Some(&account));
        let (mut resolved, mut unresolved, mut searching) = (0usize, Vec::new(), Vec::new());
        let mut out: Vec<String> = Vec::new();
        for field in &fields {
            let mut parts: Vec<String> = Vec::new();
            for token in tokens(field) {
                if token.contains('@') {
                    parts.push(token.to_string());
                    continue;
                }
                let here = self.local_names(token);
                match (here.as_slice(), &directory) {
                    // Known here by its address alone, with a directory to ask: its full name
                    // from there (issue #9), as Outlook checks the address list first.
                    ([one], Some(_)) if bare_name(one) => searching.push(token.to_string()),
                    ([one], _) => {
                        parts.push(one.to_string());
                        resolved += 1;
                        continue;
                    }
                    (_, Some(_)) => searching.push(token.to_string()),
                    ([], None) => unresolved.push(token.to_string()),
                    (_, None) => self.offer_name(NameChoice { compose: id, token: token.to_string(), people: here }),
                }
                parts.push(token.to_string());
            }
            out.push(if parts.is_empty() { String::new() } else { format!("{}; ", parts.join("; ")) });
        }
        let c = self.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
        if let [to, cc, bcc] = out.as_slice() {
            c.to = to.clone();
            c.cc = cc.clone();
            c.bcc = bcc.clone();
        }
        // Each name left is pointed out under its field once the cursor leaves it.
        if !unresolved.is_empty() {
            self.notice(NAMES_NOT_FOUND, true);
        }
        if let Some(account) = directory {
            for token in &searching {
                let check = NameCheck { compose: id, account: account.clone(), token: token.clone() };
                if !self.contacts.directory.checks.contains(&check) {
                    self.contacts.directory.checks.push(check);
                }
                self.search_directory(&account, token, false);
                // Answered before: settled now.
                if self.directory_found(&account, token).is_some() && !self.directory_searching(&account, token) {
                    self.directory_checked(&account, &directory_key(token));
                }
            }
        }
        Ok(serde_json::json!({ "resolved": resolved, "unresolved": unresolved, "searching": searching }))
    }

    /// The directory answered `query` (lower case): the names Check Names waits on for it are
    /// settled with what it and the address book have.
    pub(crate) fn directory_checked(&mut self, account: &str, query: &str) {
        let (now, later): (Vec<NameCheck>, Vec<NameCheck>) =
            std::mem::take(&mut self.contacts.directory.checks).into_iter().partition(|c| c.account == account && directory_key(&c.token) == query);
        self.contacts.directory.checks = later;
        let mut missing = false;
        for check in now {
            let mut people = self.local_names(&check.token);
            for p in self.directory_found(account, &check.token).map(|f| f.people.clone()).unwrap_or_default() {
                // Known here by its address alone: the directory's full name for it.
                match people.iter_mut().find(|x| x.email.eq_ignore_ascii_case(&p.email)) {
                    Some(known) => fill_name(known, &p),
                    None => people.push(p),
                }
            }
            match people.as_slice() {
                [] => missing = true,
                [one] => {
                    let one = one.clone();
                    self.write_name(check.compose, &check.token, &one);
                }
                _ => self.offer_name(NameChoice { compose: check.compose, token: check.token, people }),
            }
        }
        if missing {
            self.notice(NAMES_NOT_FOUND, true);
        }
    }

    /// Write the name `token` out as `with` in the message being written `compose` (the current
    /// one or one set aside), in the first field that still has it.
    fn write_name(&mut self, compose: u64, token: &str, with: &EmailAddress) -> bool {
        let Some(c) = self.compose.iter_mut().chain(self.composes.iter_mut()).find(|c| c.id == compose) else { return false };
        for field in [&mut c.to, &mut c.cc, &mut c.bcc] {
            if let Some(new) = replaced(field, token, with) {
                *field = new;
                return true;
            }
        }
        false
    }

    fn offer_name(&mut self, choice: NameChoice) {
        let choices = &mut self.contacts.directory.choices;
        choices.retain(|c| !(c.compose == choice.compose && c.token == choice.token));
        choices.push(choice);
    }

    /// A name several people fit in the message being written `compose`, for the one meant to be
    /// chosen.
    pub fn name_choice(&self, compose: u64) -> Option<&NameChoice> {
        self.contacts.directory.choices.iter().find(|c| c.compose == compose)
    }

    /// The person meant by the name `token` in the message being written (`None`: left as
    /// typed).
    pub fn choose_name(&mut self, token: &str, person: Option<EmailAddress>) -> CmdResult {
        let id = self.compose.as_ref().map(|c| c.id).ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
        let choices = &mut self.contacts.directory.choices;
        let before = choices.len();
        choices.retain(|c| !(c.compose == id && c.token == token));
        if choices.len() == before {
            return Err(CmdError::Params(format!("no one to choose for `{token}`")));
        }
        let written = person.as_ref().is_some_and(|p| self.write_name(id, token, p));
        Ok(serde_json::json!({ "token": token, "written": written }))
    }
}
