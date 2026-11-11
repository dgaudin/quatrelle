// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Calendar and contacts files opened with Quatrelle (`.ics`, `.vcf`): a stranger's text. What a
//! file holds is read and said first ([`Session::file_events`], [`Session::file_cards`]); it is
//! added only once the user says so. What is added invites nobody (an event's guests and
//! organizer stay in the file, so no server sends anything in the user's name), replaces nothing
//! (an event or a person already there under the same identifier is left as it is, and counted),
//! and is bounded ([`MAX_FILE_EVENTS`], [`MAX_FILE_CARDS`]).

use std::collections::HashSet;

use super::*;

/// Events added from one calendar file, at most.
pub const MAX_FILE_EVENTS: usize = 500;
/// People added from one contacts file, at most.
pub const MAX_FILE_CARDS: usize = 2000;

/// What a file would add, and where.
#[derive(Debug)]
pub(crate) struct FromFile<T> {
    /// What would be added, in the file's order.
    pub items: Vec<T>,
    /// Those passed over because they can't be read as an event or a person.
    pub unreadable: usize,
    /// Those passed over because one with the same identifier is already there.
    pub already: usize,
    /// Those past the most one file adds.
    pub more: usize,
    /// The account they go to, and its calendar or address book: its id, and its name.
    pub account: String,
    pub place: String,
    pub place_name: String,
}

impl<T> FromFile<T> {
    fn new(account: String, place: String, place_name: String) -> FromFile<T> {
        FromFile { items: Vec::new(), unreadable: 0, already: 0, more: 0, account, place, place_name }
    }

    /// One of the file's: kept unless it was seen, or the file has given all it may.
    fn take(&mut self, item: T, seen: bool, most: usize) {
        if seen {
            self.already = self.already.saturating_add(1);
        } else if self.items.len() >= most {
            self.more = self.more.saturating_add(1);
        } else {
            self.items.push(item);
        }
    }
}

/// An event as a file gives it, made the user's own note of it: nobody is invited to it and
/// nobody else organizes it, and no line that names a guest is kept (an alarm that sends mail
/// names its own).
fn own_event(e: &mut CalEvent) {
    e.organizer = None;
    e.attendees.clear();
    e.no_responses = false;
    e.no_new_times = false;
    // A file's Webex meeting is not the user's.
    e.webex_id = None;
    let mut depth = 0usize;
    e.extra.retain(|line| {
        let name = line.split([':', ';']).next().unwrap_or_default().trim().to_ascii_uppercase();
        match name.as_str() {
            "BEGIN" => {
                depth = depth.saturating_add(1);
                false
            }
            "END" => {
                depth = depth.saturating_sub(1);
                false
            }
            "ATTENDEE" | "ORGANIZER" => false,
            _ => depth == 0,
        }
    });
}

impl Session {
    /// What a calendar file's text would add to the calendar of the account on screen.
    pub(crate) fn file_events(&mut self, text: &str) -> Result<FromFile<CalEvent>, CmdError> {
        let account = self.pim_account(None)?;
        let offset = i64::from(self.clock.offset);
        let p = self.pim_mut(&account);
        let cal = p.default_calendar().ok_or_else(|| CmdError::Disabled("no calendar to add to".into()))?;
        let mut held = FromFile::new(account.clone(), cal.id.clone(), cal.name.clone());
        // An occurrence changed in a series shares the series' identifier: both are the file's.
        let there: HashSet<&str> = p.events.iter().map(|e| e.event.uid.as_str()).collect();
        let mut seen: HashSet<(String, Option<i64>)> = HashSet::new();
        for mut e in ical::parse(text, offset).events {
            if check_event(&e).is_err() {
                held.unreadable = held.unreadable.saturating_add(1);
                continue;
            }
            let uid = e.uid.trim().to_string();
            let known = !uid.is_empty() && (there.contains(uid.as_str()) || !seen.insert((uid, e.recurrence_id)));
            own_event(&mut e);
            held.take(e, known, MAX_FILE_EVENTS);
        }
        Ok(held)
    }

    /// Add what [`Session::file_events`] read; saved, and sent to the calendar's server, once.
    /// Answers how many went in and when the first one starts.
    pub(crate) fn add_file_events(&mut self, held: FromFile<CalEvent>) -> (usize, Option<i64>) {
        let FromFile { items, account, place, .. } = held;
        let remote = place != LOCAL;
        let first = items.first().map(|e| e.start);
        let n = items.len();
        for mut event in items {
            if event.uid.trim().is_empty() {
                event.uid = self.new_uid();
            }
            self.pim_mut(&account).events.push(StoredEvent {
                calendar: place.clone(),
                href: String::new(),
                etag: String::new(),
                event,
                dirty: remote,
                notify: false,
                respond: None,
                changed_too: false,
            });
        }
        if n > 0 {
            self.after_pim_change(&account, remote);
        }
        (n, first)
    }

    /// What a contacts file's text would add to the address book of the account on screen.
    pub(crate) fn file_cards(&mut self, text: &str) -> Result<FromFile<Card>, CmdError> {
        let account = self.pim_account(None)?;
        let p = self.pim_mut(&account);
        let book = p.default_book().ok_or_else(|| CmdError::Disabled("no address book to add to".into()))?;
        let mut held = FromFile::new(account.clone(), book.id.clone(), book.name.clone());
        let there: HashSet<&str> = p.cards.iter().map(|c| c.card.uid.as_str()).collect();
        let mut seen: HashSet<String> = HashSet::new();
        for card in quatrelle_model::vcard::parse(text) {
            if people::check_card(&card).is_err() {
                held.unreadable = held.unreadable.saturating_add(1);
                continue;
            }
            let uid = card.uid.trim().to_string();
            let known = !uid.is_empty() && (there.contains(uid.as_str()) || !seen.insert(uid));
            held.take(card, known, MAX_FILE_CARDS);
        }
        Ok(held)
    }

    /// Add what [`Session::file_cards`] read; saved, and sent to the address book's server, once.
    pub(crate) fn add_file_cards(&mut self, held: FromFile<Card>) -> usize {
        let FromFile { items, account, place, .. } = held;
        let remote = place != LOCAL;
        let n = items.len();
        for mut card in items {
            if card.uid.trim().is_empty() {
                card.uid = self.new_uid();
            }
            self.pim_mut(&account).cards.push(StoredCard { book: place.clone(), href: String::new(), etag: String::new(), card, dirty: remote });
        }
        if n > 0 {
            self.after_pim_change(&account, remote);
        }
        n
    }
}
