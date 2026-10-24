//! Graph: the sync — the folder list, one folder's messages brought up to date, the server searched, sources, and changes applied.

use std::collections::HashSet;

use super::*;

/// Refresh the folder list (and the folder ids behind it).
pub fn list_folders(c: &Client, st: &mut State, account: &str, shared: &[String]) -> Result<Vec<Folder>> {
    let mut boxes = c.mailboxes()?;
    // Other mailboxes opened here, each under a folder named after it. One this account can't
    // open (no right to it, or gone) is left out; the others still list.
    let mut owner_of_box: HashMap<String, String> = HashMap::new();
    for owner in shared {
        match c.shared_mailbox(owner) {
            Ok(theirs) => {
                for b in &theirs {
                    owner_of_box.insert(b.id.clone(), owner.clone());
                }
                boxes.extend(theirs);
            }
            Err(e) => log::warn!("{owner}'s mailbox: {e}"),
        }
    }
    let mut found = jmap::folders(account, &boxes);
    st.mailboxes = found.iter().map(|(f, id)| (f.path.clone(), id.clone())).collect();
    st.owners = found.iter().filter_map(|(f, id)| Some((f.path.clone(), owner_of_box.get(id)?.clone()))).collect();
    jmap::name_special(&mut found, &boxes);
    Ok(found.into_iter().map(|(f, _)| f).collect())
}

/// Now, in Unix seconds.
fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Bring one folder's cache up to date: its newest messages, those received in the time kept
/// ([`SyncOptions::since_days`]) when it's limited, as Outlook's Cached Exchange Mode keeps
/// them. Older mail goes, its source with it, at every sync (a folder nothing happened in
/// too), except what a search of the server found: that stays while the folder has it.
pub fn sync_folder(c: &Client, st: &mut State, store: &Store, account: &str, folder: &str, opts: &SyncOptions) -> Result<SyncReport> {
    let id = st.mailbox(folder)?.to_string();
    let owner = st.owner(folder).map(str::to_string);
    let owner = owner.as_deref();
    let since = opts.since(now());
    let (listed, (total, unread)) = c.newest(&id, opts.window, since, owner)?;
    let newest: Vec<(String, Flags)> = listed.iter().filter_map(|m| Some((m.get("id")?.as_str()?.to_string(), flags(m)))).collect();
    let mut fs = store.load_folder(account, folder)?;
    let mut report = SyncReport::default();
    if fs.validity != VALIDITY && (fs.validity != 0 || !fs.messages.is_empty()) {
        store.clear_folder(account, folder)?;
        fs = FolderState::default();
        report.reset = true;
    }
    fs.validity = VALIDITY;
    let mut wanted: HashMap<u32, Flags> = newest.iter().map(|(id, f)| (st.number(id), f.clone())).collect();
    // Found by a search of the server past the window: kept while still in the folder.
    let extra = st.found_outside(folder, &fs.messages, &wanted);
    for (found, f) in c.still_in(&id, &extra)? {
        wanted.insert(st.number(&found), f);
    }
    st.keep_found(folder, |n| wanted.contains_key(&n));
    // Summaries kept from before they had it learn when the server received them, the time
    // mail kept is measured by: a message whose own date is far older (a forward, mail imported)
    // is listed by the server and stays here, not let go and listed again at each sync.
    let received_of: HashMap<u32, i64> = listed.iter().filter_map(|m| Some((st.known(m.get("id")?.as_str()?)?, received(m)?))).collect();
    for m in fs.messages.iter_mut().filter(|m| m.received == 0) {
        if let Some(r) = received_of.get(&m.key.uid) {
            m.received = *r;
        }
    }
    // Past the window, or older than the time kept: gone, with its source.
    let old = |m: &Summary| since.is_some_and(|t| m.received_at() < t) && !st.was_found(folder, m.key.uid);
    let (kept, gone): (Vec<Summary>, Vec<Summary>) =
        std::mem::take(&mut fs.messages).into_iter().partition(|m| wanted.contains_key(&m.key.uid) && !old(m));
    for m in &gone {
        store.remove_body(&m.key);
    }
    report.removed = gone.len();
    fs.messages = kept;
    for m in &mut fs.messages {
        // Graph has no answered flag or categories here: keep those as they are.
        if let Some(f) = wanted.get(&m.key.uid) {
            let mut next = m.flags.clone();
            next.seen = f.seen;
            next.flagged = f.flagged;
            next.draft = f.draft;
            if next != m.flags {
                m.flags = next;
                report.changed += 1;
            }
        }
    }
    let missing: Vec<String> = newest.iter().filter(|(id, _)| fs.find(st.number(id)).is_none()).map(|(id, _)| id.clone()).collect();
    // Small gap: per-message GETs with internet headers. Big gap: reuse the listing entries.
    let details = if missing.len() <= FEW {
        c.headers(&missing, owner)?
    } else {
        let mut sorted = missing.clone();
        sorted.sort_unstable();
        listed.iter().filter(|m| m["id"].as_str().is_some_and(|i| sorted.binary_search_by(|x| x.as_str().cmp(i)).is_ok())).cloned().collect()
    };
    for m in details {
        let Some(id) = m.get("id").and_then(Value::as_str) else { continue };
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: st.number(id) };
        if fs.find(key.uid).is_some() {
            continue;
        }
        fs.messages.push(message_summary(&m, key));
        report.new += 1;
    }
    sort_newest_first(&mut fs.messages);
    fs.uid_next = fs.messages.iter().map(|m| m.key.uid).max().unwrap_or(0).saturating_add(1);
    store.save_folder(account, folder, &fs)?;
    prefetch(c, st, store, account, folder, &mut fs, opts)?;
    report.total = total;
    report.unread = unread;
    Ok(report)
}

