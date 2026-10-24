//! Bringing a folder's cache up to date: what's kept of it (the newest, of the mail received in
//! the time kept), what falls out, and messages' sources downloaded ahead of reading.

use std::collections::HashSet;

use super::*;

/// Downloads failing one after another that stop downloading ahead (the connection, then).
const AHEAD_FAILURES: u32 = 3;

impl<H: Http> Client<H> {
    /// The newest `limit` ids in a mailbox with their keywords, of those received after `after`
    /// (Unix seconds) when the time kept is limited, and its (total, unread) counts.
    /// A page of the server's `maxObjectsInGet` at a time (by position): asked at once, a folder
    /// kept no more than that however many it was to keep.
    async fn newest(&self, mailbox: &str, limit: usize, after: Option<i64>) -> Result<(Vec<(String, Flags)>, Option<(u32, u32)>)> {
        let limit = limit.clamp(1, 100_000);
        // `after`: received (`receivedAt`) after that time (RFC 8621 §4.4.1), as Outlook measures
        // the mail it keeps offline.
        let filter = match after {
            Some(t) => json!({"inMailbox": mailbox, "after": utc_date_string(t)}),
            None => json!({"inMailbox": mailbox}),
        };
        let (mut ids, mut counts, mut seen, mut total) = (Vec::new(), None, 0usize, None);
        for page in 0..1000 {
            if seen >= limit {
                break;
            }
            let want = (limit - seen).min(self.max_get);
            let mut calls = vec![
                (
                    "Email/query",
                    json!({"accountId": self.account, "filter": filter.clone(), "sort": [{"property": "receivedAt", "isAscending": false}],
                           "position": seen, "limit": want, "calculateTotal": page == 0}),
                ),
                (
                    "Email/get",
                    json!({"accountId": self.account, "#ids": {"resultOf": "c0", "name": "Email/query", "path": "/ids"}, "properties": ["id", "keywords"]}),
                ),
            ];
            if page == 0 {
                calls.push(("Mailbox/get", json!({"accountId": self.account, "ids": [mailbox], "properties": ["totalEmails", "unreadEmails"]})));
            }
            let r = self.mail(calls).await?;
            let order: Vec<&str> = list(r.first(), "ids").iter().filter_map(Value::as_str).collect();
            let got: HashMap<&str, Flags> =
                list(r.get(1), "list").iter().filter_map(|e| Some((e.get("id")?.as_str()?, keywords(e.get("keywords"))))).collect();
            ids.extend(order.iter().filter_map(|id| got.get(id).map(|f| (id.to_string(), f.clone()))));
            if page == 0 {
                counts = list(r.get(2), "list").first().map(|m| (count(m, "totalEmails"), count(m, "unreadEmails")));
                total = r.first().and_then(|q| q.get("total")).and_then(Value::as_u64).and_then(|t| usize::try_from(t).ok());
            }
            seen += order.len();
            // The end: nothing more, all there is, or (with no total) a short page.
            if order.is_empty() || total.map_or(order.len() < want, |t| seen >= t) {
                break;
            }
        }
        Ok((ids, counts))
    }

    /// Headers and details of messages, for the list.
    async fn headers(&self, ids: &[String]) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        for chunk in ids.chunks(self.max_get.min(200)) {
            let r = self
                .mail(vec![(
                    "Email/get",
                    json!({"accountId": self.account, "ids": chunk, "properties": ["id", "keywords", "size", "receivedAt", "preview", "hasAttachment", "headers"]}),
                )])
                .await?;
            out.extend(list(r.first(), "list").iter().cloned());
        }
        Ok(out)
    }

    /// The blob ids of messages' sources.
    async fn blobs(&self, ids: &[String]) -> Result<HashMap<String, String>> {
        let r = self.mail(vec![("Email/get", json!({"accountId": self.account, "ids": ids, "properties": ["id", "blobId"]}))]).await?;
        Ok(list(r.first(), "list")
            .iter()
            .filter_map(|e| Some((e.get("id")?.as_str()?.to_string(), e.get("blobId")?.as_str()?.to_string())))
            .collect())
    }
}

