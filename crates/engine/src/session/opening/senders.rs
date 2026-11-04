// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Who a message says it's from (issue #29): the sender check the reading pane and the message
//! list share, gathered once from the user's accounts, every account's contacts, the people
//! written to and the directory's people, and gathered again only when what it's made of changes
//! (its [`Stamp`]): never each frame, never by age. The address book is kept on disk and step by
//! step (`mailboxes/contacts.rs`), with a revision the directory's answers bump too, so there's
//! nothing to count again after a sync. A stranger's name is asked of the directory as their
//! message opens (`message_warning`); the gathering itself never asks anything.

use quatrelle_model::lookalike::{self, KnownPeople, SenderCheck, SenderWarning};

use super::*;

/// The sender check and what it was gathered from.
#[derive(Clone, Debug, Default)]
pub(crate) struct Senders {
    stamp: Stamp,
    pub(crate) check: SenderCheck,
    /// The organisation's domains, for the External tag (empty: there's no telling).
    pub(crate) org: BTreeSet<String>,
}

/// What the check is made of: when any of it changes, it's gathered again.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Stamp {
    /// Each account's address, user name and name.
    accounts: Vec<(String, String, String)>,
    /// The calendars' and contacts' revision (a card saved, an address book synced).
    pim: u64,
    /// The address book's (people written to; the directory's answers bump it too).
    book: u64,
    /// The organisation's domains as listed in Options.
    org: Vec<String>,
}

/// What the message list shows about a sender (issue #29).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SenderMarks {
    /// Show the address after the name.
    pub address: bool,
    /// Something about the sender doesn't add up (the reading pane says what).
    pub warning: bool,
    /// From outside the organisation (with the External tag on).
    pub external: bool,
}

impl Session {
    fn senders_stamp(&self) -> Stamp {
        Stamp {
            accounts: self.accounts.iter().map(|a| (a.config.email.clone(), a.config.username.clone(), a.config.name.clone())).collect(),
            pim: self.pim_revision,
            book: self.contacts_revision(),
            org: self.prefs.org_domains.clone(),
        }
    }

    /// Gather the sender check again if what it's made of changed. With no address book yet, the
    /// mail is read into one first, as [`Session::address_book`] does (in one go for a command
    /// with no window; the window's session only starts it).
    pub fn keep_senders(&mut self) {
        self.contacts_whole();
        self.keep_senders_if_changed();
    }

    /// [`Session::keep_senders`] as the window runs it each frame: only when what it's made of
    /// changed, an address book is loaded and none of the mail is waiting to be read into it. It
    /// never loads the book nor reads it.
    pub fn keep_senders_quietly(&mut self) {
        if self.contacts_book().is_some() && !self.contacts_pending() {
            self.keep_senders_if_changed();
        }
    }

    /// The window wants the check's marks in the list before any message is opened: with nothing
    /// gathered and no address book, the book's reading is started, nothing read now (a first
    /// start, no book kept on disk). The window reads it a slice a frame
    /// ([`Session::contacts_step`]) and [`Session::keep_senders_quietly`] gathers once it's done.
    /// Nothing to do otherwise: a book kept on disk is read so already.
    pub fn want_senders(&mut self) {
        if self.senders.is_none() && self.contacts_book().is_none() {
            self.contacts_step(0);
        }
    }

    fn keep_senders_if_changed(&mut self) {
        let stamp = self.senders_stamp();
        let org = self.organisation_now();
        match self.senders.as_mut() {
            Some(s) if s.stamp == stamp => return,
            // Only the domains listed in Options changed (typed a key at a time): the people
            // stay as they were gathered.
            Some(s) if (&s.stamp.accounts, s.stamp.pim, s.stamp.book) == (&stamp.accounts, stamp.pim, stamp.book) => {
                s.stamp = stamp;
                s.org = org;
            }
            _ => {
                let check = self.gather_check();
                self.senders = Some(Senders { stamp, check, org });
            }
        }
        self.warn_open();
    }

    /// The organisation's domains: those listed in Options, else those of the user's accounts,
    /// mail services left out.
    fn organisation_now(&self) -> BTreeSet<String> {
        let listed: BTreeSet<String> = self.prefs.org_domains.iter().filter_map(|d| lookalike::clean_domain(d)).collect();
        if listed.is_empty() { lookalike::own_domains(self.accounts.iter().map(|a| a.config.email.as_str())) } else { listed }
    }

    /// The check, gathered from the accounts, the cards, the directory's people and the people
    /// written to, in that order: warnings call a person by the first name they're known by, a
    /// card's or the directory's before the newest one mail gave.
    fn gather_check(&self) -> SenderCheck {
        let mut domains: BTreeSet<String> = self.accounts.iter().map(|a| lookalike::domain_of(&a.config.email)).collect();
        let mut people = KnownPeople::new();
        let mut own: Vec<String> = Vec::new();
        for a in &self.accounts {
            for email in [&a.config.email, &a.config.username] {
                if addr::is_valid_email(email.trim()) {
                    people.add_named(&a.config.name, email);
                    own.push(email.clone());
                }
            }
        }
        for p in self.pim.values() {
            for c in &p.cards {
                let name = c.card.display();
                for e in &c.card.emails {
                    people.add_named(&name, &e.value);
                }
            }
        }
        for e in self.people.index.values().filter(|e| !e.who.address.trim().is_empty()) {
            people.add_named(&e.who.name, &e.who.address);
        }
        if let Some(book) = self.contacts_book() {
            domains.extend(book.written_domains());
            for c in book.written() {
                people.add(&c.address);
            }
        }
        domains.remove("");
        SenderCheck::new(domains, people, own)
    }

