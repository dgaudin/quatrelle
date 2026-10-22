//! An in-memory Exchange Web Services server on localhost, for tests: folders (distinguished
//! ids, a deep tree), messages (find, get with their MIME source, update, move with a new id,
//! delete), creating from MIME (drafts and sending), folder changes and automatic replies, signed
//! in with NTLM on a kept-open connection (or Basic, when set to).

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
    /// The other mailbox it belongs to (a shared one), or empty for the account's own.
    pub owner: String,
}

/// Another mailbox on the server (a shared one) and what the account may do with it.
#[derive(Clone, Debug)]
pub struct SharedBox {
    pub address: String,
    pub name: String,
    /// Autodiscover maps it to the account (full access with automapping), as Outlook opens it.
    pub automap: bool,
    /// Autodiscover names it by its Exchange address only (older servers): ResolveNames finds it.
    pub legacy: bool,
    /// The account has access to it.
    pub granted: bool,
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
    /// The next request on a signed-in connection is told to sign in again (a recycled
    /// connection), once.
    pub forget_sign_in: bool,
    /// The next request on a connection already used is met with the connection closed, unread
    /// (a connection the server closed for being idle), this many times.
    pub drop_kept: usize,
    /// The next requests are answered "busy" (`ErrorServerBusy`, back off 10 ms), this many times.
    pub busy: usize,
    /// The EWS address, for Autodiscover's answer.
    pub url: String,
    /// People whose calendars this account may change (delegates with editor rights).
    pub editors: Vec<String>,
    /// The folder's rights say nothing of that (as some servers answer for another mailbox's
    /// calendar): only each item's rights do.
    pub item_rights_only: bool,
    /// Listing a mail folder's messages takes this long (a big mailbox's first sync).
    pub hold: Option<std::time::Duration>,
    /// Ids of folders listed with `PidTagAttributeHidden` true.
    pub flagged_hidden: Vec<String>,
    /// Messages whose source can't be had (the server fails on it).
    pub unreadable: Vec<String>,
    /// The public folder mailbox holding the public folders' content (Exchange 2013 and
    /// later): reading inside them is only answered when routed there.
    pub pf_mailbox: Option<String>,
    /// The public folder hierarchy mailbox (Exchange 2013 and later): listing public folders is
    /// only answered when routed there.
    pub pf_hierarchy: Option<String>,
    /// The Inbox rules Exchange keeps, as (id, name).
    pub rules: Vec<(String, String)>,
    /// The request's `X-PublicFolderMailbox` header.
    routed: String,
    /// Other mailboxes on the server.
    pub shared: Vec<SharedBox>,
    /// The `X-AnchorMailbox` header of each request about a shared mailbox's folders.
    pub anchors: Vec<String>,
    /// Outlook's navigation shortcuts, as items of the Common Views folder (hidden ones).
    pub wlinks: Vec<String>,
    /// The directory: (X.500 name, address).
    pub directory: Vec<(String, String)>,
    /// Answer item searches (the mail sync) only after this long: a mailbox taking minutes.
    pub slow_sync: Option<std::time::Duration>,
    /// Folders Exchange keeps hidden (PidTagAttributeHidden), by id.
    pub hidden: Vec<String>,
    next: u32,
}

/// A mailbox's Exchange address (its LegacyDN), as older servers name it.
/// Whether a field URI names its property on the type it's declared on, as Exchange checks it: an
/// `item:` property is on `ItemType`, a `calendar:` one on `CalendarItemType`. One on the wrong
/// type is refused for the whole request ("The request is invalid."), which is what a reminder
/// sent as `calendar:ReminderIsSet` got. What the lists don't know is left alone.
fn field_uri_is_known(uri: &str) -> bool {
    const ITEM: [&str; 8] = ["Subject", "Body", "Categories", "Sensitivity", "ReminderIsSet", "ReminderMinutesBeforeStart", "Importance", "IsDraft"];
    const CALENDAR: [&str; 10] = [
        "Start",
        "End",
        "IsAllDayEvent",
        "Location",
        "Recurrence",
        "UID",
        "RequiredAttendees",
        "OptionalAttendees",
        "LegacyFreeBusyStatus",
        "TimeZone",
    ];
    let Some((kind, name)) = uri.split_once(':') else { return false };
    match kind {
        "item" => !CALENDAR.contains(&name),
        "calendar" => !ITEM.contains(&name),
        _ => true,
    }
}

