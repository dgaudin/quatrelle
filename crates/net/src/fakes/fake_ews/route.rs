// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The fake EWS server's operations: each answered from the data, as Exchange would.

use super::*;

mod availability;
mod folders;
mod rules;
mod schema;

pub(super) use schema::in_schema_order;

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
        // A calendar no longer shared: whatever names its mailbox or its items is refused.
        if !self.withdrawn.is_empty() {
            let named = op.find("EmailAddress").map(|e| e.text.trim().to_ascii_lowercase()).is_some_and(|a| self.withdrawn.contains(&a));
            let theirs = ids(op.child("ItemIds")).iter().any(|id| {
                self.pim.iter().any(|(i, f, _)| i == id && f.strip_prefix("mailbox:").is_some_and(|w| self.withdrawn.iter().any(|x| x == w)))
            });
            if named || theirs {
                out.push(err("ErrorAccessDenied"));
                return (format!("{}Response", op.name), out);
            }
        }
        if let Some(done) = self.policy_route(op) {
            return done;
        }
        if let Some(done) = self.calendar_folders(op) {
            return (format!("{}Response", op.name), done);
        }
        if let Some(done) = self.folder_route(op).or_else(|| self.rules_route(op)) {
            return (format!("{}Response", op.name), done);
        }
        match op.name.as_str() {
            "GetAttachment" => out.extend(self.get_attachment(op)),
            "GetUserPhoto" => {
                let email = op.text_of("Email").unwrap_or_default();
                if email.is_empty() || email.ends_with("@nophoto.example") {
                    out.push(err("ErrorItemNotFound"));
                } else {
                    // Three bytes of a JPEG marker: enough to be read as a picture.
                    out.push(ok("<m:PictureData>/9j/</m:PictureData>".into()));
                }
            }
            "FindItem" if op.attr("Traversal") == Some("Associated") => {
                let folder = op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_default();
                let items = if folder == "COMMON-VIEWS" { self.wlinks.concat() } else { String::new() };
                out.push(ok(format!("<m:RootFolder IncludesLastItemInRange=\"true\"><t:Items>{items}</t:Items></m:RootFolder>")));
            }
            "ResolveNames" => return self.resolve_names(op),
            "ExpandDL" => return self.expand_dl(op),
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
            "FindItem" if op.find("DistinguishedFolderId").is_some_and(|f| f.attr("Id") == Some("recoverableitemsdeletions")) => {
                out.push(self.recoverable_page(op));
            }
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
                if self.found_in.len() < 100_000 {
                    self.found_in.extend(wanted_folders.iter().map(|f| (f.clone(), offset, max)));
                }
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
                    let mut from = Vec::new();
                    restriction.all("IsGreaterThanOrEqualTo", &mut from);
                    if let Some(cond) = from.iter().find(|c| c.find("FieldURI").and_then(|f| f.attr("FieldURI")) == Some("item:DateTimeReceived")) {
                        let since = cond.find("Constant").and_then(|c| c.attr("Value")).unwrap_or_default().to_string();
                        list.retain(|i| i.received.as_str() >= since.as_str());
                    }
                    // Replied to or forwarded (`PidTagLastVerbExecuted` within a range).
                    if let Some((lo, hi)) = super::fields::verb_range(restriction) {
                        list.retain(|i| i.verb.last.as_deref().and_then(|v| v.parse::<i64>().ok()).is_some_and(|v| (lo..=hi).contains(&v)));
                        self.calls.push("FindItem replied".into());
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
                            // `PidTagFlagStatus`: 1 complete, 2 flagged.
                            let negated = restriction.find("Not").is_some();
                            if value == "1" {
                                list.retain(|i| i.complete != negated);
                                self.calls.push("FindItem completed".into());
                            } else {
                                list.retain(|i| i.flagged != negated);
                            }
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
                            let mut x = self.item_xml(i, false, false);
                            if with_parent {
                                x = x.replace("</t:Message>", &format!("<t:ParentFolderId Id=\"{}\"/></t:Message>", i.folder));
                            }
                            return x;
                        }
                        format!(
                            "<t:Message><t:ItemId Id=\"{}\" ChangeKey=\"ck\"/><t:IsRead>{}</t:IsRead>{}{}</t:Message>",
                            i.id,
                            i.read,
                            super::flag_xml(i.flagged, i.complete, i.complete_time.as_deref(), &i.verb),
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
                let text = super::fields_named(op).contains(&"item:Body");
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
                        // As Exchange: a series deleted is an item not found.
                        None if !self.pim.iter().any(|(i, _, _)| *i == master) => out.push(err("ErrorItemNotFound")),
                        None => out.push(err("ErrorCalendarOccurrenceIndexIsOutOfRecurrenceRange")),
                    }
                }
                for id in ids(op.child("ItemIds")) {
                    if self.pim.iter().any(|(i, f, _)| *i == id && self.unrouted(f)) {
                        out.push(err("ErrorItemNotFound"));
                        continue;
                    }
                    if self.refused.contains(&id) {
                        out.push(err("ErrorAccessDenied"));
                        continue;
                    }
                    if self.refuse_fields_each && self.item_shapes.last().is_some_and(|s| s == "fields") {
                        out.push(err("ErrorInvalidPropertyRequest"));
                        continue;
                    }
                    if let Some((_, _, el)) = self.pim.iter().find(|(i, _, _)| *i == id) {
                        let mut el = el.clone();
                        attachments::listed(&mut el);
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
                    if let Some(folder) = self.items.iter().find(|i| i.id == id).map(|i| i.folder.clone())
                        && !self.unrouted(&folder)
                        && !self.unanchored(&folder)
                        && self.given_from.len() < 100_000
                    {
                        self.given_from.push(folder);
                    }
                    match self.items.iter().find(|i| i.id == id) {
                        Some(i) if self.unrouted(&i.folder) || self.unanchored(&i.folder) => out.push(err("ErrorItemNotFound")),
                        Some(i) if mime && self.unreadable.contains(&i.id) => out.push(err("ErrorInternalServerError")),
                        Some(_) if text && self.refuse_text_each => out.push(
                            "<m:R ResponseClass=\"Error\"><m:MessageText>The property can't be used with this type of item.</m:MessageText>\
                             <m:ResponseCode>ErrorInvalidPropertyRequest</m:ResponseCode></m:R>"
                                .to_string(),
                        ),
                        Some(i) => out.push(ok(format!("<m:Items>{}</m:Items>", self.with_item_policy(op, &i.id, self.item_xml(i, mime, text))))),
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
                for mut m in op.child("Items").map(|i| i.kids.clone()).unwrap_or_default() {
                    let sticky = m.text_of("ItemClass").is_some_and(|c| c.starts_with("IPM.StickyNote"));
                    // As Exchange: a message (a sticky note is one) is made with its disposition said.
                    if m.name == "Message" && op.attr("MessageDisposition").is_none() {
                        out.push(err("ErrorMessageDispositionRequired"));
                        continue;
                    }
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
                        if let Some(refused) = self.refuses_answer() {
                            out.push(refused);
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
                    if let Some(refused) = self.refuses_recipients(&m, op.attr("SendMeetingInvitations")).or_else(|| self.refuses_note_size(&m)) {
                        out.push(refused);
                        continue;
                    }
                    if m.name == "CalendarItem" || m.name == "Contact" || m.name == "Task" || sticky {
                        if m.name == "Task" {
                            super::tasks::after_create(&mut m);
                        }
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
                            complete: false,
                            complete_time: None,
                            verb: super::Verb::default(),
                            draft: true,
                            received: "2026-10-15T12:00:00Z".into(),
                            voting: Vec::new(),
                            vote: String::new(),
                            categories: Vec::new(),
                            class: "IPM.Note".into(),
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
                    // From another address: only one the account may send from.
                    let author = m.child("From").and_then(|f| f.find("EmailAddress")).map(|e| e.text.to_ascii_lowercase()).unwrap_or_default();
                    if send && !author.is_empty() && !self.send_as.contains(&author) {
                        out.push(err("ErrorSendAsDenied"));
                        continue;
                    }
                    let id = self.id("I");
                    if send {
                        self.sent.push(raw.clone());
                        self.sent_from.push(author);
                    }
                    self.items.push(It {
                        id: id.clone(),
                        folder: folder.clone(),
                        raw,
                        read: true,
                        flagged: false,
                        complete: false,
                        complete_time: None,
                        verb: super::Verb::default(),
                        draft,
                        received: "2026-10-15T12:00:00Z".into(),
                        voting: verbs,
                        vote: String::new(),
                        categories: Vec::new(),
                        class: "IPM.Note".into(),
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
                    // Changed on a copy: a change the server refuses leaves the item as it was.
                    if let Some(pos) = self.pim.iter().position(|(i, _, _)| *i == id) {
                        let Some(mut item) = self.pim.get(pos).map(|(_, _, el)| el.clone()) else { continue };
                        let el = &mut item;
                        let updates = ch.child("Updates").map(|u| u.kids.clone()).unwrap_or_default();
                        if updates.iter().filter_map(|u| u.find("FieldURI").and_then(|f| f.attr("FieldURI"))).any(|uri| !field_uri_is_known(uri)) {
                            out.push(err("ErrorInvalidRequest"));
                            continue;
                        }
                        if let Some(refused) = Data::refuses_zone(el, &updates) {
                            out.push(refused);
                            continue;
                        }
                        for u in updates {
                            // A property of Quatrelle's own, set on an appointment: replaces its namesake.
                            if u.name == "SetItemField"
                                && el.name == "CalendarItem"
                                && let Some(name) = u.child("ExtendedFieldURI").and_then(|f| f.attr("PropertyName")).map(str::to_string)
                            {
                                let named = |k: &El| k.child("ExtendedFieldURI").and_then(|f| f.attr("PropertyName")) == Some(name.as_str());
                                el.kids.retain(|k| k.name != "ExtendedProperty" || !named(k));
                                el.kids.extend(u.find("CalendarItem").into_iter().flat_map(|c| c.kids.iter().filter(|k| named(k)).cloned()));
                                continue;
                            }
                            let field =
                                u.find("FieldURI").and_then(|f| f.attr("FieldURI")).and_then(|f| f.split(':').nth(1)).unwrap_or("").to_string();
                            el.kids.retain(|k| k.name != field);
                            if u.name == "SetItemField"
                                && let Some(new) = u.kids.iter().find(|k| k.name != "FieldURI").and_then(|w| w.kids.first())
                            {
                                el.kids.push(new.clone());
                            }
                            if el.name == "Task" {
                                super::tasks::after_update(el, &u);
                            }
                        }
                        if let Some(refused) = self.refuses_recipients(&item, op.attr("SendMeetingInvitationsOrCancellations")) {
                            out.push(refused);
                            continue;
                        }
                        if let Some(slot) = self.pim.get_mut(pos) {
                            slot.2 = item;
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
                        super::take_flag_update(it, &u);
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
                    // Out of Recoverable Items: back into a folder, as Recover Deleted Items does.
                    if let Some(at) = self.recoverable.iter().position(|i| i.id == id) {
                        let mut it = self.recoverable.remove(at);
                        it.id = new.clone();
                        it.folder = to.clone();
                        self.items.push(it);
                        out.push(ok(format!("<m:Items><t:Message><t:ItemId Id=\"{new}\"/></t:Message></m:Items>")));
                        continue;
                    }
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
                    if !self.may_change(&id) || self.undeletable.contains(&id) {
                        out.push(err("ErrorAccessDenied"));
                        continue;
                    }
                    // A task goes only when told which of its occurrences do.
                    let task = self.pim.iter().any(|(i, _, el)| *i == id && el.name == "Task");
                    if task && op.attr("AffectedTaskOccurrences").is_none() {
                        out.push(err("ErrorInvalidRequest"));
                        continue;
                    }
                    // Purged from Recoverable Items (`HardDelete`): out of the user's reach.
                    if op.attr("DeleteType") == Some("HardDelete") && self.recoverable.iter().any(|i| i.id == id) {
                        let (gone, kept): (Vec<It>, Vec<It>) = std::mem::take(&mut self.recoverable).into_iter().partition(|i| i.id == id);
                        self.recoverable = kept;
                        self.purged.extend(gone);
                        out.push(ok(String::new()));
                        continue;
                    }
                    self.take_items(op, |i| i.id == id);
                    self.pim.retain(|(i, _, _)| *i != id);
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
