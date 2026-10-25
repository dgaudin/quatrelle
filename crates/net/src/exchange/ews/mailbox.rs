//! EWS: the mailbox (its folders, their messages, sending, automatic replies).

use super::*;

/// The folders Exchange makes itself at the top of a mailbox, by name: some servers don't mark
/// them hidden.
pub(super) const SYSTEM_FOLDERS: [&str; 5] = ["Working Set", "Yammer Root", "PersonMetadata", "ExternalContacts", "Social Activity Notifications"];

impl Client {
    /// The account's mail folders, with their roles and counts.
    pub fn mailboxes(&self) -> Result<Vec<Mailbox>> {
        let roles = SPECIAL;
        let hidden = ["msgfolderroot", "outbox", "conversationhistory", "syncissues"];
        let ids: String = roles.iter().map(|(n, _)| *n).chain(hidden).map(|n| distinguished(n, None)).collect();
        let doc = self.call(&format!(
            "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape><m:FolderIds>{ids}</m:FolderIds></m:GetFolder>"
        ))?;
        // One answer per asked-for folder, in order; missing ones are errors.
        let found: Vec<Option<String>> = messages(&doc)
            .iter()
            .map(|m| if m.attr("ResponseClass") == Some("Error") { None } else { m.find("FolderId").and_then(|f| f.attr("Id")).map(str::to_string) })
            .collect();
        let role_of: HashMap<String, FolderRole> = roles.iter().zip(&found).filter_map(|((_, r), id)| Some((id.clone()?, *r))).collect();
        let root = found.get(roles.len()).cloned().flatten();
        let skip: Vec<String> = found.iter().skip(roles.len() + 1).flatten().cloned().collect();

        let mut out: Vec<Mailbox> = Vec::new();
        let mut offset = 0;
        loop {
            let doc = self.call(&format!(
                "<m:FindFolder Traversal=\"Deep\"><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
                 <t:FieldURI FieldURI=\"folder:DisplayName\"/><t:FieldURI FieldURI=\"folder:ParentFolderId\"/>\
                 <t:FieldURI FieldURI=\"folder:TotalCount\"/><t:FieldURI FieldURI=\"folder:UnreadCount\"/>\
                 <t:FieldURI FieldURI=\"folder:FolderClass\"/>{HIDDEN}\
                 </t:AdditionalProperties></m:FolderShape>\
                 <m:IndexedPageFolderView MaxEntriesReturned=\"500\" Offset=\"{offset}\" BasePoint=\"Beginning\"/>\
                 <m:ParentFolderIds>{}</m:ParentFolderIds></m:FindFolder>",
                distinguished("msgfolderroot", None)
            ))?;
            let Some(m) = messages(&doc).first().map(|m| (*m).clone()) else { break };
            check_message(&m)?;
            let Some(rf) = m.child("RootFolder") else { break };
            let folders = rf.folders();
            let n = folders.len();
            for f in folders {
                let Some(id) = f.child("FolderId").and_then(|i| i.attr("Id")).map(str::to_string) else { continue };
                let class = f.text_of("FolderClass").unwrap_or("");
                // Exchange's own hidden folders aren't the user's mail: Outlook doesn't show
                // them, and some take long to read.
                let hidden = is_hidden(f);
                let top = f.child("ParentFolderId").and_then(|p| p.attr("Id")).is_some_and(|p| Some(p) == root.as_deref());
                if hidden || (top && f.text_of("DisplayName").is_some_and(|n| SYSTEM_FOLDERS.contains(&n.trim()))) {
                    continue;
                }
                if !(class.is_empty() || class == "IPF.Note" || class.starts_with("IPF.Note.")) || skip.contains(&id) || out.len() >= MAX_FOLDERS {
                    continue;
                }
                let parent = f.child("ParentFolderId").and_then(|p| p.attr("Id")).map(str::to_string).filter(|p| Some(p) != root.as_ref());
                let count = |k: &str| f.text_of(k).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
                out.push(Mailbox {
                    name: f.text_of("DisplayName").unwrap_or_default().chars().take(255).collect(),
                    parent,
                    role: role_of.get(&id).copied().unwrap_or(FolderRole::Other),
                    total: count("TotalCount"),
                    unread: count("UnreadCount"),
                    id,
                });
            }
            let last = rf.attr("IncludesLastItemInRange").is_none_or(|v| v == "true");
            offset += n;
            if last || n == 0 || out.len() >= MAX_FOLDERS {
                break;
            }
        }
        keep_rooted(&mut out);
        // Exchange's public folders, under a folder of their own (a server without them, or
        // one that won't show them, just has none).
        match self.public_folders() {
            Ok(pf) => out.extend(pf),
            Err(e) => log::info!("no public folders: {e}"),
        }
        // Shared mailboxes, each under a folder of its own (named so it can't be taken for one
        // of the account's folders).
        let mut open = Vec::new();
        self.shared_roles.borrow_mut().clear();
        for sm in self.shared_mailboxes() {
            let taken = |n: &str| out.iter().any(|m| m.parent.is_none() && m.name.eq_ignore_ascii_case(n));
            let top = if taken(&sm.name) { format!("{} ({})", sm.name, sm.address) } else { sm.name.clone() };
            match self.shared_folders(&sm, &top) {
                Ok(f) if !f.is_empty() => {
                    log::info!("shared mailbox {}: {} folders", sm.address, f.len().saturating_sub(1));
                    open.push(sm.address.clone());
                    out.extend(f);
                }
                Ok(_) => log::info!("shared mailbox {}: no folders", sm.address),
                Err(e) => log::warn!("shared mailbox {}: {e}", sm.address),
            }
        }
        *self.shared_open.borrow_mut() = open;
        Ok(out)
    }

