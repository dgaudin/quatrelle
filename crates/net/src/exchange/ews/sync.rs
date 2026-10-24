//! EWS: the sync — the folder list, one folder's messages brought up to date, the server searched, and message sources.

use super::*;

/// The query in Exchange's own syntax (`QueryString`, its Advanced Query Syntax): the words, and
/// the fields Quatrelle's queries name that Exchange knows too. A clause it has no word for
/// (`is:unread`, `keyword:…`) is left to the local index rather than sent as nonsense.
pub(super) fn aqs_of(query: &str) -> String {
    use quatrelle_model::search::{Field, Query as ModelQuery, Term};
    let q = ModelQuery::parse(query);
    let mut out: Vec<String> = Vec::new();
    for c in q.clauses.iter().filter(|c| !c.negated) {
        let part = match &c.term {
            Term::Words(field, ws) => {
                let prefix = match field {
                    Field::From => "from:",
                    Field::To => "to:",
                    Field::Subject => "subject:",
                    Field::Body => "body:",
                    Field::Any => "",
                };
                ws.iter().map(|w| format!("{prefix}{w}")).collect::<Vec<_>>().join(" ")
            }
            Term::Phrase(field, s) => {
                let s = s.replace('"', " ");
                let prefix = match field {
                    Field::From => "from:",
                    Field::To => "to:",
                    Field::Subject => "subject:",
                    Field::Body => "body:",
                    Field::Any => "",
                };
                format!("{prefix}\"{s}\"")
            }
            _ => continue,
        };
        out.push(part);
    }
    out.join(" ")
}

/// Messages the server finds for `query` in `folders` (paths), as the headers a sync reads:
/// Exchange searches with its own index, bodies included, so a message whose text isn't on this
/// computer is found too — what Outlook does. `limit` bounds the answer (its newest ones).
pub fn search(c: &Client, st: &mut State, account: &str, folders: &[String], query: &str, limit: usize) -> Result<Vec<Summary>> {
    let query = aqs_of(query);
    if query.is_empty() {
        return Ok(Vec::new());
    }
    // The folders this account's state knows, and their ids on the server.
    let mut known: Vec<(String, String)> = Vec::new();
    for path in folders {
        if let Ok(id) = st.mailbox(path) {
            known.push((path.clone(), id.to_string()));
        }
        // A folder list of a mailbox's size (a hundred and ten here) is one small request; a
        // thousand would not be.
        if known.len() >= 250 {
            break;
        }
    }
    if known.is_empty() {
        return Ok(Vec::new());
    }
    let ids: String = known.iter().map(|(_, id)| format!("<t:FolderId Id=\"{}\"/>", esc(id))).collect();
    let m = c.call_one(&format!(
        "<m:FindItem Traversal=\"Shallow\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{HEADER_FIELDS}{FLAG_FIELDS}         <t:FieldURI FieldURI=\"item:ParentFolderId\"/><t:FieldURI FieldURI=\"item:HasAttachments\"/><t:FieldURI FieldURI=\"item:Size\"/></t:AdditionalProperties></m:ItemShape>\
         <m:IndexedPageItemView MaxEntriesReturned=\"{}\" Offset=\"0\" BasePoint=\"Beginning\"/>\
         <m:SortOrder><t:FieldOrder Order=\"Descending\"><t:FieldURI FieldURI=\"item:DateTimeReceived\"/></t:FieldOrder></m:SortOrder>\
         <m:ParentFolderIds>{ids}</m:ParentFolderIds>\
         <m:QueryString>{}</m:QueryString></m:FindItem>",
        limit.clamp(1, 500),
        esc(&query)
    ))?;
    let path_of: HashMap<&str, &str> = known.iter().map(|(path, id)| (id.as_str(), path.as_str())).collect();
    let first = known.first().map(|(path, _)| path.as_str()).unwrap_or_default();
    let mut out = Vec::new();
    let Some(items) = m.find("Items") else { return Ok(out) };
    for it in &items.kids {
        let Some(id) = it.child("ItemId").and_then(|i| i.attr("Id")) else { continue };
        // The folder it's in, by the id it says it belongs to (a request may span several).
        let folder = it.child("ParentFolderId").and_then(|p| p.attr("Id")).and_then(|i| path_of.get(i).copied()).unwrap_or(first);
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: st.number(id) };
        let received = it.text_of("DateTimeReceived").and_then(jmap::utc_date).unwrap_or(0);
        let size = it.text_of("Size").and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(0);
        let mut found = mime::summary_from_header(&header_block(it), key, flags(it), size, received);
        found.has_attachments = it.text_of("HasAttachments").is_some_and(|v| v == "true");
        out.push(found);
    }
    Ok(out)
}