/// Bring one folder's cache up to date, as of `now` (Unix seconds, from the caller: portable code
/// doesn't read the clock). What came in before the time kept ([`SyncOptions::since`]) isn't
/// listed, so it leaves the cache with its source at every sync, the folder changed or not.
pub async fn sync_folder<H: Http>(
    c: &Client<H>,
    st: &mut State,
    store: &Store,
    account: &str,
    folder: &str,
    opts: &SyncOptions,
    now: i64,
) -> Result<SyncReport> {
    let mailbox = st.mailbox(folder)?.to_string();
    let (newest, counts) = c.newest(&mailbox, opts.window, opts.since(now)).await?;
    let mut fs = store.load_folder(account, folder)?;
    let mut report = SyncReport::default();
    if fs.validity != VALIDITY && (fs.validity != 0 || !fs.messages.is_empty()) {
        store.clear_folder(account, folder)?;
        fs = FolderState::default();
        report.reset = true;
    }
    fs.validity = VALIDITY;
    let wanted: HashMap<u32, Flags> = newest.iter().map(|(id, f)| (st.number(id), f.clone())).collect();
    let mut wanted = wanted;
    // Found by a search of the server past the window: kept while still in the mailbox.
    let extra = st.found_outside(folder, &fs.messages, &wanted);
    for (id, f) in c.still_in(&mailbox, &extra).await? {
        wanted.insert(st.number(&id), f);
    }
    st.keep_found(folder, |n| wanted.contains_key(&n));
    // Gone, out of the window or received before the time kept (the server measures it, by
    // when it received each message: a message's own date can be far older), and flag changes.
    let (kept, gone): (Vec<Summary>, Vec<Summary>) = std::mem::take(&mut fs.messages).into_iter().partition(|m| wanted.contains_key(&m.key.uid));
    for m in &gone {
        store.remove_body(&m.key);
    }
    report.removed = gone.len();
    fs.messages = kept;
    for m in &mut fs.messages {
        if let Some(f) = wanted.get(&m.key.uid)
            && m.flags != *f
        {
            m.flags = f.clone();
            report.changed += 1;
        }
    }
    // New.
    let missing: Vec<String> = newest.iter().filter(|(id, _)| st.known(id).is_some_and(|n| fs.find(n).is_none())).map(|(id, _)| id.clone()).collect();
    for e in c.headers(&missing).await? {
        let Some(id) = e.get("id").and_then(Value::as_str) else { continue };
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: st.number(id) };
        if fs.find(key.uid).is_some() {
            continue;
        }
        fs.messages.push(email_summary(&e, key));
        report.new += 1;
    }
    sort_newest_first(&mut fs.messages);
    fs.uid_next = fs.messages.iter().map(|m| m.key.uid).max().unwrap_or(0).saturating_add(1);
    store.save_folder(account, folder, &fs)?;
    prefetch(c, st, store, account, folder, &mut fs, opts).await?;
    let (total, unread) = counts.unwrap_or((u32::try_from(fs.messages.len()).unwrap_or(u32::MAX), fs.unread()));
    report.total = total;
    report.unread = unread;
    Ok(report)
}

impl<H: Http> Client<H> {
    /// The newest `limit` ids in a mailbox that `filter` matches.
    async fn search(&self, mailbox: &str, filter: &Value, limit: usize) -> Result<Vec<String>> {
        let filter = json!({"operator": "AND", "conditions": [{"inMailbox": mailbox}, filter]});
        let r = self
            .mail(vec![(
                "Email/query",
                json!({"accountId": self.account, "filter": filter, "sort": [{"property": "receivedAt", "isAscending": false}], "limit": limit.min(FOUND_MAX)}),
            )])
            .await?;
        Ok(list(r.first(), "ids").iter().filter_map(Value::as_str).map(str::to_string).take(limit).collect())
    }