    /// A folder's (total, unread).
    /// A folder's counts, and when anything in it last changed (`PR_LOCAL_COMMIT_TIME_MAX`,
    /// empty when the server doesn't say): `(total, unread, stamp)`.
    pub(super) fn counts(&self, folder: &str) -> Result<(u32, u32, String)> {
        let m = self.call_one(&format!(
            "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
             <t:FieldURI FieldURI=\"folder:TotalCount\"/><t:FieldURI FieldURI=\"folder:UnreadCount\"/>\
             <t:ExtendedFieldURI PropertyTag=\"0x670A\" PropertyType=\"SystemTime\"/></t:AdditionalProperties></m:FolderShape>\
             <m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:GetFolder>",
            esc(folder)
        ))?;
        let f = m.folders().into_iter().next();
        let count = |k: &str| f.and_then(|f| f.text_of(k)).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
        let stamp = f.and_then(|f| extended_text(f, "0x670A")).map(|v| v.trim().to_string()).unwrap_or_default();
        Ok((count("TotalCount"), count("UnreadCount"), stamp))
    }

    /// The newest `limit` messages' ids and flags, a page of up to a thousand at a time (EWS
    /// pages by offset; it says when the last page is in); with `since`, only those received
    /// since then (Unix seconds), as Outlook keeps mail offline in Cached Exchange Mode.
    pub(super) fn newest(&self, folder: &str, limit: usize, since: Option<i64>) -> Result<Vec<(String, Flags)>> {
        self.newest_where(folder, limit, since, false)
    }

    /// [`Client::newest`], those carrying categories only (what Outlook put on them).
    pub(super) fn categorized(&self, folder: &str, limit: usize, since: Option<i64>) -> Result<Vec<(String, Flags)>> {
        self.newest_where(folder, limit, since, true)
    }