/// A page of a folder's mail older than what's kept (`before`, Unix seconds), newest first from
/// `offset`, read on the server with what a list shows (Cached Exchange Mode's "more on the
/// server"): numbered, not kept. Whether the server has more past it.
pub fn older(c: &Client, st: &mut State, account: &str, folder: &str, before: i64, offset: usize, page: usize) -> Result<(Vec<Summary>, bool)> {
    let id = st.mailbox(folder)?.to_string();
    let m = c.call_one(&format!(
        "<m:FindItem Traversal=\"Shallow\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{HEADER_FIELDS}{FLAG_FIELDS}<t:FieldURI FieldURI=\"item:HasAttachments\"/><t:FieldURI FieldURI=\"item:Size\"/></t:AdditionalProperties></m:ItemShape>\
         <m:IndexedPageItemView MaxEntriesReturned=\"{}\" Offset=\"{offset}\" BasePoint=\"Beginning\"/>\
         <m:Restriction><t:IsLessThan><t:FieldURI FieldURI=\"item:DateTimeReceived\"/><t:FieldURIOrConstant><t:Constant Value=\"{}\"/></t:FieldURIOrConstant></t:IsLessThan></m:Restriction>\
         <m:SortOrder><t:FieldOrder Order=\"Descending\"><t:FieldURI FieldURI=\"item:DateTimeReceived\"/></t:FieldOrder></m:SortOrder>\
         <m:ParentFolderIds><t:FolderId Id=\"{}\"/></m:ParentFolderIds></m:FindItem>",
        page.clamp(1, 1000),
        crate::ews_pim::utc(before),
        esc(&id)
    ))?;
    let more = m.find("RootFolder").and_then(|r| r.attr("IncludesLastItemInRange")) == Some("false");
    let mut out = Vec::new();
    for it in m.find("Items").map(|i| i.kids.as_slice()).unwrap_or_default() {
        let Some(item) = it.child("ItemId").and_then(|i| i.attr("Id")) else { continue };
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: st.number(item) };
        out.push(item_summary(it, key));
    }
    Ok((out, more))
}

/// A message's source read on the server for showing, not kept (mail older than what's kept).
pub fn read_online(c: &Client, st: &mut State, store: &Store, key: &MessageKey) -> Result<Vec<u8>> {
    let id = server_id(c, st, store, key)?.ok_or_else(|| NetError::No("the message is no longer on the server".into()))?;
    c.download(&id)
}

/// Refresh the folder list (and the folder ids behind it).
pub fn list_folders(c: &Client, st: &mut State, account: &str) -> Result<Vec<Folder>> {
    let boxes = c.mailboxes()?;
    let mut found = jmap::folders(account, &boxes);
    st.mailboxes = found.iter().map(|(f, id)| (f.path.clone(), id.clone())).collect();
    jmap::name_special(&mut found, &boxes);
    // A shared mailbox's folders say whose they are, and what each is there.
    let shared = c.shared_roles.borrow();
    for (f, id) in &mut found {
        if let Some((address, role)) = shared.get(id.as_str()) {
            f.mailbox = Some(address.clone());
            f.mailbox_role = *role;
        }
    }
    Ok(found.into_iter().map(|(f, _)| f).collect())
}