/// A message's list entry from its details.
fn message_summary(m: &Value, key: MessageKey) -> Summary {
    let received = m.get("receivedDateTime").and_then(Value::as_str).and_then(jmap::utc_date).unwrap_or(0);
    let mut s = mime::summary_from_header(&header_block(m), key, flags(m), 0, received);
    s.preview = m.get("bodyPreview").and_then(Value::as_str).map(mime::preview).unwrap_or_default();
    s.has_attachments = m.get("hasAttachments").and_then(Value::as_bool).unwrap_or(false);
    s.voting = voting_choices(m);
    s
}

/// The messages of `folder` the server finds for a search (issue #4), the newest `cap` of them:
/// its words through `$search` (KQL), or, with none, its read and flag conditions through
/// `$filter`. Those not kept yet come into its list and stay there past the window while the
/// folder has them. Their keys, and how many came in.
pub fn find_on_server(
    c: &Client,
    st: &mut State,
    store: &Store,
    account: &str,
    folder: &str,
    query: &WordQuery,
    cap: usize,
) -> Result<(Vec<MessageKey>, usize)> {
    let id = st.mailbox(folder)?.to_string();
    let mut fs = store.load_folder(account, folder)?;
    if fs.validity != VALIDITY {
        return Ok((Vec::new(), 0));
    }
    let cap = cap.min(jmap::FOUND_MAX);
    let ask = if query.text.is_empty() {
        let mut f = Vec::new();
        if let Some(u) = query.unread {
            f.push(format!("isRead eq {}", !u));
        }
        if let Some(on) = query.flagged {
            f.push(format!("flag/flagStatus {} 'flagged'", if on { "eq" } else { "ne" }));
        }
        if let Some(a) = query.attachment {
            f.push(format!("hasAttachments eq {a}"));
        }
        format!("$filter={}", http::pct(&f.join(" and ")))
    } else {
        format!("$search={}", http::pct(&format!("\"{}\"", query.text.replace('"', "\\\""))))
    };
    let found = c.all(&format!("/me/mailFolders/{}/messages?{ask}&$top={}&$select={HEADER_FIELDS}", seg(&id), cap.clamp(1, 1000)), cap)?;
    let found: Vec<&Value> =
        found.iter().filter(|m| query.admits(&flags(m), m.get("hasAttachments").and_then(Value::as_bool).unwrap_or(false))).collect();
    let mut numbers = Vec::new();
    let mut new = 0;
    for m in found {
        let Some(mid) = m.get("id").and_then(Value::as_str) else { continue };
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: st.number(mid) };
        numbers.push(key.uid);
        if fs.find(key.uid).is_none() {
            fs.messages.push(message_summary(m, key));
            new += 1;
        }
    }
    st.add_found(folder, &numbers);
    if new > 0 {
        sort_newest_first(&mut fs.messages);
        fs.uid_next = fs.messages.iter().map(|m| m.key.uid).max().unwrap_or(0).saturating_add(1);
        store.save_folder(account, folder, &fs)?;
    }
    Ok((numbers.iter().filter_map(|n| fs.find(*n).map(|m| m.key.clone())).collect(), new))
}