    fn newest_where(&self, folder: &str, limit: usize, since: Option<i64>, categorized: bool) -> Result<Vec<(String, Flags)>> {
        let limit = limit.clamp(1, MAX_KEPT);
        let mut conditions = String::new();
        if categorized {
            conditions.push_str("<t:Exists><t:FieldURI FieldURI=\"item:Categories\"/></t:Exists>");
        }
        if let Some(t) = since {
            conditions.push_str(&format!(
                "<t:IsGreaterThanOrEqualTo><t:FieldURI FieldURI=\"item:DateTimeReceived\"/><t:FieldURIOrConstant><t:Constant Value=\"{}\"/></t:FieldURIOrConstant></t:IsGreaterThanOrEqualTo>",
                crate::ews_pim::utc(t)
            ));
        }
        let restriction = match (categorized, since.is_some()) {
            (false, false) => String::new(),
            (true, true) => format!("<m:Restriction><t:And>{conditions}</t:And></m:Restriction>"),
            _ => format!("<m:Restriction>{conditions}</m:Restriction>"),
        };
        let mut out: Vec<(String, Flags)> = Vec::new();
        while out.len() < limit {
            let page = (limit - out.len()).min(1000);
            let m = self.call_one(&format!(
                "<m:FindItem Traversal=\"Shallow\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{FLAG_FIELDS}</t:AdditionalProperties></m:ItemShape>\
                 <m:IndexedPageItemView MaxEntriesReturned=\"{}\" Offset=\"{}\" BasePoint=\"Beginning\"/>{restriction}\
                 <m:SortOrder><t:FieldOrder Order=\"Descending\"><t:FieldURI FieldURI=\"item:DateTimeReceived\"/></t:FieldOrder></m:SortOrder>\
                 <m:ParentFolderIds><t:FolderId Id=\"{}\"/></m:ParentFolderIds></m:FindItem>",
                page,
                out.len(),
                esc(folder)
            ))?;
            let last = m.find("RootFolder").and_then(|r| r.attr("IncludesLastItemInRange")) != Some("false");
            let Some(items) = m.find("Items") else { break };
            let got: Vec<(String, Flags)> =
                items.kids.iter().filter_map(|it| Some((it.child("ItemId")?.attr("Id")?.to_string(), flags(it)))).collect();
            let n = got.len();
            out.extend(got);
            if last || n == 0 || n < page {
                break;
            }
        }
        Ok(out)
    }

    /// The newest `limit` ids of a folder's messages a search finds (one page: `limit` is at most
    /// a few hundred).
    pub(super) fn find(&self, folder: &str, q: &WordQuery, limit: usize) -> Result<Vec<String>> {
        let (restriction, words) = if q.text.is_empty() {
            (restriction(q), String::new())
        } else {
            (String::new(), format!("<m:QueryString>{}</m:QueryString>", esc(&q.text)))
        };
        let m = self.call_one(&format!(
            "<m:FindItem Traversal=\"Shallow\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{FLAG_FIELDS}\
             <t:FieldURI FieldURI=\"item:HasAttachments\"/></t:AdditionalProperties></m:ItemShape>\
             <m:IndexedPageItemView MaxEntriesReturned=\"{}\" Offset=\"0\" BasePoint=\"Beginning\"/>{restriction}\
             <m:SortOrder><t:FieldOrder Order=\"Descending\"><t:FieldURI FieldURI=\"item:DateTimeReceived\"/></t:FieldOrder></m:SortOrder>\
             <m:ParentFolderIds><t:FolderId Id=\"{}\"/></m:ParentFolderIds>{words}</m:FindItem>",
            limit.clamp(1, 1000),
            esc(folder)
        ))?;
        let Some(items) = m.find("Items") else { return Ok(Vec::new()) };
        Ok(items
            .kids
            .iter()
            .filter(|it| q.admits(&flags(it), it.text_of("HasAttachments").is_some_and(|v| v == "true")))
            .filter_map(|it| Some(it.child("ItemId")?.attr("Id")?.to_string()))
            .take(limit)
            .collect())
    }

    /// Of these messages, those still in a folder, with their flags.
    pub(super) fn still_in(&self, folder: &str, ids: &[String]) -> Result<Vec<(String, Flags)>> {
        let mut out = Vec::new();
        for chunk in ids.chunks(BATCH) {
            let list: String = chunk.iter().map(|id| format!("<t:ItemId Id=\"{}\"/>", esc(id))).collect();
            let doc = self.call(&format!(
                "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{FLAG_FIELDS}\
                 <t:FieldURI FieldURI=\"item:ParentFolderId\"/></t:AdditionalProperties></m:ItemShape><m:ItemIds>{list}</m:ItemIds></m:GetItem>"
            ))?;
            for m in messages(&doc).into_iter().filter(|m| m.attr("ResponseClass") != Some("Error")) {
                for it in m.child("Items").map(|i| i.kids.as_slice()).unwrap_or_default() {
                    let Some(id) = it.child("ItemId").and_then(|i| i.attr("Id")) else { continue };
                    if it.child("ParentFolderId").and_then(|p| p.attr("Id")).is_none_or(|p| p == folder) {
                        out.push((id.to_string(), flags(it)));
                    }
                }
            }
        }
        Ok(out)
    }

