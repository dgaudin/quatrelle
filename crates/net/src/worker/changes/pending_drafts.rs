// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! An account's worker: drafts that couldn't be saved to the server, kept on this computer until
//! it connects again, and taken off the list one by one as each is filed.

use super::*;

/// Drafts that couldn't be saved to the server (offline, or it failed), kept with the account
/// until it connects again.
pub(in crate::worker) const PENDING_DRAFTS: &str = "drafts-pending.json";

#[derive(serde::Serialize, serde::Deserialize)]
pub(in crate::worker) struct PendingDraft {
    pub(in crate::worker) folder: String,
    #[serde(default)]
    pub(in crate::worker) replaces: Option<quatrelle_model::MessageKey>,
    pub(in crate::worker) raw: Vec<u8>,
}

/// Keep a draft on this computer for the next connection. A list that can't be read stops it:
/// saved as just this one, the drafts waiting in it would be lost.
pub(in crate::worker) fn keep_draft(
    store: &Store,
    account: &str,
    folder: &str,
    replaces: Option<quatrelle_model::MessageKey>,
    raw: Vec<u8>,
) -> Result<()> {
    let mut list: Vec<PendingDraft> = store.load_account_doc(account, PENDING_DRAFTS)?;
    // A newer version of the same draft replaces the one kept.
    if let Some(r) = &replaces {
        list.retain(|d| d.replaces.as_ref() != Some(r));
    }
    // The same version kept again (filing it failed), or the same draft closed once more with
    // no copy known to replace (its Message-ID, as the engine gives its copies the one it
    // went by): one entry, not two.
    let id = draft_id(&raw);
    list.retain(|d| d.raw != raw && (id.is_none() || draft_id(&d.raw) != id));
    list.push(PendingDraft { folder: folder.to_string(), replaces, raw });
    store.save_account_doc(account, PENDING_DRAFTS, &list).map_err(NetError::from)
}

/// The Message-ID of a draft's source, when it has one.
fn draft_id(raw: &[u8]) -> Option<String> {
    quatrelle_mime::parse(raw).map(|p| p.message_id).filter(|id| !id.trim().is_empty())
}

/// A draft filed on the server leaves the list kept, and only then (the list is emptied from
/// the disk one draft at a time, as each is filed). One that can't be taken off stays, and
/// would be filed again: logged, never fatal.
pub(in crate::worker) fn forget_draft(store: &Store, account: &str, raw: &[u8]) {
    let result = store.load_account_doc::<Vec<PendingDraft>>(account, PENDING_DRAFTS).and_then(|mut list| {
        let before = list.len();
        list.retain(|d| d.raw != raw);
        if list.len() == before { Ok(()) } else { store.save_account_doc(account, PENDING_DRAFTS, &list) }
    });
    if let Err(e) = result {
        log::warn!("{account}: the draft filed couldn't be taken off the list kept: {e}");
    }
}