/// The newest sources, downloaded with the folder's list.
fn prefetch(c: &Client, st: &State, store: &Store, account: &str, folder: &str, fs: &mut FolderState, opts: &SyncOptions) -> Result<()> {
    let cap = ahead_cap(opts);
    let wanted: Vec<(MessageKey, String)> = fs
        .messages
        .iter()
        .take(opts.prefetch)
        .filter(|m| worth_ahead(m, opts, cap) && !store.has_body(&m.key))
        .filter_map(|m| Some((m.key.clone(), st.id(m.key.uid)?)))
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }
    let filled = download_into(c, st, store, fs, &wanted, cap)?;
    // The same messages the sync just kept, a few of them filled in.
    if !filled.is_empty() {
        store.save_folder_changed(account, folder, fs, &filled)?;
    }
    Ok(())
}

/// The largest source kept ahead of reading: none when every message kept is downloaded whole.
fn ahead_cap(opts: &SyncOptions) -> Option<usize> {
    (!opts.full).then(|| usize::try_from(opts.prefetch_max_bytes).unwrap_or(usize::MAX))
}

/// Whether a message's source is downloaded ahead of reading. Graph lists no sizes: unless all
/// of it is downloaded in the end, mail with attachments waits until it's opened, and so does
/// one already found larger than `cap`.
fn worth_ahead(m: &Summary, opts: &SyncOptions, cap: Option<usize>) -> bool {
    opts.full || (!m.has_attachments && cap.is_none_or(|cap| m.size == 0 || usize::try_from(m.size).is_ok_and(|s| s <= cap)))
}

/// Download these messages' sources (key, server id) into the cache, and fill their list
/// entries from them. A source larger than `cap` isn't kept, but its size is, so it isn't asked
/// for again. The numbers of the entries filled.
fn download_into(
    c: &Client,
    st: &State,
    store: &Store,
    fs: &mut FolderState,
    wanted: &[(MessageKey, String)],
    cap: Option<usize>,
) -> Result<HashSet<u32>> {
    let mut filled = HashSet::new();
    let mut failures = crate::sync::AheadFailures::new();
    for (key, id) in wanted {
        let raw = match c.download(id, st.owner(&key.folder)) {
            Ok(raw) => {
                failures.ok();
                raw
            }
            Err(e) => {
                failures.failed(&format!("{}/{}: message {}", key.account, key.folder, key.uid), e)?;
                continue;
            }
        };
        let Some(m) = fs.find_mut(key.uid) else { continue };
        if cap.is_some_and(|cap| raw.len() > cap) {
            if m.size == 0 {
                m.size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
                filled.insert(key.uid);
            }
            continue;
        }
        store.write_body(&m.key, &raw)?;
        fill_from_source(m, &raw);
        filled.insert(key.uid);
    }
    Ok(filled)
}

