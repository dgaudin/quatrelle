// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! EWS: marks Outlook set before Quatrelle read them, asked for once rather than every folder
//! listed again (minutes on a big mailbox): the follow-ups marked complete.

use super::*;

/// Completed follow-ups asked for in one request at most (a page; more come in the next ones).
#[cfg(not(test))]
const TICKS_PAGE: usize = 1000;
#[cfg(test)]
const TICKS_PAGE: usize = 2;

/// The follow-ups of a folder marked complete in Outlook before this version read the mark
/// (`PidTagFlagStatus` 1, [`quatrelle_model::FOLLOW_UP_COMPLETE`]): a sync that asks only what
/// changed never brings them. The folder's completed messages are asked for alone, their ids and
/// marks only (the time it was done with them), and those kept here take them; nothing else is
/// read. How many changed.
#[allow(clippy::too_many_arguments)]
pub(super) fn catch_up_completed(
    c: &Client,
    store: &Store,
    st: &State,
    account: &str,
    folder: &str,
    id: &str,
    window: usize,
    since: Option<i64>,
) -> Result<usize> {
    let found = match c.completed(id, window, since, TICKS_PAGE) {
        Ok(f) => f,
        // A server that won't search so: they come as each message changes.
        Err(NetError::No(e)) => {
            log::info!("{account}: {}: completed follow-ups not asked for ({e})", utf7::decode(folder));
            return Ok(0);
        }
        Err(e) => return Err(e),
    };
    let changed = take_found(store, st, account, folder, &found)?;
    if changed > 0 {
        log::info!("{account}: {}: {changed} follow-ups marked complete in Outlook read", utf7::decode(folder));
    }
    Ok(changed)
}

/// What the server says of these messages (ids and marks), taken by those of a folder's kept
/// here and saved: how many changed.
pub(super) fn take_found(store: &Store, st: &State, account: &str, folder: &str, found: &[(String, Flags)]) -> Result<usize> {
    if found.is_empty() {
        return Ok(0);
    }
    let mut fs = store.load_folder(account, folder)?;
    let mut changed = std::collections::HashSet::new();
    for (item, f) in found {
        if let Some(m) = st.known(item).and_then(|n| fs.find_mut(n))
            && take_marks(&mut m.flags, f)
        {
            changed.insert(m.key.uid);
        }
    }
    if !changed.is_empty() {
        store.save_folder_changed(account, folder, &fs, &changed)?;
    }
    Ok(changed.len())
}