/// Bring one folder's cache up to date.
pub fn sync_folder(c: &Client, st: &mut State, store: &Store, account: &str, folder: &str, opts: &SyncOptions) -> Result<SyncReport> {
    let id = st.mailbox(folder)?.to_string();
    let (total, unread, stamp) = c.counts(&id)?;
    // As it was when last read in full (same last change, same counts, same window, not long
    // ago): nothing to list. A pass over a hundred folders was a listing of each, up to its
    // whole window, every time — most of them unchanged since the last pass.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    let mut mark = jmap::FolderMark {
        stamp,
        total,
        unread,
        window: opts.window,
        at: now,
        ahead: opts.prefetch,
        sync: String::new(),
        too_big: None,
        days: opts.since_days,
        // A listing reads every message's categories.
        categories: true,
    };
    let same = |m: &jmap::FolderMark| {
        !mark.stamp.is_empty()
            && m.stamp == mark.stamp
            && m.total == total
            && m.unread == unread
            && m.window == mark.window
            && m.ahead == mark.ahead
            && m.days == mark.days
            && m.categories
            && now - m.at < MARK_TRUSTED
    };
    let kept = store.folder_kept(account, folder, VALIDITY);
    if kept && st.marks.get(folder).is_some_and(same) {
        return Ok(SyncReport { total, unread, ..Default::default() });
    }
    let before = st.marks.remove(folder);
    // Its state as last read known: only what changed since is asked (new mail, read marks,
    // deletions), not a listing of the folder's whole window.
    let too_big = before.as_ref().and_then(|m| m.too_big).filter(|at| now - at < TOO_BIG_ASKED_AGAIN);
    // Mail kept from so long ago (Cached Exchange Mode's "Mail to keep offline").
    let oldest = opts.since(now);
    // Followed by its changes while what's kept is within what was (a shorter time lets go of
    // the older there); a longer one lists it again for what it now keeps. All of it kept, then
    // two years (the owner's change), listed every folder again from the start.
    let within = |was: u32, now: u32| was == now || (now != 0 && (was == 0 || was > now));
    let following =
        |m: &jmap::FolderMark| kept && !opts.list_only && !m.sync.is_empty() && m.window == opts.window && within(m.days, opts.since_days);
    if let Some((since, read)) = before.filter(following).map(|m| (m.sync, m.categories)) {
        match changes(c, st, store, account, folder, &since, opts) {
            Ok((mut report, sync)) => {
                // Categories Outlook put on before they were read here: asked for once.
                if !read {
                    report.changed += catch_up_categories(c, store, st, account, folder, &id, opts.window, oldest)?;
                }
                mark.sync = sync;
                st.marks.insert(folder.to_string(), mark);
                report.total = total;
                report.unread = unread;
                return Ok(report);
            }
            // A state the server no longer takes (too old, the folder rebuilt): listed in full.
            Err(NetError::No(e)) => log::info!("{account}: {}: changes since the last read refused ({e}); listed in full", utf7::decode(folder)),
            Err(e) => return Err(e),
        }
    }
    // Listed in full; the folder's state taken first, so a change made while it's listed comes
    // again next time rather than being missed. Not for a folder only listed, nor one known to
    // be too big for it (asked again a week later).
    mark.too_big = too_big;
    if !opts.list_only && too_big.is_none() {
        match state_now(c, &id) {
            Ok(Some(state)) => mark.sync = state,
            Ok(None) => {
                log::info!("{account}: {}: too many messages to follow its changes; listed in full, asked again in a week", utf7::decode(folder));
                mark.too_big = Some(now);
            }
            Err(e) => log::info!("{account}: {}: no sync state ({e}); listed in full next time too", utf7::decode(folder)),
        }
    }
    let newest = c.newest(&id, opts.window, oldest)?;
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
    let (kept, gone): (Vec<Summary>, Vec<Summary>) = std::mem::take(&mut fs.messages).into_iter().partition(|m| wanted.contains_key(&m.key.uid));
    for m in &gone {
        store.remove_body(&m.key);
    }
    report.removed = gone.len();
    fs.messages = kept;
    for m in &mut fs.messages {
        if let Some(f) = wanted.get(&m.key.uid)
            && take_marks(&mut m.flags, f)
        {
            report.changed += 1;
        }
    }
    let missing: Vec<String> = newest.iter().filter(|(id, _)| fs.find(st.number(id)).is_none()).map(|(id, _)| id.clone()).collect();
    for it in c.headers(&missing)? {
        let Some(id) = it.child("ItemId").and_then(|i| i.attr("Id")) else { continue };
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: st.number(id) };
        if fs.find(key.uid).is_some() {
            continue;
        }
        fs.messages.push(item_summary(&it, key));
        report.new += 1;
    }
    sort_newest_first(&mut fs.messages);
    fs.uid_next = fs.messages.iter().map(|m| m.key.uid).max().unwrap_or(0).saturating_add(1);
    store.save_folder(account, folder, &fs)?;
    prefetch(c, st, store, account, folder, &mut fs, opts)?;
    // Read in full: noted as it was, for the next pass to leave alone if nothing changes.
    st.marks.insert(folder.to_string(), mark);
    report.total = total;
    report.unread = unread;
    Ok(report)
}