    /// What's open says what the check says now (a sender just saved as a contact is fine).
    fn warn_open(&mut self) {
        let reading = self.reading.as_ref().and_then(|r| r.parsed.as_ref().map(|p| self.warning_now(&r.key, p)));
        let windows: Vec<Option<Option<SenderWarning>>> =
            self.windows.iter().map(|w| w.parsed.as_ref().map(|p| self.warning_now(&w.key, p))).collect();
        if let (Some(warning), Some(r)) = (reading, self.reading.as_mut()) {
            r.warning = warning;
        }
        for (w, warning) in self.windows.iter_mut().zip(windows) {
            if let Some(warning) = warning {
                w.warning = warning;
            }
        }
    }

    /// What doesn't add up about an open message's sender, the check kept up to date first. With
    /// nothing to say about a stranger, their name is asked of the directory: its answer gathers
    /// the check again, and what's open says what it found.
    pub(crate) fn message_warning(&mut self, key: &MessageKey, parsed: &mime::Parsed) -> Option<SenderWarning> {
        self.keep_senders();
        let warning = self.warning_now(key, parsed);
        if warning.is_none() {
            self.ask_about_sender(key, parsed);
        }
        warning
    }

    /// [`Session::message_warning`] as the check stands, asking nothing: what the sender's header
    /// says, else what the message's own signature does.
    fn warning_now(&self, key: &MessageKey, parsed: &mime::Parsed) -> Option<SenderWarning> {
        let listed = self.listed(key, parsed);
        let s = self.senders.as_ref()?;
        s.check.check(&parsed.from, listed).or_else(|| s.check.check_signature(&parsed.from, &parsed.text, listed, &s.org))
    }

    /// The message came through a list or in bulk as its own header says (a message opened in a
    /// window, or from another folder, needn't be in the list; an Exchange listing carries no
    /// `List-Id`), or as its row does: a list sends its members' mail under their names.
    fn listed(&self, key: &MessageKey, parsed: &mime::Parsed) -> bool {
        parsed.bulk || self.summary(key).is_some_and(|m| m.bulk)
    }

    /// The directory question: an opened message's sender, neither the user nor known by address,
    /// whose name is a person's, has that name asked of the user's Exchange directory, as a
    /// person would type it. Not for a list's or bulk mail (excused from the warning anyway)
    /// unless it comes from a mail service; nothing without an Exchange account.
    fn ask_about_sender(&mut self, key: &MessageKey, parsed: &mime::Parsed) {
        let Some(s) = self.senders.as_ref() else { return };
        let email = parsed.from.email.trim();
        if email.is_empty() || s.check.knows(email) {
            return;
        }
        if self.listed(key, parsed) && !lookalike::is_free_mail(&lookalike::domain_of(email)) {
            return;
        }
        let Some(account) = self.directory_account(Some(&key.account)) else { return };
        let Some(name) = lookalike::lookup_name(&parsed.from.name) else { return };
        self.ask_directory_for_sender(&account, &name);
    }

    /// What doesn't add up about a sender outside the mailbox (a message file, an attached
    /// message): `listed`, it came through a list or in bulk. Nothing for the user's own.
    pub fn sender_warning(&mut self, from: &EmailAddress, listed: bool) -> Option<SenderWarning> {
        self.keep_senders();
        self.senders.as_ref().and_then(|s| s.check.check(from, listed))
    }

    /// The domains the user deals with: their own accounts', and those of the people they write
    /// to (what a look-alike sender would imitate).
    pub fn known_domains(&mut self) -> BTreeSet<String> {
        self.keep_senders();
        self.senders.as_ref().map(|s| s.check.domains().clone()).unwrap_or_default()
    }

    /// The organisation's domains, for the External tag: the ones the user listed, else those of
    /// their accounts (mail services left out). Empty: there's no telling who's external.
    pub fn org_domains(&mut self) -> BTreeSet<String> {
        self.keep_senders();
        self.senders.as_ref().map(|s| s.org.clone()).unwrap_or_default()
    }

    /// The organisation's domains as the check last gathered them (`None` before it's gathered).
    pub fn organisation(&self) -> Option<&BTreeSet<String>> {
        self.senders.as_ref().map(|s| &s.org)
    }

    /// Whether `email` is from outside the organisation, with the External tag on (never the
    /// user's own: the organisation's domains needn't be those of every account).
    pub fn is_external(&self, email: &str) -> bool {
        self.prefs.external_tag && self.senders.as_ref().is_some_and(|s| !s.check.is_own(email) && lookalike::is_external(email, &s.org))
    }

    /// What the message list shows about a message's sender, as the check last gathered knows
    /// (nothing before it's gathered, nor for the user's own mail).
    pub fn sender_marks(&self, m: &Summary) -> SenderMarks {
        let Some(s) = &self.senders else { return SenderMarks::default() };
        let email = m.from.email.trim();
        if email.is_empty() || s.check.is_own(email) {
            return SenderMarks::default();
        }
        let name = m.from.name.trim();
        let named = !name.is_empty() && !name.eq_ignore_ascii_case(email);
        let address = named
            && match self.prefs.sender_addresses {
                SenderAddresses::Always => true,
                SenderAddresses::Strangers => !s.check.knows(email),
                SenderAddresses::Never => false,
            };
        SenderMarks { address, warning: s.check.check(&m.from, m.bulk).is_some(), external: self.is_external(email) }
    }
}
