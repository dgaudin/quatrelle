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
    let newest = c.newest(&id, opts.window)?;
    let (total, unread) = c.counts(&id)?;
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
    report.total = total;
    report.unread = unread;
    Ok(report)
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

fn prefetch(c: &Client, st: &State, store: &Store, account: &str, folder: &str, fs: &mut FolderState, opts: &SyncOptions) -> Result<()> {
    let wanted: Vec<(u32, String)> = fs
        .messages
        .iter()
        .take(opts.prefetch)
        .filter(|m| u64::from(m.size) <= u64::from(opts.prefetch_max_bytes) && !store.has_body(&m.key))
        .filter_map(|m| Some((m.key.uid, st.id(m.key.uid)?.to_string())))
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }
    let mut failures = crate::sync::AheadFailures::new();
    for (n, id) in &wanted {
        let raw = match c.download(id) {
            Ok(raw) => {
                failures.ok();
                raw
            }
            Err(e) => {
                failures.failed(&format!("{account}/{folder}: message {n}"), e)?;
                continue;
            }
        };
        let Some(m) = fs.find_mut(*n) else { continue };
        store.write_body(&m.key, &raw)?;
        fill_from_source(m, &raw);
    }
    store.save_folder(account, folder, fs)?;
    Ok(())
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
    let known = st.id(key.uid).filter(|_| key.validity == VALIDITY).map(str::to_string);
    let id = match known {
        Some(id) => id,
        None => {
            // The numbering no longer has this message (a state reloaded, a folder listed under
            // another numbering): it is still there, and the cache knows its Message-ID. Find it
            // again and number it, rather than telling the reader an old message is gone.
            let mid =
                store.load_folder(&key.account, &key.folder).ok().and_then(|f| f.find(key.uid).map(|m| m.message_id.clone())).unwrap_or_default();
            let folder_id = st.mailbox(&key.folder).ok().map(str::to_string);
            let again = match (mid.is_empty(), folder_id) {
                (false, Some(folder_id)) => id_by_message_id(c, &folder_id, &mid)?,
                _ => None,
            };
            match again {
                Some(id) => {
                    log::info!("{}: {} #{} wasn't in the numbering: found again by its Message-ID", key.account, utf7::decode(&key.folder), key.uid);
                    st.set_number(&id, key.uid);
                    id
                }
                None => {
                    log::warn!(
                        "{}: {} #{} (validity {}) isn't in the numbering ({} ids) and wasn't found by its Message-ID",
                        key.account,
                        utf7::decode(&key.folder),
                        key.uid,
                        key.validity,
                        st.len()
                    );
                    return Err(NetError::No("the message is no longer on the server".into()));
                }
            }
        }
    };
    let raw = c.download(&id)?;
    store.write_body(key, &raw)?;
    let mut fs = store.load_folder(&key.account, &key.folder)?;
    if let Some(m) = fs.find_mut(key.uid) {
        fill_from_source(m, &raw);
        store.save_folder(&key.account, &key.folder, &fs)?;
    }
    Ok(raw)
}