/// How long a folder's mark is trusted (seconds): past this, it's read again anyway (a change
/// the server's stamp doesn't show, a flag set elsewhere, still comes in) — its changes only,
/// when its state is known.
const MARK_TRUSTED: i64 = 6 * 3600;

/// Pages of changes read at most in one go (a folder with more catches up at the next read).
#[cfg(not(test))]
const CHANGE_PAGES: usize = 200;
#[cfg(test)]
const CHANGE_PAGES: usize = 3;

/// How long a folder found too big to follow by its changes is listed without asking (seconds).
const TOO_BIG_ASKED_AGAIN: i64 = 7 * 86_400;

/// A folder's state as of now (`SyncFolderItems` from nothing, every page read and its items
/// let go: only the state matters). Too big a folder has none (`None`).
fn state_now(c: &Client, folder: &str) -> Result<Option<String>> {
    let mut state = String::new();
    for _ in 0..CHANGE_PAGES {
        let (_, next, last) = c.changes_since(folder, &state)?;
        state = next;
        if last {
            return Ok(Some(state));
        }
    }
    Ok(None)
}

/// The categories of a folder's kept messages, read once: those carrying any, asked for alone
/// (all of them listed again is minutes on a big mailbox). How many changed.
#[allow(clippy::too_many_arguments)]
fn catch_up_categories(
    c: &Client,
    store: &Store,
    st: &State,
    account: &str,
    folder: &str,
    id: &str,
    window: usize,
    since: Option<i64>,
) -> Result<usize> {
    let found = match c.categorized(id, window, since) {
        Ok(f) => f,
        // A server that won't search so: they come as each message changes.
        Err(NetError::No(e)) => {
            log::info!("{account}: {}: categories not asked for ({e})", utf7::decode(folder));
            return Ok(0);
        }
        Err(e) => return Err(e),
    };
    if found.is_empty() {
        return Ok(0);
    }
    let mut fs = store.load_folder(account, folder)?;
    let mut changed = std::collections::HashSet::new();
    for (item, f) in &found {
        if let Some(m) = st.known(item).and_then(|n| fs.find_mut(n))
            && take_marks(&mut m.flags, f)
        {
            changed.insert(m.key.uid);
        }
    }
    if !changed.is_empty() {
        store.save_folder_changed(account, folder, &fs, &changed)?;
    }
    log::info!("{account}: {}: {} messages' categories read", utf7::decode(folder), changed.len());
    Ok(changed.len())
}

