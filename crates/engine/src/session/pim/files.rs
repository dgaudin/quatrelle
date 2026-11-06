// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The files kept with an appointment or a contact ([`quatrelle_model::pim::files`]): listed for
//! its window or page, and their bytes for Open and Save As (`event.saveFile`,
//! `contact.saveFile`). Only what the item carries is read: nothing is fetched.

use quatrelle_model::pim::files::{self, KeptFile};

use super::*;

/// A file's name when its line gives none (a file name, not interface text).
const NO_NAME: &str = "attachment";

impl Session {
    /// The event a window shows at `occurrence`: a series' changed occurrence when it carries
    /// files of its own (Outlook copies them to it, and an import keeps them there), else the
    /// series or the lone event.
    fn event_with_files(&self, uid: &str, occurrence: Option<i64>) -> Option<&CalEvent> {
        let (account, stored) = self.find_event(uid)?;
        let own = occurrence.filter(|_| stored.event.rrule.is_some()).and_then(|at| {
            self.pim.get(account)?.events.iter().find(|e| {
                e.calendar == stored.calendar
                    && e.event.uid == stored.event.uid
                    && e.event.recurrence_id == Some(at)
                    && files::event_has_files(&e.event)
            })
        });
        Some(own.map_or(&stored.event, |e| &e.event))
    }

    /// The files kept with an event (at `occurrence`, for a series).
    pub fn event_files(&self, uid: &str, occurrence: Option<i64>) -> Vec<KeptFile> {
        self.event_with_files(uid, occurrence).map(files::event_files).unwrap_or_default()
    }

    /// An event's file `index`: its name and bytes.
    pub fn event_file(&self, uid: &str, occurrence: Option<i64>, index: usize) -> Result<(String, Vec<u8>), CmdError> {
        let e = self.event_with_files(uid, occurrence).ok_or_else(|| CmdError::Failed(format!("no event `{uid}`")))?;
        let name = files::event_files(e).get(index).map(|f| f.name.clone()).ok_or_else(|| CmdError::Params(format!("no file {index}")))?;
        let data = files::event_file_data(e, index).ok_or_else(|| CmdError::Failed(format!("{name} can't be read")))?;
        Ok((named(name), data))
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

/// A file's name, or [`NO_NAME`] when it has none.
fn named(name: String) -> String {
    if name.trim().is_empty() { NO_NAME.to_string() } else { name }
}
