//! An in-memory Exchange Web Services server on localhost, for tests: folders (distinguished
//! ids, a deep tree), messages (find, get with their MIME source, update, move with a new id,
//! delete), creating from MIME (drafts and sending), folder changes and automatic replies, signed
//! in with NTLM on a kept-open connection (or Basic, when set to).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use crate::ews::{El, esc, parse, unb64};

pub const USER: &str = "riley";
pub const DOMAIN: &str = "CONTOSO";
pub const PASSWORD: &str = "pw";

#[derive(Clone, Debug)]
pub struct Fo {
    pub id: String,
    pub name: String,
    pub parent: String,
    pub class: String,
    /// The distinguished id (`inbox`, `sentitems`…).
    pub known: Option<String>,
}

#[derive(Clone, Debug)]
pub struct It {
    pub id: String,
    pub folder: String,
    pub raw: Vec<u8>,
    pub read: bool,
    pub flagged: bool,
    pub draft: bool,
    pub received: String,
    /// The voting buttons (`PidLidVerbStream`), when it has some.
    pub voting: Vec<u8>,
    /// The choice a vote carries (`PidLidVerbResponse`).
    pub vote: String,
}

pub struct Data {
    pub folders: Vec<Fo>,
    pub items: Vec<It>,
    /// Appointments and contacts: (id, folder, the element as created).
    pub pim: Vec<(String, String, El)>,
    /// MIME sources sent.
    pub sent: Vec<Vec<u8>>,
    /// Votes sent: (subject, choice).
    pub votes: Vec<(String, String)>,
    /// (state, start, end, message)
    /// State, start, end, inside reply, outside reply, outside audience.
    pub oof: (String, String, String, String, String, String),
    /// Sign in with Basic instead of NTLM.
    pub basic: bool,
    /// Operations asked for, in order.
    pub calls: Vec<String>,
    /// NTLM handshakes done (one per connection).
    pub handshakes: usize,
    /// The EWS address, for Autodiscover's answer.
    pub url: String,
    /// People whose calendars this account may change (delegates with editor rights).
    pub editors: Vec<String>,
    /// The public folder mailbox holding the public folders' content (Exchange 2013 and
    /// later): reading inside them is only answered when routed there.
    pub pf_mailbox: Option<String>,
    /// The request's `X-PublicFolderMailbox` header.
    routed: String,
    /// Other mailboxes this account may open (shared, or as a delegate); their folders hang
    /// under `ROOT-<address>`.
    pub shared: Vec<String>,
    /// Outlook's navigation shortcuts, as items of the Common Views folder (hidden ones).
    pub wlinks: Vec<String>,
    /// The directory: (X.500 name, address).
    pub directory: Vec<(String, String)>,
    /// Answer item searches (the mail sync) only after this long: a mailbox taking minutes.
    pub slow_sync: Option<std::time::Duration>,
    next: u32,
}

/// The public folder content mailbox's GUID, as `PR_REPLICA_LIST` names it.
pub const PF_GUID: &str = "1ec2a236-ed93-4f88-b9c6-33e63fa4aa44";

impl Default for Data {
    fn default() -> Data {
        let mut d = Data {
            folders: Vec::new(),
            items: Vec::new(),
            pim: Vec::new(),
            sent: Vec::new(),
            votes: Vec::new(),
            oof: ("Disabled".into(), String::new(), String::new(), String::new(), String::new(), "All".into()),
            basic: false,
            calls: Vec::new(),
            handshakes: 0,
            url: String::new(),
            editors: Vec::new(),
            pf_mailbox: None,
            routed: String::new(),
            shared: Vec::new(),
            wlinks: Vec::new(),
            directory: Vec::new(),
            slow_sync: None,
            next: 0,
        };
        d.folders.push(Fo {
            id: "ROOT".into(),
            name: "Top of Information Store".into(),
            parent: String::new(),
            class: "IPF.Note".into(),
            known: Some("msgfolderroot".into()),
        });
        d
    }
}

impl Data {
    fn id(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    pub fn folder(&mut self, name: &str, parent: Option<&str>, known: Option<&str>, class: &str) -> String {
        let id = self.id("F");
        self.folders.push(Fo {
            id: id.clone(),
            name: name.into(),
            parent: parent.unwrap_or("ROOT").into(),
            class: class.into(),
            known: known.map(str::to_string),
        });
        id
    }

    pub fn deliver(&mut self, folder: &str, raw: &[u8], read: bool, received: &str) -> String {
        let id = self.id("I");
        self.items.push(It {
            id: id.clone(),
            folder: folder.into(),
            raw: raw.to_vec(),
            read,
            flagged: false,
            draft: false,
            received: received.into(),
            voting: Vec::new(),
            vote: String::new(),
        });
        id
    }

    /// A folder of another mailbox (`parent` `None`: at its top), the mailbox's root made the
    /// first time.
    pub fn shared_folder(&mut self, owner: &str, name: &str, parent: Option<&str>, known: Option<&str>) -> String {
        let root = format!("ROOT-{owner}");
        if !self.folders.iter().any(|f| f.id == root) {
            self.folders.push(Fo {
                id: root.clone(),
                name: "Top of Information Store".into(),
                parent: String::new(),
                class: "IPF.Note".into(),
                known: Some("msgfolderroot".into()),
            });
        }
        self.folder(name, Some(parent.unwrap_or(&root)), known, "IPF.Note")
    }

    /// Whose mailbox a folder is in (`None`: this account's).
    fn owner_of(&self, folder: &str) -> Option<String> {
        let mut at = folder.to_string();
        for _ in 0..32 {
            if let Some(o) = at.strip_prefix("ROOT-") {
                return Some(o.to_string());
            }
            match self.folders.iter().find(|f| f.id == at) {
                Some(f) if !f.parent.is_empty() => at = f.parent.clone(),
                _ => return None,
            }
        }
        None
    }

    pub fn by_name(&self, name: &str) -> Option<&Fo> {
        self.folders.iter().find(|f| f.name == name)
    }

    fn folder_id(&self, el: &El) -> Option<String> {
        match el.name.as_str() {
            "FolderId" => el.attr("Id").map(str::to_string),
            // Another mailbox's folder: its items are kept as `mailbox:<address>`.
            "DistinguishedFolderId" if el.child("Mailbox").is_some() => {
                let who = el.find("EmailAddress").map(|e| e.text.to_ascii_lowercase())?;
                // A mailbox shared with this account: its own well-known folder.
                if self.shared.contains(&who) {
                    let known = el.attr("Id")?;
                    return self
                        .folders
                        .iter()
                        .find(|f| f.known.as_deref() == Some(known) && self.owner_of(&f.id).as_deref() == Some(who.as_str()))
                        .map(|f| f.id.clone());
                }
                let id = format!("mailbox:{who}");
                self.pim.iter().any(|(_, f, _)| *f == id).then_some(id)
            }
            "DistinguishedFolderId" => {
                let known = el.attr("Id")?;
                self.folders.iter().find(|f| f.known.as_deref() == Some(known) && self.owner_of(&f.id).is_none()).map(|f| f.id.clone())
            }
            _ => None,
        }
    }

    fn folder_xml(&self, f: &Fo) -> String {
        let total = self.items.iter().filter(|i| i.folder == f.id).count() + self.pim.iter().filter(|(_, x, _)| *x == f.id).count();
        let unread = self.items.iter().filter(|i| i.folder == f.id && !i.read).count();
        // Public folders name the mailbox holding their content (a GUID, as ASCII, NUL-ended).
        let replica = if self.pf_mailbox.is_some() && self.public_ids().contains(&f.id) {
            format!(
                "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x6698\" PropertyType=\"Binary\"/><t:Value>{}</t:Value></t:ExtendedProperty>",
                crate::imap::base64(format!("{PF_GUID}\0").as_bytes())
            )
        } else {
            String::new()
        };
        format!(
            "<t:Folder><t:FolderId Id=\"{}\"/><t:ParentFolderId Id=\"{}\"/><t:FolderClass>{}</t:FolderClass><t:DisplayName>{}</t:DisplayName><t:TotalCount>{total}</t:TotalCount><t:ChildFolderCount>0</t:ChildFolderCount><t:UnreadCount>{unread}</t:UnreadCount>{replica}</t:Folder>",
            f.id,
            f.parent,
            f.class,
            esc(&f.name)
        )
    }

    /// Whether a request for this folder's content goes unanswered: a public folder, asked
    /// without being routed to the mailbox holding it.
    fn unrouted(&self, folder: &str) -> bool {
        self.pf_mailbox.as_ref().is_some_and(|m| *m != self.routed) && self.public_ids().iter().any(|p| p == folder)
    }

    fn item_xml(&self, it: &It, mime_content: bool) -> String {
        let p = mailcraft_mime::parse(&it.raw).unwrap_or_default();
        let mb = |a: &mailcraft_model::EmailAddress| {
            format!(
                "<t:Mailbox><t:Name>{}</t:Name><t:EmailAddress>{}</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType></t:Mailbox>",
                esc(&a.name),
                esc(&a.email)
            )
        };
        let ext = |tag: &str, v: i64| {
            format!(
                "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"{tag}\" PropertyType=\"Integer\"/><t:Value>{v}</t:Value></t:ExtendedProperty>"
            )
        };
        let mut x = format!("<t:Message><t:ItemId Id=\"{}\" ChangeKey=\"ck\"/>", it.id);
        if mime_content {
            x.push_str(&format!("<t:MimeContent CharacterSet=\"UTF-8\">{}</t:MimeContent>", crate::imap::base64(&it.raw)));
        }
        x.push_str(&format!(
            "<t:Subject>{}</t:Subject><t:DateTimeReceived>{}</t:DateTimeReceived><t:Size>{}</t:Size><t:HasAttachments>false</t:HasAttachments>",
            esc(&p.subject),
            it.received,
            it.raw.len()
        ));
        if it.flagged {
            x.push_str(&ext("0x1090", 2));
        }
        x.push_str(&ext("0x0E07", if it.draft { 8 } else { 1 }));
        if !it.voting.is_empty() {
            x.push_str(&format!(
                "<t:ExtendedProperty><t:ExtendedFieldURI DistinguishedPropertySetId=\"Common\" PropertyId=\"34080\" PropertyType=\"Binary\"/><t:Value>{}</t:Value></t:ExtendedProperty>",
                crate::imap::base64(&it.voting)
            ));
        }
        x.push_str(&format!("<t:ToRecipients>{}</t:ToRecipients>", p.to.iter().map(mb).collect::<String>()));
        if !p.cc.is_empty() {
            x.push_str(&format!("<t:CcRecipients>{}</t:CcRecipients>", p.cc.iter().map(mb).collect::<String>()));
        }
        x.push_str(&format!("<t:From>{}</t:From>", mb(&p.from)));
        x.push_str(&format!("<t:InternetMessageId>&lt;{}&gt;</t:InternetMessageId><t:IsRead>{}</t:IsRead></t:Message>", esc(&p.message_id), it.read));
        x
    }

    /// The public folders root and everything under it.
    fn public_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.folders.iter().filter(|f| f.known.as_deref() == Some("publicfoldersroot")).map(|f| f.id.clone()).collect();
        let mut i = 0;
        while let Some(id) = ids.get(i).cloned() {
            ids.extend(self.folders.iter().filter(|f| f.parent == id).map(|f| f.id.clone()));
            i += 1;
        }
        ids
    }