/// Whole downloads in the background, a few at a time while nothing else is asked: at most
/// `budget` sources of the folder's cached messages not downloaded yet, the newest first. With
/// [`SyncOptions::full`] every one comes, whatever its size, mail with attachments too; without
/// it, those the folder's sync would download ahead ([`worth_ahead`]). How many of its cached
/// messages are still to download after it (one the server won't give stays among them).
pub(crate) fn download_ahead(c: &Client, st: &State, store: &Store, account: &str, folder: &str, opts: &SyncOptions, budget: usize) -> Result<usize> {
    let mut fs = store.load_folder(account, folder)?;
    if fs.validity != VALIDITY {
        return Ok(0);
    }
    let cap = ahead_cap(opts);
    // One look at the folder's sources, not one per message (a folder may keep 100,000).
    let mut have: HashSet<u32> = store.cached_bodies(account, folder, VALIDITY).into_iter().collect();
    let to_get = |m: &Summary, have: &HashSet<u32>| !have.contains(&m.key.uid) && worth_ahead(m, opts, cap) && st.id(m.key.uid).is_some();
    let wanted: Vec<(MessageKey, String)> =
        fs.messages.iter().filter(|m| to_get(m, &have)).take(budget).filter_map(|m| Some((m.key.clone(), st.id(m.key.uid)?))).collect();
    if !wanted.is_empty() {
        let filled = download_into(c, st, store, &mut fs, &wanted, cap)?;
        if !filled.is_empty() {
            store.save_folder_changed(account, folder, &fs, &filled)?;
        }
        have.extend(wanted.iter().filter(|(k, _)| store.has_body(k)).map(|(k, _)| k.uid));
    }
    Ok(fs.messages.iter().filter(|m| to_get(m, &have)).count())
}

/// Download one message's source (and cache it).
pub fn fetch_body(c: &Client, st: &State, store: &Store, key: &MessageKey) -> Result<Vec<u8>> {
    if let Some(raw) = store.read_body(key) {
        return Ok(raw);
    }
    let id = st.id(key.uid).filter(|_| key.validity == VALIDITY).ok_or_else(|| NetError::No("the message is no longer on the server".into()))?;
    let raw = c.download(&id, st.owner(&key.folder))?;
    store.write_body(key, &raw)?;
    let mut fs = store.load_folder(&key.account, &key.folder)?;
    if let Some(m) = fs.find_mut(key.uid) {
        fill_from_source(m, &raw);
        store.save_folder(&key.account, &key.folder, &fs)?;
    }
    Ok(raw)
}

/// The voting buttons on a message (its `PidLidVerbStream`, when asked for).
fn voting_choices(m: &Value) -> Vec<String> {
    m.get("singleValueExtendedProperties")
        .and_then(Value::as_array)
        .and_then(|props| props.iter().find(|p| p.get("id").and_then(Value::as_str).is_some_and(|id| id.to_ascii_lowercase().ends_with("id 0x8520"))))
        .and_then(|p| p.get("value").and_then(Value::as_str))
        .and_then(|v| crate::ews::unb64(v.trim()))
        .map(|b| voting::choices(&b))
        .unwrap_or_default()
}

/// Vote on a message (see [`Client::vote`]).
pub fn vote(c: &Client, st: &State, key: &MessageKey, choice: &str, subject: &str) -> Result<()> {
    let id = st.id(key.uid).filter(|_| key.validity == VALIDITY).ok_or_else(|| NetError::No("the message is no longer on the server".into()))?;
    c.vote(&id, choice, subject, st.owner(&key.folder))
}

