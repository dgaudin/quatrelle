// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The session: the list of accounts that couldn't be read when it opened is never saved over, so
//! whatever would change the accounts is refused first.

use super::*;

/// What is said when the list of accounts couldn't be read and something would change it (the
/// reason follows, after a colon).
pub const ACCOUNTS_UNREAD: &str = "the list of accounts couldn't be read, so it isn't saved";

impl Session {
    /// `Err` while the list of accounts wasn't read: called first by whatever changes the accounts,
    /// before it touches anything in memory, so a refused change has nothing to undo.
    pub(crate) fn accounts_writable(&self) -> Result<(), CmdError> {
        match &self.accounts_unread {
            None => Ok(()),
            Some(why) => Err(CmdError::Failed(format!("{ACCOUNTS_UNREAD}: {why}"))),
        }
    }
}