/// What changed in a folder since its state `since`, made on its list: new messages listed
/// (their details asked for), read and flag marks set, deleted ones taken out. The report, and
/// the folder's state now.
fn changes(c: &Client, st: &mut State, store: &Store, account: &str, folder: &str, since: &str, opts: &SyncOptions) -> Result<(SyncReport, String)> {
    let id = st.mailbox(folder)?.to_string();
    let mut fs = store.load_folder(account, folder)?;
    let mut report = SyncReport::default();
    let mut state = since.to_string();
    let mut created: Vec<String> = Vec::new();
    let set = |m: &mut Summary, f: &Flags, report: &mut SyncReport| {
        if take_marks(&mut m.flags, f) {
            report.changed += 1;
        }
    };
    for _ in 0..CHANGE_PAGES {
        let (list, next, last) = c.changes_since(&id, &state)?;
        for ch in list {
            match ch {
                Change::Created(item, f) => match st.known(&item).and_then(|n| fs.find_mut(n)) {
                    Some(m) => set(m, &f, &mut report),
                    None => created.push(item),
                },
                // Only what's kept here: a change to a message past the window isn't news.
                Change::Updated(item, f) => {
                    if let Some(m) = st.known(&item).and_then(|n| fs.find_mut(n)) {
                        set(m, &f, &mut report);
                    }
                }
                Change::Read(item, on) => {
                    if let Some(m) = st.known(&item).and_then(|n| fs.find_mut(n))
                        && m.flags.seen != on
                    {
                        m.flags.seen = on;
                        report.changed += 1;
                    }
                }
                Change::Deleted(item) => {
                    if let Some(n) = st.known(&item)
                        && let Some(at) = fs.messages.iter().position(|m| m.key.uid == n)
                    {
                        let m = fs.messages.remove(at);
                        store.remove_body(&m.key);
                        report.removed += 1;
                    }
                }
            }
        }
        state = next;
        if last {
            break;
        }
    }
    created.dedup();
    for it in c.headers(&created)? {
        let Some(item) = it.child("ItemId").and_then(|i| i.attr("Id")) else { continue };
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: st.number(item) };
        if fs.find(key.uid).is_some() {
            continue;
        }
        fs.messages.push(item_summary(&it, key));
        report.new += 1;
    }
    // Mail grown older than what's kept goes, as Outlook lets it go from the offline file (a
    // search of the server's finds stay while they're there); followed by its changes, a
    // folder kept everything it ever had.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    if let Some(oldest) = opts.since(now) {
        let (keep, old): (Vec<Summary>, Vec<Summary>) =
            std::mem::take(&mut fs.messages).into_iter().partition(|m| m.received_at() >= oldest || st.was_found(folder, m.key.uid));
        for m in &old {
            store.remove_body(&m.key);
        }
        report.removed += old.len();
        fs.messages = keep;
    }
    if report.touched() {
        sort_newest_first(&mut fs.messages);
        fs.uid_next = fs.messages.iter().map(|m| m.key.uid).max().unwrap_or(0).saturating_add(1);
        store.save_folder(account, folder, &fs)?;
    }
    prefetch(c, st, store, account, folder, &mut fs, opts)?;
    Ok((report, state))
}

/// An item's list entry from its details.
fn item_summary(it: &El, key: MessageKey) -> Summary {
    let received = it.text_of("DateTimeReceived").and_then(jmap::utc_date).unwrap_or(0);
    let size = it.text_of("Size").and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(0);
    let mut s = mime::summary_from_header(&header_block(it), key, flags(it), size, received);
    s.has_attachments = it.text_of("HasAttachments").is_some_and(|v| v == "true");
    s.voting = voting_choices(it);
    s
}

