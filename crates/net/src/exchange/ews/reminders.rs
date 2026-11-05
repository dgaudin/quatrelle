// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Exchange's reminders as EWS gives them (issue #40): a `GetReminders` answer read (hostile:
//! capped, nothing in it trusted), the server's version as each answer says it
//! (`ServerVersionInfo`), and a request under Exchange 2013's version, which the reminders'
//! operations need (the version the rest asks for refuses them).

use super::*;

/// The most reminders read from one answer.
pub const MAX_REMINDERS: usize = 500;
/// The longest item id taken from it (Exchange's are a few hundred characters).
const MAX_ID: usize = 4096;

/// One reminder the server lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    /// Its item (an occurrence's own, for a series) and that item's version.
    pub item: String,
    pub change_key: String,
    /// Its series' item, for an occurrence (empty otherwise).
    pub master: String,
    /// The event's UID, when the server gives it.
    pub uid: String,
    /// The occurrence's start (Unix seconds).
    pub start: i64,
}

/// The reminders a `GetReminders` answer lists: those with an item and a start that can be read.
pub fn reminders_listed(doc: &El) -> Vec<Listed> {
    let Some(list) = doc.find("Reminders") else { return Vec::new() };
    let id = |e: Option<&El>| e.and_then(|e| e.attr("Id")).filter(|i| i.len() <= MAX_ID).unwrap_or("").to_string();
    list.children("Reminder")
        .take(MAX_REMINDERS)
        .filter_map(|r| {
            let item = id(r.child("ItemId"));
            let start = r.text_of("StartDate").and_then(|t| quatrelle_jmap::utc_date(t.trim()))?;
            let change_key = r.child("ItemId").and_then(|i| i.attr("ChangeKey")).filter(|k| k.len() <= MAX_ID).unwrap_or("").to_string();
            let uid = r.text_of("UID").map(str::trim).filter(|u| u.len() <= MAX_ID).unwrap_or("").to_string();
            (!item.is_empty()).then(|| Listed { item, change_key, master: id(r.child("RecurringMasterItemId")), uid, start })
        })
        .collect()
}

/// The server's major version as an answer says it (`ServerVersionInfo`: 15 for Exchange 2013
/// and later, 14 for 2010, 8 for 2007).
pub fn server_major(doc: &El) -> Option<u32> {
    doc.child("Envelope")?.child("Header")?.child("ServerVersionInfo")?.attr("MajorVersion")?.trim().parse().ok()
}

impl Client {
    /// The server's major version, once an answer said it.
    pub(crate) fn server_major(&self) -> Option<u32> {
        self.server_major.get()
    }

    /// The version an answer gives, noted the first time.
    pub(super) fn note_version(&self, doc: &El) {
        if self.server_major.get().is_none()
            && let Some(v) = server_major(doc)
        {
            self.server_major.set(Some(v));
        }
    }

    /// An operation of Exchange 2013 and later, in the user's own mailbox, asked under that
    /// version: the answer's elements. A busy server is for later.
    pub(crate) fn call_2013(&self, body: &str) -> Result<El> {
        match self.call_once_in(envelope_as("Exchange2013", body)) {
            Ok(doc) => Ok(doc),
            Err(Busy::Failed(e)) => Err(e),
            Err(Busy::Wait(_)) => Err(NetError::Later("Exchange is busy and asked to be left alone for a while".into())),
        }
    }
}
