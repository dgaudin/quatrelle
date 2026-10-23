//! Downloading ahead: kept messages' whole sources, so they read (and are searched) offline.

use std::collections::HashSet;

use super::*;

/// Download these messages' sources, ten to a `UID FETCH` and no more than one literal's worth
/// together ([`body_chunks`]: each command stays well inside IMAP's per-command cap), filling in
/// what only a source tells (preview, attachments) and saving the list after each. How many came.
pub(super) fn download<S: Read + Write>(
    c: &mut Client<S>,
    store: &Store,
    account: &str,
    folder: &str,
    st: &mut FolderState,
    uids: &[u32],
) -> Result<usize> {
    let mut n = 0;
    let sized: Vec<(u32, u32)> = uids.iter().map(|u| (*u, st.find(*u).map_or(0, |m| m.size))).collect();
    for chunk in body_chunks(&sized) {
        let mut came = HashSet::new();
        for f in c.uid_fetch(&uid_set(&chunk), "UID BODY.PEEK[]", "")? {
            let Some(raw) = &f.body else { continue };
            let Some(m) = st.find_mut(f.uid) else { continue };
            store.write_body(&m.key, raw)?;
            fill_from_source(m, raw);
            came.insert(f.uid);
        }
        // Only these messages changed: a big folder's other pages stand.
        store.save_folder_changed(account, folder, st, &came)?;
        n += came.len();
    }
    Ok(n)
}

/// Download ahead, a few at a time while nothing else is asked: the sources of `folder`'s kept
/// messages not downloaded yet, newest first, `budget` of them at most. When every message
/// kept is to be downloaded ([`SyncOptions::full`]) any size is (up to what a source can be
/// read at all, [`crate::imap::MAX_LITERAL`]), else up to [`SyncOptions::prefetch_max_bytes`].
///
/// How many of those still have no source after it: 0, the folder is all here. A folder whose
/// UIDVALIDITY changed since it was listed waits for its sync (0). A message the server won't
/// give is passed over (and asked again next time); a lost connection, a refused sign-in or
/// several refusals running end it.
pub(crate) fn download_ahead<S: Read + Write>(
    c: &mut Client<S>,
    store: &Store,
    account: &str,
    folder: &str,
    opts: &SyncOptions,
    budget: usize,
) -> Result<usize> {
    let mut st = store.load_folder(account, folder)?;
    if st.validity == 0 {
        return Ok(0);
    }
    let biggest = if opts.full { u32::try_from(crate::imap::MAX_LITERAL).unwrap_or(u32::MAX) } else { opts.prefetch_max_bytes };
    let missing = |st: &FolderState| {
        // One look at the folder's sources rather than one per message (a folder of 100,000).
        let have: HashSet<u32> = store.cached_bodies(account, folder, st.validity).into_iter().collect();
        st.messages.iter().filter(|m| m.size <= biggest && !have.contains(&m.key.uid)).map(|m| m.key.uid).collect::<Vec<u32>>()
    };
    let todo = missing(&st);
    if todo.is_empty() || budget == 0 {
        return Ok(todo.len());
    }
    if c.selected() != Some(folder) && c.select(folder, false)?.validity != st.validity {
        return Ok(0);
    }
    let now: Vec<u32> = todo.into_iter().take(budget).collect();
    let refused = |e: &NetError| matches!(e, NetError::No(_) | NetError::Bad(_));
    let mut failures = AheadFailures::new();
    for chunk in uid_chunks(&now, 10) {
        match download(c, store, account, folder, &mut st, &chunk) {
            Ok(_) => failures.ok(),
            Err(e) if !refused(&e) => return Err(e),
            // Ten refused: one at a time, the one the server won't give passed over.
            Err(_) => {
                for uid in chunk {
                    match download(c, store, account, folder, &mut st, &[uid]) {
                        Ok(_) => failures.ok(),
                        Err(e) if !refused(&e) => return Err(e),
                        Err(e) => failures.failed(&format!("{account}/{folder}: message {uid}"), e)?,
                    }
                }
            }
        }
    }
    Ok(missing(&st).len())
}
