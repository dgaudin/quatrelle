//! The session: the address book built from the mail kept here (everyone written to or heard
//! from, and the people saved in Contacts), for the recipient boxes' suggestions, Select Names
//! and the Focused Inbox.
//!
//! Building it reads every folder of every account: seconds on a big mailbox (hundreds of
//! thousands of summaries, read from disk again each time, since the cache keeps only so many).
//! Done whole at the first name typed into To, that froze the window; so, like the search index,
//! it is built a slice at a time ([`Session::contacts_step`]), the window drawing between slices
//! and asking for what is known so far (`*_now`). Commands and the CLI, with no frames to draw,
//! take the whole book ([`Session::suggest_contacts`], [`Session::address_book`]).

use super::*;

impl Session {
    /// People to suggest for a recipient box, best first, from the whole address book: built
    /// first, whole, when there is none (the window asks [`Session::suggest_contacts_now`]
    /// instead, which never waits).
    pub fn suggest_contacts(&mut self, query: &str, limit: usize) -> Vec<Contact> {
        self.contacts_whole();
        self.suggest_contacts_now(query, limit)
    }

    /// A whole book to answer from: built now, whole, when there is none. One that mail has
    /// made stale is whole enough (a few minutes behind) and is built again a slice at a time
    /// by the window: a command run from it (Send, resolving names) doesn't read the mailbox
    /// again whole.
    fn contacts_whole(&mut self) {
        if self.contacts.is_none() {
            self.contacts_step(usize::MAX);
        }
    }

    /// People to suggest from what the address book knows now: all of it once built, else what
    /// has been read so far. A build that is due starts here; the window takes it on, a slice
    /// per frame.
    pub fn suggest_contacts_now(&mut self, query: &str, limit: usize) -> Vec<Contact> {
        self.contacts_step(0);
        self.contacts_known().map(|b| b.suggest(query, limit, self.clock.now)).unwrap_or_default()
    }

    /// The address book, as Outlook's Select Names dialog lists it: `people` for the people
    /// saved in Contacts, anything else for everyone known (Contacts and everyone mail was
    /// exchanged with), the whole of it, built first when there is none. Filtered by `query`
    /// (name or address), sorted by name.
    pub fn address_book(&mut self, source: &str, query: &str) -> Vec<EmailAddress> {
        self.contacts_whole();
        self.address_book_now(source, query)
    }

    /// [`Session::address_book`] from what is known now (see [`Session::suggest_contacts_now`]):
    /// the window's, listed again as the book fills in.
    pub fn address_book_now(&mut self, source: &str, query: &str) -> Vec<EmailAddress> {
        const MAX: usize = 2000;
        let book = source.strip_prefix("book:").and_then(|b| b.split_once(':'));
        if source == "people" || book.is_some() {
            let mut v: Vec<EmailAddress> = self
                .people(query)
                .into_iter()
                .filter(|p| book.is_none_or(|(a, b)| p.account == a && p.book == b))
                .flat_map(|p| p.card.emails.iter().map(|e| EmailAddress::new(p.card.display(), e.value.clone())).collect::<Vec<_>>())
                .collect();
            v.truncate(MAX);
            return v;
        }
        self.contacts_step(0);
        self.contacts_known().map(|b| b.list(query, MAX).into_iter().map(|c| c.address).collect()).unwrap_or_default()
    }

    /// The address books Select Names can show besides everyone (`all`) and every contact
    /// (`people`): each account's, as (source, name, the account's address).
    pub fn address_books(&self) -> Vec<(String, String, String)> {
        self.accounts
            .iter()
            .filter_map(|a| self.pim.get(&a.config.id).map(|p| (a, p)))
            .flat_map(|(a, p)| p.books.iter().map(move |b| (format!("book:{}:{}", a.config.id, b.id), b.name.clone(), a.config.email.clone())))
            .collect()
    }

    /// Whether the address book is being built (or built again): the window calls
    /// [`Session::contacts_step`] with a slice each frame until it isn't.
    pub fn contacts_pending(&self) -> bool {
        self.contacts_build.is_some()
    }

    /// Bumped whenever what the address book answers changes: a build done, or a first one
    /// grown by a slice (the book from before answers while it is built again).
    pub fn contacts_revision(&self) -> u64 {
        self.contacts_revision
    }

    /// The book that answers now: the one built, even while another is under way (a stale book
    /// beats a partial one), else the partial one.
    fn contacts_known(&self) -> Option<&ContactBook> {
        self.contacts.as_ref().or_else(|| self.contacts_build.as_ref().map(|b| &b.book))
    }

    /// Build the address book from every folder kept here, `budget` messages at most now:
    /// `usize::MAX` reads all of it, 0 only starts a build that is due (none built, or mail
    /// arrived some minutes ago: `CONTACTS_EVERY`). Whether the book is complete: while
    /// it isn't, the window calls again next frame, drawing in between, so a mailbox of any
    /// size never freezes it. The Focused Inbox tells people written to by the book, so its
    /// list is made again when a build is done.
    pub fn contacts_step(&mut self, budget: usize) -> bool {
        let due = self.contacts_stale && self.clock.now.saturating_sub(self.contacts_built) >= CONTACTS_EVERY;
        if self.contacts_build.is_none() && (self.contacts.is_none() || due) {
            self.contacts_built = self.clock.now;
            self.contacts_stale = false;
            let mine: Vec<String> = self.accounts.iter().flat_map(|a| [a.config.email.clone(), a.config.username.clone()]).collect();
            let scan: Vec<(String, String)> =
                self.accounts.iter().flat_map(|a| a.folders.iter().filter(|f| f.selectable).map(|f| (a.config.id.clone(), f.path.clone()))).collect();
            self.contacts_build = Some(ContactsBuild { scan, folder: 0, pos: 0, mine, book: ContactBook::new() });
        }
        let Some(mut b) = self.contacts_build.take() else { return true };
        let mut added = 0usize;
        while added < budget
            && let Some((a, f)) = b.scan.get(b.folder).cloned()
        {
            // A folder is read on its first message of the slice (from disk when the cache let
            // it go) and kept until its last: one folder's read is the most one slice costs.
            let n = self.folder_state(&a, &f).messages.len();
            while b.pos < n && added < budget {
                if let Some(m) = self.folder_state(&a, &f).messages.get(b.pos) {
                    b.book.add(m, &b.mine);
                }
                b.pos += 1;
                added += 1;
            }
            if b.pos >= n {
                b.folder += 1;
                b.pos = 0;
            }
        }
        if b.folder < b.scan.len() {
            // A first book grows as it is read: what it answers changed.
            if added > 0 && self.contacts.is_none() {
                self.contacts_revision = self.contacts_revision.wrapping_add(1);
            }
            self.contacts_build = Some(b);
            return false;
        }
        // People saved in an address book come first.
        for p in self.pim.values() {
            for c in &p.cards {
                for e in &c.card.emails {
                    b.book.add_saved(&EmailAddress::new(c.card.display(), e.value.clone()), self.clock.now);
                }
            }
        }
        self.contacts = Some(b.book);
        self.contacts_revision = self.contacts_revision.wrapping_add(1);
        if self.focus_split() {
            self.refresh_list();
        }
        true
    }

    /// The address book is let go (the people saved changed): built again when next wanted.
    pub(crate) fn forget_contacts(&mut self) {
        self.contacts = None;
        self.contacts_build = None;
    }
}