/// The messages of `folder` the server finds for a search (issue #4), the newest `cap` of them:
/// its words as a `QueryString` (AQS), or, with none, its read and flag conditions as a
/// `Restriction`. Those not kept yet come into its list and stay there past the window while the
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
    let ids = c.find(&id, query, cap.min(jmap::FOUND_MAX))?;
    let numbers: Vec<u32> = ids.iter().map(|i| st.number(i)).collect();
    let missing: Vec<String> = ids.iter().zip(&numbers).filter(|(_, n)| fs.find(**n).is_none()).map(|(i, _)| i.clone()).collect();
    let mut new = 0;
    for it in c.headers(&missing)? {
        let Some(i) = it.child("ItemId").and_then(|i| i.attr("Id")) else { continue };
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: st.number(i) };
        if fs.find(key.uid).is_none() {
            fs.messages.push(item_summary(&it, key));
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

/// A `Restriction` for a search's read, flag and attachment conditions.
pub(super) fn restriction(q: &WordQuery) -> String {
    let equal = |field: &str, value: &str| {
        format!("<t:IsEqualTo>{field}<t:FieldURIOrConstant><t:Constant Value=\"{value}\"/></t:FieldURIOrConstant></t:IsEqualTo>")
    };
    let mut parts = Vec::new();
    if let Some(u) = q.unread {
        parts.push(equal("<t:FieldURI FieldURI=\"message:IsRead\"/>", if u { "false" } else { "true" }));
    }
    if let Some(on) = q.flagged {
        let e = equal("<t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/>", "2");
        parts.push(if on { e } else { format!("<t:Not>{e}</t:Not>") });
    }
    if let Some(a) = q.attachment {
        parts.push(equal("<t:FieldURI FieldURI=\"item:HasAttachments\"/>", if a { "true" } else { "false" }));
    }
    match parts.len() {
        0 => String::new(),
        1 => format!("<m:Restriction>{}</m:Restriction>", parts.concat()),
        _ => format!("<m:Restriction><t:And>{}</t:And></m:Restriction>", parts.concat()),
    }
}

/// Messages downloaded ahead in one request at most, and their size together: one request per
/// message, a thousand messages a folder, was most of a first sync.
const AHEAD_BATCH: usize = 10;
const AHEAD_BATCH_BYTES: u64 = 8 << 20;

fn prefetch(c: &Client, st: &State, store: &Store, account: &str, folder: &str, fs: &mut FolderState, opts: &SyncOptions) -> Result<()> {
    let wanted: Vec<(u32, String, u64)> = fs
        .messages
        .iter()
        .take(opts.prefetch)
        .filter(|m| u64::from(m.size) <= u64::from(opts.prefetch_max_bytes) && !store.has_body(&m.key))
        .filter_map(|m| Some((m.key.uid, st.id(m.key.uid)?, u64::from(m.size))))
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }
    download(c, store, account, folder, Some(fs), wanted, true)?;
    store.save_folder(account, folder, fs)?;
    Ok(())
}

/// What of a folder is still to be downloaded ahead, worked out once — the whole list read and
/// every message looked up took a third of a second in a folder of 50,000, at every step of
/// twenty — and trusted while the folder's list stays the same ([`Store::folder_version`]).
#[derive(Debug, Default)]
pub struct AheadTodo {
    folder: String,
    version: u64,
    /// Each message's number and size, the newest last (taken from the end).
    left: Vec<(u32, u64)>,
}