/// Apply one change on the server.
pub fn apply_remote(c: &Client, st: &State, op: &Op) -> Result<()> {
    // Each message by its id, in its own mailbox (another one opened here is under `/users`).
    let ids = |keys: &[MessageKey]| -> Vec<(String, String)> {
        keys.iter().filter(|k| k.validity == VALIDITY).filter_map(|k| Some((st.id(k.uid)?, k.folder.clone()))).collect()
    };
    let at = |folder: &str| who(st.owner(folder));
    let patch_all = |keys: &[MessageKey], patch: Value| -> Result<()> {
        let reqs: Vec<(&str, String, Option<Value>)> =
            ids(keys).into_iter().map(|(id, f)| ("PATCH", format!("{}/messages/{}", at(&f), seg(&id)), Some(patch.clone()))).collect();
        c.batch_ok(&reqs)
    };
    match op {
        Op::SetFlag { keys, flag: Flag::Seen, on } => patch_all(keys, json!({"isRead": on})),
        Op::SetFlag { keys, flag: Flag::Flagged, on } => patch_all(keys, json!({"flag": {"flagStatus": if *on { "flagged" } else { "notFlagged" }}})),
        // Answered and forwarded are Exchange's to set; categories stay on this computer.
        Op::SetFlag { .. } | Op::SetKeyword { .. } => Ok(()),
        Op::Move { keys, to } => {
            let dest = st.mailbox(to)?.to_string();
            let reqs: Vec<(&str, String, Option<Value>)> = ids(keys)
                .into_iter()
                .filter(|(_, from)| from != to)
                .map(|(id, f)| ("POST", format!("{}/messages/{}/move", at(&f), seg(&id)), Some(json!({"destinationId": dest}))))
                .collect();
            c.batch_ok(&reqs)
        }
        Op::Expunge { keys } => {
            let reqs: Vec<(&str, String, Option<Value>)> =
                ids(keys).into_iter().map(|(id, f)| ("DELETE", format!("{}/messages/{}", at(&f), seg(&id)), None)).collect();
            c.batch_ok(&reqs)
        }
        Op::CreateFolder { path } => {
            let (parent, name) = split_path(st, path)?;
            let me = at(path);
            let url = parent.map_or_else(|| format!("{me}/mailFolders"), |p| format!("{me}/mailFolders/{}/childFolders", seg(&p)));
            c.post(&url, &json!({"displayName": name})).map(|_| ())
        }
        Op::RenameFolder { from, to } => {
            let id = st.mailbox(from)?.to_string();
            let (old_parent, _) = split_path(st, from)?;
            let (parent, name) = split_path(st, to)?;
            let mut id = id;
            if parent != old_parent {
                let dest = parent.clone().unwrap_or_else(|| "msgfolderroot".into());
                let moved = c.post(&format!("{}/mailFolders/{}/move", at(from), seg(&id)), &json!({"destinationId": dest}))?;
                id = moved.get("id").and_then(Value::as_str).unwrap_or(&id).to_string();
            }
            let body = serde_json::to_vec(&json!({"displayName": name})).map_err(|e| NetError::Protocol(e.to_string()))?;
            c.send_raw("PATCH", &format!("{}/mailFolders/{}", at(from), seg(&id)), "application/json", body).map(|_| ())
        }
        Op::DeleteFolder { path } => {
            let id = st.mailbox(path)?.to_string();
            c.send_raw("DELETE", &format!("{}/mailFolders/{}", at(path), seg(&id)), "application/json", Vec::new()).map(|_| ())
        }
        Op::EmptyFolder { path } => c.empty(st.mailbox(path)?, st.owner(path)),
    }
}

/// The parent folder id (`None` at the top) and the leaf name of a folder path.
fn split_path(st: &State, path: &str) -> Result<(Option<String>, String)> {
    match path.rsplit_once('/') {
        Some((parent, leaf)) => Ok((Some(st.mailbox(parent)?.to_string()), utf7::decode(leaf))),
        None => Ok((None, utf7::decode(path))),
    }
}

/// Store a draft in a folder.
pub fn append(c: &Client, st: &State, folder: &str, raw: &[u8]) -> Result<String> {
    c.import(raw, Some(st.mailbox(folder)?), st.owner(folder))
}

/// Send a message. The MIME source names the recipients for Exchange, so anyone only on the
/// envelope (Bcc) goes into a `Bcc:` header, which Exchange removes before delivery.
pub fn send(c: &Client, raw: &[u8], recipients: &[String]) -> Result<()> {
    let shown: Vec<String> = mime::parse(raw).map(|p| p.to.iter().chain(&p.cc).map(|a| a.email.to_ascii_lowercase()).collect()).unwrap_or_default();
    let bcc: Vec<&str> = recipients.iter().map(String::as_str).filter(|r| !shown.contains(&r.to_ascii_lowercase())).collect();
    if bcc.is_empty() {
        return c.send_mime(raw);
    }
    let mut with = format!("Bcc: {}\r\n", bcc.join(", ")).into_bytes();
    with.extend_from_slice(raw);
    c.send_mime(&with)
}
