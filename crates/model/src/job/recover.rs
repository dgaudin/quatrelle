// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What an Exchange server keeps of the items deleted for good (its Recoverable Items), as the
//! window "Recover deleted items" lists them.

use serde::{Deserialize, Serialize};

/// The most items one page of the list carries, and the most the window holds in all: the
/// server may keep thousands, and what it says is never trusted for its size.
pub const RECOVERABLE_PAGE: usize = 200;
pub const RECOVERABLE_MAX: usize = 20_000;

/// The sort of an item the server keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecoverableKind {
    #[default]
    Mail,
    Appointment,
    Contact,
    Task,
    Other,
}

impl RecoverableKind {
    /// The sort an item class says (`IPM.Note`, `IPM.Appointment`, `IPM.Contact`, `IPM.Task`…).
    pub fn of_class(class: &str) -> RecoverableKind {
        let c = class.trim().to_ascii_lowercase();
        let is = |p: &str| c == p || c.strip_prefix(p).is_some_and(|r| r.starts_with('.'));
        if c.is_empty() || is("ipm.note") || is("ipm.post") || is("report") || c.starts_with("report.") {
            RecoverableKind::Mail
        } else if is("ipm.appointment") || is("ipm.schedule") {
            RecoverableKind::Appointment
        } else if is("ipm.contact") || is("ipm.distlist") {
            RecoverableKind::Contact
        } else if is("ipm.task") {
            RecoverableKind::Task
        } else {
            RecoverableKind::Other
        }
    }
}

/// One item in the server's Recoverable Items.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recoverable {
    /// Its id on the server.
    pub id: String,
    pub kind: RecoverableKind,
    pub subject: String,
    /// Who it is from (a name or an address), when it has a sender.
    pub from: String,
    /// When it was deleted (Unix seconds), or last changed when the server doesn't tell; 0: unknown.
    pub deleted: i64,
    pub size: u64,
}