    /// Of these messages, those still in a mailbox, with their keywords.
    async fn still_in(&self, mailbox: &str, ids: &[String]) -> Result<Vec<(String, Flags)>> {
        let mut out = Vec::new();
        for chunk in ids.chunks(self.max_get.clamp(1, 200)) {
            let r = self
                .mail(vec![("Email/get", json!({"accountId": self.account, "ids": chunk, "properties": ["id", "keywords", "mailboxIds"]}))])
                .await?;
            out.extend(
                list(r.first(), "list")
                    .iter()
                    .filter(|e| e.get("mailboxIds").and_then(|m| m.get(mailbox)) == Some(&Value::Bool(true)))
                    .filter_map(|e| Some((e.get("id")?.as_str()?.to_string(), keywords(e.get("keywords"))))),
            );
        }
        Ok(out)
    }
}

/// The messages of `folder` the server finds for `filter` ([`search_filter`]), the newest `cap`
/// of them: those not kept yet come into its list, and stay there past the window while the
/// server has them there (issue #4). Their keys, and how many came in. A folder never synced is
/// left to its sync.
pub async fn find_on_server<H: Http>(
    c: &Client<H>,
    st: &mut State,
    store: &Store,
    account: &str,
    folder: &str,
    filter: &Value,
    cap: usize,
) -> Result<(Vec<MessageKey>, usize)> {
    let mailbox = st.mailbox(folder)?.to_string();
    let mut fs = store.load_folder(account, folder)?;
    if fs.validity != VALIDITY {
        return Ok((Vec::new(), 0));
    }
    let ids = c.search(&mailbox, filter, cap.min(FOUND_MAX)).await?;
    let numbers: Vec<u32> = ids.iter().map(|id| st.number(id)).collect();
    let missing: Vec<String> = ids.iter().zip(&numbers).filter(|(_, n)| fs.find(**n).is_none()).map(|(id, _)| id.clone()).collect();
    let mut new = 0;
    for e in c.headers(&missing).await? {
        let Some(id) = e.get("id").and_then(Value::as_str) else { continue };
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: st.number(id) };
        if fs.find(key.uid).is_none() {
            fs.messages.push(email_summary(&e, key));
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

/// The newest [`SyncOptions::prefetch`] sources, with the folder's list.
async fn prefetch<H: Http>(
    c: &Client<H>,
    st: &State,
    store: &Store,
    account: &str,
    folder: &str,
    fs: &mut FolderState,
    opts: &SyncOptions,
) -> Result<()> {
    let wanted: Vec<(u32, String)> = fs
        .messages
        .iter()
        .take(opts.prefetch)
        .filter(|m| m.size <= opts.prefetch_max_bytes && !store.has_body(&m.key))
        .filter_map(|m| Some((m.key.uid, st.id(m.key.uid)?)))
        .collect();
    download_sources(c, store, account, folder, fs, &wanted, wanted.len()).await.map(drop)
}

/// Download, newest first, the sources of up to `budget` of a folder's kept messages that have
/// none yet — whatever their size when [`SyncOptions::full`], else up to
/// [`SyncOptions::prefetch_max_bytes`] — so that what's kept is downloaded whole in the end, a
/// few at a time, as Outlook's Cached Exchange Mode keeps it.
///
/// How many are still to download after it. One the server no longer has, or wouldn't give this
/// time, is left for when it's opened (or for the next sync to take out) and isn't counted:
/// going on while some are left always comes to an end.
pub async fn download_ahead<H: Http>(
    c: &Client<H>,
    st: &State,
    store: &Store,
    account: &str,
    folder: &str,
    opts: &SyncOptions,
    budget: usize,
) -> Result<usize> {
    let mut fs = store.load_folder(account, folder)?;
    if fs.validity != VALIDITY {
        return Ok(0);
    }
    let mut todo: Vec<&Summary> = fs
        .messages
        .iter()
        .filter(|m| (opts.full || m.size <= opts.prefetch_max_bytes) && st.id(m.key.uid).is_some() && !store.has_body(&m.key))
        .collect();
    // As the list shows them: the newest at the top come down first.
    todo.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.key.cmp(&a.key)));
    let todo: Vec<(u32, String)> = todo.iter().filter_map(|m| Some((m.key.uid, st.id(m.key.uid)?))).collect();
    let ahead = download_sources(c, store, account, folder, &mut fs, &todo, budget).await?;
    Ok(todo.len().saturating_sub(ahead.got.len().saturating_add(ahead.skipped)))
}

