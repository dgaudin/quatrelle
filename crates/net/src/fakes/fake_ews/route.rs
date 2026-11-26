// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The fake EWS server's operations: each answered from the data, as Exchange would.

use super::*;

mod availability;
pub(super) mod folders;
mod mailtips;
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
        if op.name == "FindFolder" && super::server::take_one(&mut self.find_folder_fails) {
            return ("FindFolderResponse".into(), vec![err("ErrorAccessDenied")]);
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
                let mut items: Vec<&String> = if folder == "COMMON-VIEWS" {
                    let hidden = &self.hidden_wlinks;
                    self.wlinks.iter().filter(|w| !wlink_id(w).is_some_and(|id| hidden.contains(&id))).collect()
                } else {
                    Vec::new()
                };
                // A restriction on the message class is honored, as a client looking for the
                // navigation shortcuts asks ([MS-OXOCFG] 2.2.9).
                if let Some(class) = class_wanted(op) {
                    items.retain(|w| wlink_class(w).as_deref() == Some(class.as_str()));
                }
                // One page, and whether more follow, as Exchange answers an indexed page.
                let view = op.child("IndexedPageItemView");
                let max: usize = view.and_then(|v| v.attr("MaxEntriesReturned")).and_then(|o| o.parse().ok()).unwrap_or(1000);
                let offset: usize = view.and_then(|v| v.attr("Offset")).and_then(|o| o.parse().ok()).unwrap_or(0);
                let total = items.len();
                let wanted = wanted_props(op);
                let page: String = items.iter().skip(offset).take(max).map(|w| props_only(w, &wanted)).collect();
                let last = !self.wlinks_more && offset.saturating_add(max) >= total;
                out.push(ok(format!("<m:RootFolder IncludesLastItemInRange=\"{last}\"><t:Items>{page}</t:Items></m:RootFolder>")));
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
                    // What may be more than mail, or was replied to all (an `Or` of classes by
                    // their start and of last verbs): refused, or the messages it finds.
                    if let Some(kinds) = super::fields::kinds_search(restriction) {
                        self.calls.push("FindItem kinds".into());
                        if self.kinds_refused {
                            out.push(err("ErrorInvalidRestriction"));
                            return ("FindItemResponse".into(), out);
                        }
                        list.retain(|i| kinds.admits(i));
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
                        } else if cond.find("ExtendedFieldURI").and_then(|e| e.attr("PropertyTag")) == Some("0x1090") {
                            // `PidTagFlagStatus`: 1 complete, 2 flagged.
                            let negated = restriction.find("Not").is_some();
                            if value == "1" {
                                list.retain(|i| i.complete != negated);
                                self.calls.push("FindItem completed".into());
                            } else {
                                list.retain(|i| i.flagged != negated);
                                self.calls.push("FindItem followUps".into());
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
                let kind_asked = super::fields::kind_asked(op);
                let last = offset + max >= list.len();
                let items: String = list
                    .iter()
                    .skip(offset)
                    .take(max)
                    .map(|i| {
                        if with_fields {
                            let mut x = super::fields::with_kind(self.item_xml(i, false, false), i, op, false);
                            if with_parent {
                                x = x.replace("</t:Message>", &format!("<t:ParentFolderId Id=\"{}\"/></t:Message>", i.folder));
                            }
                            return x;
                        }
                        format!(
                            "<t:Message><t:ItemId Id=\"{}\" ChangeKey=\"ck\"/><t:IsRead>{}</t:IsRead>{}{}{}</t:Message>",
                            i.id,
                            i.read,
                            super::flag_xml(i.flagged, i.complete, i.complete_time.as_deref(), &i.verb, &i.follow_up),
                            super::categories_xml(&i.categories),
                            super::fields::kind_xml(i, kind_asked.0, kind_asked.1)
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
                    if self.occurrence_deleted(&master, index) {
                        out.push(err("ErrorCalendarOccurrenceIsDeletedFromRecurrence"));
                        continue;
                    }
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
                        Some(i) => {
                            let x = super::fields::with_kind(self.item_xml(i, mime, text), i, op, text);
                            out.push(ok(format!("<m:Items>{}</m:Items>", self.with_item_policy(op, &i.id, x))))
                        }
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
                    // A navigation shortcut in the Common Views folder ([MS-OXOCFG] 2.2.9): an
                    // associated item, made with its item id and the properties FindItem asks for.
                    if folder == "COMMON-VIEWS" {
                        let id = self.id("W");
                        let class = m.text_of("ItemClass").unwrap_or_default().to_string();
                        let subject = m.text_of("Subject").unwrap_or_default().to_string();
                        let mut stored = xml(&m);
                        stored = stored.replacen('>', &format!("><t:ItemId Id=\"{}\" ChangeKey=\"ck\"/>", esc(&id)), 1);
                        let extra = format!(
                            "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x1A\" PropertyType=\"String\"/><t:Value>{}</t:Value></t:ExtendedProperty>\
                             <t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0xE1D\" PropertyType=\"String\"/><t:Value>{}</t:Value></t:ExtendedProperty>",
                            esc(&class),
                            esc(&subject)
                        );
                        stored = stored.replacen("</t:Message>", &format!("{extra}</t:Message>"), 1);
                        if self.hide_created_group {
                            self.hidden_wlinks.push(id.clone());
                        }
                        self.wlinks.push(stored);
                        out.push(ok(format!("<m:Items><t:Message><t:ItemId Id=\"{id}\" ChangeKey=\"ck\"/></t:Message></m:Items>")));
                        continue;
                    }
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
                            follow_up: Default::default(),
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
                        follow_up: Default::default(),
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
                    // A navigation shortcut ([MS-OXOCFG] 2.2.9): its fields changed as asked.
                    if let Some(pos) = self.wlinks.iter().position(|w| wlink_id(w).as_deref() == Some(id.as_str())) {
                        let mut e = parse(&self.wlinks[pos]);
                        let mut item = e.kids.remove(0);
                        let updates = ch.child("Updates").map(|u| u.kids.clone()).unwrap_or_default();
                        for u in updates {
                            if u.name != "SetItemField" {
                                continue;
                            }
                            if let Some(field) = u.find("FieldURI").and_then(|f| f.attr("FieldURI")) {
                                let field = field.split(':').nth(1).unwrap_or("").to_string();
                                if !field.is_empty() {
                                    let value = u.kids.iter().find(|k| k.name != "FieldURI").and_then(|w| w.kids.first()).cloned();
                                    item.kids.retain(|k| k.name != field);
                                    if let Some(v) = value {
                                        if field == "Subject" {
                                            set_ext(&mut item, "0xE1D", "String", &v.text);
                                        }
                                        item.kids.push(v);
                                    }
                                }
                            }
                            if let Some(tag) = u.child("ExtendedFieldURI").and_then(|f| f.attr("PropertyTag"))
                                && let Some(value) = u.find("ExtendedProperty").and_then(|p| p.text_of("Value"))
                            {
                                let kind = u.child("ExtendedFieldURI").and_then(|f| f.attr("PropertyType")).unwrap_or("String");
                                set_ext(&mut item, tag, kind, value);
                            }
                        }
                        self.wlinks[pos] = xml(&item);
                        out.push(ok(String::new()));
                        continue;
                    }
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
                            // A property of Quatrelle's own, known by its name: set on an appointment, in
                            // place of its namesake, or taken off it (noted in `calls`, for the tests to
                            // see what was asked).
                            if matches!(u.name.as_str(), "SetItemField" | "DeleteItemField")
                                && el.name == "CalendarItem"
                                && let Some(name) = u.child("ExtendedFieldURI").and_then(|f| f.attr("PropertyName")).map(str::to_string)
                            {
                                let named = |k: &El| k.child("ExtendedFieldURI").and_then(|f| f.attr("PropertyName")) == Some(name.as_str());
                                el.kids.retain(|k| k.name != "ExtendedProperty" || !named(k));
                                if u.name == "SetItemField" {
                                    el.kids.extend(u.find("CalendarItem").into_iter().flat_map(|c| c.kids.iter().filter(|k| named(k)).cloned()));
                                } else {
                                    self.calls.push(format!("DeleteItemField {name}"));
                                }
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
                    // A navigation shortcut ([MS-OXOCFG] 2.2.9) goes.
                    if self.delete_busy.contains(&id) && self.wlinks.iter().any(|w| wlink_id(w).as_deref() == Some(id.as_str())) {
                        out.push(err("ErrorServerBusy"));
                        continue;
                    }
                    if let Some(pos) = self.wlinks.iter().position(|w| wlink_id(w).as_deref() == Some(id.as_str())) {
                        self.wlinks.remove(pos);
                        self.hidden_wlinks.retain(|h| h != &id);
                        out.push(ok(String::new()));
                        continue;
                    }
                    if !self.may_change(&id) || self.undeletable.contains(&id) {
                        out.push(err("ErrorAccessDenied"));
                        continue;
                    }
                    // Not now: the server is busy with it (until the test clears `delete_busy`).
                    if self.delete_busy.contains(&id) {
                        out.push(err("ErrorServerBusy"));
                        continue;
                    }
                    // One the server doesn't have (deleted in Outlook meanwhile).
                    if !self.items.iter().any(|i| i.id == id)
                        && !self.recoverable.iter().any(|i| i.id == id)
                        && !self.pim.iter().any(|(i, _, _)| *i == id)
                    {
                        out.push(err("ErrorItemNotFound"));
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
                    if self.pim.iter().any(|(i, _, el)| *i == id && el.name == "CalendarItem") {
                        self.cancellations.push((id.clone(), op.attr("SendMeetingCancellations").unwrap_or_default().to_string()));
                        self.delete_occurrence(&id);
                    }
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
            "GetMailTips" if self.mail_tips_refused => out.push(err("ErrorInvalidOperation")),
            "GetMailTips" => out.push(self.mail_tips(op)),
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

/// The id of a navigation shortcut kept as its XML.
fn wlink_id(w: &str) -> Option<String> {
    parse(w).find("ItemId")?.attr("Id").map(str::to_string)
}

/// Set an extended property of a shortcut kept as an element, replacing its namesake.
fn set_ext(item: &mut El, tag: &str, kind: &str, value: &str) {
    item.kids.retain(|k| {
        !(k.name == "ExtendedProperty" && k.child("ExtendedFieldURI").and_then(|u| u.attr("PropertyTag")).is_some_and(|t| crate::ews::is_tag(t, tag)))
    });
    if let Some(e) = parse(&format!(
        "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"{tag}\" PropertyType=\"{kind}\"/><t:Value>{}</t:Value></t:ExtendedProperty>",
        esc(value)
    ))
    .kids
    .first()
    {
        item.kids.push(e.clone());
    }
}

/// The message class a `FindItem` restriction asks for, when it asks for one (`IsEqualTo` on
/// `message:MessageClass`).
fn class_wanted(op: &El) -> Option<String> {
    let eq = op.child("Restriction")?.child("IsEqualTo")?;
    if eq.find("FieldURI")?.attr("FieldURI")? != "message:MessageClass" {
        return None;
    }
    Some(eq.find("Constant")?.attr("Value")?.to_string())
}

/// The extended properties a `FindItem` asks for, by tag and type: a stored property is only
/// given back under the type it was stored with, as Exchange does.
fn wanted_props(op: &El) -> Vec<(String, String)> {
    op.find("AdditionalProperties")
        .map(|a| {
            a.children("ExtendedFieldURI")
                .filter_map(|u| Some((norm_tag(u.attr("PropertyTag")?), u.attr("PropertyType")?.to_ascii_lowercase())))
                .collect()
        })
        .unwrap_or_default()
}

/// A property tag as `0x` and four hex digits, whichever way it was written.
fn norm_tag(t: &str) -> String {
    let hex = t.trim().strip_prefix("0x").or_else(|| t.trim().strip_prefix("0X"));
    hex.and_then(|h| u32::from_str_radix(h, 16).ok()).map(|n| format!("0x{n:04X}")).unwrap_or_else(|| t.trim().to_ascii_lowercase())
}

/// A stored shortcut with only the extended properties asked for under their own type.
fn props_only(w: &str, wanted: &[(String, String)]) -> String {
    if wanted.is_empty() {
        return w.to_string();
    }
    let e = parse(w);
    let Some(mut m) = e.kids.first().cloned() else { return w.to_string() };
    m.kids.retain(|k| {
        if k.name != "ExtendedProperty" {
            return true;
        }
        let Some(u) = k.child("ExtendedFieldURI") else { return false };
        let tag = u.attr("PropertyTag").map(norm_tag);
        let ty = u.attr("PropertyType").map(str::to_ascii_lowercase);
        wanted.iter().any(|(wt, wy)| tag.as_deref() == Some(wt.as_str()) && ty.as_deref() == Some(wy.as_str()))
    });
    xml(&m)
}

/// A navigation shortcut's message class (`0x001A`), as it was kept.
fn wlink_class(w: &str) -> Option<String> {
    let e = parse(w);
    let m = e.kids.first()?;
    m.children("ExtendedProperty")
        .find(|p| p.child("ExtendedFieldURI").and_then(|u| u.attr("PropertyTag")).is_some_and(|t| crate::ews::is_tag(t, "0x001A")))
        .and_then(|p| p.text_of("Value"))
        .map(str::to_string)
}
