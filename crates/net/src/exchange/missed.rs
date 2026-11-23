// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What an Exchange folder listing (EWS or Graph) couldn't list this time — a shared mailbox, the
//! public folders, the online archive — said beside the folders it did list, so that what such a
//! zone held isn't taken for gone.

use std::collections::HashSet;

use quatrelle_model::Folder;

/// A folder listing: the folders, and what it couldn't list.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Listed {
    pub folders: Vec<Folder>,
    pub missed: Missed,
}

/// What a listing couldn't say, so that nothing of it is taken for gone.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Missed {
    /// Shared mailboxes asked for whose folders didn't list (refused, empty, cut), by address.
    pub shared: Vec<String>,
    /// The mailboxes the account opens couldn't be told this time (Autodiscover failed): any
    /// shared mailbox listed before may still be there.
    pub shared_unknown: bool,
    pub public: PublicMissed,
    /// The online archive didn't list, or couldn't be asked about.
    pub archive: bool,
}

/// What of the public folders didn't list.
#[derive(Debug, Default, Clone, PartialEq)]
pub enum PublicMissed {
    /// They listed.
    #[default]
    None,
    /// None of them listed: their list refused or empty, or the mailbox holding it couldn't be
    /// asked for. A server without public folders answers so too.
    All,
    /// The folders under these (their ids on the server) didn't list whole.
    Under(HashSet<String>),
}

impl Missed {
    /// Whether anything didn't list.
    pub fn any(&self) -> bool {
        self.zones() > 0
    }

    /// How many zones didn't list: each shared mailbox, the mailboxes the account opens when they
    /// couldn't be told, the public folders (or each of their branches), the archive. A count, for
    /// the log: it names nothing.
    pub fn zones(&self) -> usize {
        let public = match &self.public {
            PublicMissed::None => 0,
            PublicMissed::All => 1,
            PublicMissed::Under(parents) => parents.len(),
        };
        self.shared.len().saturating_add(usize::from(self.shared_unknown)).saturating_add(public).saturating_add(usize::from(self.archive))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A branch set with no parent in it misses nothing; each zone counts once.
    #[test]
    fn what_was_missed_is_counted_by_zone() {
        assert!(!Missed::default().any());
        assert!(!Missed { public: PublicMissed::Under(HashSet::new()), ..Default::default() }.any());
        let all = Missed {
            shared: vec!["sales@contoso.example".into(), "team@contoso.example".into()],
            shared_unknown: true,
            public: PublicMissed::Under(["P1".to_string(), "P2".to_string()].into()),
            archive: true,
        };
        assert_eq!(all.zones(), 6);
        assert_eq!(Missed { public: PublicMissed::All, ..Default::default() }.zones(), 1);
    }
}
