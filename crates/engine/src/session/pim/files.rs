// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The files kept with an appointment or a contact ([`quatrelle_model::pim::files`]): listed for
//! its window or page, and their bytes for Open and Save As (`event.saveFile`,
//! `contact.saveFile`). Only what the item carries is read, but for an Exchange appointment's file
//! left on its server (R25): asked of it when it's opened or saved ([`Job::FetchEventFile`]), and
//! written where asked when it comes, as Outlook's Cached Exchange Mode fetches a file not yet
//! there. A file fetched stays in memory for the run (within [`FETCHED_MAX`]): opened again, it
//! isn't asked for again.

use quatrelle_model::pim::files::{self, KeptFile};

use super::*;

/// A file's name when its line gives none (a file name, not interface text).
const NO_NAME: &str = "attachment";

/// The bytes of the files fetched this run kept in memory, at most (the oldest go first).
const FETCHED_MAX: usize = 64 << 20;

/// Files on their way at most: past it, the oldest request is let go.
const WAITING_MAX: usize = 64;

/// Appointments' files asked of the server, and what's to be done with each when it comes.
#[derive(Debug, Default)]
pub struct FilesComing {
    next: u64,
    waiting: Vec<Coming>,
    /// Those fetched this run, by account and what the server knows them by ([`held`]), the
    /// latest last.
    fetched: Vec<(String, Vec<u8>)>,
    /// Files written to be opened: the interface hands each to the system.
    to_open: Vec<String>,
}

/// A file asked of the server: where it's to be written, and whether opened then.
#[derive(Debug)]
struct Coming {
    ask: u64,
    /// The file: its account and what the server knows it by ([`held`]).
    key: String,
    name: String,
    path: String,
    open: bool,
}

impl FilesComing {
    fn fetched(&self, key: &str) -> Option<&[u8]> {
        self.fetched.iter().rev().find(|(k, _)| !key.is_empty() && k == key).map(|(_, d)| d.as_slice())
    }

    fn keep(&mut self, key: String, data: Vec<u8>) {
        self.fetched.retain(|(k, _)| *k != key);
        if data.len() > FETCHED_MAX {
            return;
        }
        self.fetched.push((key, data));
        while self.fetched.iter().map(|(_, d)| d.len()).sum::<usize>() > FETCHED_MAX {
            self.fetched.remove(0);
        }
    }
}

impl Session {
    /// The event a window shows at `occurrence`: a series' changed occurrence when it carries
    /// files of its own (Outlook copies them to it, and an import keeps them there), else the
    /// series or the lone event; with its account.
    fn stored_with_files(&self, uid: &str, occurrence: Option<i64>) -> Option<(&str, &StoredEvent)> {
        let (account, stored) = self.find_event(uid)?;
        let own = occurrence.filter(|_| stored.event.rrule.is_some()).and_then(|at| {
            self.pim.get(account)?.events.iter().find(|e| {
                e.calendar == stored.calendar
                    && e.event.uid == stored.event.uid
                    && e.event.recurrence_id == Some(at)
                    && files::event_has_files(&e.event)
            })
        });
        Some((account, own.unwrap_or(stored)))
    }

    fn event_with_files(&self, uid: &str, occurrence: Option<i64>) -> Option<&CalEvent> {
        self.stored_with_files(uid, occurrence).map(|(_, e)| &e.event)
    }

    /// The files kept with an event (at `occurrence`, for a series).
    pub fn event_files(&self, uid: &str, occurrence: Option<i64>) -> Vec<KeptFile> {
        self.event_with_files(uid, occurrence).map(files::event_files).unwrap_or_default()
    }

    /// An event's file `index`: its name and bytes (one on the server, once fetched this run).
    pub fn event_file(&self, uid: &str, occurrence: Option<i64>, index: usize) -> Result<(String, Vec<u8>), CmdError> {
        let (account, stored) = self.stored_with_files(uid, occurrence).ok_or_else(|| CmdError::Failed(format!("no event `{uid}`")))?;
        let e = &stored.event;
        let name = files::event_files(e).get(index).map(|f| f.name.clone()).ok_or_else(|| CmdError::Params(format!("no file {index}")))?;
        if let Some(reference) = files::event_file_reference(e, index) {
            return match self.files_coming.fetched(&held(account, stored, &reference)) {
                Some(data) => Ok((named(name), data.to_vec())),
                None => Err(CmdError::Failed(format!("{} is on the server: open it to download it", named(name)))),
            };
        }
        let data = files::event_file_data(e, index).ok_or_else(|| CmdError::Failed(format!("{name} can't be read")))?;
        Ok((named(name), data))
    }

