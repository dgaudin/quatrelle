//! An address book built from mail already in the cache, for To/Cc suggestions: Outlook's
//! AutoComplete list, kept on disk and added to as mail comes and goes.
//!
//! Every sender and recipient seen counts. People you write to (the recipients of your own
//! messages) rank well above people who only write to you, and recent contact counts more than
//! old: score = 4 · sent + received, halved for every year since the last message. People saved
//! in Contacts rank as if written to often. An address taken out of the suggestions (Outlook's
//! ×) stays out of them, whatever mail comes from it, until it's written to again.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::{EmailAddress, Summary};

/// One person.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Contact {
    pub address: EmailAddress,
    /// Messages you sent them.
    pub sent: u32,
    /// Messages from them (or with them in copy).
    pub received: u32,
    /// The newest message involving them (Unix seconds).
    pub last: i64,
    /// Saved in an address book (laid over the mail counted; not kept with the book).
    #[serde(skip)]
    pub saved: bool,
}

/// What being saved in Contacts counts for: as many messages written to them.
const SAVED_WEIGHT: f64 = 32.0;

impl Contact {
    pub fn score(&self, now: i64) -> f64 {
        let counted = 4.0 * f64::from(self.sent) + f64::from(self.received);
        // Someone saved in Contacts doesn't fade with time.
        if self.saved {
            return counted + SAVED_WEIGHT;
        }
        let years = (now.saturating_sub(self.last)).max(0) as f64 / (365.0 * 86_400.0);
        counted * 0.5f64.powf(years)
    }
}

/// Most addresses seen; a mailbox full of newsletters stays bounded.
const MAX_CONTACTS: usize = 20_000;

/// Most addresses taken out of the suggestions that are remembered.
const MAX_FORGOTTEN: usize = 5_000;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ContactBook {
    #[serde(default)]
    by_email: HashMap<String, Contact>,
    /// Taken out of the suggestions: mail from them doesn't bring them back, writing to them does.
    #[serde(default)]
    forgotten: BTreeSet<String>,
}

impl ContactBook {
    pub fn new() -> ContactBook {
        ContactBook::default()
    }

    pub fn len(&self) -> usize {
        self.by_email.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_email.is_empty()
    }

    /// What the book knows about an address.
    pub fn get(&self, email: &str) -> Option<&Contact> {
        self.by_email.get(&email.trim().to_ascii_lowercase())
    }

    fn note(&mut self, a: &EmailAddress, date: i64, sent: bool) {
        let email = a.email.trim().to_ascii_lowercase();
        if !crate::addr::is_valid_email(&email) || email.contains("noreply") || email.contains("no-reply") || email.starts_with("mailer-daemon@") {
            return;
        }
        // Taken out of the suggestions: back only once written to.
        if sent {
            self.forgotten.remove(&email);
        } else if self.forgotten.contains(&email) {
            return;
        }
        if self.by_email.len() >= MAX_CONTACTS && !self.by_email.contains_key(&email) {
            return;
        }
        let c = self.by_email.entry(email.clone()).or_insert_with(|| Contact { address: EmailAddress::new("", email), ..Default::default() });
        // The newest non-empty name wins (people fix their display names).
        if !a.name.trim().is_empty() && (c.address.name.is_empty() || date >= c.last) {
            c.address.name = a.name.trim().to_string();
        }
        if sent {
            c.sent = c.sent.saturating_add(1);
        } else {
            c.received = c.received.saturating_add(1);
        }
        c.last = c.last.max(date);
    }

    /// Someone saved in an address book: ranked as if written to often (see
    /// [`ContactBook::clear_saved`]: laid over what mail counted, and taken off again).
    pub fn add_saved(&mut self, a: &EmailAddress) {
        let email = a.email.trim().to_ascii_lowercase();
        // Taken out of the suggestions: out of them as one of the Contacts too (still in People).
        if !crate::addr::is_valid_email(&email)
            || self.forgotten.contains(&email)
            || (self.by_email.len() >= MAX_CONTACTS && !self.by_email.contains_key(&email))
        {
            return;
        }
        let c = self.by_email.entry(email.clone()).or_insert_with(|| Contact { address: EmailAddress::new("", email), ..Default::default() });
        if c.address.name.is_empty() && !a.name.trim().is_empty() {
            c.address.name = a.name.trim().to_string();
        }
        c.saved = true;
    }

    /// Take off what [`ContactBook::add_saved`] laid over the mail counted (the people saved
    /// changed): someone known only from Contacts goes.
    pub fn clear_saved(&mut self) {
        self.by_email.retain(|_, c| {
            c.saved = false;
            c.sent > 0 || c.received > 0
        });
    }

