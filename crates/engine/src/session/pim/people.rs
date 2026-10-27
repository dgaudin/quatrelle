//! People: each account's address books, and the pictures the directory keeps of people.

use super::*;

/// A person in an address book.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    pub account: String,
    pub book: String,
    pub card: Card,
}

impl Session {
    /// Everyone in the address books matching a search, by name.
    pub fn people(&self, query: &str) -> Vec<Person> {
        let mut out: Vec<Person> = self
            .pim
            .iter()
            .flat_map(|(a, p)| {
                p.cards.iter().filter(|c| c.card.matches(query)).map(|c| Person { account: a.clone(), book: c.book.clone(), card: c.card.clone() })
            })
            .collect();
        out.sort_by_key(|p| p.card.display().to_lowercase());
        out
    }

    /// The contact saved with this address (any case), the first by name when there are several:
    /// what a message's header names, looked up as it's drawn. Only addresses are compared, and
    /// only the one found copied (a search, every card's every field in lower case, every match
    /// copied and sorted, took 40 % of the window's time for a message's header, 27/10).
    pub fn person_by_email(&self, email: &str) -> Option<Person> {
        let email = email.trim();
        if email.is_empty() {
            return None;
        }
        let mut found: Option<(&String, &StoredCard)> = None;
        for (a, p) in &self.pim {
            for c in p.cards.iter().filter(|c| c.card.emails.iter().any(|e| e.value.trim().eq_ignore_ascii_case(email))) {
                let first = found.is_none_or(|(_, f)| c.card.display().to_lowercase() < f.card.display().to_lowercase());
                if first {
                    found = Some((a, c));
                }
            }
        }
        found.map(|(a, c)| Person { account: a.clone(), book: c.book.clone(), card: c.card.clone() })
    }

    pub fn find_person(&self, uid: &str) -> Option<Person> {
        self.pim.iter().find_map(|(a, p)| {
            p.cards.iter().find(|c| c.card.uid == uid).map(|c| Person { account: a.clone(), book: c.book.clone(), card: c.card.clone() })
        })
    }

    /// Add someone; returns their uid.
    pub fn create_card(&mut self, account: Option<&str>, mut card: Card) -> CmdResult {
        let account = self.pim_account(account)?;
        check_card(&card)?;
        if card.uid.trim().is_empty() {
            card.uid = self.new_uid();
        }
        let p = self.pim_mut(&account);
        let book = p.default_book().map(|b| b.id.clone()).ok_or_else(|| CmdError::Disabled("no address book to add to".into()))?;
        let uid = card.uid.clone();
        p.cards.retain(|c| c.card.uid != uid);
        p.cards.push(StoredCard { book: book.clone(), href: String::new(), etag: String::new(), card, dirty: book != LOCAL });
        self.after_pim_change(&account, book != LOCAL);
        Ok(serde_json::json!({"uid": uid, "account": account}))
    }

    /// Whether a person is in an address book that can't be changed here (an organisation's
    /// shared contacts).
    pub fn person_read_only(&self, uid: &str) -> bool {
        self.pim.values().any(|p| p.cards.iter().any(|c| c.card.uid == uid && p.books.iter().any(|b| b.id == c.book && b.read_only)))
    }

    /// The picture the directory keeps for an address, once it has been fetched.
    pub fn person_photo(&self, email: &str) -> Option<&quatrelle_model::vcard::Photo> {
        self.person_photos.get(&email.trim().to_ascii_lowercase()).and_then(Option::as_ref)
    }

    /// Whether a picture was asked for (and answered, or none to have): the window stops asking
    /// once it has.
    pub fn person_photo_known(&self, email: &str) -> bool {
        let email = email.trim().to_ascii_lowercase();
        self.person_photos.contains_key(&email) || self.photo_asked.contains(&email)
    }

    /// Whether some account is one that keeps pictures of people (Exchange, Microsoft 365).
    pub fn pictures_possible(&self) -> bool {
        self.accounts.iter().any(|a| a.config.ews.is_some() || a.config.graph.is_some())
    }

