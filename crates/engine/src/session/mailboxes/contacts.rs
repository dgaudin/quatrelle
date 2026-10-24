//! The session: the address book built from the mail kept here (everyone written to or heard
//! from, and the people saved in Contacts), for the recipient boxes' suggestions, Select Names
//! and the Focused Inbox.
//!
//! Like Outlook's AutoComplete list it is kept on disk (`contacts.json`), with how far each
//! folder was read: a start reads nothing, and a folder a sync changed has only its new mail
//! read (a message numbered above the folder's mark). Every folder is read whole once, the first
//! time. Reading a big mailbox whole takes seconds (hundreds of thousands of summaries, from
//! disk, since the cache keeps only so many): done whole at the first name typed into To, and
//! again every few minutes as mail came, it froze the window. So the reading is done a slice at
//! a time ([`Session::contacts_step`]), the window drawing between slices and asking for what is
//! known so far (`*_now`); commands and the CLI, with no frames to draw, read what is left in one
//! go ([`Session::suggest_contacts`], [`Session::address_book`]).
//!
//! An address taken out of the suggestions (Outlook's ×, [`Session::forget_contact`]) stays out,
//! whatever mail comes from it, until it's written to.

use std::collections::VecDeque;

use super::*;

/// The address book's file.
const CONTACTS_DOC: &str = "contacts.json";

/// How often (seconds) an address book that changed is written.
const SAVE_EVERY: i64 = 60;

/// The address book and how far it has read the mail.
#[derive(Default)]
pub(crate) struct Contacts {
    /// The book: `None` until it's first wanted (loaded with the session when kept on disk).
    book: Option<ContactBook>,
    /// How far each folder was read: (validity, the highest number read).
    marks: HashMap<(String, String), (u32, u32)>,
    /// Folders to read (new, or changed by a sync), and the one being read: where, and the
    /// mark it had when its reading began.
    todo: VecDeque<(String, String)>,
    at: Option<Reading>,
    /// The people saved in Contacts changed: laid over the book again.
    saved_changed: bool,
    /// Changed since it was written, and when it last was.
    unsaved: bool,
    saved_at: i64,
    /// Bumped whenever what the book answers changes: the window works its suggestions out
    /// again only then.
    revision: u64,
}

/// A folder being read into the book, a slice at a time.
struct Reading {
    folder: (String, String),
    /// How far through its list.
    pos: usize,
    /// The mark it had (`None`: read whole), and the highest number met so far.
    from: Option<u32>,
    top: u32,
    validity: u32,
}

/// What `contacts.json` holds.
#[derive(Default, Serialize, Deserialize)]
struct ContactsDoc {
    #[serde(default)]
    book: ContactBook,
    /// (account, folder, validity, the highest number read).
    #[serde(default)]
    marks: Vec<(String, String, u32, u32)>,
}

impl Contacts {
    /// The book kept on disk, if there is one.
    pub(crate) fn load(store: &Store) -> Contacts {
        let doc: ContactsDoc = store.load_doc(CONTACTS_DOC).unwrap_or_else(|e| {
            log::warn!("the address book couldn't be read ({e}): it's read from the mail again");
            ContactsDoc::default()
        });
        if doc.marks.is_empty() {
            return Contacts::default();
        }
        let mut book = doc.book;
        // What was saved without its Contacts layer (an older file): only mail counts.
        book.clear_saved();
        Contacts {
            book: Some(book),
            marks: doc.marks.into_iter().map(|(a, f, v, n)| ((a, f), (v, n))).collect(),
            saved_changed: true,
            ..Default::default()
        }
    }

    fn queue(&mut self, folder: (String, String)) {
        if !self.todo.contains(&folder) {
            self.todo.push_back(folder);
        }
    }
}

impl Session {
    /// People to suggest for a recipient box, best first, from the whole address book: what is
    /// left to read of the mail is read first, in one go (the window asks
    /// [`Session::suggest_contacts_now`] instead, which never waits).
    pub fn suggest_contacts(&mut self, query: &str, limit: usize) -> Vec<Contact> {
        self.contacts_whole();
        self.suggest_contacts_now(query, limit)
    }

    /// The book with all the mail read: in one go, when there's no book yet. One already there
    /// answers as it is (the mail new since is read a slice at a time by the window): a command
    /// run from it (Send, resolving names) doesn't read a big folder whole.
    fn contacts_whole(&mut self) {
        if self.contacts.book.is_none() || self.contacts.marks.is_empty() {
            self.contacts_step(usize::MAX);
        }
    }