    /// Recipients of a message just sent: written to (and back in the suggestions if they were
    /// taken out). `mine`: the user's own addresses, not counted.
    pub fn add_sent<'a>(&mut self, to: impl IntoIterator<Item = &'a EmailAddress>, date: i64, mine: &[String]) {
        for a in to {
            if !mine.iter().any(|e| e.eq_ignore_ascii_case(a.email.trim())) {
                self.note(a, date, true);
            }
        }
    }

    /// Take an address out of the suggestions (Outlook's ×): gone from the book, and mail from it
    /// doesn't bring it back until it's written to. Whether it was there.
    pub fn forget(&mut self, email: &str) -> bool {
        let email = email.trim().to_ascii_lowercase();
        let was = self.by_email.remove(&email).is_some();
        if self.forgotten.len() >= MAX_FORGOTTEN
            && let Some(first) = self.forgotten.iter().next().cloned()
        {
            self.forgotten.remove(&first);
        }
        self.forgotten.insert(email);
        was
    }

    /// Count a message. `mine` are the user's own addresses: a message from one of them is one
    /// you sent, so its recipients are people you write to.
    pub fn add(&mut self, m: &Summary, mine: &[String]) {
        let from_me = mine.iter().any(|e| e.eq_ignore_ascii_case(m.from.email.trim()));
        if from_me {
            for a in m.to.iter().chain(&m.cc) {
                if !mine.iter().any(|e| e.eq_ignore_ascii_case(a.email.trim())) {
                    self.note(a, m.date, true);
                }
            }
        } else {
            self.note(&m.from, m.date, false);
        }
    }

    /// The domains of the people the user writes to (or saved in an address book): the ones a
    /// lookalike sender would imitate ([`crate::lookalike`]).
    pub fn written_domains(&self) -> impl Iterator<Item = String> + '_ {
        self.by_email.values().filter(|c| c.sent > 0).map(|c| crate::lookalike::domain_of(&c.address.email)).filter(|d| !d.is_empty())
    }

    /// Up to `limit` contacts matching `query` (the start of the address, of the name, or of any
    /// word of the name; case-insensitive), best first.
    pub fn suggest(&self, query: &str, limit: usize, now: i64) -> Vec<Contact> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<(&Contact, f64)> = self
            .by_email
            .values()
            .filter(|c| {
                let name = c.address.name.to_lowercase();
                c.address.email.starts_with(&q)
                    || name.starts_with(&q)
                    || name.split(|ch: char| ch.is_whitespace() || ch == '-' || ch == '.').any(|w| w.starts_with(&q))
                    || c.address.email.split(['.', '_', '-', '@']).any(|w| w.starts_with(&q))
            })
            .map(|c| (c, c.score(now)))
            .collect();
        hits.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.address.email.cmp(&b.0.address.email)));
        hits.into_iter().take(limit).map(|(c, _)| c.clone()).collect()
    }
}

impl ContactBook {
    /// Everyone known whose name or address contains `query` (everyone for an empty one), by
    /// name: the address book's list.
    pub fn list(&self, query: &str, limit: usize) -> Vec<Contact> {
        let q = query.trim().to_lowercase();
        let mut v: Vec<Contact> = self
            .by_email
            .values()
            .filter(|c| q.is_empty() || c.address.name.to_lowercase().contains(&q) || c.address.email.to_lowercase().contains(&q))
            .cloned()
            .collect();
        let key = |c: &Contact| if c.address.name.trim().is_empty() { c.address.email.to_lowercase() } else { c.address.name.to_lowercase() };
        v.sort_by_key(key);
        v.truncate(limit);
        v
    }
}

/// The address being typed in a recipient box: the text after the last `,` or `;`.
pub fn last_token(field: &str) -> &str {
    field.rsplit([',', ';']).next().unwrap_or("").trim_start()
}

/// `field` with its last, half-typed address replaced by `a` and a separator for the next one.
pub fn complete(field: &str, a: &EmailAddress) -> String {
    let keep = match field.rfind([',', ';']) {
        Some(i) => format!("{}, ", field.get(..i).unwrap_or("").trim_end()),
        None => String::new(),
    };
    format!("{keep}{a}, ")
}