    /// Whether a picture of that address can be had: one their contact item carries (an Exchange
    /// address book's), or one the directory keeps for a colleague's address. A directory only
    /// knows its own people, so an outsider's address is not asked for.
    pub fn photo_possible_for(&self, email: &str) -> bool {
        let email = email.trim();
        if email.is_empty() || !self.pictures_possible() {
            return false;
        }
        let own = self
            .pim
            .values()
            .flat_map(|p| p.cards.iter())
            .any(|c| c.href.starts_with("contact:") && c.card.emails.iter().any(|e| e.value.trim().eq_ignore_ascii_case(email)));
        if own {
            return true;
        }
        let domain = email.rsplit_once('@').map(|(_, d)| d.to_ascii_lowercase()).unwrap_or_default();
        !domain.is_empty()
            && self.accounts.iter().any(|a| {
                (a.config.ews.is_some() || a.config.graph.is_some())
                    && a.config.email.rsplit_once('@').is_some_and(|(_, d)| d.eq_ignore_ascii_case(&domain))
            })
    }

    /// Ask the directory (Exchange, Microsoft 365) for a person's picture, once: an address it
    /// doesn't know, or a server that keeps no pictures, is remembered as having none.
    pub fn fetch_person_photo(&mut self, account: Option<&str>, email: &str) -> CmdResult {
        let email = email.trim().to_ascii_lowercase();
        if email.is_empty() || !email.contains('@') {
            return Err(CmdError::Params("an email address is needed".into()));
        }
        if self.person_photo_known(&email) {
            return Ok(serde_json::json!({"known": true, "picture": self.person_photo(&email).is_some()}));
        }
        let can = |a: &AccountState| a.config.ews.is_some() || a.config.graph.is_some();
        let domain = email.rsplit_once('@').map(|(_, d)| d.to_ascii_lowercase()).unwrap_or_default();
        let chosen = match account {
            Some(id) => self.account(id).filter(|a| can(a)).map(|a| a.config.id.clone()),
            None => self
                .accounts
                .iter()
                .find(|a| can(a) && a.config.email.rsplit_once('@').is_some_and(|(_, d)| d.eq_ignore_ascii_case(&domain)))
                .or_else(|| self.accounts.iter().find(|a| can(a)))
                .map(|a| a.config.id.clone()),
        };
        let Some(account) = chosen else {
            return Err(CmdError::Disabled("no Exchange account to ask for pictures".into()));
        };
        // The contact item's own picture first (Outlook's contacts keep one), then the
        // directory's: the address book's id is what the server knows the item by.
        let contact = self
            .pim
            .values()
            .flat_map(|p| p.cards.iter())
            .find(|c| c.card.emails.iter().any(|e| e.value.trim().eq_ignore_ascii_case(&email)))
            .and_then(|c| c.href.strip_prefix("contact:").map(str::to_string))
            .filter(|id| !id.is_empty());
        self.photo_asked.insert(email.clone());
        self.jobs.push(Job::FetchPhoto { account, email, contact });
        Ok(serde_json::json!({"asked": true}))
    }

    pub fn update_card(&mut self, uid: &str, change: impl FnOnce(&mut Card)) -> CmdResult {
        let account = self.find_person(uid).map(|p| p.account).ok_or_else(|| CmdError::Params(format!("no contact `{uid}`")))?;
        if self.person_read_only(uid) {
            return Err(CmdError::Disabled("this contact is in a shared address book that can't be changed here".into()));
        }
        let p = self.pim_mut(&account);
        let Some(c) = p.cards.iter_mut().find(|c| c.card.uid == uid) else { return Err(CmdError::Params(format!("no contact `{uid}`"))) };
        let mut card = c.card.clone();
        change(&mut card);
        check_card(&card)?;
        c.card = card;
        let remote = c.book != LOCAL;
        c.dirty |= remote;
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": uid}))
    }

    pub fn delete_card(&mut self, uid: &str) -> CmdResult {
        let account = self.find_person(uid).map(|p| p.account).ok_or_else(|| CmdError::Params(format!("no contact `{uid}`")))?;
        if self.person_read_only(uid) {
            return Err(CmdError::Disabled("this contact is in a shared address book that can't be changed here".into()));
        }
        let p = self.pim_mut(&account);
        let mut remote = false;
        for c in p.cards.iter().filter(|c| c.card.uid == uid && !c.href.is_empty()) {
            remote = true;
            p.deleted.push(Tombstone { href: c.href.clone(), etag: c.etag.clone(), ..Default::default() });
        }
        p.cards.retain(|c| c.card.uid != uid);
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"deleted": uid}))
    }
}

fn check_card(c: &Card) -> Result<(), CmdError> {
    if c.display().trim().is_empty() {
        return Err(CmdError::Params("give the contact a name or an address".into()));
    }
    for e in &c.emails {
        if !addr::is_valid_email(e.value.trim()) {
            return Err(CmdError::Params(format!("`{}` is not an email address", e.value)));
        }
    }
    Ok(())
}