    /// An event's file `index` left on its server, asked of it, to be written to `path` (and
    /// opened, `open`) when it comes: its name. `None`: it's here (or fetched already), to be
    /// written at once. Only an appointment of an account's calendar, read from its server, has
    /// one there (an imported or subscribed one names nothing the account's server holds).
    pub(crate) fn fetch_event_file(
        &mut self,
        uid: &str,
        occurrence: Option<i64>,
        index: usize,
        path: &str,
        open: bool,
    ) -> Result<Option<String>, CmdError> {
        let (account, stored) = self.stored_with_files(uid, occurrence).ok_or_else(|| CmdError::Failed(format!("no event `{uid}`")))?;
        let Some(reference) = files::event_file_reference(&stored.event, index) else { return Ok(None) };
        let name = named(files::event_files(&stored.event).get(index).map(|f| f.name.clone()).unwrap_or_default());
        let key = held(account, stored, &reference);
        if key.is_empty() {
            return Err(CmdError::Failed(format!("{name} isn't on this computer, and no server of this account holds it")));
        }
        if self.files_coming.fetched(&key).is_some() {
            return Ok(None);
        }
        let account = account.to_string();
        let fc = &mut self.files_coming;
        // Asked again for the same place while on its way: once is enough.
        if !fc.waiting.iter().any(|c| c.key == key && c.path == path) {
            fc.next = fc.next.wrapping_add(1);
            let ask = fc.next;
            if fc.waiting.len() >= WAITING_MAX {
                fc.waiting.remove(0);
            }
            fc.waiting.push(Coming { ask, key, name: name.clone(), path: path.to_string(), open });
            self.jobs.push(Job::FetchEventFile { account, reference, ask });
        }
        self.notice_with("Downloading {name}…", &[("name", &name)], false);
        Ok(Some(name))
    }

    /// A file asked of the server came ([`Event::EventFile`]): written where it was to go, then
    /// opened or said saved; or why it couldn't be.
    pub(crate) fn event_file_came(&mut self, ask: u64, data: Vec<u8>, error: Option<String>) {
        let fc = &mut self.files_coming;
        let Some(at) = fc.waiting.iter().position(|c| c.ask == ask) else { return };
        let c = fc.waiting.remove(at);
        if let Some(e) = error {
            self.notice_with("{name} could not be downloaded: {error}", &[("name", &c.name), ("error", &e)], true);
            return;
        }
        if let Err(e) = std::fs::write(&c.path, &data) {
            self.notice_with("{name} could not be downloaded: {error}", &[("name", &c.name), ("error", &e.to_string())], true);
            return;
        }
        crate::cmd::mark_from_internet(std::path::Path::new(&c.path));
        self.files_coming.keep(c.key, data);
        if c.open {
            self.files_coming.to_open.push(c.path);
        } else {
            self.notice_with("Saved {path}", &[("path", &c.path)], false);
        }
    }

    /// The files written to be opened since the last call: the interface opens them.
    pub fn take_files_to_open(&mut self) -> Vec<String> {
        std::mem::take(&mut self.files_coming.to_open)
    }

    /// The files kept with a contact.
    pub fn card_files(&self, uid: &str) -> Vec<KeptFile> {
        self.find_card(uid).map(files::card_files).unwrap_or_default()
    }

    /// A contact's file `index`: its name and bytes.
    pub fn card_file(&self, uid: &str, index: usize) -> Result<(String, Vec<u8>), CmdError> {
        let c = self.find_card(uid).ok_or_else(|| CmdError::Failed(format!("no contact `{uid}`")))?;
        let name = files::card_files(c).get(index).map(|f| f.name.clone()).ok_or_else(|| CmdError::Params(format!("no file {index}")))?;
        let data = files::card_file_data(c, index).ok_or_else(|| CmdError::Failed(format!("{name} can't be read")))?;
        Ok((named(name), data))
    }

    /// A contact's card, without copying it.
    fn find_card(&self, uid: &str) -> Option<&Card> {
        self.pim.values().find_map(|p| p.cards.iter().find(|c| c.card.uid == uid).map(|c| &c.card))
    }
}

/// What tells a file on the server apart: its account and what the server knows it by; empty
/// for an appointment no server of the account holds (imported, subscribed to by its link),
/// whose line names nothing to fetch.
fn held(account: &str, stored: &StoredEvent, reference: &str) -> String {
    if stored.href.is_empty() || stored.calendar == LOCAL || quatrelle_model::pim::is_web_calendar(&stored.calendar) {
        return String::new();
    }
    format!("{account}\n{reference}")
}

/// A file's name, or [`NO_NAME`] when it has none.
fn named(name: String) -> String {
    if name.trim().is_empty() { NO_NAME.to_string() } else { name }
}
