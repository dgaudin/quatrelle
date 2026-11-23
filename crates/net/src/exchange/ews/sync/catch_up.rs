// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! EWS: marks Outlook set before Quatrelle read them, asked for once rather than every folder
//! listed again (minutes on a big mailbox): the follow-ups marked complete, the messages replied
//! to or forwarded, and what mail kept before is (a meeting message, a report…).

use super::*;

/// Messages asked for in one request at most (a page; more come in the next ones).
#[cfg(not(test))]
const TICKS_PAGE: usize = 1000;
#[cfg(test)]
const TICKS_PAGE: usize = 2;

/// Each pass a folder is still to have, once (a folder with nothing kept yet has nothing to
/// read: it's listed in full): how many messages changed. A pass done is noted at once, so one
/// cut short after it (offline) isn't made again. The pass for kinds never stops the folder being
/// read: failed, it waits for Quatrelle's next start.
#[allow(clippy::too_many_arguments)]
pub(super) fn catch_up_once(
    c: &Client,
    store: &Store,
    st: &mut State,
    account: &str,
    folder: &str,
    id: &str,
    kept: bool,
    window: usize,
    since: Option<i64>,
) -> Result<usize> {
    let mut changed = 0;
    if st.ticks_wanted(folder) {
        if kept {
            changed += catch_up_completed(c, store, st, account, folder, id, window, since)?;
        }
        st.ticks_read(folder);
    }
    if st.replies_wanted(folder) {
        if kept {
            changed += catch_up_replied(c, store, st, account, folder, id, window, since)?;
        }
        st.replies_read(folder);
    }
    if st.kinds_due(folder) {
        if !kept {
            st.kinds_read(folder);
        } else {
            match catch_up_kinds(c, store, st, account, folder, id, window, since) {
                Ok(n) => {
                    changed += n;
                    st.kinds_read(folder);
                }
                // A big folder timing out, a connection cut: read all the same (its new mail
                // matters more), asked again when Quatrelle next starts, not at every sync.
                Err(e) => {
                    log::warn!("{account}: what messages kept are couldn't be asked of a folder ({}); asked again at the next start", failure(&e));
                    st.kinds_failed(folder);
                }
            }
        }
    }
    Ok(changed)
}

/// What mail of a folder kept before this version read it is (a meeting message, a report, an
/// automatic reply, encrypted…), and whether it was replied to all (`PidTagLastVerbExecuted` 103,
/// read as a reply before): the messages that may be so asked for alone ([`Client::kinds`]), their
/// ids, marks and kinds only, a page of a thousand at a time, and those kept here take them;
/// nothing is downloaded. The others are ordinary mail as far as Exchange says: their sources
/// will tell. How many changed.
#[allow(clippy::too_many_arguments)]
fn catch_up_kinds(c: &Client, store: &Store, st: &State, account: &str, folder: &str, id: &str, window: usize, since: Option<i64>) -> Result<usize> {
    let found = match c.kinds(id, window, since, TICKS_PAGE) {
        Ok(f) => f,
        // A server that won't search so: the sources tell.
        Err(NetError::No(_)) => {
            log::info!("{account}: a folder can't be searched for what its messages are; their sources will tell");
            return Ok(0);
        }
        Err(e) => return Err(e),
    };
    let changed = take_kinds(store, st, account, folder, &found)?;
    if changed > 0 {
        log::info!("{account}: what {changed} messages kept are read, of {} found", found.len());
    }
    Ok(changed)
}

/// What kind of failure it was, in a word: the log names no folder, nor what the server said.
fn failure(e: &NetError) -> &'static str {
    match e {
        NetError::Io(_) | NetError::Tls(_) => "network",
        NetError::Closed(_) => "connection closed",
        NetError::Later(_) => "server busy",
        NetError::Protocol(_) => "answer not understood",
        NetError::Auth(_) => "sign-in",
        NetError::Store(_) => "cache",
        _ => "other",
    }
}

/// What the pass for kinds found, taken by the messages of a folder's kept here: their marks
/// ([`take_marks`]), their kind when the server says more than mail
/// ([`quatrelle_model::MessageKind::told_by_server`]), what protects them when nothing did. Only
/// the pages changed are written; how many messages changed.
fn take_kinds(store: &Store, st: &State, account: &str, folder: &str, found: &[Found]) -> Result<usize> {
    if found.is_empty() {
        return Ok(0);
    }
    let mut fs = store.load_folder(account, folder)?;
    // Where each is: a big folder's thousands found aren't each looked for through the list.
    let at: HashMap<u32, usize> = fs.messages.iter().enumerate().map(|(i, m)| (m.key.uid, i)).collect();
    let mut changed = std::collections::HashSet::new();
    for f in found {
        let Some(m) = st.known(&f.id).and_then(|n| at.get(&n)).and_then(|i| fs.messages.get_mut(*i)) else { continue };
        let marks = take_marks(&mut m.flags, &f.flags);
        let kind = m.kind.told_by_server(f.kind);
        let smime = if m.smime.is_none() { f.smime } else { m.smime };
        if marks || kind != m.kind || smime != m.smime {
            (m.kind, m.smime) = (kind, smime);
            changed.insert(m.key.uid);
        }
    }
    if !changed.is_empty() {
        store.save_folder_changed(account, folder, &fs, &changed)?;
    }
    Ok(changed.len())
}

/// The messages of a folder replied to or forwarded in Outlook before this version read the
/// verb (`PidTagLastVerbExecuted` 102 to 104): asked for alone, their ids and marks only (the
/// time it was done with them), a page of a thousand at a time, and those kept here take them;
/// nothing else is read. How many changed.
#[allow(clippy::too_many_arguments)]
fn catch_up_replied(
    c: &Client,
    store: &Store,
    st: &State,
    account: &str,
    folder: &str,
    id: &str,
    window: usize,
    since: Option<i64>,
) -> Result<usize> {
    let found = match c.replied(id, window, since, TICKS_PAGE) {
        Ok(f) => f,
        // A server that won't search so: they come as each message changes.
        Err(NetError::No(e)) => {
            log::info!("{account}: {}: replies and forwards not asked for ({e})", utf7::decode(folder));
            return Ok(0);
        }
        Err(e) => return Err(e),
    };
    let changed = take_found(store, st, account, folder, &found)?;
    if changed > 0 {
        log::info!("{account}: {}: {changed} messages replied to or forwarded in Outlook read", utf7::decode(folder));
    }
    Ok(changed)
}

/// The follow-ups of a folder marked complete in Outlook before this version read the mark
/// (`PidTagFlagStatus` 1, [`quatrelle_model::FOLLOW_UP_COMPLETE`]): a sync that asks only what
/// changed never brings them. The folder's completed messages are asked for alone, their ids and
/// marks only (the time it was done with them), and those kept here take them; nothing else is
/// read. How many changed.
#[allow(clippy::too_many_arguments)]
fn catch_up_completed(
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