/// What downloading ahead did: the messages whose source came down, and how many were left
/// for when they're opened (the server no longer had them, or wouldn't give them).
#[derive(Default)]
struct Ahead {
    got: HashSet<u32>,
    skipped: usize,
}

/// Download the sources of `todo` (in its order) until `budget` came down, and keep the folder's
/// list with what they tell. The next is taken in place of one left for when it's opened.
async fn download_sources<H: Http>(
    c: &Client<H>,
    store: &Store,
    account: &str,
    folder: &str,
    fs: &mut FolderState,
    todo: &[(u32, String)],
    budget: usize,
) -> Result<Ahead> {
    let mut ahead = Ahead::default();
    if todo.is_empty() || budget == 0 {
        return Ok(ahead);
    }
    let r = fetch_sources(c, store, folder, fs, todo, budget, &mut ahead).await;
    // What came down is kept even when it stopped halfway.
    if !ahead.got.is_empty() {
        store.save_folder_changed(account, folder, fs, &ahead.got)?;
    }
    r.map(|()| ahead)
}

/// [`download_sources`]' downloads, their blob ids asked a `maxObjectsInGet` at a time. A
/// message gone from the server, or whose source fails, is left for when it's opened; a refused
/// sign-in, or [`AHEAD_FAILURES`] failures running (the connection, then), stop it.
async fn fetch_sources<H: Http>(
    c: &Client<H>,
    store: &Store,
    folder: &str,
    fs: &mut FolderState,
    todo: &[(u32, String)],
    budget: usize,
    ahead: &mut Ahead,
) -> Result<()> {
    let (mut next, mut failing) = (0usize, 0u32);
    while ahead.got.len() < budget && next < todo.len() {
        let end = next.saturating_add(budget.saturating_sub(ahead.got.len()).min(c.max_get.clamp(1, 200))).min(todo.len());
        let batch = todo.get(next..end).unwrap_or_default();
        next = end.max(next.saturating_add(1));
        let blobs = c.blobs(&batch.iter().map(|(_, id)| id.clone()).collect::<Vec<_>>()).await?;
        for (n, id) in batch {
            let raw = match blobs.get(id) {
                Some(blob) => c.download(blob).await,
                // Gone since the folder was listed: its next sync takes it out.
                None => {
                    ahead.skipped += 1;
                    continue;
                }
            };
            let raw = match raw {
                Ok(raw) => raw,
                Err(e) => {
                    failing += 1;
                    if matches!(e, Error::Auth(_)) || failing >= AHEAD_FAILURES {
                        return Err(e);
                    }
                    log::info!("{folder}: message {n} left for when it's opened: {e}");
                    ahead.skipped += 1;
                    continue;
                }
            };
            failing = 0;
            let Some(m) = fs.find_mut(*n) else {
                ahead.skipped += 1;
                continue;
            };
            store.write_body(&m.key, &raw)?;
            fill_from_source(m, &raw);
            ahead.got.insert(*n);
        }
    }
    Ok(())
}

/// Download one message's source (and cache it).
pub async fn fetch_body<H: Http>(c: &Client<H>, st: &State, store: &Store, key: &MessageKey) -> Result<Vec<u8>> {
    if let Some(raw) = store.read_body(key) {
        return Ok(raw);
    }
    let gone = || Error::No("the message is no longer on the server".into());
    let id = st.id(key.uid).filter(|_| key.validity == VALIDITY).ok_or_else(gone)?;
    let blob = c.blobs(std::slice::from_ref(&id)).await?.remove(&id).ok_or_else(gone)?;
    let raw = c.download(&blob).await?;
    store.write_body(key, &raw)?;
    let mut fs = store.load_folder(&key.account, &key.folder)?;
    if let Some(m) = fs.find_mut(key.uid) {
        fill_from_source(m, &raw);
        store.save_folder(&key.account, &key.folder, &fs)?;
    }
    Ok(raw)
}