/// Download at most `budget` of a folder's kept messages not on this computer yet, newest
/// first — whole, however big, when everything kept is to be ([`SyncOptions::full`], as Cached
/// Exchange Mode keeps its offline file) — while nothing else is asked; how many are still
/// missing. Only the sources are written: the folder's list stays as its sync saved it (written
/// again for every few messages, a big folder's list was gigabytes).
#[allow(clippy::too_many_arguments)]
pub fn download_ahead(
    c: &Client,
    st: &State,
    store: &Store,
    account: &str,
    folder: &str,
    opts: &SyncOptions,
    budget: usize,
    todo: &mut AheadTodo,
) -> Result<usize> {
    let version = store.folder_version(account, folder);
    if todo.folder != folder || todo.version != version || version == 0 {
        let fs = store.load_folder(account, folder)?;
        let cap = if opts.full { u64::MAX } else { u64::from(opts.prefetch_max_bytes) };
        let mut left: Vec<(u32, u64)> =
            fs.messages.iter().filter(|m| u64::from(m.size) <= cap && !store.has_body(&m.key)).map(|m| (m.key.uid, u64::from(m.size))).collect();
        left.reverse();
        *todo = AheadTodo { folder: folder.to_string(), version, left };
    }
    let mut now: Vec<(u32, String, u64)> = Vec::new();
    while now.len() < budget
        && let Some((n, size)) = todo.left.pop()
    {
        // Opened meanwhile, or no longer numbered: nothing to do.
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: n };
        if let Some(id) = st.id(n).filter(|_| !store.has_body(&key)) {
            now.push((n, id.to_string(), size));
        }
    }
    download(c, store, account, folder, None, now, false)?;
    Ok(todo.left.len())
}

/// Download these messages' sources (uid, item id, size) and keep them; with `fill`, their list
/// entries (in `fs`) take what the sources say. How many were written.
fn download(
    c: &Client,
    store: &Store,
    account: &str,
    folder: &str,
    mut fs: Option<&mut FolderState>,
    wanted: Vec<(u32, String, u64)>,
    fill: bool,
) -> Result<usize> {
    // Batches of a few, kept small in bytes (one big message goes alone).
    let mut batches: Vec<Vec<(u32, String)>> = Vec::new();
    let mut bytes = 0u64;
    for (n, id, size) in wanted {
        let full = batches.last().is_none_or(|b| b.len() >= AHEAD_BATCH || bytes.saturating_add(size) > AHEAD_BATCH_BYTES);
        if full {
            batches.push(Vec::new());
            bytes = 0;
        }
        bytes = bytes.saturating_add(size);
        if let Some(b) = batches.last_mut() {
            b.push((n, id));
        }
    }
    let mut failures = crate::sync::AheadFailures::new();
    let mut written = 0usize;
    for batch in batches {
        let ids: Vec<String> = batch.iter().map(|(_, id)| id.clone()).collect();
        let answers = match c.download_many(&ids) {
            Ok(a) => a,
            Err(e) => {
                failures.failed(&format!("{account}/{folder}: {} messages", ids.len()), e)?;
                continue;
            }
        };
        for ((n, _), answer) in batch.iter().zip(answers) {
            let raw = match answer {
                Ok(raw) => {
                    failures.ok();
                    raw
                }
                Err(e) => {
                    failures.failed(&format!("{account}/{folder}: message {n}"), e)?;
                    continue;
                }
            };
            match fs.as_deref_mut() {
                Some(fs) => {
                    let Some(m) = fs.find_mut(*n) else { continue };
                    store.write_body(&m.key, &raw)?;
                    if fill {
                        fill_from_source(m, &raw);
                    }
                }
                None => {
                    store.write_body(&MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: *n }, &raw)?
                }
            }
            written += 1;
        }
    }
    Ok(written)
}

/// The item id of the message whose `Message-ID` is `mid`, in a folder (Exchange keeps it with
/// its angle brackets; both are tried).
fn id_by_message_id(c: &Client, folder_id: &str, mid: &str) -> Result<Option<String>> {
    for want in [format!("<{mid}>"), mid.to_string()] {
        let m = c.call_one(&format!(
            "<m:FindItem Traversal=\"Shallow\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape></m:ItemShape>\
             <m:IndexedPageItemView MaxEntriesReturned=\"1\" Offset=\"0\" BasePoint=\"Beginning\"/>\
             <m:ParentFolderIds><t:FolderId Id=\"{}\"/></m:ParentFolderIds>\
             <m:Restriction><t:IsEqualTo><t:FieldURI FieldURI=\"message:InternetMessageId\"/>\
             <t:FieldURIOrConstant><t:Constant Value=\"{}\"/></t:FieldURIOrConstant></t:IsEqualTo></m:Restriction></m:FindItem>",
            esc(folder_id),
            esc(&want)
        ))?;
        if let Some(id) = m.find("Items").and_then(|items| items.kids.first()).and_then(|it| it.child("ItemId")).and_then(|i| i.attr("Id")) {
            return Ok(Some(id.to_string()));
        }
    }
    Ok(None)
}

