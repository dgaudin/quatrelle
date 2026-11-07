// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What the fake EWS server keeps of the mail deleted (`DeleteItem`, `EmptyFolder`), as
//! Exchange does by the request's `DeleteType`: `SoftDelete` puts it in Recoverable Items
//! (Deletions), where Recover Deleted Items in Outlook and Outlook on the web brings it back
//! while the retention lasts; `HardDelete` keeps nothing the user can reach (Purges).

use super::*;

impl Data {
    /// Take the items `gone` picks out of their folders, as `op` asks.
    pub(super) fn take_items(&mut self, op: &El, gone: impl Fn(&It) -> bool) {
        let (taken, kept): (Vec<It>, Vec<It>) = std::mem::take(&mut self.items).into_iter().partition(|i| gone(i));
        self.items = kept;
        match op.attr("DeleteType") {
            Some("SoftDelete") => self.recoverable.extend(taken),
            Some("HardDelete") => self.purged.extend(taken),
            _ => {}
        }
    }
}