/// Backspace right after a whole recipient (`Name <address>`, or one ended by `,` or `;`):
/// the field without it and where the cursor goes (characters), as Outlook takes a resolved name
/// away in one go. `None` when the cursor is inside a half-typed address (an ordinary backspace).
pub fn remove_recipient(field: &str, cursor: usize) -> Option<(String, usize)> {
    let chars: Vec<char> = field.chars().collect();
    let cursor = cursor.min(chars.len());
    // Separators outside quotes and angle brackets.
    let mut seps: Vec<usize> = Vec::new();
    let (mut quoted, mut angle) = (false, false);
    for (i, c) in chars.iter().enumerate().take(cursor) {
        match c {
            '"' if !angle => quoted = !quoted,
            '<' if !quoted => angle = true,
            '>' if !quoted => angle = false,
            ',' | ';' if !quoted && !angle => seps.push(i),
            _ => {}
        }
    }
    if quoted || angle {
        return None;
    }
    let mut end = cursor;
    while end > 0 && chars.get(end - 1).is_some_and(|c| c.is_whitespace()) {
        end -= 1;
    }
    let last = *chars.get(end.checked_sub(1)?)?;
    let entry_end = match last {
        ',' | ';' => end - 1,
        '>' => end,
        _ => return None,
    };
    let start = seps.iter().rev().find(|s| **s < entry_end).map_or(0, |s| s + 1);
    let entry: String = chars.get(start..entry_end)?.iter().collect();
    if entry.trim().is_empty() {
        return None;
    }
    // A plain address ended by a separator counts too; a half-typed word before `>` doesn't.
    if last == '>' && !entry.contains('<') {
        return None;
    }
    let keep_to = if start == 0 { 0 } else { start };
    let before: String = chars.get(..keep_to).unwrap_or(&[]).iter().collect();
    let after: String = chars.get(cursor..).unwrap_or(&[]).iter().collect();
    let before = if after.trim().is_empty() && keep_to > 0 { format!("{} ", before.trim_end()) } else { before };
    let at = before.chars().count();
    Some((format!("{before}{}", after.trim_start()), at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MessageKey;

    /// Issue #1: a wrong address picked from the suggestions goes with one Backspace.
    #[test]
    fn backspace_takes_a_whole_recipient() {
        let f = "Ana Lima <ana@x.example>, Zoe <zoe@x.example>, ";
        let (out, at) = remove_recipient(f, f.chars().count()).unwrap();
        assert_eq!(out, "Ana Lima <ana@x.example>, ");
        assert_eq!(at, out.chars().count());
        let (out, _) = remove_recipient(&out, out.chars().count()).unwrap();
        assert_eq!(out, "");
        let f = "\"Park, Zoe\" <zoe@x.example>";
        assert_eq!(remove_recipient(f, f.chars().count()).unwrap().0, "", "a comma in a quoted name isn't a separator");
        assert_eq!(remove_recipient("ana@x.example; ", 15).unwrap().0, "");
        // Half-typed, or inside an address: an ordinary backspace.
        assert_eq!(remove_recipient("Ana Lima <ana@x.example>, zo", 28), None);
        assert_eq!(remove_recipient("Ana <ana@x.exa", 14), None);
        assert_eq!(remove_recipient("", 0), None);
        assert_eq!(remove_recipient(", ", 2), None);
        // In the middle: the one before the cursor goes, the rest stays.
        let f = "Ana <a@x.example>, Zoe <z@x.example>";
        let (out, at) = remove_recipient(f, 18).unwrap();
        assert_eq!((out.as_str(), at), ("Zoe <z@x.example>", 0));
        assert_eq!(remove_recipient("abc", 99), None);
    }

    fn msg(from: (&str, &str), to: &[(&str, &str)], date: i64) -> Summary {
        Summary {
            key: MessageKey::default(),
            from: EmailAddress::new(from.0, from.1),
            to: to.iter().map(|(n, e)| EmailAddress::new(*n, *e)).collect(),
            date,
            ..Default::default()
        }
    }

    #[test]
    fn people_you_write_to_come_first() {
        let me = vec!["me@studio.example".to_string()];
        let mut b = ContactBook::new();
        let now = 2_000_000_000;
        for _ in 0..3 {
            b.add(&msg(("Zara Newsletter", "zara@news.example"), &[("Me", "me@studio.example")], now - 100), &me);
        }
        b.add(&msg(("Me", "me@studio.example"), &[("Zoe Park", "zoe@studio.example")], now - 100), &me);
        b.add(&msg(("Me", "me@studio.example"), &[("Zoe Park", "ZOE@studio.example")], now - 50), &me);
        b.add(&msg(("Robot", "noreply@x.example"), &[], now), &me);
        assert_eq!(b.len(), 2, "no-reply senders and my own address are left out");
        let s = b.suggest("z", 5, now);
        let emails: Vec<&str> = s.iter().map(|c| c.address.email.as_str()).collect();
        assert_eq!(emails, vec!["zoe@studio.example", "zara@news.example"]);
        assert_eq!(s[0].sent, 2);
        assert_eq!(s[0].address.name, "Zoe Park");
        // Any word of the name, or the domain, matches.
        assert_eq!(b.suggest("park", 5, now).len(), 1);
        assert_eq!(b.suggest("news", 5, now).len(), 1);
        assert!(b.suggest("  ", 5, now).is_empty());
        assert!(b.suggest("q", 5, now).is_empty());
    }

    #[test]
    fn old_contacts_fade() {
        let me = vec!["me@x.example".to_string()];
        let mut b = ContactBook::new();
        let year = 365 * 86_400;
        let now = 10 * year;
        b.add(&msg(("Me", "me@x.example"), &[("Ann Old", "ann.old@x.example")], now - 5 * year), &me);
        b.add(&msg(("Ann New", "ann.new@x.example"), &[], now), &me);
        let s = b.suggest("ann", 5, now);
        assert_eq!(s[0].address.email, "ann.new@x.example");
    }

    /// Outlook's ×: an address taken out stays out whatever mail comes from it, until it's
    /// written to. People saved in Contacts are a layer over the mail counted: they rank high,
    /// don't fade, come off again (someone known only from Contacts goes), and aren't kept with
    /// the book on disk.
    #[test]
    fn taken_out_saved_and_kept() {
        let me = vec!["me@x.example".to_string()];
        let mut b = ContactBook::new();
        let now = 2_000_000_000;
        b.add(&msg(("Bob Seller", "bob@shop.example"), &[], now), &me);
        assert!(b.forget("BOB@shop.example"));
        assert!(b.suggest("bob", 5, now).is_empty());
        b.add(&msg(("Bob Seller", "bob@shop.example"), &[], now), &me);
        assert!(b.suggest("bob", 5, now).is_empty(), "mail from it doesn't bring it back");
        b.add_sent(&[EmailAddress::new("Bob", "bob@shop.example"), EmailAddress::new("Me", "me@x.example")], now, &me);
        assert_eq!(b.suggest("bob", 5, now)[0].sent, 1, "written to: back");
        assert!(b.get("me@x.example").is_none(), "my own address isn't a contact");
        // Saved in Contacts, never written to: first, whatever the age.
        b.add(&msg(("Bea Often", "bea@x.example"), &[], now - 20 * 365 * 86_400), &me);
        b.add_saved(&EmailAddress::new("Bea Often", "bea@x.example"));
        b.add_saved(&EmailAddress::new("Ben Saved", "ben@x.example"));
        let s = b.suggest("b", 5, now);
        assert!(s[0].saved && s[1].saved, "{s:?}");
        // Kept on disk without the layer, and the layer comes off.
        let kept: ContactBook = serde_json::from_str(&serde_json::to_string(&b).unwrap()).unwrap();
        assert!(kept.get("bea@x.example").is_some_and(|c| !c.saved));
        b.clear_saved();
        assert!(b.get("ben@x.example").is_none(), "known only from Contacts: gone");
        assert!(b.get("bea@x.example").is_some_and(|c| !c.saved && c.received == 1));
        let kept: ContactBook = serde_json::from_str(&serde_json::to_string(&b).unwrap()).unwrap();
        kept.suggest("bob", 5, now).iter().for_each(|c| assert_eq!(c.address.email, "bob@shop.example"));
        let mut kept = kept;
        kept.forget("bea@x.example");
        kept.add_saved(&EmailAddress::new("Bea Often", "bea@x.example"));
        assert!(kept.get("bea@x.example").is_none(), "taken out: not suggested as one of the Contacts either");
        let again: ContactBook = serde_json::from_str(&serde_json::to_string(&kept).unwrap()).unwrap();
        let mut again = again;
        again.add(&msg(("Bea Often", "bea@x.example"), &[], now), &me);
        assert!(again.get("bea@x.example").is_none(), "taken out, kept taken out on disk");
    }

    #[test]
    fn completing_the_last_address() {
        assert_eq!(last_token("Ann <a@x>, zo"), "zo");
        assert_eq!(last_token("zo"), "zo");
        assert_eq!(last_token("a@x; "), "");
        let z = EmailAddress::new("Zoe Park", "zoe@x.example");
        assert_eq!(complete("Ann <a@x>, zo", &z), "Ann <a@x>, Zoe Park <zoe@x.example>, ");
        assert_eq!(complete("zo", &z), "Zoe Park <zoe@x.example>, ");
    }
}