/// Download one message's source (and cache it).
pub fn fetch_body(c: &Client, st: &mut State, store: &Store, key: &MessageKey) -> Result<Vec<u8>> {
    if let Some(raw) = store.read_body(key) {
        return Ok(raw);
    }
    let id = server_id(c, st, store, key)?.ok_or_else(|| NetError::No("the message is no longer on the server".into()))?;
    let raw = c.download(&id)?;
    store.write_body(key, &raw)?;
    let mut fs = store.load_folder(&key.account, &key.folder)?;
    if let Some(m) = fs.find_mut(key.uid) {
        fill_from_source(m, &raw);
        store.save_folder(&key.account, &key.folder, &fs)?;
    }
    Ok(raw)
}

/// The server's id for a cached message. When the numbering no longer has it (a state reloaded,
/// a folder listed under another numbering, a reader's numbers lost with a quit mid-sync), it's
/// still there and the cache knows its Message-ID: found again and numbered as it was, rather
/// than an old message said to be gone, or a change to it never made on the server. `None`:
/// not found.
pub fn server_id(c: &Client, st: &mut State, store: &Store, key: &MessageKey) -> Result<Option<String>> {
    if let Some(id) = st.id(key.uid).filter(|_| key.validity == VALIDITY) {
        return Ok(Some(id.to_string()));
    }
    let mid = store.load_folder(&key.account, &key.folder).ok().and_then(|f| f.find(key.uid).map(|m| m.message_id.clone())).unwrap_or_default();
    let folder_id = st.mailbox(&key.folder).ok().map(str::to_string);
    let again = match (mid.is_empty(), folder_id) {
        (false, Some(folder_id)) => id_by_message_id(c, &folder_id, &mid)?,
        _ => None,
    };
    match &again {
        Some(id) => {
            log::info!("{}: {} #{} wasn't in the numbering: found again by its Message-ID", key.account, utf7::decode(&key.folder), key.uid);
            st.set_number(id, key.uid);
        }
        None => log::warn!(
            "{}: {} #{} (validity {}) isn't in the numbering ({} ids) and wasn't found by its Message-ID",
            key.account,
            utf7::decode(&key.folder),
            key.uid,
            key.validity,
            st.len()
        ),
    }
    Ok(again)
}

/// Messages a change names that the numbering lost, looked for again ([`server_id`]) before the
/// change goes to the server, a few at most: left out, the change was made here only and the
/// server's state came back over it at the next full read.
pub fn find_lost(c: &Client, st: &mut State, store: &Store, keys: &[MessageKey]) -> Result<()> {
    let lost: Vec<&MessageKey> = keys.iter().filter(|k| k.validity == VALIDITY && st.id(k.uid).is_none()).collect();
    if lost.len() > LOST_LOOKED_FOR {
        let account = keys.first().map_or("", |k| k.account.as_str());
        log::warn!("{account}: {} messages of a change have no server id: the first {LOST_LOOKED_FOR} are looked for", lost.len());
    }
    for key in lost.into_iter().take(LOST_LOOKED_FOR) {
        server_id(c, st, store, key)?;
    }
    Ok(())
}

/// How many lost messages one change looks for ([`find_lost`]): each is a search of its folder.
const LOST_LOOKED_FOR: usize = 50;