    /// Whether this account may change an item: anything of its own, and someone else's only where
    /// they made it an editor.
    fn may_change(&self, id: &str) -> bool {
        match self.pim.iter().find(|(i, _, _)| i == id).and_then(|(_, f, _)| f.strip_prefix("mailbox:")) {
            Some(who) => self.editors.iter().any(|e| e == who),
            None => true,
        }
    }

    fn pim_folder(&self, f: Option<&El>) -> bool {
        let Some(id) = f.and_then(|f| self.folder_id(f)) else { return false };
        id.starts_with("mailbox:") || self.folders.iter().any(|x| x.id == id && x.class != "IPF.Note")
    }

    /// Answer one operation: (response element name, response messages).
    fn route(&mut self, op: &El) -> (String, Vec<String>) {
        let ok = |inner: String| format!("<m:R ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>{inner}</m:R>");
        let err =
            |code: &str| format!("<m:R ResponseClass=\"Error\"><m:MessageText>{code}</m:MessageText><m:ResponseCode>{code}</m:ResponseCode></m:R>");
        let ids = |el: Option<&El>| -> Vec<String> {
            el.map(|e| e.kids.iter().filter_map(|k| k.attr("Id").map(str::to_string)).collect()).unwrap_or_default()
        };
        let mut out = Vec::new();
        match op.name.as_str() {
            "GetFolder" => {
                for f in op.child("FolderIds").map(|f| f.kids.clone()).unwrap_or_default() {
                    // Someone else's calendar: what this account may do there.
                    if let Some(who) = self.folder_id(&f).and_then(|id| id.strip_prefix("mailbox:").map(str::to_string)) {
                        let edit = self.editors.contains(&who);
                        out.push(ok(format!(
                            "<m:Folders><t:CalendarFolder><t:FolderId Id=\"mailbox:{who}\"/><t:EffectiveRights><t:CreateAssociated>false</t:CreateAssociated>\
                             <t:CreateContents>{edit}</t:CreateContents><t:CreateHierarchy>false</t:CreateHierarchy><t:Delete>{edit}</t:Delete>\
                             <t:Modify>false</t:Modify><t:Read>true</t:Read></t:EffectiveRights></t:CalendarFolder></m:Folders>"
                        )));
                        continue;
                    }
                    match self.folder_id(&f).and_then(|id| self.folders.iter().find(|x| x.id == id).cloned()) {
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
                let want = op.child("UnresolvedEntry").map(|e| e.text.clone()).unwrap_or_default();
                match self.directory.iter().find(|(dn, _)| dn.eq_ignore_ascii_case(&want)) {
                    Some((_, email)) => out.push(ok(format!(
                        "<m:ResolutionSet TotalItemsInView=\"1\"><t:Resolution><t:Mailbox><t:Name>{email}</t:Name>\
                         <t:EmailAddress>{email}</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType></t:Mailbox></t:Resolution></m:ResolutionSet>"
                    ))),
                    None => out.push(err("ErrorNameResolutionNoResults")),
                }
            }
            // Shallow (public folders): the folders right under the one asked for.
            "FindFolder" if op.attr("Traversal") == Some("Shallow") => {
                let parent = op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_default();
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
            "FindFolder" => {
                // Deep: everything under the root (the mailbox's, not the public folders), in pages.
                let view = op.child("IndexedPageFolderView");
                let offset: usize = view.and_then(|v| v.attr("Offset")).and_then(|o| o.parse().ok()).unwrap_or(0);
                let max: usize = view.and_then(|v| v.attr("MaxEntriesReturned")).and_then(|o| o.parse().ok()).unwrap_or(1000).min(3);
                let public = self.public_ids();
                // Under the mailbox asked for: this account's, or one shared with it.
                let Some(root) = op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.folder_id(f)) else {
                    out.push(err("ErrorAccessDenied"));
                    return ("FindFolderResponse".into(), out);
                };
                let owner = self.owner_of(&root);
                let all: Vec<Fo> = self
                    .folders
                    .iter()
                    .filter(|f| f.id != "ROOT" && !f.id.starts_with("ROOT-") && !public.contains(&f.id) && self.owner_of(&f.id) == owner)
                    .cloned()
                    .collect();
                let page: Vec<String> = all.iter().skip(offset).take(max).map(|f| self.folder_xml(f)).collect();
                let last = offset + max >= all.len();
                out.push(ok(format!(
                    "<m:RootFolder TotalItemsInView=\"{}\" IncludesLastItemInRange=\"{last}\"><t:Folders>{}</t:Folders></m:RootFolder>",
                    all.len(),
                    page.concat()
                )));
            }
            "FindItem" if self.pim_folder(op.child("ParentFolderIds").and_then(|p| p.kids.first())) => {
                let folder = op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_default();
                if self.unrouted(&folder) {
                    out.push(err("ErrorFolderNotFound"));
                    return ("FindItemResponse".into(), out);
                }
                let items: String =
                    self.pim.iter().filter(|(_, f, _)| *f == folder).map(|(id, _, _)| format!("<t:Item><t:ItemId Id=\"{id}\"/></t:Item>")).collect();
                out.push(ok(format!("<m:RootFolder IncludesLastItemInRange=\"true\"><t:Items>{items}</t:Items></m:RootFolder>")));
            }
            "FindItem" => {
                // A folder the mailbox hasn't (a Notes folder, say): as Exchange answers.
                let Some(folder) = op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.folder_id(f)) else {
                    out.push(err("ErrorFolderNotFound"));
                    return ("FindItemResponse".into(), out);
                };
                let max: usize =
                    op.child("IndexedPageItemView").and_then(|v| v.attr("MaxEntriesReturned")).and_then(|o| o.parse().ok()).unwrap_or(1000);
                let mut list: Vec<&It> = self.items.iter().filter(|i| i.folder == folder).collect();
                list.sort_by(|a, b| b.received.cmp(&a.received));
                let items: String = list
                    .iter()
                    .take(max)
                    .map(|i| {
                        format!(
                            "<t:Message><t:ItemId Id=\"{}\" ChangeKey=\"ck\"/><t:IsRead>{}</t:IsRead>{}</t:Message>",
                            i.id,
                            i.read,
                            if i.flagged {
                                "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/><t:Value>2</t:Value></t:ExtendedProperty>"
                            } else {
                                ""
                            }
                        )
                    })
                    .collect();
                out.push(ok(format!(
                    "<m:RootFolder TotalItemsInView=\"{}\" IncludesLastItemInRange=\"true\"><t:Items>{items}</t:Items></m:RootFolder>",
                    list.len()
                )));
            }
            "GetItem" => {
                let mime = op.find("IncludeMimeContent").is_some_and(|m| m.text == "true");
                for id in ids(op.child("ItemIds")) {
                    if self.pim.iter().any(|(i, f, _)| *i == id && self.unrouted(f)) {
                        out.push(err("ErrorItemNotFound"));
                        continue;
                    }
                    if let Some((_, _, el)) = self.pim.iter().find(|(i, _, _)| *i == id) {
                        let mut el = el.clone();
                        el.kids.insert(
                            0,
                            El {
                                name: "ItemId".into(),
                                attrs: vec![("Id".into(), id.clone()), ("ChangeKey".into(), "ck".into())],
                                ..Default::default()
                            },
                        );
                        out.push(ok(format!("<m:Items>{}</m:Items>", xml(&el))));
                        continue;
                    }
                    match self.items.iter().find(|i| i.id == id) {
                        Some(i) => out.push(ok(format!("<m:Items>{}</m:Items>", self.item_xml(i, mime)))),
                        None => out.push(err("ErrorItemNotFound")),
                    }
                }
            }
            "CreateItem" => {
                let send = op.attr("MessageDisposition") == Some("SendAndSaveCopy");
                let folder = op.child("SavedItemFolderId").and_then(|s| s.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_default();
                for m in op.child("Items").map(|i| i.kids.clone()).unwrap_or_default() {
                    let sticky = m.text_of("ItemClass").is_some_and(|c| c.starts_with("IPM.StickyNote"));
                    if folder.strip_prefix("mailbox:").is_some_and(|who| !self.editors.iter().any(|e| e == who)) {
                        out.push(err("ErrorAccessDenied"));
                        continue;
                    }
                    if m.name == "CalendarItem" || m.name == "Contact" || sticky {
                        let id = self.id("P");
                        out.push(ok(format!("<m:Items><t:{0}><t:ItemId Id=\"{id}\" ChangeKey=\"ck\"/></t:{0}></m:Items>", m.name)));
                        self.pim.push((id, folder.clone(), m));
                        continue;
                    }
                    // A reply made by Exchange from the original (a voting response starts so).
                    if m.name == "ReplyToItem" {
                        let reference = m.child("ReferenceItemId").and_then(|r| r.attr("Id")).unwrap_or_default().to_string();
                        let Some(orig) = self.items.iter().find(|i| i.id == reference).map(|i| mailcraft_mime::parse(&i.raw).unwrap_or_default())
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
                        voting: Vec::new(),
                        vote: String::new(),
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
                    if let Some((_, _, el)) = self.pim.iter_mut().find(|(i, _, _)| *i == id) {
                        for u in ch.child("Updates").map(|u| u.kids.clone()).unwrap_or_default() {
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
                            let subject = mailcraft_mime::parse(&it.raw).map(|p| p.subject).unwrap_or_default();
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

pub struct Server {
    pub url: String,
    pub data: Arc<Mutex<Data>>,
}

fn respond(s: &mut impl Write, status: u16, headers: &[(&str, String)], body: &[u8]) {
    let mut head = format!("HTTP/1.1 {status} X\r\nContent-Type: text/xml; charset=utf-8\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

/// Start the server; it runs until the test process ends.
pub fn serve(data: Data) -> Server {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}/EWS/Exchange.asmx", l.local_addr().unwrap().port());
    let mut data = data;
    data.url = url.clone();
    let data = Arc::new(Mutex::new(data));
    let shared = data.clone();
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(s) = conn else { continue };
            let shared = shared.clone();
            std::thread::spawn(move || serve_connection(s, &shared));
        }
    });
    Server { url, data }
}

/// Requests on one kept-open connection: NTLM signs it in once.
fn serve_connection(mut s: std::net::TcpStream, shared: &Arc<Mutex<Data>>) {
    let mut reader = BufReader::new(s.try_clone().unwrap());
    let mut signed_in = false;
    let mut challenge: Option<crate::ntlm::Challenge> = None;
    loop {
        let mut first = String::new();
        if reader.read_line(&mut first).unwrap_or(0) == 0 {
            return;
        }
        let (mut len, mut auth, mut routed) = (0usize, String::new(), String::new());
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                if k.eq_ignore_ascii_case("content-length") {
                    len = v.trim().parse().unwrap_or(0);
                }
                if k.eq_ignore_ascii_case("authorization") {
                    auth = v.trim().to_string();
                }
                if k.eq_ignore_ascii_case("x-publicfoldermailbox") {
                    routed = v.trim().to_string();
                }
            }
        }
        let mut body = vec![0u8; len];
        let _ = reader.read_exact(&mut body);
        let basic = shared.lock().unwrap().basic;
        if basic {
            let expected = format!("Basic {}", crate::imap::base64(format!("{DOMAIN}\\{USER}:{PASSWORD}").as_bytes()));
            if auth != expected {
                respond(&mut s, 401, &[("WWW-Authenticate", "Basic realm=\"x\"".into())], b"");
                continue;
            }
        } else if !signed_in {
            match auth.strip_prefix("NTLM ").and_then(unb64) {
                Some(m) if m.get(8) == Some(&1) => {
                    let c = crate::ntlm::make_challenge([9; 8], &[]);
                    challenge = crate::ntlm::challenge(&c).ok();
                    respond(&mut s, 401, &[("WWW-Authenticate", format!("NTLM {}", crate::imap::base64(&c)))], b"");
                    continue;
                }
                Some(m) if m.get(8) == Some(&3) => {
                    let ok = challenge.as_ref().is_some_and(|c| crate::ntlm::verify(c, &m, USER, DOMAIN, PASSWORD));
                    if !ok {
                        respond(&mut s, 401, &[("WWW-Authenticate", "NTLM".into()), ("WWW-Authenticate", "Negotiate".into())], b"");
                        continue;
                    }
                    signed_in = true;
                    shared.lock().unwrap().handshakes += 1;
                }
                _ => {
                    respond(&mut s, 401, &[("WWW-Authenticate", "Negotiate".into()), ("WWW-Authenticate", "NTLM".into())], b"");
                    continue;
                }
            }
        }
        let doc = parse(&String::from_utf8_lossy(&body));
        // Autodiscover: this server's own address.
        if doc.find("Autodiscover").is_some_and(|a| a.find("EMailAddress").is_some()) {
            let (url, pf) = {
                let d = shared.lock().unwrap();
                (d.url.clone(), d.pf_mailbox.clone())
            };
            // The public folder mailbox, asked for by its GUID: its address.
            let asked = doc.find("EMailAddress").map(|e| e.text.clone()).unwrap_or_default();
            if let Some(pf) = pf.filter(|_| asked.starts_with(PF_GUID)) {
                let answer = format!(
                    "<?xml version=\"1.0\"?><Autodiscover><Response><User><DisplayName>Public folders</DisplayName>\
                     <AutoDiscoverSMTPAddress>{pf}</AutoDiscoverSMTPAddress></User><Account><AccountType>email</AccountType><Action>settings</Action>\
                     <Protocol><Type>EXCH</Type><EwsUrl>{url}</EwsUrl></Protocol></Account></Response></Autodiscover>"
                );
                respond(&mut s, 200, &[], answer.as_bytes());
                continue;
            }
            let answer = format!(
                "<?xml version=\"1.0\"?><Autodiscover><Response><User><DisplayName>Riley</DisplayName></User><Account><AccountType>email</AccountType><Action>settings</Action>\
                 <Protocol><Type>EXCH</Type><EwsUrl>{url}</EwsUrl></Protocol></Account></Response></Autodiscover>"
            );
            respond(&mut s, 200, &[], answer.as_bytes());
            continue;
        }
        let Some(op) = doc.find("Body").and_then(|b| b.kids.first()).cloned() else {
            respond(
                &mut s,
                500,
                &[],
                b"<soap:Envelope><soap:Body><soap:Fault><faultstring>no operation</faultstring></soap:Fault></soap:Body></soap:Envelope>",
            );
            continue;
        };
        let slow = shared.lock().unwrap().slow_sync.filter(|_| op.name == "FindItem");
        if let Some(wait) = slow {
            std::thread::sleep(wait);
        }
        let mut d = shared.lock().unwrap();
        d.calls.push(op.name.clone());
        d.routed = routed;
        let (name, parts) = d.route(&op);
        drop(d);
        let inner = if name.starts_with("GetUserOof") || name.starts_with("SetUserOof") {
            parts.concat()
        } else {
            let tag = name.trim_end_matches("Response").to_string() + "ResponseMessage";
            format!(
                "<m:ResponseMessages>{}</m:ResponseMessages>",
                parts.concat().replace("<m:R ", &format!("<m:{tag} ")).replace("</m:R>", &format!("</m:{tag}>"))
            )
        };
        let xml = format!(
            "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><m:{name} xmlns:m=\"m\" xmlns:t=\"t\">{inner}</m:{name}></s:Body></s:Envelope>"
        );
        respond(&mut s, 200, &[], xml.as_bytes());
    }
}

/// An element as XML again (for items kept as they were created).
fn xml(e: &El) -> String {
    let attrs: String = e.attrs.iter().map(|(k, v)| format!(" {k}=\"{}\"", esc(v))).collect();
    let inner: String = e.kids.iter().map(xml).collect();
    format!("<t:{0}{attrs}>{1}{inner}</t:{0}>", e.name, esc(&e.text))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use mailcraft_model::job::Secret;
    use mailcraft_model::{AccountConfig, Event, Flag, Job, Op, SyncStatus};
    use mailcraft_store::Store;

    use super::*;

    fn until(pool: &crate::worker::Pool, what: &str, mut f: impl FnMut(&Event) -> bool) -> Vec<Event> {
        let end = Instant::now() + Duration::from_secs(20);
        let mut seen = Vec::new();
        while Instant::now() < end {
            if let Some(e) = pool.wait(Duration::from_millis(100)) {
                let hit = f(&e);
                seen.push(e);
                if hit {
                    return seen;
                }
            }
        }
        panic!("timed out waiting for {what}; saw {seen:?}");
    }

    fn eventually(what: &str, mut f: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(20);
        while Instant::now() < end {
            if f() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("timed out waiting for {what}");
    }

    fn mail(subject: &str, id: &str) -> Vec<u8> {
        format!("From: Zoe Park <zoe@contoso.example>\r\nTo: riley@contoso.example\r\nSubject: {subject}\r\nMessage-ID: <{id}@x>\r\n\r\nHello about {subject}.\r\n")
            .into_bytes()
    }

    #[test]
    fn an_on_premises_exchange_account_works_through_ews() {
        let mut d = Data::default();
        let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        d.folder("Drafts", None, Some("drafts"), "IPF.Note");
        let sent = d.folder("Sent Items", None, Some("sentitems"), "IPF.Note");
        let trash = d.folder("Deleted Items", None, Some("deleteditems"), "IPF.Note");
        d.folder("Outbox", None, Some("outbox"), "IPF.Note");
        d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
        let projects = d.folder("Projects", None, None, "IPF.Note");
        d.folder("2024", Some(&projects), None, "IPF.Note");
        let first = d.deliver(&inbox, &mail("Storyboard", "m1"), true, "2024-05-06T09:00:00Z");
        let second = d.deliver(&inbox, &mail("Sound mix", "m2"), false, "2024-05-06T10:00:00Z");
        d.deliver(&projects, &mail("Plan", "m3"), true, "2024-05-01T10:00:00Z");
        let server = serve(d);
        let data = server.data.clone();

        let dir = std::env::temp_dir().join(format!("mailcraft-ews-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir).unwrap();
        let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
        let account = AccountConfig {
            id: "a1".into(),
            name: "Riley".into(),
            email: "riley@contoso.example".into(),
            username: format!("{DOMAIN}\\{USER}"),
            ews: Some(server.url.clone()),
            ..Default::default()
        };
        assert!(!account.is_local() && account.is_api());

        // A wrong password means signing in again.
        pool.submit(Job::Start { account: account.clone(), password: Some(Secret("wrong".into())) });
        until(&pool, "sign in again", |e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }));

        pool.submit(Job::Start { account: account.clone(), password: Some(Secret(PASSWORD.into())) });
        let seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
        let folders = seen
            .iter()
            .rev()
            .find_map(|e| match e {
                Event::Folders { folders, .. } => Some(folders.clone()),
                _ => None,
            })
            .unwrap();
        let mut paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
        paths.sort_unstable();
        assert_eq!(paths, ["Deleted Items", "Drafts", "INBOX", "Projects", "Projects/2024", "Sent Items"], "pages followed; no Outbox, no calendar");
        let inbox_folder = folders.iter().find(|f| f.path == "INBOX").unwrap();
        assert_eq!((inbox_folder.total, inbox_folder.unread), (2, 1));
        let st = store.load_folder("a1", "INBOX").unwrap();
        let subjects: Vec<&str> = st.messages.iter().map(|m| m.subject.as_str()).collect();
        assert_eq!(subjects, ["Sound mix", "Storyboard"], "newest first");
        assert!(st.messages[1].flags.seen && !st.messages[0].flags.seen);
        assert_eq!(st.messages[0].from.email, "zoe@contoso.example");
        assert!(store.has_body(&st.messages[0].key), "the newest sources are downloaded");
        assert!(data.lock().unwrap().handshakes <= 2, "NTLM signs in the connection once, not every request");

        // Open a message whose source isn't cached.
        let key = st.messages[0].key.clone();
        store.remove_body(&key);
        pool.submit(Job::FetchBody { key: key.clone() });
        until(&pool, "body", |e| matches!(e, Event::Body { key: k, raw } if *k == key && raw.starts_with(b"From:")));

        // Read, flag, move (Exchange gives the moved message a new id).
        pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true } });
        eventually("read on the server", || data.lock().unwrap().items.iter().any(|i| i.id == second && i.read));
        pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Flagged, on: true } });
        eventually("flagged on the server", || data.lock().unwrap().items.iter().any(|i| i.id == second && i.flagged));
        pool.submit(Job::Apply { account: "a1".into(), op: Op::Move { keys: vec![key.clone()], to: "Projects".into() } });
        eventually("moved on the server", || {
            data.lock().unwrap().items.iter().any(|i| i.folder == projects && String::from_utf8_lossy(&i.raw).contains("Sound mix"))
        });
        until(&pool, "the move shows in Projects", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Projects"));
        eventually("in Projects here, flagged", || {
            store.load_folder("a1", "Projects").unwrap().messages.iter().any(|m| m.subject == "Sound mix" && m.flags.flagged)
        });

        // Folders: create, rename and move to the top, delete.
        pool.submit(Job::Apply { account: "a1".into(), op: Op::CreateFolder { path: "Projects/Caf&AOk-".into() } });
        eventually("created", || data.lock().unwrap().by_name("Café").is_some_and(|f| f.parent == projects));
        until(&pool, "folder list", |e| matches!(e, Event::Folders { folders, .. } if folders.iter().any(|f| f.path == "Projects/Caf&AOk-")));
        pool.submit(Job::Apply { account: "a1".into(), op: Op::RenameFolder { from: "Projects/Caf&AOk-".into(), to: "Clients".into() } });
        eventually("renamed", || data.lock().unwrap().by_name("Clients").is_some_and(|f| f.parent == "ROOT"));
        until(&pool, "renamed in the list", |e| matches!(e, Event::Folders { folders, .. } if folders.iter().any(|f| f.path == "Clients")));
        pool.submit(Job::Apply { account: "a1".into(), op: Op::DeleteFolder { path: "Clients".into() } });
        eventually("deleted", || data.lock().unwrap().by_name("Clients").is_none());

        // Automatic replies.
        pool.submit(Job::GetVacation { account: "a1".into() });
        until(&pool, "replies read", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if !v.enabled));
        let away = mailcraft_model::job::Vacation {
            enabled: true,
            subject: String::new(),
            text: "Back on Monday".into(),
            from: Some("2026-10-15".into()),
            until: Some("2026-10-20".into()),
            ..Default::default()
        };
        pool.submit(Job::SetVacation { account: "a1".into(), vacation: away.clone() });
        until(&pool, "replies on", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == away));
        assert_eq!(data.lock().unwrap().oof.0, "Scheduled");
        // Another reply for people outside the organisation, to the user's contacts only.
        let split = mailcraft_model::job::Vacation { external: Some("Out of office".into()), outside: "known".into(), ..away.clone() };
        pool.submit(Job::SetVacation { account: "a1".into(), vacation: split.clone() });
        until(&pool, "two replies", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == split));
        let oof = data.lock().unwrap().oof.clone();
        assert_eq!((oof.4.as_str(), oof.5.as_str()), ("Out of office", "Known"));
        pool.submit(Job::SetVacation { account: "a1".into(), vacation: away.clone() });
        until(&pool, "one reply again", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == away));
        pool.submit(Job::SetVacation { account: "a1".into(), vacation: mailcraft_model::job::Vacation { text: "refuse".into(), ..away.clone() } });
        until(&pool, "a refused change", |e| matches!(e, Event::Vacation { vacation: Some(v), error: Some(_), .. } if *v == away));

        // Empty Deleted Items; delete for good.
        for i in 0..3 {
            data.lock().unwrap().deliver(&trash, &mail(&format!("Old {i}"), &format!("t{i}")), true, "2024-01-01T00:00:00Z");
        }
        pool.submit(Job::Apply { account: "a1".into(), op: Op::EmptyFolder { path: "Deleted Items".into() } });
        eventually("emptied", || !data.lock().unwrap().items.iter().any(|i| i.folder == trash));
        let gone = store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == "Storyboard").unwrap().key.clone();
        pool.submit(Job::Apply { account: "a1".into(), op: Op::Expunge { keys: vec![gone] } });
        eventually("deleted for good", || !data.lock().unwrap().items.iter().any(|i| i.id == first));

        // Send: the Bcc goes in the MIME for Exchange; Sent Items has the copy.
        let raw = b"From: riley@contoso.example\r\nTo: zoe@contoso.example\r\nSubject: e2e\r\nMessage-ID: <e2e@x>\r\n\r\nhi\r\n".to_vec();
        let send = |id: u64, to: Vec<String>| Job::Send {
            account: "a1".into(),
            id,
            raw: raw.clone(),
            from: "riley@contoso.example".into(),
            recipients: to,
            sent_folder: Some("Sent Items".into()),
        };
        pool.submit(send(7, vec!["zoe@contoso.example".into(), "boss@contoso.example".into()]));
        until(&pool, "sent", |e| matches!(e, Event::Sent { id: 7, .. }));
        let out = String::from_utf8(data.lock().unwrap().sent.last().unwrap().clone()).unwrap();
        assert!(out.starts_with("Bcc: boss@contoso.example\r\nFrom:"), "{out}");
        assert!(data.lock().unwrap().items.iter().any(|i| i.folder == sent));
        pool.submit(send(8, vec!["nobody@contoso.example".into()]));
        until(&pool, "refused", |e| matches!(e, Event::SendFailed { id: 8, .. }));

        // A draft replaces its earlier copy.
        pool.submit(Job::SaveDraft { account: "a1".into(), raw: mail("Draft one", "d1"), folder: "Drafts".into(), replaces: None });
        until(&pool, "draft", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Drafts"));
        let draft = store.load_folder("a1", "Drafts").unwrap().messages.iter().find(|m| m.subject == "Draft one").unwrap().key.clone();
        assert!(store.load_folder("a1", "Drafts").unwrap().messages.iter().any(|m| m.flags.draft));
        pool.submit(Job::SaveDraft { account: "a1".into(), raw: mail("Draft two", "d2"), folder: "Drafts".into(), replaces: Some(draft) });
        eventually("one draft left", || data.lock().unwrap().items.iter().filter(|i| i.draft).count() == 1);
        pool.submit(Job::Remove { account: "a1".into() });
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn calendars_and_contacts_sync_and_autodiscover_finds_the_server() {
        use mailcraft_model::ical::CalEvent;
        use mailcraft_model::pim::{LOCAL, Pim, StoredCard, StoredEvent, Tombstone};
        use mailcraft_model::vcard::{Card, Typed};
        let mut d = Data::default();
        d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
        d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
        let standup = parse(
            "<t:CalendarItem><t:Subject>Stand-up</t:Subject><t:Body BodyType=\"Text\">Daily</t:Body><t:Start>2026-10-05T09:00:00Z</t:Start>\
             <t:End>2026-10-05T09:15:00Z</t:End><t:IsAllDayEvent>false</t:IsAllDayEvent><t:Location>Room 2</t:Location><t:UID>standup-1</t:UID>\
             <t:Recurrence><t:WeeklyRecurrence><t:Interval>1</t:Interval><t:DaysOfWeek>Monday Wednesday Friday</t:DaysOfWeek></t:WeeklyRecurrence>\
             <t:NoEndRecurrence><t:StartDate>2026-10-05Z</t:StartDate></t:NoEndRecurrence></t:Recurrence></t:CalendarItem>",
        );
        d.pim.push((
            "OLD".into(),
            cal.clone(),
            parse("<t:CalendarItem><t:Subject>Gone</t:Subject><t:Start>2026-10-01T09:00:00Z</t:Start></t:CalendarItem>").kids[0].clone(),
        ));
        d.pim.push(("E1".into(), cal.clone(), standup.kids[0].clone()));
        let server = serve(d);
        let user = format!("{DOMAIN}\\{USER}");
        assert_eq!(crate::ews::autodiscover("riley@contoso.example", &user, PASSWORD, Some(&server.url)).unwrap(), server.url);
        let c = crate::ews::Client::connect(&server.url, &user, PASSWORD, "riley@contoso.example").unwrap();
        let mut pim = Pim::default();
        pim.events.push(StoredEvent {
            calendar: LOCAL.into(),
            event: CalEvent {
                uid: "local-1".into(),
                summary: "Poster review".into(),
                start: 1_792_000_000,
                end: 1_792_003_600,
                rrule: Some("FREQ=DAILY;COUNT=3".into()),
                ..Default::default()
            },
            dirty: true,
            ..Default::default()
        });
        pim.cards.push(StoredCard {
            book: LOCAL.into(),
            card: Card {
                uid: "p1".into(),
                name: "Ben Ito".into(),
                emails: vec![Typed { value: "ben@contoso.example".into(), kind: String::new() }],
                ..Default::default()
            },
            dirty: true,
            ..Default::default()
        });
        pim.deleted.push(Tombstone { href: "OLD".into(), etag: String::new(), ..Default::default() });
        let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
        assert!(sync.errors.is_empty(), "{:?}", sync.errors);
        assert_eq!(sync.deleted, ["OLD"]);
        let standup = sync.events.iter().find(|e| e.event.summary == "Stand-up").unwrap();
        assert_eq!(standup.event.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO,WE,FR"));
        assert_eq!(
            (standup.event.uid.as_str(), standup.event.location.as_str(), standup.event.end - standup.event.start),
            ("standup-1", "Room 2", 900)
        );
        let poster = sync.events.iter().find(|e| e.event.summary == "Poster review").unwrap();
        assert_eq!(poster.event.rrule.as_deref(), Some("FREQ=DAILY;COUNT=3"), "made on the server with its recurrence");
        assert!(sync.cards.iter().any(|k| k.card.name == "Ben Ito" && k.card.emails[0].value == "ben@contoso.example"));
        assert_eq!(sync.pushed_events.len(), 1);
        // A change goes field by field; the series keeps its id.
        let mut changed = pim.clone();
        changed.deleted.clear();
        changed.cards.clear();
        changed.events = vec![StoredEvent {
            href: standup.href.clone(),
            event: CalEvent { summary: "Stand-up (moved)".into(), rrule: None, ..standup.event.clone() },
            dirty: true,
            ..standup.clone()
        }];
        let sync = crate::ews_pim::sync(&c, &changed, 1_792_000_000).unwrap();
        let moved = sync.events.iter().find(|e| e.href == standup.href).unwrap();
        assert_eq!((moved.event.summary.as_str(), moved.event.rrule.as_deref()), ("Stand-up (moved)", None));
        // No Notes folder on this server: notes stay on this computer.
        assert!(sync.notes.is_none());
    }

    #[test]
    fn a_shared_mailbox_opens_under_the_account_and_keeps_its_mail() {
        let mut d = Data::default();
        let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        d.folder("Deleted Items", None, Some("deleteditems"), "IPF.Note");
        d.deliver(&inbox, &mail("Mine", "o1"), true, "2024-05-06T09:00:00Z");
        let support = "support@contoso.example";
        d.shared = vec![support.into()];
        let their_inbox = d.shared_folder(support, "Inbox", None, Some("inbox"));
        let their_trash = d.shared_folder(support, "Deleted Items", None, Some("deleteditems"));
        d.shared_folder(support, "Escalated", Some(&their_inbox), None);
        let ticket = d.deliver(&their_inbox, &mail("Printer on fire", "s1"), false, "2024-05-06T10:00:00Z");
        let server = serve(d);
        let data = server.data.clone();
        let dir = std::env::temp_dir().join(format!("mailcraft-ews-shared-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir).unwrap();
        let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
        let account = AccountConfig {
            id: "a1".into(),
            email: "riley@contoso.example".into(),
            username: format!("{DOMAIN}\\{USER}"),
            ews: Some(server.url.clone()),
            shared_mailboxes: vec![support.into(), "boss@contoso.example".into()],
            ..Default::default()
        };
        pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
        let seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
        let folders = seen.iter().rev().find_map(|e| match e {
            Event::Folders { folders, .. } => Some(folders.clone()),
            _ => None,
        });
        let paths: Vec<String> = folders.unwrap().iter().map(|f| f.path.clone()).collect();
        for want in
            ["INBOX", support, "support@contoso.example/Inbox", "support@contoso.example/Inbox/Escalated", "support@contoso.example/Deleted Items"]
        {
            assert!(paths.iter().any(|p| p == want), "{want} in {paths:?}");
        }
        assert!(!paths.iter().any(|p| p.starts_with("boss@")), "one it can't open is left out");
        eventually("their inbox", || store.load_folder("a1", "support@contoso.example/Inbox").is_ok_and(|f| f.messages.len() == 1));
        let key = store.load_folder("a1", "support@contoso.example/Inbox").unwrap().messages[0].key.clone();
        pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true } });
        eventually("read in their mailbox", || data.lock().unwrap().items.iter().any(|i| i.id == ticket && i.read));
        pool.submit(Job::Apply { account: "a1".into(), op: Op::Move { keys: vec![key], to: "support@contoso.example/Deleted Items".into() } });
        eventually("in their Deleted Items", || data.lock().unwrap().items.iter().any(|i| i.folder == their_trash));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn voting_buttons_come_in_and_a_vote_goes_back_as_exchange_counts_it() {
        let mut d = Data::default();
        let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        d.folder("Drafts", None, Some("drafts"), "IPF.Note");
        let sent = d.folder("Sent Items", None, Some("sentitems"), "IPF.Note");
        let lunch = d.deliver(&inbox, &mail("Team lunch on Friday?", "v1"), false, "2024-05-06T09:00:00Z");
        d.items.iter_mut().find(|i| i.id == lunch).unwrap().voting = mailcraft_model::voting::stream(&["Yes", "No", "Maybe"]);
        d.deliver(&inbox, &mail("No vote here", "v2"), false, "2024-05-06T08:00:00Z");
        let server = serve(d);
        let data = server.data.clone();
        let dir = std::env::temp_dir().join(format!("mailcraft-ews-vote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir).unwrap();
        let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
        let account = AccountConfig {
            id: "a1".into(),
            email: "riley@contoso.example".into(),
            username: format!("{DOMAIN}\\{USER}"),
            ews: Some(server.url.clone()),
            ..Default::default()
        };
        pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
        until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
        let st = store.load_folder("a1", "INBOX").unwrap();
        let m = st.messages.iter().find(|m| m.subject == "Team lunch on Friday?").unwrap();
        assert_eq!(m.voting, ["Yes", "No", "Maybe"]);
        assert!(st.messages.iter().find(|m| m.subject == "No vote here").unwrap().voting.is_empty());

        let key = m.key.clone();
        pool.submit(Job::Vote { key: key.clone(), choice: "Maybe".into() });
        let seen = until(&pool, "voted", |e| matches!(e, Event::Voted { .. }));
        assert!(matches!(seen.last(), Some(Event::Voted { error: None, choice, .. }) if choice == "Maybe"), "{seen:?}");
        {
            let d = data.lock().unwrap();
            assert_eq!(d.votes, [("Maybe: Team lunch on Friday?".to_string(), "Maybe".to_string())]);
            let reply = d.items.iter().find(|i| i.folder == sent).unwrap();
            let text = String::from_utf8_lossy(&reply.raw);
            assert!(text.contains("To: zoe@contoso.example") && text.contains("In-Reply-To: <v1@x>"), "a reply to the sender: {text}");
            assert!(!reply.draft);
        }
        // Remembered with the message.
        assert_eq!(store.load_folder("a1", "INBOX").unwrap().find(key.uid).unwrap().voted, "Maybe");

        // A message that's gone says so.
        let gone = mailcraft_model::MessageKey { uid: 9999, ..key };
        pool.submit(Job::Vote { key: gone, choice: "Yes".into() });
        let seen = until(&pool, "vote refused", |e| matches!(e, Event::Voted { .. }));
        assert!(matches!(seen.last(), Some(Event::Voted { error: Some(_), .. })));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sticky_notes_sync_with_the_notes_folder() {
        use mailcraft_model::pim::{Note, Pim, StoredNote};
        let mut d = Data::default();
        d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
        d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
        let notes = d.folder("Notes", None, Some("notes"), "IPF.StickyNote");
        let old = parse(
            "<t:Message><t:ItemClass>IPM.StickyNote</t:ItemClass><t:Subject>Call Ana</t:Subject><t:Body BodyType=\"Text\">Call Ana\r\nabout the mix</t:Body>\
             <t:ExtendedProperty><t:ExtendedFieldURI PropertySetId=\"0006200E-0000-0000-C000-000000000046\" PropertyId=\"35584\" PropertyType=\"Integer\"/><t:Value>2</t:Value></t:ExtendedProperty></t:Message>",
        );
        d.pim.push(("N1".into(), notes, old.kids[0].clone()));
        let server = serve(d);
        let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
        let mut pim = Pim::default();
        pim.notes.push(StoredNote {
            note: Note { uid: "n-local".into(), text: "Buy paper\n170 gsm".into(), color: 1, modified: 5 },
            dirty: true,
            ..Default::default()
        });
        let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
        assert!(sync.errors.is_empty(), "{:?}", sync.errors);
        let list = sync.notes.unwrap();
        let ana = list.iter().find(|n| n.href == "note:N1").unwrap();
        assert_eq!((ana.note.text.as_str(), ana.note.color), ("Call Ana\nabout the mix", 2));
        assert!(list.iter().any(|n| n.note.text == "Buy paper\n170 gsm" && n.note.color == 1), "made in the Notes folder, its colour kept");
        assert_eq!(sync.pushed_notes.len(), 1);
        assert!(sync.pushed_notes[0].href.starts_with("note:"));
    }

    #[test]
    fn someone_elses_calendar_opens_read_only() {
        use mailcraft_model::pim::{Pim, shared_calendar_id};
        let mut d = Data::default();
        d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
        d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
        let review = parse(
            "<t:CalendarItem><t:Subject>Budget review</t:Subject><t:Start>2026-10-06T13:00:00Z</t:Start><t:End>2026-10-06T14:00:00Z</t:End>\
             <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>budget-1</t:UID></t:CalendarItem>",
        );
        d.pim.push(("S1".into(), "mailbox:ana@contoso.example".into(), review.kids[0].clone()));
        let server = serve(d);
        let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
        let pim = Pim { shared: vec!["ana@contoso.example".into(), "nobody@contoso.example".into()], ..Default::default() };
        let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
        let id = shared_calendar_id("ana@contoso.example");
        let cals = sync.calendars.clone().unwrap();
        assert!(cals.iter().any(|x| x.id == id && x.read_only && x.name == "ana@contoso.example"));
        assert!(sync.events.iter().any(|e| e.calendar == id && e.event.summary == "Budget review"));
        assert!(sync.synced_calendars.contains(&id));
        // One that can't be read says so, and the rest still syncs.
        assert!(sync.errors.iter().any(|e| e.starts_with("nobody@contoso.example's calendar")), "{:?}", sync.errors);
        assert!(!sync.synced_calendars.contains(&shared_calendar_id("nobody@contoso.example")));
    }

    /// Automatic replies answer at once while the mailbox takes minutes to sync: they don't wait
    /// behind it (File › Info said "Checking with the server…" all the while).
    #[test]
    fn automatic_replies_answer_while_the_mailbox_syncs() {
        let mut d = Data::default();
        let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        d.deliver(&inbox, &mail("Storyboard", "m1"), true, "2024-05-06T09:00:00Z");
        d.slow_sync = Some(Duration::from_secs(30));
        let server = serve(d);
        let dir = std::env::temp_dir().join(format!("mailcraft-ews-oof-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut pool = crate::worker::Pool::new(Store::open(&dir).unwrap(), Arc::new(crate::worker::MemoryCredentials::default()));
        let account = AccountConfig {
            id: "a1".into(),
            email: "riley@contoso.example".into(),
            username: format!("{DOMAIN}\\{USER}"),
            ews: Some(server.url.clone()),
            ..Default::default()
        };
        pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
        until(&pool, "syncing", |e| matches!(e, Event::Folders { .. }));
        let asked = Instant::now();
        pool.submit(Job::GetVacation { account: "a1".into() });
        until(&pool, "replies read", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if !v.enabled));
        assert!(asked.elapsed() < Duration::from_secs(10), "waited {:?}", asked.elapsed());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An Outlook navigation shortcut to someone's calendar, in a group of the Calendar pane.
    fn wlink(class: &str, section: i64, kind: i64, group: &str, name: &str, dn: &str) -> String {
        let mut eid = vec![0u8; 4];
        eid.extend([0xDC, 0xA7, 0x40, 0xC8, 0xC0, 0x42, 0x10, 0x1A, 0xB4, 0xB9, 0x08, 0x00, 0x2B, 0x2F, 0xE1, 0x82]);
        eid.extend([1, 0, 0, 0, 0, 0, 0, 0]);
        eid.extend(dn.as_bytes());
        eid.push(0);
        let p = |tag: &str, kind: &str, v: &str| {
            format!(
                "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"{tag}\" PropertyType=\"{kind}\"/><t:Value>{v}</t:Value></t:ExtendedProperty>"
            )
        };
        format!(
            "<t:Message><t:ItemId Id=\"W{name}\"/>{}{}{}{}{}{}</t:Message>",
            p("0x1a", "String", class),
            p("0xe1d", "String", name),
            p("0x6849", "Integer", &kind.to_string()),
            p("0x6851", "String", group),
            p("0x6852", "Integer", &section.to_string()),
            p("0x6854", "Binary", &crate::imap::base64(&eid)),
        )
    }

    /// The calendars Outlook keeps in groups of its Calendar pane come in under those groups,
    /// read as other people's calendars; shortcuts to mail folders, to this account's own
    /// calendar, or to names the directory doesn't know are left alone.
    #[test]
    fn outlook_calendar_groups_come_in_with_their_calendars() {
        use mailcraft_model::pim::{Pim, shared_calendar_id};
        let mut d = Data::default();
        d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
        d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
        let standup = parse(
            "<t:CalendarItem><t:Subject>Stand-up</t:Subject><t:Start>2026-10-06T08:00:00Z</t:Start><t:End>2026-10-06T08:15:00Z</t:End>\
             <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>standup-1</t:UID></t:CalendarItem>",
        );
        d.pim.push(("S1".into(), "mailbox:ana@contoso.example".into(), standup.kids[0].clone()));
        d.pim.push(("S2".into(), "mailbox:room.lyon@contoso.example".into(), standup.kids[0].clone()));
        let dn = |who: &str| format!("/o=Contoso/ou=Exchange Administrative Group (FYDIBOHF23SPDLT)/cn=Recipients/cn={who}");
        d.directory = vec![
            (dn("ana"), "Ana@Contoso.example".into()),
            (dn("lyon"), "room.lyon@contoso.example".into()),
            (dn("riley"), "riley@contoso.example".into()),
        ];
        let link = "IPM.Microsoft.WunderBar.Link";
        d.wlinks = vec![
            wlink(link, 3, 2, "Équipe : Studio", "Ana Lima", &dn("ana")),
            wlink(link, 3, 2, "Salles", "Salle Lyon", &dn("lyon")),
            // Twice, in another group: one calendar.
            wlink(link, 3, 2, "Calendriers partagés", "Ana", &dn("ana")),
            // This account's own calendar, a mail folder shortcut, a group header, someone unknown.
            wlink(link, 3, 2, "Mes calendriers", "Riley", &dn("riley")),
            wlink(link, 1, 2, "Favorites", "Ana's Inbox", &dn("ana")),
            wlink(link, 3, 4, "Salles", "", ""),
            wlink(link, 3, 2, "Salles", "Ghost", &dn("ghost")),
        ];
        let server = serve(d);
        let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
        let links = crate::ews_pim::calendar_links(&c).unwrap();
        assert_eq!(
            links.iter().map(|l| (l.email.as_str(), l.name.as_str(), l.group.as_str())).collect::<Vec<_>>(),
            [("ana@contoso.example", "Ana Lima", "Équipe : Studio"), ("room.lyon@contoso.example", "Salle Lyon", "Salles")]
        );
        // Opened here by address as well: once, under its group.
        let pim = Pim { shared: vec!["ana@contoso.example".into()], ..Default::default() };
        let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
        let cals = sync.calendars.clone().unwrap();
        let ana = shared_calendar_id("ana@contoso.example");
        let room = shared_calendar_id("room.lyon@contoso.example");
        assert_eq!(cals.iter().filter(|x| x.id == ana).count(), 1);
        assert!(cals.iter().any(|x| x.id == room && x.name == "Salle Lyon" && x.group == "Salles" && x.read_only), "{cals:?}");
        assert!(sync.events.iter().any(|e| e.calendar == room && e.event.summary == "Stand-up"));
        assert!(sync.synced_calendars.contains(&room));
    }

    /// Someone who made this account an editor of their calendar: events go in, change and go
    /// there; someone who didn't keeps theirs read-only.
    #[test]
    fn someone_elses_calendar_can_be_changed_by_an_editor() {
        use mailcraft_model::ical::CalEvent;
        use mailcraft_model::pim::{Pim, StoredEvent, Tombstone, shared_calendar_id};
        let mut d = Data { editors: vec!["ana@contoso.example".into()], ..Default::default() };
        d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
        d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
        let item = |subject: &str, uid: &str| {
            parse(&format!(
                "<t:CalendarItem><t:Subject>{subject}</t:Subject><t:Start>2026-10-06T13:00:00Z</t:Start><t:End>2026-10-06T14:00:00Z</t:End>\
                 <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>{uid}</t:UID></t:CalendarItem>"
            ))
            .kids[0]
                .clone()
        };
        d.pim.push(("S1".into(), "mailbox:ana@contoso.example".into(), item("Budget review", "budget-1")));
        d.pim.push(("S2".into(), "mailbox:ana@contoso.example".into(), item("Team lunch", "lunch-1")));
        d.pim.push(("B1".into(), "mailbox:ben@contoso.example".into(), item("Ben's call", "call-1")));
        let server = serve(d);
        let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
        let (ana, ben) = (shared_calendar_id("ana@contoso.example"), shared_calendar_id("ben@contoso.example"));
        let mut pim = Pim { shared: vec!["ana@contoso.example".into(), "ben@contoso.example".into()], ..Default::default() };
        let first = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
        let cals = first.calendars.clone().unwrap();
        let ro = |id: &str| cals.iter().find(|x| x.id == id).map(|x| x.read_only);
        assert_eq!((ro(&ana), ro(&ben)), (Some(false), Some(true)));
        let mut review = first.events.iter().find(|e| e.event.summary == "Budget review").cloned().unwrap();
        let lunch = first.events.iter().find(|e| e.event.summary == "Team lunch").cloned().unwrap();
        review.event.summary = "Budget review (moved)".into();
        review.dirty = true;
        pim.events = vec![
            review,
            StoredEvent {
                calendar: ana.clone(),
                event: CalEvent {
                    uid: "new-1".into(),
                    summary: "Poster check".into(),
                    start: 1_792_000_000,
                    end: 1_792_003_600,
                    ..Default::default()
                },
                dirty: true,
                ..Default::default()
            },
        ];
        pim.deleted = vec![Tombstone { href: lunch.href.clone(), etag: String::new(), calendar: ana.clone() }];
        let second = crate::ews_pim::sync(&c, &pim, 1_792_000_100).unwrap();
        assert!(second.errors.is_empty(), "{:?}", second.errors);
        assert!(second.pushed_events.iter().all(|e| e.calendar == ana), "they stay in Ana's calendar");
        let d = server.data.lock().unwrap();
        let in_ana: Vec<String> = d
            .pim
            .iter()
            .filter(|(_, f, _)| f == "mailbox:ana@contoso.example")
            .filter_map(|(_, _, el)| el.text_of("Subject").map(str::to_string))
            .collect();
        assert!(in_ana.contains(&"Budget review (moved)".to_string()) && in_ana.contains(&"Poster check".to_string()), "{in_ana:?}");
        assert!(!in_ana.contains(&"Team lunch".to_string()), "deleted");
        assert!(!d.pim.iter().any(|(_, f, el)| !f.starts_with("mailbox:") && el.text_of("Subject") == Some("Poster check")), "not in this mailbox");
    }

    #[test]
    fn public_folders_list_under_their_own_folder_and_sync() {
        let mut d = Data { pf_mailbox: Some("PFContent@contoso.example".into()), ..Default::default() };
        d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
        d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
        let root = d.folder("IPM_SUBTREE", None, Some("publicfoldersroot"), "IPF.Note");
        let team = d.folder("Team", Some(&root), None, "IPF.Note");
        let news = d.folder("Announcements", Some(&team), None, "IPF.Note");
        let rooms = d.folder("Rooms", Some(&root), None, "IPF.Appointment");
        let shared = d.folder("Company contacts", Some(&team), None, "IPF.Contact");
        d.pim.push((
            "PC1".into(),
            shared,
            parse("<t:Contact><t:ItemId Id=\"PC1\" ChangeKey=\"k\"/><t:DisplayName>Front desk</t:DisplayName><t:EmailAddresses><t:Entry Key=\"EmailAddress1\">desk@contoso.example</t:Entry></t:EmailAddresses></t:Contact>")
                .kids[0]
                .clone(),
        ));
        d.pim.push((
            "PE1".into(),
            rooms,
            parse("<t:CalendarItem><t:Subject>Room 2 booked</t:Subject><t:Start>2026-10-16T09:00:00Z</t:Start><t:End>2026-10-16T10:00:00Z</t:End><t:UID>room-1</t:UID></t:CalendarItem>")
                .kids[0]
                .clone(),
        ));
        d.deliver(
            &news,
            b"From: hr@contoso.example\r\nSubject: Holiday hours\r\nMessage-ID: <h@x>\r\n\r\nClosed Friday.\r\n",
            false,
            "2026-10-14T09:00:00Z",
        );
        let server = serve(d);
        let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
        let boxes = c.mailboxes().unwrap();
        let names: Vec<&str> = boxes.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["Inbox", "Public Folders", "Team", "Announcements"], "the room calendar isn't mail");
        let pf = boxes.iter().find(|m| m.name == "Public Folders").unwrap();
        let team_box = boxes.iter().find(|m| m.name == "Team").unwrap();
        assert_eq!((pf.parent.as_deref(), team_box.parent.as_deref()), (None, Some(pf.id.as_str())));
        assert_eq!(boxes.iter().find(|m| m.name == "Announcements").unwrap().parent.as_deref(), Some(team_box.id.as_str()));
        // Their mail syncs as any folder's.
        let store = crate::fake::tests_store("public-folders");
        let mut st = crate::jmap::State::default();
        let folders = crate::ews::list_folders(&c, &mut st, "a1", &[]).unwrap();
        let news_path = folders.iter().find(|f| f.path.ends_with("Announcements")).map(|f| f.path.clone()).unwrap();
        crate::ews::sync_folder(&c, &mut st, &store, "a1", &news_path, &crate::sync::SyncOptions::default()).unwrap();
        let fs = store.load_folder("a1", &news_path).unwrap();
        assert_eq!(fs.messages.len(), 1);
        assert_eq!(fs.messages[0].subject, "Holiday hours");
        let _ = std::fs::remove_dir_all(store.root());
        // Issue #1: the organisation's shared contacts and calendars come in, read-only.
        let sync = crate::ews_pim::sync(&c, &mailcraft_model::pim::Pim::default(), 0).unwrap();
        let books = sync.books.clone().unwrap_or_default();
        let book = books.iter().find(|b| b.id.starts_with("public:")).unwrap();
        assert_eq!((book.name.as_str(), book.read_only), ("Team › Company contacts", true));
        assert!(sync.cards.iter().any(|k| k.book == book.id && k.card.display() == "Front desk"));
        let cals = sync.calendars.clone().unwrap_or_default();
        let room = cals.iter().find(|c| c.id.starts_with("public:")).unwrap();
        assert_eq!((room.name.as_str(), room.read_only), ("Rooms", true));
        assert!(sync.events.iter().any(|e| e.calendar == room.id && e.event.summary == "Room 2 booked"));
        // A server without public folders lists the mailbox's alone.
        let mut d = Data::default();
        d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        let server = serve(d);
        let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
        assert_eq!(c.mailboxes().unwrap().len(), 1);
    }

    #[test]
    fn basic_sign_in_when_that_is_what_the_server_offers() {
        let mut d = Data { basic: true, ..Default::default() };
        d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        let server = serve(d);
        let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
        assert_eq!(c.mailboxes().unwrap().len(), 1);
        assert!(matches!(crate::ews::Client::connect(&server.url, "x", "y", "e"), Err(crate::NetError::Auth(_))));
    }
}