    /// People to suggest from what the address book knows now. Reading the mail that is left
    /// starts here; the window takes it on, a slice per frame.
    pub fn suggest_contacts_now(&mut self, query: &str, limit: usize) -> Vec<Contact> {
        self.contacts_step(0);
        self.contacts.book.as_ref().map(|b| b.suggest(query, limit, self.clock.now)).unwrap_or_default()
    }

    /// The address book, as Outlook's Select Names dialog lists it: `people` for the people
    /// saved in Contacts, anything else for everyone known (Contacts and everyone mail was
    /// exchanged with), with all the mail read first. Filtered by `query` (name or address),
    /// sorted by name.
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
        self.contacts.book.as_ref().map(|b| b.list(query, MAX).into_iter().map(|c| c.address).collect()).unwrap_or_default()
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

    /// What the address book knows about an address.
    pub(crate) fn known_contact(&self, email: &str) -> Option<&Contact> {
        self.contacts.book.as_ref().and_then(|b| b.get(email))
    }

    /// The domains of the people written to: the ones a lookalike sender would imitate. With no
    /// book yet, the mail is read into one first, in one go (a book kept on disk answers at once).
    pub(crate) fn written_domains(&mut self) -> Vec<String> {
        self.contacts_whole();
        self.contacts.book.as_ref().map(|b| b.written_domains().collect()).unwrap_or_default()
    }

    /// Whether mail is left to read into the address book: the window calls
    /// [`Session::contacts_step`] with a slice each frame until there's none.
    pub fn contacts_pending(&self) -> bool {
        self.contacts.at.is_some() || !self.contacts.todo.is_empty() || self.contacts.saved_changed
    }

    /// Bumped whenever what the address book answers changes.
    pub fn contacts_revision(&self) -> u64 {
        self.contacts.revision
    }

    /// A folder changed (a sync, an edit): its new mail is read into the book.
    pub(crate) fn contacts_folder_changed(&mut self, account: &str, folder: &str) {
        if self.contacts.book.is_some() {
            self.contacts.queue((account.to_string(), folder.to_string()));
        }
    }

    /// The people saved in Contacts changed: laid over the book again (not read again from
    /// the mail, as the whole book was).
    pub(crate) fn forget_contacts(&mut self) {
        self.contacts.saved_changed = true;
    }

    /// Take an address out of the suggestions (Outlook's ×): it stays out, whatever mail comes
    /// from it, until it's written to. Written at once.
    pub fn forget_contact(&mut self, email: &str) -> bool {
        self.contacts_step(0);
        let Some(book) = self.contacts.book.as_mut() else { return false };
        let was = book.forget(email);
        self.contacts.revision = self.contacts.revision.wrapping_add(1);
        self.contacts.unsaved = true;
        self.save_contacts();
        was
    }

    /// The recipients of a message being sent: written to, suggested first from now on (and
    /// back in the suggestions if they were taken out), as Outlook's AutoComplete list does.
    pub(crate) fn contacts_sent(&mut self, recipients: &[EmailAddress]) {
        self.contacts_step(0);
        let mine = self.contacts_mine();
        let now = self.clock.now;
        if let Some(book) = self.contacts.book.as_mut() {
            book.add_sent(recipients, now, &mine);
            self.contacts.revision = self.contacts.revision.wrapping_add(1);
            self.contacts.unsaved = true;
        }
    }

    /// The user's own addresses: a message from one of them is one they sent.
    fn contacts_mine(&self) -> Vec<String> {
        self.accounts.iter().flat_map(|a| [a.config.email.clone(), a.config.username.clone()]).collect()
    }

    /// Write the address book if it changed since it was last written (and as Quatrelle closes).
    pub fn save_contacts(&mut self) {
        let c = &mut self.contacts;
        let (Some(store), Some(book)) = (&self.store, &c.book) else { return };
        if !c.unsaved {
            return;
        }
        // Without the Contacts layer: only what the mail counted is kept.
        let mut book = book.clone();
        book.clear_saved();
        let mut marks: Vec<(String, String, u32, u32)> = c.marks.iter().map(|((a, f), (v, n))| (a.clone(), f.clone(), *v, *n)).collect();
        marks.sort();
        match store.save_doc(CONTACTS_DOC, &ContactsDoc { book, marks }) {
            Ok(()) => {
                c.unsaved = false;
                c.saved_at = self.clock.now;
            }
            Err(e) => log::warn!("the address book wasn't saved: {e}"),
        }
    }