    /// What changed in a folder since `state` (`SyncFolderItems`; empty: from nothing, every
    /// message a creation), a page of up to [`CHANGES_PAGE`]: the changes, the state to ask from
    /// next, and whether that was all of them.
    pub(super) fn changes_since(&self, folder: &str, state: &str) -> Result<(Vec<Change>, String, bool)> {
        let since = if state.is_empty() { String::new() } else { format!("<m:SyncState>{}</m:SyncState>", esc(state)) };
        let m = self.call_one(&format!(
            "<m:SyncFolderItems><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{FLAG_FIELDS}</t:AdditionalProperties></m:ItemShape>\
             <m:SyncFolderId><t:FolderId Id=\"{}\"/></m:SyncFolderId>{since}<m:MaxChangesReturned>{CHANGES_PAGE}</m:MaxChangesReturned>\
             <m:SyncScope>NormalItems</m:SyncScope></m:SyncFolderItems>",
            esc(folder)
        ))?;
        let next = m
            .text_of("SyncState")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| NetError::No("no sync state in the answer".into()))?;
        let last = m.text_of("IncludesLastItemInRange").is_none_or(|v| v.trim() == "true");
        let id_of = |el: &El| el.find("ItemId").and_then(|i| i.attr("Id")).map(str::to_string);
        let mut out = Vec::new();
        for ch in m.child("Changes").map(|c| c.kids.as_slice()).unwrap_or_default() {
            // A creation or an update carries the item (a message, a meeting request…).
            let item = || ch.kids.iter().find(|k| k.child("ItemId").is_some());
            match ch.name.as_str() {
                "Create" => {
                    if let Some(it) = item()
                        && let Some(id) = id_of(it)
                    {
                        out.push(Change::Created(id, flags(it)));
                    }
                }
                "Update" => {
                    if let Some(it) = item()
                        && let Some(id) = id_of(it)
                    {
                        out.push(Change::Updated(id, flags(it)));
                    }
                }
                "Delete" => {
                    if let Some(id) = id_of(ch) {
                        out.push(Change::Deleted(id));
                    }
                }
                "ReadFlagChange" => {
                    if let Some(id) = id_of(ch) {
                        out.push(Change::Read(id, ch.text_of("IsRead").is_some_and(|v| v.trim() == "true")));
                    }
                }
                _ => {}
            }
        }
        Ok((out, next, last))
    }

    /// Messages' details, for the list.
    pub(super) fn headers(&self, ids: &[String]) -> Result<Vec<El>> {
        let mut out = Vec::new();
        for chunk in ids.chunks(BATCH) {
            let list: String = chunk.iter().map(|id| format!("<t:ItemId Id=\"{}\"/>", esc(id))).collect();
            let doc = self.call(&format!(
                "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{HEADER_FIELDS}{FLAG_FIELDS}</t:AdditionalProperties></m:ItemShape>\
                 <m:ItemIds>{list}</m:ItemIds></m:GetItem>"
            ))?;
            for m in messages(&doc) {
                if m.attr("ResponseClass") == Some("Error") {
                    continue;
                }
                if let Some(items) = m.child("Items") {
                    out.extend(items.kids.iter().cloned());
                }
            }
        }
        Ok(out)
    }

    /// A message's MIME source.
    /// Several messages' sources in one request, each with its own answer (one the server won't
    /// give fails alone), in the order asked.
    pub(super) fn download_many(&self, ids: &[String]) -> Result<Vec<Result<Vec<u8>>>> {
        let list: String = ids.iter().map(|id| format!("<t:ItemId Id=\"{}\"/>", esc(id))).collect();
        let doc = self.call(&format!(
            "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:IncludeMimeContent>true</t:IncludeMimeContent></m:ItemShape>\
             <m:ItemIds>{list}</m:ItemIds></m:GetItem>"
        ))?;
        let answers = messages(&doc);
        if answers.len() != ids.len() {
            return Err(NetError::Protocol(format!("{} answers for {} messages", answers.len(), ids.len())));
        }
        Ok(answers
            .into_iter()
            .map(|m| {
                check_message(m)?;
                let mime = m.find("MimeContent").ok_or_else(|| NetError::Protocol("the message has no source".into()))?;
                unb64(&mime.text).ok_or_else(|| NetError::Protocol("a damaged message source".into()))
            })
            .collect())
    }

    pub fn download(&self, id: &str) -> Result<Vec<u8>> {
        let m = self.call_one(&format!(
            "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:IncludeMimeContent>true</t:IncludeMimeContent></m:ItemShape>\
             <m:ItemIds><t:ItemId Id=\"{}\"/></m:ItemIds></m:GetItem>",
            esc(id)
        ))?;
        let mime = m.find("MimeContent").ok_or_else(|| NetError::Protocol("the message has no source".into()))?;
        unb64(&mime.text).ok_or_else(|| NetError::Protocol("a damaged message source".into()))
    }

    /// Store a MIME message in a folder (as a draft when it's one); its id.
    pub fn create(&self, raw: &[u8], folder: &str, draft: bool) -> Result<String> {
        let flags = if draft {
            "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x0E07\" PropertyType=\"Integer\"/><t:Value>8</t:Value></t:ExtendedProperty>"
        } else {
            ""
        };
        let m = self.call_one(&format!(
            "<m:CreateItem MessageDisposition=\"SaveOnly\"><m:SavedItemFolderId><t:FolderId Id=\"{}\"/></m:SavedItemFolderId>\
             <m:Items><t:Message><t:MimeContent CharacterSet=\"UTF-8\">{}</t:MimeContent>{flags}</t:Message></m:Items></m:CreateItem>",
            esc(folder),
            b64(raw)
        ))?;
        Ok(m.find("ItemId").and_then(|i| i.attr("Id")).unwrap_or_default().to_string())
    }

    /// Answer a message's voting buttons with `choice`: a reply to it (so it stays in the
    /// conversation), its subject the choice's, carrying the choice (`PidLidVerbResponse`) for the
    /// sender's mail client to count; sent, with a copy in Sent Items.
    pub fn vote(&self, id: &str, choice: &str, subject: &str) -> Result<()> {
        let item = |m: &El| {
            let i = m.find("ItemId");
            let id = i.and_then(|i| i.attr("Id")).unwrap_or_default().to_string();
            let ck = i.and_then(|i| i.attr("ChangeKey")).filter(|c| !c.is_empty()).map(|c| format!(" ChangeKey=\"{}\"", esc(c))).unwrap_or_default();
            (id, ck)
        };
        let original = self.call_one(&format!(
            "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape></m:ItemShape><m:ItemIds><t:ItemId Id=\"{}\"/></m:ItemIds></m:GetItem>",
            esc(id)
        ))?;
        let (oid, ock) = item(&original);
        let reply = self.call_one(&format!(
            "<m:CreateItem MessageDisposition=\"SaveOnly\"><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"drafts\"/></m:SavedItemFolderId>\
             <m:Items><t:ReplyToItem><t:ReferenceItemId Id=\"{}\"{ock}/></t:ReplyToItem></m:Items></m:CreateItem>",
            esc(&oid)
        ))?;
        let (rid, rck) = item(&reply);
        if rid.is_empty() {
            return Err(NetError::Protocol("Exchange didn't make the reply".into()));
        }
        let field =
            format!("<t:ExtendedFieldURI DistinguishedPropertySetId=\"Common\" PropertyId=\"{}\" PropertyType=\"String\"/>", voting::VERB_RESPONSE);
        let updated = self.call_one(&format!(
            "<m:UpdateItem MessageDisposition=\"SaveOnly\" ConflictResolution=\"AlwaysOverwrite\"><m:ItemChanges><t:ItemChange>\
             <t:ItemId Id=\"{}\"{rck}/><t:Updates>\
             <t:SetItemField><t:FieldURI FieldURI=\"item:Subject\"/><t:Message><t:Subject>{}</t:Subject></t:Message></t:SetItemField>\
             <t:SetItemField>{field}<t:Message><t:ExtendedProperty>{field}<t:Value>{}</t:Value></t:ExtendedProperty></t:Message></t:SetItemField>\
             </t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>",
            esc(&rid),
            esc(&voting::response_subject(choice, subject)),
            esc(choice)
        ))?;
        let (_, uck) = item(&updated);
        let ck = if uck.is_empty() { rck } else { uck };
        self.call_one(&format!(
            "<m:SendItem SaveItemToFolder=\"true\"><m:ItemIds><t:ItemId Id=\"{}\"{ck}/></m:ItemIds>\
             <m:SavedItemFolderId><t:DistinguishedFolderId Id=\"sentitems\"/></m:SavedItemFolderId></m:SendItem>",
            esc(&rid)
        ))
        .map(drop)
    }

    /// Send a MIME message, keeping a copy in Sent Items.
    pub fn send_mime(&self, raw: &[u8]) -> Result<()> {
        // Voting buttons asked for: Exchange's verb stream ([MS-OXOMSG] PidLidVerbStream), which
        // Outlook shows as buttons; the header that carried them here stays here.
        let (raw, choices) = mime::take_voting(raw);
        let verbs = if choices.is_empty() {
            String::new()
        } else {
            let names: Vec<&str> = choices.iter().map(String::as_str).collect();
            format!(
                "<t:ExtendedProperty><t:ExtendedFieldURI DistinguishedPropertySetId=\"Common\" PropertyId=\"{}\" PropertyType=\"Binary\"/>\
                 <t:Value>{}</t:Value></t:ExtendedProperty>",
                voting::VERB_STREAM,
                b64(&voting::stream(&names))
            )
        };
        self.call_one(&format!(
            "<m:CreateItem MessageDisposition=\"SendAndSaveCopy\"><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"sentitems\"/></m:SavedItemFolderId>\
             <m:Items><t:Message><t:MimeContent CharacterSet=\"UTF-8\">{}</t:MimeContent>{verbs}</t:Message></m:Items></m:CreateItem>",
            b64(&raw)
        ))
        .map(drop)
    }

    /// The automatic replies.
    pub fn vacation(&self) -> Result<quatrelle_model::job::Vacation> {
        let doc = self.call(&format!(
            "<m:GetUserOofSettingsRequest><t:Mailbox><t:Address>{}</t:Address></t:Mailbox></m:GetUserOofSettingsRequest>",
            esc(&self.email)
        ))?;
        if let Some(rm) = doc.find("ResponseMessage") {
            check_message(rm)?;
        }
        let s = doc.find("OofSettings").ok_or_else(|| NetError::Protocol("no automatic reply settings".into()))?;
        let state = s.text_of("OofState").unwrap_or("Disabled");
        let day = |name: &str, end: bool| -> Option<String> {
            let t = s.find(name)?.text.clone();
            let secs = jmap::utc_date(&t)?;
            // The end is the moment after the last day.
            let secs = if end { secs - 1 } else { secs };
            let (y, m, d) = quatrelle_model::dates::civil_from_days(secs.div_euclid(86_400));
            Some(format!("{y:04}-{m:02}-{d:02}"))
        };
        let scheduled = state == "Scheduled";
        let reply = |which: &str| {
            let m = s.find(which).and_then(|r| r.child("Message")).map(|m| m.text.clone()).unwrap_or_default();
            if m.contains('<') { mime::html_to_text(&m).trim().to_string() } else { m.trim().to_string() }
        };
        let (text, external) = (reply("InternalReply"), reply("ExternalReply"));
        let outside = match s.text_of("ExternalAudience").unwrap_or("All") {
            "None" => "none",
            "Known" => "known",
            _ => "",
        };
        Ok(quatrelle_model::job::Vacation {
            enabled: state != "Disabled",
            subject: String::new(),
            // Outside, the same unless it says something else (an empty one is the same).
            external: (!external.is_empty() && external != text).then_some(external),
            outside: outside.to_string(),
            text,
            from: if scheduled { day("StartTime", false) } else { None },
            until: if scheduled { day("EndTime", true) } else { None },
        })
    }

    pub fn set_vacation(&self, v: &quatrelle_model::job::Vacation) -> Result<()> {
        let at = |day: &Option<String>, end: bool| -> Option<String> {
            let d = day.as_deref().filter(|d| quatrelle_model::job::is_day(d))?;
            Some(if end { format!("{d}T23:59:59Z") } else { format!("{d}T00:00:00Z") })
        };
        let (start, end) = (at(&v.from, false), at(&v.until, true));
        let state = match (v.enabled, start.is_some() || end.is_some()) {
            (false, _) => "Disabled",
            (true, true) => "Scheduled",
            (true, false) => "Enabled",
        };
        let duration = format!(
            "<t:Duration><t:StartTime>{}</t:StartTime><t:EndTime>{}</t:EndTime></t:Duration>",
            start.unwrap_or_else(|| "2000-01-01T00:00:00Z".into()),
            end.unwrap_or_else(|| "2100-01-01T00:00:00Z".into())
        );
        let text = esc(&v.text).replace('\n', "&lt;br&gt;");
        let external = esc(v.external_text()).replace('\n', "&lt;br&gt;");
        let audience = match v.outside.as_str() {
            "none" => "None",
            "known" => "Known",
            _ => "All",
        };
        let doc = self.call(&format!(
            "<m:SetUserOofSettingsRequest><t:Mailbox><t:Address>{}</t:Address></t:Mailbox><t:UserOofSettings>\
             <t:OofState>{state}</t:OofState><t:ExternalAudience>{audience}</t:ExternalAudience>{duration}\
             <t:InternalReply><t:Message>{text}</t:Message></t:InternalReply><t:ExternalReply><t:Message>{external}</t:Message></t:ExternalReply>\
             </t:UserOofSettings></m:SetUserOofSettingsRequest>",
            esc(&self.email)
        ))?;
        match doc.find("ResponseMessage") {
            Some(rm) => check_message(rm),
            None => Ok(()),
        }
    }

    /// The picture the directory keeps for an address (`GetUserPhoto`, Exchange 2013 and later):
    /// its bytes and type. `No` when the mailbox has none, or the server predates it.
    pub fn user_photo(&self, email: &str) -> Result<(String, Vec<u8>)> {
        let asked =
            self.call(&format!("<m:GetUserPhoto><m:Email>{}</m:Email><m:SizeRequested>HR240x240</m:SizeRequested></m:GetUserPhoto>", esc(email)));
        match asked {
            Ok(doc) => photo_of(&doc),
            // Exchange says "no picture" (a shared mailbox's, often) with a bare HTTP 404: an
            // answer, not a broken connection (the account showed an error, and connected
            // again, at every start).
            Err(NetError::Protocol(m)) if m == requests::answered_http(404) => Err(NetError::No("no picture for this person".into())),
            Err(e) => Err(e),
        }
    }

    /// The picture a contact item carries (Outlook keeps a contact's photo as an attachment of
    /// the item, marked as the contact's picture): its bytes and type. `No` when the contact has
    /// none.
    pub fn contact_photo(&self, id: &str) -> Result<(String, Vec<u8>)> {
        let doc = self.call(&format!(
            "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape>\
             <t:AdditionalProperties><t:FieldURI FieldURI=\"item:Attachments\"/></t:AdditionalProperties></m:ItemShape>\
             <m:ItemIds><t:ItemId Id=\"{}\"/></m:ItemIds></m:GetItem>",
            esc(id)
        ))?;
        let m = messages(&doc).first().copied().ok_or_else(|| NetError::Protocol("an empty answer".into()))?;
        check_message(m)?;
        let mut files = Vec::new();
        if let Some(items) = m.child("Items") {
            items.all("FileAttachment", &mut files);
        }
        // Exchange marks it so; a server that doesn't name the picture as Outlook's does still
        // gives it the name Outlook gives it.
        let photo = files
            .iter()
            .find(|f| f.attr("IsContactPhoto") == Some("true"))
            .or_else(|| files.iter().find(|f| f.text_of("Name").is_some_and(|n| n.starts_with("ContactPicture"))));
        let att = photo.and_then(|f| f.child("AttachmentId")).and_then(|a| a.attr("Id")).filter(|a| !a.is_empty());
        let Some(att) = att else { return Err(NetError::No("no picture for this person".into())) };
        let doc =
            self.call(&format!("<m:GetAttachment><m:AttachmentIds><t:AttachmentId Id=\"{}\"/></m:AttachmentIds></m:GetAttachment>", esc(att)))?;
        let m = messages(&doc).first().copied().ok_or_else(|| NetError::Protocol("an empty answer".into()))?;
        check_message(m)?;
        // Inside the attachment, not directly under the response.
        let data = m.find("Content").map(|c| c.text.trim()).unwrap_or_default();
        if data.is_empty() {
            return Err(NetError::No("no picture for this person".into()));
        }
        let bytes = unb64(data).ok_or_else(|| NetError::Protocol("the picture isn't base64".into()))?;
        if bytes.is_empty() || bytes.len() > crate::MAX_PICTURE {
            return Err(NetError::No("no picture for this person".into()));
        }
        Ok((crate::picture_media(&bytes).to_string(), bytes))
    }
}
