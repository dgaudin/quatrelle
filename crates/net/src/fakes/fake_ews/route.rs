//! The fake EWS server's operations: each answered from the data, as Exchange would.

use super::*;

impl Data {
    /// Answer one operation: (response element name, response messages).
    pub(super) fn route(&mut self, op: &El) -> (String, Vec<String>) {
        let ok = |inner: String| format!("<m:R ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>{inner}</m:R>");
        let err =
            |code: &str| format!("<m:R ResponseClass=\"Error\"><m:MessageText>{code}</m:MessageText><m:ResponseCode>{code}</m:ResponseCode></m:R>");
        let ids = |el: Option<&El>| -> Vec<String> {
            el.map(|e| e.kids.iter().filter_map(|k| k.attr("Id").map(str::to_string)).collect()).unwrap_or_default()
        };
        let mut out = Vec::new();
        match op.name.as_str() {
            "GetAttachment" => {
                // A contact's picture: the bytes of any attachment this mailbox's items carry.
                for id in ids(op.child("AttachmentIds")) {
                    let known = self.pim.iter().any(|(_, _, el)| {
                        let mut files = Vec::new();
                        el.all("FileAttachment", &mut files);
                        files.iter().any(|f| f.child("AttachmentId").and_then(|a| a.attr("Id")) == Some(id.as_str()))
                    });
                    if known {
                        out.push(ok("<m:Attachments><t:FileAttachment><t:Content>/9j/</t:Content></t:FileAttachment></m:Attachments>".into()));
                    } else {
                        out.push(err("ErrorItemNotFound"));
                    }
                }
            }
            "GetUserPhoto" => {
                let email = op.text_of("Email").unwrap_or_default();
                if email.is_empty() || email.ends_with("@nophoto.example") {
                    out.push(err("ErrorItemNotFound"));
                } else {
                    // Three bytes of a JPEG marker: enough to be read as a picture.
                    out.push(ok("<m:PictureData>/9j/</m:PictureData>".into()));
                }
            }
            "GetInboxRules" => {
                let list: String = self
                    .rules
                    .iter()
                    .enumerate()
                    .map(|(i, (id, name))| {
                        format!(
                            "<t:Rule><t:RuleId Id=\"{}\"/><t:DisplayName>{}</t:DisplayName><t:Priority>{}</t:Priority><t:IsEnabled>true</t:IsEnabled></t:Rule>",
                            esc(id),
                            esc(name),
                            i + 1
                        )
                    })
                    .collect();
                out.push(ok(format!("<m:OutlookRuleBlobExists>false</m:OutlookRuleBlobExists><m:InboxRules>{list}</m:InboxRules>")));
            }
            "UpdateInboxRules" if self.outlook_rules && op.text_of("RemoveOutlookRuleBlob") != Some("true") => {
                out.push(err("ErrorOutlookRuleBlobExists"));
            }
            "UpdateInboxRules" => {
                self.outlook_rules = false;
                let mut problem = None;
                if let Some(ops) = op.child("Operations") {
                    for o in &ops.kids {
                        match o.name.as_str() {
                            "CreateRuleOperation" => {
                                let Some(rule) = o.child("Rule") else { continue };
                                let name = rule.text_of("DisplayName").unwrap_or_default().to_string();
                                let id = self.id("R");
                                self.rules.push((id, name));
                            }
                            "SetRuleOperation" => {
                                let Some(rule) = o.child("Rule") else { continue };
                                let id = rule.child("RuleId").and_then(|r| r.attr("Id")).unwrap_or_default().to_string();
                                let name = rule.text_of("DisplayName").unwrap_or_default().to_string();
                                match self.rules.iter_mut().find(|(i, _)| *i == id) {
                                    Some(r) => r.1 = name,
                                    None => problem = Some("ErrorRuleNotFound"),
                                }
                            }
                            "DeleteRuleOperation" => {
                                let id = o.child("RuleId").and_then(|r| r.attr("Id")).unwrap_or_default().to_string();
                                self.rules.retain(|(i, _)| *i != id);
                            }
                            _ => {}
                        }
                    }
                }
                out.push(match problem {
                    Some(code) => err(code),
                    None => ok(String::new()),
                });
            }
            "GetFolder" => {
                for f in op.child("FolderIds").map(|f| f.kids.clone()).unwrap_or_default() {
                    // Someone else's calendar: what this account may do there.
                    if let Some(who) = self.folder_id(&f).and_then(|id| id.strip_prefix("mailbox:").map(str::to_string)) {
                        let edit = self.editors.contains(&who) && !self.item_rights_only;
                        out.push(ok(format!(
                            "<m:Folders><t:CalendarFolder><t:FolderId Id=\"mailbox:{who}\"/><t:EffectiveRights><t:CreateAssociated>false</t:CreateAssociated>\
                             <t:CreateContents>{edit}</t:CreateContents><t:CreateHierarchy>false</t:CreateHierarchy><t:Delete>{edit}</t:Delete>\
                             <t:Modify>false</t:Modify><t:Read>true</t:Read></t:EffectiveRights></t:CalendarFolder></m:Folders>"
                        )));
                        continue;
                    }
                    if matches!(self.shared_of(&f), Some(Err(()))) {
                        out.push(err("ErrorAccessDenied"));
                        continue;
                    }
                    let root = f.attr("Id") == Some("publicfoldersroot");
                    match self.folder_id(&f).and_then(|id| self.folders.iter().find(|x| x.id == id).cloned()) {
                        // The root is the hierarchy's; any other public folder, its content's.
                        Some(fo) if root && self.unrouted_hierarchy(&fo.id) => out.push(err("ErrorFolderNotFound")),
                        Some(fo) if !root && self.unrouted(&fo.id) => out.push(err("ErrorFolderNotFound")),
                        Some(fo) => out.push(ok(format!("<m:Folders>{}</m:Folders>", self.folder_xml(&fo)))),
                        None => out.push(err("ErrorFolderNotFound")),
                    }
                }
            }
            // The mailbox's own root (above the mail folders): its Common Views folder.
            "FindFolder"
                if op.attr("Traversal") == Some("Shallow")
                    && op.child("ParentFolderIds").and_then(|p| p.kids.first()).is_some_and(|f| f.attr("Id") == Some("root")) =>
            {
                out.push(ok("<m:RootFolder IncludesLastItemInRange=\"true\"><t:Folders><t:Folder><t:FolderId Id=\"COMMON-VIEWS\"/>\
                     <t:DisplayName>Common Views</t:DisplayName></t:Folder><t:Folder><t:FolderId Id=\"FINDER\"/><t:DisplayName>Finder</t:DisplayName>\
                     </t:Folder></t:Folders></m:RootFolder>"
                    .into()));
            }
            "FindItem" if op.attr("Traversal") == Some("Associated") => {
                let folder = op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_default();
                let items = if folder == "COMMON-VIEWS" { self.wlinks.concat() } else { String::new() };
                out.push(ok(format!("<m:RootFolder IncludesLastItemInRange=\"true\"><t:Items>{items}</t:Items></m:RootFolder>")));
            }
            "ResolveNames" => {
                let entry = op.child("UnresolvedEntry").map(|e| e.text.clone()).unwrap_or_default();
                // `SMTP:` asks for that exact address.
                let want = entry.strip_prefix("SMTP:").map_or(entry.clone(), str::to_string);
                // Another mailbox the server names by its Exchange address (its LegacyDN), or a
                // person the directory knows (X.500 name → address).
                let found = self
                    .shared
                    .iter()
                    .find(|b| legacy_dn(&b.address).eq_ignore_ascii_case(&want) || b.address.eq_ignore_ascii_case(&want))
                    .map(|b| (b.name.clone(), b.address.clone()))
                    .or_else(|| {
                        self.directory.iter().find(|(dn, _)| dn.eq_ignore_ascii_case(&want)).map(|(_, email)| (email.clone(), email.clone()))
                    });
                match found {
                    Some((name, email)) => out.push(ok(format!(
                        "<m:ResolutionSet TotalItemsInView=\"1\" IncludesLastItemInRange=\"true\"><t:Resolution><t:Mailbox><t:Name>{}</t:Name>\
                         <t:EmailAddress>{}</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType></t:Mailbox></t:Resolution></m:ResolutionSet>",
                        esc(&name),
                        esc(&email)
                    ))),
                    None => out.push(err("ErrorNameResolutionNoResults")),
                }
            }
            // Shallow (public folders): the folders right under the one asked for.
            "FindFolder" if op.attr("Traversal") == Some("Shallow") => {
                let parent = op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_default();
                if self.unrouted_hierarchy(&parent) {
                    out.push(err("ErrorFolderNotFound"));
                    return ("FindFolderResponse".into(), out);
                }
                let kids: Vec<String> = self
                    .folders
                    .iter()
                    .filter(|f| f.parent == parent)
                    .map(|f| {
                        let n = self.folders.iter().filter(|x| x.parent == f.id).count();
                        self.folder_xml(f)
                            .replace("<t:ChildFolderCount>0</t:ChildFolderCount>", &format!("<t:ChildFolderCount>{n}</t:ChildFolderCount>"))
                    })
                    .collect();
                out.push(ok(format!("<m:RootFolder IncludesLastItemInRange=\"true\"><t:Folders>{}</t:Folders></m:RootFolder>", kids.concat())));
            }
            "FindFolder" if op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.shared_of(f)).is_some() => {
                // Deep, in a shared mailbox: refused without access to it.
                let Some(Ok(owner)) = op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.shared_of(f)) else {
                    out.push(err("ErrorAccessDenied"));
                    return ("FindFolderResponse".into(), out);
                };
                let all: Vec<String> = self
                    .folders
                    .iter()
                    .filter(|f| f.owner == owner && f.known.as_deref() != Some("msgfolderroot"))
                    .map(|f| self.folder_xml(f))
                    .collect();
                out.push(ok(format!("<m:RootFolder IncludesLastItemInRange=\"true\"><t:Folders>{}</t:Folders></m:RootFolder>", all.concat())));
            }
            "FindFolder" => {
                // Deep: everything under the root (the mailbox's, not the public folders), in pages.
                let view = op.child("IndexedPageFolderView");
                let offset: usize = view.and_then(|v| v.attr("Offset")).and_then(|o| o.parse().ok()).unwrap_or(0);
                let max: usize = view.and_then(|v| v.attr("MaxEntriesReturned")).and_then(|o| o.parse().ok()).unwrap_or(1000).min(3);
                let public = self.public_ids();
                let all: Vec<Fo> = self.folders.iter().filter(|f| f.id != "ROOT" && f.owner.is_empty() && !public.contains(&f.id)).cloned().collect();
                let page: Vec<String> = all.iter().skip(offset).take(max).map(|f| self.folder_xml(f)).collect();
                let last = offset + max >= all.len();
                out.push(ok(format!(
                    "<m:RootFolder TotalItemsInView=\"{}\" IncludesLastItemInRange=\"{last}\"><t:Folders>{}</t:Folders></m:RootFolder>",
                    all.len(),
                    page.concat()
                )));
            }
            // As Exchange: CalendarItemType can't be searched on.
            "FindItem" if op.find("FieldURI").is_some_and(|f| f.attr("FieldURI") == Some("calendar:CalendarItemType")) => {
                out.push(err("ErrorUnsupportedPathForQuery"));
            }
            "FindItem" if self.pim_folder(op.child("ParentFolderIds").and_then(|p| p.kids.first())) => {
                let folder = op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_default();
                if self.unrouted(&folder) {
                    out.push(err("ErrorFolderNotFound"));
                    return ("FindItemResponse".into(), out);
                }
                // A calendar asked for from a date: what ends then or later, and recurring series.
                let since = op.find("Constant").and_then(|c| c.attr("Value")).filter(|v| v.contains('T')).map(str::to_string);
                let wanted = |e: &El| since.as_deref().is_none_or(|s| e.text_of("End").is_none_or(|end| end >= s) || e.child("Recurrence").is_some());
                // Occurrences that became items of their own are listed by their series, not
                // by the folder.
                let own = |e: &El| !matches!(e.text_of("CalendarItemType"), Some("Occurrence" | "Exception"));
                let items: String = self
                    .pim
                    .iter()
                    .filter(|(_, f, e)| *f == folder && wanted(e) && own(e))
                    .map(|(id, _, _)| format!("<t:Item><t:ItemId Id=\"{id}\"/></t:Item>"))
                    .collect();
                out.push(ok(format!("<m:RootFolder IncludesLastItemInRange=\"true\"><t:Items>{items}</t:Items></m:RootFolder>")));
            }
            // What changed since a state: here the state is the folder as it was (its messages'
            // ids and marks), and the changes are what differs now — as Exchange answers them.
            "SyncFolderItems" => out.extend(self.sync_folder_items(op)),
            "FindItem" => {
                // A folder the mailbox hasn't (a Notes folder, say): as Exchange answers.
                let wanted_folders: Vec<String> =
                    op.child("ParentFolderIds").map(|p| p.kids.iter().filter_map(|f| self.folder_id(f)).collect()).unwrap_or_default();
                if wanted_folders.is_empty() {
                    out.push(err("ErrorFolderNotFound"));
                    return ("FindItemResponse".into(), out);
                }
                if wanted_folders.iter().any(|f| self.unrouted(f)) {
                    out.push(err("ErrorFolderNotFound"));
                    return ("FindItemResponse".into(), out);
                }
                let view = op.child("IndexedPageItemView");
                let max: usize = view.and_then(|v| v.attr("MaxEntriesReturned")).and_then(|o| o.parse().ok()).unwrap_or(1000);
                // Pages by offset, as Exchange does (up to 1000 at a time).
                let offset: usize = view.and_then(|v| v.attr("Offset")).and_then(|o| o.parse().ok()).unwrap_or(0);
                let max = max.min(1000);
                let mut list: Vec<&It> = self.items.iter().filter(|i| wanted_folders.contains(&i.folder)).collect();
                // A query: Exchange's own index (AQS words), over subjects, addresses and bodies.
                if let Some(q) = op.child("QueryString") {
                    list.retain(|i| crate::fake_graph::words_match(&q.text, &i.raw));
                }
                // A restriction the server knows: a message found by its `Message-ID`, or the
                // read and flag state.
                if let Some(restriction) = op.find("Restriction") {
                    let uri = restriction.find("FieldURI").and_then(|f| f.attr("FieldURI")).unwrap_or_default();
                    let value = restriction.find("Constant").and_then(|c| c.attr("Value")).unwrap_or_default();
                    if uri == "message:InternetMessageId" {
                        let want = value.trim_matches(['<', '>']).to_ascii_lowercase();
                        list.retain(|i| quatrelle_mime::parse(&i.raw).is_some_and(|p| p.message_id.to_ascii_lowercase() == want));
                    }
                    // Mail received since then (Cached Exchange Mode's mail kept offline); the
                    // times are all written alike, so they compare as text.
                    if let Some(cond) = restriction.find("IsGreaterThanOrEqualTo")
                        && cond.find("FieldURI").and_then(|f| f.attr("FieldURI")) == Some("item:DateTimeReceived")
                    {
                        let since = cond.find("Constant").and_then(|c| c.attr("Value")).unwrap_or_default().to_string();
                        list.retain(|i| i.received.as_str() >= since.as_str());
                    }
                    // Those carrying categories.
                    if restriction.find("Exists").and_then(|e| e.find("FieldURI")).and_then(|f| f.attr("FieldURI")) == Some("item:Categories") {
                        list.retain(|i| !i.categories.is_empty());
                        self.calls.push("FindItem categorized".into());
                    }
                    // And older than that (read on the server, past what's kept).
                    if let Some(cond) = restriction.find("IsLessThan")
                        && cond.find("FieldURI").and_then(|f| f.attr("FieldURI")) == Some("item:DateTimeReceived")
                    {
                        let before = cond.find("Constant").and_then(|c| c.attr("Value")).unwrap_or_default().to_string();
                        list.retain(|i| i.received.as_str() < before.as_str());
                    }
                    let mut all = Vec::new();
                    restriction.all("IsEqualTo", &mut all);
                    for cond in all {
                        let value = cond.find("Constant").and_then(|c| c.attr("Value")).unwrap_or("");
                        if cond.find("FieldURI").and_then(|f| f.attr("FieldURI")) == Some("message:IsRead") {
                            let want = value == "true";
                            list.retain(|i| i.read == want);
                        } else if cond.find("ExtendedFieldURI").is_some() {
                            let negated = restriction.find("Not").is_some();
                            list.retain(|i| i.flagged != negated);
                        }
                    }
                }
                list.sort_by(|a, b| b.received.cmp(&a.received));
                // With the headers asked for (a search): the fields a list shows, and the folder
                // each message is in.
                let mut uris = Vec::new();
                op.all("FieldURI", &mut uris);
                let asked = |u: &str| uris.iter().any(|f| f.attr("FieldURI") == Some(u));
                let with_fields = asked("item:Subject");
                let with_parent = asked("item:ParentFolderId");
                let last = offset + max >= list.len();
                let items: String = list
                    .iter()
                    .skip(offset)
                    .take(max)
                    .map(|i| {
                        if with_fields {
                            let mut x = self.item_xml(i, false);
                            if with_parent {
                                x = x.replace("</t:Message>", &format!("<t:ParentFolderId Id=\"{}\"/></t:Message>", i.folder));
                            }
                            return x;
                        }
                        format!(
                            "<t:Message><t:ItemId Id=\"{}\" ChangeKey=\"ck\"/><t:IsRead>{}</t:IsRead>{}{}</t:Message>",
                            i.id,
                            i.read,
                            if i.flagged {
                                "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/><t:Value>2</t:Value></t:ExtendedProperty>"
                            } else {
                                ""
                            },
                            super::categories_xml(&i.categories)
                        )
                    })
                    .collect();
                out.push(ok(format!(
                    "<m:RootFolder TotalItemsInView=\"{}\" IncludesLastItemInRange=\"{last}\"><t:Items>{items}</t:Items></m:RootFolder>",
                    list.len()
                )));
            }
            "GetItem" => {
                let mime = op.find("IncludeMimeContent").is_some_and(|m| m.text == "true");
                // An occurrence of a series by its place in it: its own item, made on demand.
                let occurrences: Vec<(String, usize)> = op
                    .child("ItemIds")
                    .map(|l| {
                        l.children("OccurrenceItemId")
                            .filter_map(|o| Some((o.attr("RecurringMasterId")?.to_string(), o.attr("InstanceIndex")?.parse().ok()?)))
                            .collect()
                    })
                    .unwrap_or_default();
                for (master, index) in occurrences {
                    match self.occurrence_item(&master, index) {
                        Some(id) => {
                            out.push(ok(format!("<m:Items><t:CalendarItem><t:ItemId Id=\"{id}\" ChangeKey=\"ck\"/></t:CalendarItem></m:Items>")))
                        }
                        None => out.push(err("ErrorCalendarOccurrenceIndexIsOutOfRecurrenceRange")),
                    }
                }
                for id in ids(op.child("ItemIds")) {
                    if self.pim.iter().any(|(i, f, _)| *i == id && self.unrouted(f)) {
                        out.push(err("ErrorItemNotFound"));
                        continue;
                    }
                    if let Some((_, _, el)) = self.pim.iter().find(|(i, _, _)| *i == id) {
                        let mut el = el.clone();
                        // A series lists the occurrences that became items of their own.
                        if el.child("Recurrence").is_some() {
                            el.kids.extend(parse(&self.modified_occurrences(&id)).kids);
                        }
                        el.kids.insert(
                            0,
                            El {
                                name: "ItemId".into(),
                                attrs: vec![("Id".into(), id.clone()), ("ChangeKey".into(), "ck".into())],
                                ..Default::default()
                            },
                        );
                        // Each item's own rights (`item:EffectiveRights`), when asked for.
                        let mut fields = Vec::new();
                        op.all("FieldURI", &mut fields);
                        // An appointment's time zones only when asked for, as Exchange gives them.
                        if !fields.iter().any(|f| f.attr("FieldURI") == Some("calendar:StartTimeZone")) {
                            el.kids.retain(|k| k.name != "StartTimeZone" && k.name != "EndTimeZone");
                        }
                        if fields.iter().any(|f| f.attr("FieldURI") == Some("item:EffectiveRights")) {
                            let edit = self.may_change(&id);
                            let rights = parse(&format!(
                                "<t:EffectiveRights><t:CreateAssociated>false</t:CreateAssociated><t:CreateContents>false</t:CreateContents>\
                                 <t:CreateHierarchy>false</t:CreateHierarchy><t:Delete>{edit}</t:Delete><t:Modify>{edit}</t:Modify><t:Read>true</t:Read></t:EffectiveRights>"
                            ));
                            el.kids.extend(rights.kids);
                        }
                        out.push(ok(format!("<m:Items>{}</m:Items>", xml(&el))));
                        continue;
                    }
                    match self.items.iter().find(|i| i.id == id) {
                        Some(i) if self.unrouted(&i.folder) => out.push(err("ErrorItemNotFound")),
                        Some(i) if mime && self.unreadable.contains(&i.id) => out.push(err("ErrorInternalServerError")),
                        Some(i) => out.push(ok(format!("<m:Items>{}</m:Items>", self.item_xml(i, mime)))),
                        None => out.push(err("ErrorItemNotFound")),
                    }
                }
            }
            "CreateItem" => {
                let send = op.attr("MessageDisposition") == Some("SendAndSaveCopy");
                if let Some(how) = op.attr("SendMeetingInvitations") {
                    self.meeting_sends.push(how.to_string());
                }
                let folder = op.child("SavedItemFolderId").and_then(|s| s.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_default();
                for m in op.child("Items").map(|i| i.kids.clone()).unwrap_or_default() {
                    let sticky = m.text_of("ItemClass").is_some_and(|c| c.starts_with("IPM.StickyNote"));
                    if folder.strip_prefix("mailbox:").is_some_and(|who| !self.editors.iter().any(|e| e == who)) {
                        out.push(err("ErrorAccessDenied"));
                        continue;
                    }
                    // An answer to a meeting: noted; a declined one leaves the calendar.
                    if matches!(m.name.as_str(), "AcceptItem" | "TentativelyAcceptItem" | "DeclineItem") {
                        let reference = m.child("ReferenceItemId").and_then(|r| r.attr("Id")).unwrap_or_default().to_string();
                        if !self.pim.iter().any(|(id, _, _)| *id == reference) {
                            out.push(err("ErrorItemNotFound"));
                            continue;
                        }
                        if m.name == "DeclineItem" {
                            self.pim.retain(|(id, _, _)| *id != reference);
                        }
                        self.answers.push((reference, m.name.clone(), send));
                        out.push(ok(String::new()));
                        continue;
                    }
                    // Exchange checks a new appointment's or task's fields against the schema's order.
                    if (m.name == "CalendarItem" || m.name == "Task") && !in_schema_order(&m) {
                        out.push(err("ErrorSchemaValidation"));
                        continue;
                    }
                    if m.name == "CalendarItem" || m.name == "Contact" || m.name == "Task" || sticky {
                        let id = self.id("P");
                        out.push(ok(format!("<m:Items><t:{0}><t:ItemId Id=\"{id}\" ChangeKey=\"ck\"/></t:{0}></m:Items>", m.name)));
                        self.pim.push((id, folder.clone(), m));
                        continue;
                    }
                    // A reply made by Exchange from the original (a voting response starts so).
                    if m.name == "ReplyToItem" {
                        let reference = m.child("ReferenceItemId").and_then(|r| r.attr("Id")).unwrap_or_default().to_string();
                        let Some(orig) = self.items.iter().find(|i| i.id == reference).map(|i| quatrelle_mime::parse(&i.raw).unwrap_or_default())
                        else {
                            out.push(err("ErrorItemNotFound"));
                            continue;
                        };
                        let raw = format!(
                            "From: riley@contoso.example\r\nTo: {}\r\nSubject: RE: {}\r\nIn-Reply-To: <{}>\r\n\r\n",
                            orig.from.email, orig.subject, orig.message_id
                        );
                        let id = self.id("I");
                        self.items.push(It {
                            id: id.clone(),
                            folder: folder.clone(),
                            raw: raw.into_bytes(),
                            read: true,
                            flagged: false,
                            draft: true,
                            received: "2026-10-15T12:00:00Z".into(),
                            voting: Vec::new(),
                            vote: String::new(),
                            categories: Vec::new(),
                        });
                        out.push(ok(format!("<m:Items><t:Message><t:ItemId Id=\"{id}\" ChangeKey=\"ck\"/></t:Message></m:Items>")));
                        continue;
                    }
                    let raw = m.child("MimeContent").and_then(|c| unb64(&c.text)).unwrap_or_default();
                    if send && String::from_utf8_lossy(&raw).contains("nobody@") {
                        out.push(err("ErrorInvalidRecipients"));
                        continue;
                    }
                    let draft = m.children("ExtendedProperty").any(|p| p.text_of("Value") == Some("8"));
                    // Voting buttons sent with it (the verb stream, PidLidVerbStream).
                    let verbs = m
                        .children("ExtendedProperty")
                        .find(|p| p.child("ExtendedFieldURI").and_then(|u| u.attr("PropertyId")) == Some("34080"))
                        .and_then(|p| p.text_of("Value"))
                        .and_then(unb64)
                        .unwrap_or_default();
                    let id = self.id("I");
                    if send {
                        self.sent.push(raw.clone());
                    }
                    self.items.push(It {
                        id: id.clone(),
                        folder: folder.clone(),
                        raw,
                        read: true,
                        flagged: false,
                        draft,
                        received: "2026-10-15T12:00:00Z".into(),
                        voting: verbs,
                        vote: String::new(),
                        categories: Vec::new(),
                    });
                    out.push(ok(if send { String::new() } else { format!("<m:Items><t:Message><t:ItemId Id=\"{id}\"/></t:Message></m:Items>") }));
                }
            }
            "UpdateItem" => {
                for ch in op.child("ItemChanges").map(|c| c.kids.clone()).unwrap_or_default() {
                    let id = ch.child("ItemId").and_then(|i| i.attr("Id")).unwrap_or_default().to_string();
                    if !self.may_change(&id) {
                        out.push(err("ErrorAccessDenied"));
                        continue;
                    }
                    // As Exchange: resolving against the item as last seen needs its change key.
                    let keyed = ch.child("ItemId").and_then(|i| i.attr("ChangeKey")).is_some_and(|k| !k.is_empty());
                    if op.attr("ConflictResolution") == Some("AutoResolve") && !keyed {
                        out.push(err("ErrorChangeKeyRequiredForWriteOperations"));
                        continue;
                    }
                    if let Some(how) = op.attr("SendMeetingInvitationsOrCancellations").filter(|_| self.pim.iter().any(|(i, _, _)| *i == id)) {
                        self.meeting_sends.push(how.to_string());
                    }
                    if let Some((_, _, el)) = self.pim.iter_mut().find(|(i, _, _)| *i == id) {
                        let updates = ch.child("Updates").map(|u| u.kids.clone()).unwrap_or_default();
                        if updates.iter().filter_map(|u| u.find("FieldURI").and_then(|f| f.attr("FieldURI"))).any(|uri| !field_uri_is_known(uri)) {
                            out.push(err("ErrorInvalidRequest"));
                            continue;
                        }
                        for u in updates {
                            let field =
                                u.find("FieldURI").and_then(|f| f.attr("FieldURI")).and_then(|f| f.split(':').nth(1)).unwrap_or("").to_string();
                            el.kids.retain(|k| k.name != field);
                            if u.name == "SetItemField"
                                && let Some(new) = u.kids.iter().find(|k| k.name != "FieldURI").and_then(|w| w.kids.first())
                            {
                                el.kids.push(new.clone());
                            }
                        }
                        out.push(ok(format!("<m:Items><t:CalendarItem><t:ItemId Id=\"{id}\" ChangeKey=\"ck2\"/></t:CalendarItem></m:Items>")));
                        continue;
                    }
                    let Some(it) = self.items.iter_mut().find(|i| i.id == id) else {
                        out.push(err("ErrorItemNotFound"));
                        continue;
                    };
                    for u in ch.child("Updates").map(|u| u.kids.clone()).unwrap_or_default() {
                        if let Some(r) = u.find("IsRead") {
                            it.read = r.text == "true";
                        }
                        if u.find("ExtendedFieldURI").and_then(|e| e.attr("PropertyTag")) == Some("0x1090") {
                            it.flagged = u.name == "SetItemField";
                        }
                        if u.find("FieldURI").and_then(|f| f.attr("FieldURI")) == Some("item:Categories") {
                            let set = u.find("Categories").filter(|_| u.name == "SetItemField");
                            it.categories = set.map(|c| c.children("String").map(|s| s.text.clone()).collect()).unwrap_or_default();
                        }
                        if u.find("ExtendedFieldURI").and_then(|e| e.attr("PropertyId")) == Some("34084") {
                            it.vote = u.find("Value").map(|v| v.text.clone()).unwrap_or_default();
                        }
                        if u.find("FieldURI").and_then(|f| f.attr("FieldURI")) == Some("item:Subject")
                            && let Some(subject) = u.find("Subject").map(|s| s.text.clone())
                        {
                            let text = String::from_utf8_lossy(&it.raw).into_owned();
                            let kept: String = text.split_inclusive("\r\n").filter(|l| !l.starts_with("Subject:")).collect();
                            it.raw = format!("Subject: {subject}\r\n{kept}").into_bytes();
                        }
                    }
                    out.push(ok(format!("<m:Items><t:Message><t:ItemId Id=\"{id}\" ChangeKey=\"ck2\"/></t:Message></m:Items>")));
                }
            }
            "SendItem" => {
                let to = op.child("SavedItemFolderId").and_then(|t| t.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_default();
                for id in ids(op.child("ItemIds")) {
                    match self.items.iter_mut().find(|i| i.id == id) {
                        Some(it) => {
                            it.folder = to.clone();
                            it.draft = false;
                            let subject = quatrelle_mime::parse(&it.raw).map(|p| p.subject).unwrap_or_default();
                            let (raw, vote) = (it.raw.clone(), it.vote.clone());
                            self.sent.push(raw);
                            self.votes.push((subject, vote));
                            out.push(ok(String::new()));
                        }
                        None => out.push(err("ErrorItemNotFound")),
                    }
                }
            }
            "MoveItem" => {
                let to = op.child("ToFolderId").and_then(|t| t.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_default();
                for id in ids(op.child("ItemIds")) {
                    let new = self.id("I");
                    match self.items.iter_mut().find(|i| i.id == id) {
                        Some(it) => {
                            // Exchange gives a moved item a new id.
                            it.id = new.clone();
                            it.folder = to.clone();
                            out.push(ok(format!("<m:Items><t:Message><t:ItemId Id=\"{new}\"/></t:Message></m:Items>")));
                        }
                        None => out.push(err("ErrorItemNotFound")),
                    }
                }
            }
            "DeleteItem" => {
                for id in ids(op.child("ItemIds")) {
                    if !self.may_change(&id) {
                        out.push(err("ErrorAccessDenied"));
                        continue;
                    }
                    self.items.retain(|i| i.id != id);
                    self.pim.retain(|(i, _, _)| *i != id);
                    out.push(ok(String::new()));
                }
            }
            "CreateFolder" => {
                let parent = op.child("ParentFolderId").and_then(|p| p.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_else(|| "ROOT".into());
                let name = op.find("DisplayName").map(|d| d.text.clone()).unwrap_or_default();
                let id = self.folder(&name, Some(&parent), None, "IPF.Note");
                out.push(ok(format!("<m:Folders><t:Folder><t:FolderId Id=\"{id}\"/></t:Folder></m:Folders>")));
            }
            "MoveFolder" => {
                let to = op.child("ToFolderId").and_then(|t| t.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_else(|| "ROOT".into());
                for id in ids(op.child("FolderIds")) {
                    if let Some(f) = self.folders.iter_mut().find(|f| f.id == id) {
                        f.parent = to.clone();
                    }
                    out.push(ok(format!("<m:Folders><t:Folder><t:FolderId Id=\"{id}\"/></t:Folder></m:Folders>")));
                }
            }
            "UpdateFolder" => {
                for ch in op.child("FolderChanges").map(|c| c.kids.clone()).unwrap_or_default() {
                    let id = ch.child("FolderId").and_then(|i| i.attr("Id")).unwrap_or_default().to_string();
                    let name = ch.find("DisplayName").map(|d| d.text.clone()).unwrap_or_default();
                    if let Some(f) = self.folders.iter_mut().find(|f| f.id == id) {
                        f.name = name;
                    }
                    out.push(ok(String::new()));
                }
            }
            "DeleteFolder" => {
                for id in ids(op.child("FolderIds")) {
                    self.folders.retain(|f| f.id != id);
                    out.push(ok(String::new()));
                }
            }
            "EmptyFolder" => {
                for id in ids(op.child("FolderIds")) {
                    self.items.retain(|i| i.folder != id);
                    out.push(ok(String::new()));
                }
            }
            "GetUserConfiguration" => out.push(self.user_configuration(op)),
            "GetRoomLists" => {
                if self.rooms_refused {
                    let refused = "<m:ResponseCode>ErrorInvalidOperation</m:ResponseCode><m:MessageText>not on this server</m:MessageText>";
                    return ("GetRoomListsResponse".into(), vec![refused.into()]);
                }
                let list: String = self
                    .room_lists
                    .iter()
                    .map(|(name, email)| {
                        format!(
                            "<t:Address><t:Name>{}</t:Name><t:EmailAddress>{}</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType>\
                             <t:MailboxType>PublicDL</t:MailboxType></t:Address>",
                            esc(name),
                            esc(email)
                        )
                    })
                    .collect();
                return ("GetRoomListsResponse".into(), vec![format!("<m:ResponseCode>NoError</m:ResponseCode><m:RoomLists>{list}</m:RoomLists>")]);
            }
            "GetRooms" => {
                let wanted = op.find("EmailAddress").map(|e| e.text.trim().to_ascii_lowercase()).unwrap_or_default();
                if !self.room_lists.iter().any(|(_, e)| e.eq_ignore_ascii_case(&wanted)) {
                    let refused = "<m:ResponseCode>ErrorNameResolutionNoMailbox</m:ResponseCode><m:MessageText>no such room list</m:MessageText>";
                    return ("GetRoomsResponse".into(), vec![refused.into()]);
                }
                let rooms: String = self
                    .rooms
                    .iter()
                    .filter(|(_, _, list)| list.eq_ignore_ascii_case(&wanted))
                    .map(|(name, email, _)| {
                        format!(
                            "<t:Room><t:Id><t:Name>{}</t:Name><t:EmailAddress>{}</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType>\
                             <t:MailboxType>Mailbox</t:MailboxType></t:Id></t:Room>",
                            esc(name),
                            esc(email)
                        )
                    })
                    .collect();
                return ("GetRoomsResponse".into(), vec![format!("<m:ResponseCode>NoError</m:ResponseCode><m:Rooms>{rooms}</m:Rooms>")]);
            }
            "GetUserAvailabilityRequest" => return ("GetUserAvailabilityResponse".into(), vec![self.availability(op)]),
            "GetUserOofSettingsRequest" => {
                let (state, start, end, msg, outside, audience) = self.oof.clone();
                return (
                    "GetUserOofSettingsResponse".into(),
                    vec![format!(
                        "<ResponseMessage ResponseClass=\"Success\"><ResponseCode>NoError</ResponseCode></ResponseMessage>\
                         <OofSettings><OofState>{state}</OofState><ExternalAudience>{audience}</ExternalAudience>\
                         <Duration><StartTime>{start}</StartTime><EndTime>{end}</EndTime></Duration>\
                         <InternalReply><Message>{}</Message></InternalReply><ExternalReply><Message>{}</Message></ExternalReply></OofSettings>",
                        esc(&msg),
                        esc(&outside)
                    )],
                );
            }
            "SetUserOofSettingsRequest" => {
                let s = op.child("UserOofSettings");
                let get = |n: &str| s.and_then(|s| s.find(n)).map(|e| e.text.clone()).unwrap_or_default();
                let msg = s.and_then(|s| s.find("InternalReply")).and_then(|r| r.child("Message")).map(|m| m.text.clone()).unwrap_or_default();
                if msg == "refuse" {
                    let refused = "<ResponseMessage ResponseClass=\"Error\"><MessageText>refused</MessageText><ResponseCode>ErrorInvalidParameter</ResponseCode></ResponseMessage>";
                    return ("SetUserOofSettingsResponse".into(), vec![refused.into()]);
                }
                let outside = s.and_then(|s| s.find("ExternalReply")).and_then(|r| r.child("Message")).map(|m| m.text.clone()).unwrap_or_default();
                self.oof = (get("OofState"), get("StartTime"), get("EndTime"), msg, outside, get("ExternalAudience"));
                return (
                    "SetUserOofSettingsResponse".into(),
                    vec!["<ResponseMessage ResponseClass=\"Success\"><ResponseCode>NoError</ResponseCode></ResponseMessage>".into()],
                );
            }
            other => {
                out.push(err(&format!("ErrorInvalidOperation:{other}")));
            }
        }
        (format!("{}Response", op.name), out)
    }
}

impl Data {
    /// A `GetUserAvailability` answer: as Exchange checks the request (the schema's order, a
    /// time zone, a window of a day or more), then each mailbox's calendar between the dates —
    /// its own calendar items, another's folder here, and the busy stretches given — or its
    /// refusal.
    fn availability(&mut self, op: &El) -> String {
        let names: Vec<&str> = op.kids.iter().map(|k| k.name.as_str()).collect();
        if names != ["TimeZone", "MailboxDataArray", "FreeBusyViewOptions"] {
            return "<m:ResponseCode>ErrorSchemaValidation</m:ResponseCode><m:MessageText>The request failed schema validation</m:MessageText>"
                .into();
        }
        let window = op.find("TimeWindow");
        let (start, end) = (
            window.and_then(|w| w.text_of("StartTime")).unwrap_or("").trim().to_string(),
            window.and_then(|w| w.text_of("EndTime")).unwrap_or("").trim().to_string(),
        );
        let (Some(from), Some(to)) = (quatrelle_jmap::utc_date(&start), quatrelle_jmap::utc_date(&end)) else {
            return "<m:ResponseCode>ErrorInvalidTimeInterval</m:ResponseCode><m:MessageText>The time window is invalid</m:MessageText>".into();
        };
        if to - from < 86_400 || to - from > 62 * 86_400 || start.ends_with('Z') {
            return "<m:ResponseCode>ErrorInvalidTimeInterval</m:ResponseCode><m:MessageText>The time window must be a day to two months</m:MessageText>".into();
        }
        let emails: Vec<String> = op
            .find("MailboxDataArray")
            .map(|a| a.children("MailboxData").filter_map(|m| m.find("Address").map(|x| x.text.trim().to_ascii_lowercase())).collect())
            .unwrap_or_default();
        self.availability_asked.push((emails.clone(), start, end));
        let own_calendar = self.folders.iter().find(|f| f.known.as_deref() == Some("calendar") && f.owner.is_empty()).map(|f| f.id.clone());
        let mut out = String::new();
        for email in &emails {
            if self.no_freebusy.iter().any(|e| e.eq_ignore_ascii_case(email)) {
                out.push_str(
                    "<m:FreeBusyResponse><m:ResponseMessage ResponseClass=\"Error\"><m:MessageText>No free/busy access</m:MessageText>\
                     <m:ResponseCode>ErrorNoFreeBusyAccess</m:ResponseCode></m:ResponseMessage></m:FreeBusyResponse>",
                );
                continue;
            }
            let folder = if email == "riley@contoso.example" { own_calendar.clone() } else { Some(format!("mailbox:{email}")) };
            let mut events: Vec<(String, String, String, String)> = self
                .pim
                .iter()
                .filter(|(_, f, el)| Some(f) == folder.as_ref() && el.name == "CalendarItem")
                .filter_map(|(_, _, el)| {
                    Some((
                        el.text_of("Start")?.trim().trim_end_matches('Z').to_string(),
                        el.text_of("End")?.trim().trim_end_matches('Z').to_string(),
                        el.text_of("LegacyFreeBusyStatus").unwrap_or("Busy").trim().to_string(),
                        el.text_of("Subject").unwrap_or("").to_string(),
                    ))
                })
                .collect();
            events.extend(
                self.taken.iter().filter(|(e, ..)| e.eq_ignore_ascii_case(email)).map(|(_, s, e, k, subject)| {
                    (s.trim_end_matches('Z').to_string(), e.trim_end_matches('Z').to_string(), k.clone(), subject.clone())
                }),
            );
            let items: String = events
                .iter()
                .filter(|(s, e, _, _)| quatrelle_jmap::utc_date(e).is_some_and(|e| e > from) && quatrelle_jmap::utc_date(s).is_some_and(|s| s < to))
                .map(|(s, e, kind, subject)| {
                    format!(
                        "<t:CalendarEvent><t:StartTime>{s}</t:StartTime><t:EndTime>{e}</t:EndTime><t:BusyType>{kind}</t:BusyType>\
                         <t:CalendarEventDetails><t:Subject>{}</t:Subject><t:IsMeeting>true</t:IsMeeting></t:CalendarEventDetails></t:CalendarEvent>",
                        esc(subject)
                    )
                })
                .collect();
            out.push_str(&format!(
                "<m:FreeBusyResponse><m:ResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode></m:ResponseMessage>\
                 <m:FreeBusyView><t:FreeBusyViewType>Detailed</t:FreeBusyViewType><t:CalendarEventArray>{items}</t:CalendarEventArray></m:FreeBusyView>\
                 </m:FreeBusyResponse>"
            ));
        }
        format!("<m:FreeBusyResponseArray>{out}</m:FreeBusyResponseArray>")
    }
}

/// An appointment's fields as EWS's schema orders them (those Quatrelle writes): the item's
/// (`ItemType`), then the appointment's (`CalendarItemType`).
const CALENDAR_ITEM_ORDER: &[&str] = &[
    "ItemClass",
    "Subject",
    "Sensitivity",
    "Body",
    "Categories",
    "Importance",
    "ReminderIsSet",
    "ReminderMinutesBeforeStart",
    "UID",
    "Start",
    "End",
    "IsAllDayEvent",
    "LegacyFreeBusyStatus",
    "Location",
    "IsResponseRequested",
    "RequiredAttendees",
    "OptionalAttendees",
    "Resources",
    "Recurrence",
    "MeetingTimeZone",
    "StartTimeZone",
    "EndTimeZone",
    "AllowNewTimeProposal",
];

/// A task's fields as EWS's schema orders them (those Quatrelle writes): the item's, then the
/// task's (`TaskType`).
const TASK_ORDER: &[&str] = &[
    "ItemClass",
    "Subject",
    "Sensitivity",
    "Body",
    "Categories",
    "Importance",
    "ReminderDueBy",
    "ReminderIsSet",
    "CompleteDate",
    "DueDate",
    "PercentComplete",
    "StartDate",
    "Status",
];

/// Whether a new appointment's or task's fields come in the schema's order (one it doesn't
/// list is let be).
pub(super) fn in_schema_order(item: &El) -> bool {
    let order = if item.name == "Task" { TASK_ORDER } else { CALENDAR_ITEM_ORDER };
    let places: Vec<usize> = item.kids.iter().filter_map(|k| order.iter().position(|n| *n == k.name)).collect();
    places.windows(2).all(|w| w.first() <= w.get(1))
}
