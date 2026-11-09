// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The session: the online archive is read-only in this version. One place decides
//! ([`Session::read_only_folder`]); every path that moves, deletes, archives, files as junk or
//! changes folders asks it first, before anything is changed.

use super::*;

/// What every refused change in the online archive says.
pub const ARCHIVE_READ_ONLY: &str = "the online archive is read-only in this version: use Outlook to move or delete its mail";

fn refusal() -> CmdError {
    CmdError::Disabled(ARCHIVE_READ_ONLY.into())
}

impl Session {
    /// Whether nothing may be changed in this folder but a read mark: a folder of the online archive.
    pub fn read_only_folder(&self, account: &str, folder: &str) -> bool {
        self.in_online_archive(account, folder)
    }

    /// Whether any of the selected messages is in the online archive (a search can mix it with
    /// the mailbox's own mail: the whole selection is then read-only).
    pub fn selection_read_only(&self) -> bool {
        self.selection.iter().any(|k| self.read_only_folder(&k.account, &k.folder))
    }

    /// Refuse a change to these messages when any is in the archive, so nothing is changed.
    pub(crate) fn refuse_archive_mail<'a>(&self, keys: impl IntoIterator<Item = &'a MessageKey>) -> Result<(), CmdError> {
        if keys.into_iter().any(|k| self.read_only_folder(&k.account, &k.folder)) { Err(refusal()) } else { Ok(()) }
    }

    /// Refuse a change that touches this folder of the account when it is in the archive.
    pub(crate) fn refuse_archive_folder(&self, account: &str, folder: &str) -> Result<(), CmdError> {
        if self.read_only_folder(account, folder) { Err(refusal()) } else { Ok(()) }
    }

    /// Refuse the selection when any of it is in the archive.
    pub(crate) fn refuse_archive_selection(&self) -> Result<(), CmdError> {
        if self.selection_read_only() { Err(refusal()) } else { Ok(()) }
    }

    /// Whether a Quick Step that moves mail can't run on the selection: some of it is in the
    /// archive, or the step files it in a folder of the archive. A step that only marks can.
    pub(crate) fn quick_step_refused(&self, actions: &[quatrelle_model::organize::RuleAction]) -> bool {
        use quatrelle_model::organize::RuleAction;
        let moves = actions.iter().any(|a| matches!(a, RuleAction::MoveTo(_) | RuleAction::MoveToRole(_) | RuleAction::Delete));
        if !moves {
            return false;
        }
        let into = actions.iter().any(|a| match a {
            RuleAction::MoveTo(f) => self.selection.iter().any(|k| self.read_only_folder(&k.account, f)),
            _ => false,
        });
        self.selection_read_only() || into
    }
}