fn legacy_dn(address: &str) -> String {
    format!("/o=Contoso/ou=Exchange Administrative Group/cn=Recipients/cn={}", address.split('@').next().unwrap_or(address))
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
            forget_sign_in: false,
            drop_kept: 0,
            busy: 0,
            url: String::new(),
            editors: Vec::new(),
            item_rights_only: false,
            hold: None,
            flagged_hidden: Vec::new(),
            unreadable: Vec::new(),
            pf_mailbox: None,
            pf_hierarchy: None,
            rules: Vec::new(),
            routed: String::new(),
            shared: Vec::new(),
            anchors: Vec::new(),
            wlinks: Vec::new(),
            directory: Vec::new(),
            slow_sync: None,
            hidden: Vec::new(),
            next: 0,
        };
        d.folders.push(Fo {
            id: "ROOT".into(),
            name: "Top of Information Store".into(),
            parent: String::new(),
            class: "IPF.Note".into(),
            known: Some("msgfolderroot".into()),
            owner: String::new(),
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
            owner: String::new(),
        });
        id
    }

    /// Another mailbox on the server with its root, Inbox and Outbox: its Inbox's id.
    pub fn shared_mailbox(&mut self, b: SharedBox) -> String {
        let owner = b.address.to_ascii_lowercase();
        let make = |d: &mut Data, name: &str, parent: &str, known: Option<&str>| {
            let id = d.id("S");
            d.folders.push(Fo {
                id: id.clone(),
                name: name.into(),
                parent: parent.into(),
                class: "IPF.Note".into(),
                known: known.map(str::to_string),
                owner: owner.clone(),
            });
            id
        };
        let root = make(self, "Top of Information Store", "", Some("msgfolderroot"));
        let inbox = make(self, "Inbox", &root, Some("inbox"));
        make(self, "Outbox", &root, Some("outbox"));
        self.shared.push(b);
        inbox
    }

    /// A folder in a shared mailbox, under `parent`.
    pub fn shared_folder(&mut self, owner: &str, name: &str, parent: &str) -> String {
        let id = self.id("S");
        self.folders.push(Fo {
            id: id.clone(),
            name: name.into(),
            parent: parent.into(),
            class: "IPF.Note".into(),
            known: None,
            owner: owner.to_ascii_lowercase(),
        });
        id
    }

    /// The shared mailbox an element names (`<t:Mailbox>`), if it's one; `Err` when the account
    /// has no access to it.
    fn shared_of(&self, el: &El) -> Option<Result<String, ()>> {
        let who = el.find("Mailbox").and_then(|m| m.text_of("EmailAddress"))?.to_ascii_lowercase();
        let b = self.shared.iter().find(|b| b.address.eq_ignore_ascii_case(&who))?;
        Some(if b.granted { Ok(who) } else { Err(()) })
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

    pub fn by_name(&self, name: &str) -> Option<&Fo> {
        self.folders.iter().find(|f| f.name == name)
    }

    fn folder_id(&self, el: &El) -> Option<String> {
        match el.name.as_str() {
            "FolderId" => el.attr("Id").map(str::to_string),
            // Another mailbox's folder: its items are kept as `mailbox:<address>`.
            "DistinguishedFolderId" if el.child("Mailbox").is_some() && self.shared_of(el).is_some() => {
                let owner = self.shared_of(el)?.ok()?;
                let known = el.attr("Id")?;
                self.folders.iter().find(|f| f.owner == owner && f.known.as_deref() == Some(known)).map(|f| f.id.clone())
            }
            "DistinguishedFolderId" if el.child("Mailbox").is_some() => {
                let who = el.find("EmailAddress").map(|e| e.text.to_ascii_lowercase())?;
                let id = format!("mailbox:{who}");
                self.pim.iter().any(|(_, f, _)| *f == id).then_some(id)
            }
            "DistinguishedFolderId" => {
                let known = el.attr("Id")?;
                self.folders.iter().find(|f| f.owner.is_empty() && f.known.as_deref() == Some(known)).map(|f| f.id.clone())
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
        // Hidden folders (0x10f4, PidTagAttributeHidden), as either list marks them.
        let replica = if self.flagged_hidden.contains(&f.id) || self.hidden.contains(&f.id) {
            format!(
                "{replica}<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x10f4\" PropertyType=\"Boolean\"/><t:Value>true</t:Value></t:ExtendedProperty>"
            )
        } else {
            replica
        };
        // As Exchange answers: each folder in an element of its kind (EWS's folder array).
        let kind = match f.class.as_str() {
            c if c.starts_with("IPF.Contact") => "ContactsFolder",
            c if c.starts_with("IPF.Appointment") => "CalendarFolder",
            c if c.starts_with("IPF.Task") => "TasksFolder",
            _ => "Folder",
        };
        format!(
            "<t:{kind}><t:FolderId Id=\"{}\"/><t:ParentFolderId Id=\"{}\"/><t:FolderClass>{}</t:FolderClass><t:DisplayName>{}</t:DisplayName><t:TotalCount>{total}</t:TotalCount><t:ChildFolderCount>0</t:ChildFolderCount><t:UnreadCount>{unread}</t:UnreadCount>{replica}</t:{kind}>",
            f.id,
            f.parent,
            f.class,
            esc(&f.name)
        )
    }

    /// Whether listing under this folder goes unanswered: the public folders, asked without
    /// being routed to the hierarchy mailbox.
    fn unrouted_hierarchy(&self, folder: &str) -> bool {
        self.pf_hierarchy.as_ref().is_some_and(|m| *m != self.routed) && self.public_ids().iter().any(|p| p == folder)
    }

    /// Whether a request for this folder's content goes unanswered: a public folder, asked
    /// without being routed to the mailbox holding it.
    fn unrouted(&self, folder: &str) -> bool {
        self.pf_mailbox.as_ref().is_some_and(|m| *m != self.routed) && self.public_ids().iter().any(|p| p == folder)
    }

    fn item_xml(&self, it: &It, mime_content: bool) -> String {
        let p = quatrelle_mime::parse(&it.raw).unwrap_or_default();
        let mb = |a: &quatrelle_model::EmailAddress| {
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
}

/// An element as XML again (for items kept as they were created).
fn xml(e: &El) -> String {
    let attrs: String = e.attrs.iter().map(|(k, v)| format!(" {k}=\"{}\"", esc(v))).collect();
    let inner: String = e.kids.iter().map(xml).collect();
    format!("<t:{0}{attrs}>{1}{inner}</t:{0}>", e.name, esc(&e.text))
}

mod route;
mod server;

pub use server::serve;

#[cfg(test)]
mod tests;