    /// Read mail into the address book, `budget` messages at most now: `usize::MAX` reads all
    /// that's left, 0 only starts (a first book, the folders not read yet). Whether all is read:
    /// while it isn't, the window calls again next frame, drawing in between, so a mailbox of
    /// any size never freezes it. The Focused Inbox tells people written to by the book, so its
    /// list is made again when a reading is done.
    pub fn contacts_step(&mut self, budget: usize) -> bool {
        if self.contacts.book.is_none() {
            self.contacts.book = Some(ContactBook::new());
            self.contacts.saved_changed = true;
        }
        // Folders never read (a first book, a new account, a reading cut short): read whole.
        if self.contacts.todo.is_empty() && self.contacts.at.is_none() {
            let unread: Vec<(String, String)> = self
                .accounts
                .iter()
                .flat_map(|a| a.folders.iter().filter(|f| f.selectable).map(|f| (a.config.id.clone(), f.path.clone())))
                .filter(|k| !self.contacts.marks.contains_key(k))
                .collect();
            for k in unread {
                self.contacts.queue(k);
            }
        }
        let busy = self.contacts.at.is_some() || !self.contacts.todo.is_empty();
        let mine = if busy { self.contacts_mine() } else { Vec::new() };
        let mut done = 0usize;
        let mut added = false;
        while done < budget {
            let Some(mut r) = self.contacts.at.take().or_else(|| self.contacts_begin()) else { break };
            let (a, f) = r.folder.clone();
            let st = self.folder_state(&a, &f);
            let n = st.messages.len();
            let from = r.from;
            let mut taken: Vec<Arc<Summary>> = Vec::new();
            while r.pos < n && done < budget {
                if let Some(m) = self.folder_state(&a, &f).messages.get(r.pos) {
                    r.top = r.top.max(m.key.uid);
                    if from.is_none_or(|mark| m.key.uid > mark) && m.key.validity == r.validity {
                        taken.push(m.clone());
                    }
                }
                r.pos += 1;
                done += 1;
            }
            if let Some(book) = self.contacts.book.as_mut() {
                added |= !taken.is_empty();
                for m in &taken {
                    book.add(m, &mine);
                }
            }
            if r.pos >= n {
                self.contacts.marks.insert(r.folder, (r.validity, r.top));
                self.contacts.unsaved = true;
            } else {
                self.contacts.at = Some(r);
            }
        }
        let read = self.contacts.at.is_none() && self.contacts.todo.is_empty();
        let mut changed = added;
        if read && self.contacts.saved_changed {
            self.contacts.saved_changed = false;
            self.lay_saved_over();
            changed = true;
        }
        if changed {
            self.contacts.revision = self.contacts.revision.wrapping_add(1);
        }
        // Written now and then as it changes, and once a first reading is done.
        if self.contacts.unsaved && (self.clock.now.saturating_sub(self.contacts.saved_at) >= SAVE_EVERY || (read && busy)) {
            self.save_contacts();
        }
        if read && busy && self.focus_split() {
            self.refresh_list();
        }
        read
    }

    /// The next folder to read, with its mark: read whole without one; when its numbering
    /// changed (a folder reset), its mark only moves (counting it all again would count twice).
    fn contacts_begin(&mut self) -> Option<Reading> {
        let folder = self.contacts.todo.pop_front()?;
        let validity = self.folder_state(&folder.0, &folder.1).validity;
        let from = match self.contacts.marks.get(&folder) {
            Some(&(v, top)) if v == validity || top == 0 => Some(top),
            Some(_) => {
                let top = self.folder_state(&folder.0, &folder.1).messages.iter().map(|m| m.key.uid).max().unwrap_or(0);
                self.contacts.marks.insert(folder, (validity, top));
                self.contacts.unsaved = true;
                return self.contacts_begin();
            }
            None => None,
        };
        // A mark of nothing (a folder read before its first sync) reads it whole.
        let from = from.filter(|&top| top > 0);
        Some(Reading { folder, pos: 0, from, top: from.unwrap_or(0), validity })
    }

    /// Lay the people saved in Contacts over the book (ranked as if written to often).
    fn lay_saved_over(&mut self) {
        let Some(book) = self.contacts.book.as_mut() else { return };
        book.clear_saved();
        for p in self.pim.values() {
            for c in &p.cards {
                for e in &c.card.emails {
                    book.add_saved(&EmailAddress::new(c.card.display(), e.value.clone()));
                }
            }
        }
    }
}
