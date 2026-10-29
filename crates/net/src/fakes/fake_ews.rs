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
    /// Its categories, by name (`item:Categories`).
    pub categories: Vec<String>,
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
    /// Folders listed without their last change (as a server that doesn't keep it).
    pub no_commit_time: bool,
    /// The mailbox's limits in kilobytes (warning, sending, receiving), on its root folder.
    pub limits: Option<(u64, u64, u64)>,
    /// Outlook keeps its own copy of the rules: every change is refused until one removes it.
    pub outlook_rules: bool,
    /// Changes a `SyncFolderItems` answers at most (fewer than asked: to see the pages).
    pub sync_page: Option<usize>,
    /// The next request on a connection already used is met with the connection closed, unread
    /// (a connection the server closed for being idle), this many times.
    pub drop_kept: usize,
    /// The next requests are answered "busy" (`ErrorServerBusy`, back off 10 ms), this many times.
    pub busy: usize,
    /// The EWS address, for Autodiscover's answer.
    pub url: String,
    /// People whose calendars this account may change (delegates with editor rights).
    pub editors: Vec<String>,
    /// Addresses this account may send from (Send As or Send on Behalf on them).
    pub send_as: Vec<String>,
    /// The address each message sent said it was from (`t:From`; empty: the account's own).
    pub sent_from: Vec<String>,
    /// The folder's rights say nothing of that (as some servers answer for another mailbox's
    /// calendar): only each item's rights do.
    pub item_rights_only: bool,
    /// Listing a mail folder's messages takes this long (a big mailbox's first sync).
    pub hold: Option<std::time::Duration>,
    /// Ids of folders listed with `PidTagAttributeHidden` true.
    pub flagged_hidden: Vec<String>,
    /// Messages whose source can't be had (the server fails on it).
    pub unreadable: Vec<String>,
    /// Appointments `GetItem` refuses (`ErrorAccessDenied`), by id: a colleague's occurrences
    /// Exchange wouldn't give, though their series came.
    pub refused: Vec<String>,
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
    /// The Global Address List: (name, address), found by the start of a name or an address.
    pub people: Vec<(String, String)>,
    /// The `User-Agent` of each request.
    pub agents: Vec<String>,
    /// Refuse (HTTP 403) every request naming another `User-Agent`: a server that lets in only
    /// the clients it knows.
    pub only_agent: Option<String>,
    /// Answer item searches (the mail sync) only after this long: a mailbox taking minutes.
    pub slow_sync: Option<std::time::Duration>,
    /// Give messages' whole sources only after this long (a big attachment on a slow line).
    pub slow_sources: Option<std::time::Duration>,
    /// Answer `GetFolder` (a folder's counts, the first step of reading it) only after this long.
    pub slow_counts: Option<std::time::Duration>,
    /// Folders Exchange keeps hidden (PidTagAttributeHidden), by id.
    pub hidden: Vec<String>,
    /// How each appointment created or changed asked the guests to be told
    /// (`SendMeetingInvitations`, `SendMeetingInvitationsOrCancellations`), in order.
    pub meeting_sends: Vec<String>,
    /// Meetings answered (Outlook's buttons): the item, how (`AcceptItem`…), and whether the
    /// organizer was sent the answer.
    pub answers: Vec<(String, String, bool)>,
    /// The room lists the organisation set up (name, address) — none on most on-premises
    /// servers — and the rooms (name, address, the list's address).
    pub room_lists: Vec<(String, String)>,
    pub rooms: Vec<(String, String, String)>,
    /// `GetRoomLists` isn't available on this server (refused as an invalid operation).
    pub rooms_refused: bool,
    /// Mailboxes whose free/busy this account may not read (`ErrorNoFreeBusyAccess`).
    pub no_freebusy: Vec<String>,
    /// Busy stretches besides the calendars' items: (address, start, end, `BusyType`, subject).
    pub taken: Vec<(String, String, String, String, String)>,
    /// The `GetUserAvailability` requests seen: the addresses asked about and the window.
    pub availability_asked: Vec<(Vec<String>, String, String)>,
    /// The server tells of changes as they happen (streaming notifications); without it
    /// `Subscribe` is refused, as by a server older than Exchange 2010 SP1.
    pub streaming: bool,
    /// The subscriptions it has: their ids and the folders each is for (`None`: all of the
    /// account's own). One taken away is forgotten, as by a server restarted.
    pub subscriptions: Vec<(String, Option<Vec<String>>)>,
    /// How long one answer of notifications stays open.
    pub stream_for: std::time::Duration,
    /// The `Cookie` each answer of notifications was asked with.
    pub stream_cookies: Vec<String>,
    /// Outlook's master category list (the `CategoryList` configuration's XML), when it made one.
    pub category_list: Option<String>,
    /// What each subscription last told of: what changed since is told when it's next asked,
    /// as a subscription keeps its notifications between two answers.
    told: std::collections::HashMap<String, streaming::Seen>,
    next: u32,
}

/// An item's categories as Exchange lists them (nothing for none).
pub(super) fn categories_xml(names: &[String]) -> String {
    if names.is_empty() {
        return String::new();
    }
    format!("<t:Categories>{}</t:Categories>", names.iter().map(|n| format!("<t:String>{}</t:String>", esc(n))).collect::<String>())
}

/// A mailbox's Exchange address (its LegacyDN), as older servers name it.
/// Whether a field URI names its property on the type it's declared on, as Exchange checks it: an
/// `item:` property is on `ItemType`, a `calendar:` one on `CalendarItemType`. One on the wrong
/// type is refused for the whole request ("The request is invalid."), which is what a reminder
/// sent as `calendar:ReminderIsSet` got. What the lists don't know is left alone.
fn field_uri_is_known(uri: &str) -> bool {
    const ITEM: [&str; 8] = ["Subject", "Body", "Categories", "Sensitivity", "ReminderIsSet", "ReminderMinutesBeforeStart", "Importance", "IsDraft"];
    const CALENDAR: [&str; 11] = [
        "Start",
        "End",
        "IsAllDayEvent",
        "Location",
        "Recurrence",
        "UID",
        "RequiredAttendees",
        "OptionalAttendees",
        "Resources",
        "LegacyFreeBusyStatus",
        "TimeZone",
    ];
    const TASK: [&str; 5] = ["DueDate", "StartDate", "PercentComplete", "Status", "CompleteDate"];
    let Some((kind, name)) = uri.split_once(':') else { return false };
    match kind {
        "item" => !CALENDAR.contains(&name) && !TASK.contains(&name),
        "calendar" => !ITEM.contains(&name),
        "task" => TASK.contains(&name),
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
            no_commit_time: false,
            limits: None,
            outlook_rules: false,
            sync_page: None,
            busy: 0,
            url: String::new(),
            editors: Vec::new(),
            send_as: Vec::new(),
            sent_from: Vec::new(),
            item_rights_only: false,
            hold: None,
            flagged_hidden: Vec::new(),
            unreadable: Vec::new(),
            refused: Vec::new(),
            pf_mailbox: None,
            pf_hierarchy: None,
            rules: Vec::new(),
            routed: String::new(),
            shared: Vec::new(),
            anchors: Vec::new(),
            wlinks: Vec::new(),
            directory: Vec::new(),
            people: Vec::new(),
            agents: Vec::new(),
            only_agent: None,
            slow_sync: None,
            slow_sources: None,
            slow_counts: None,
            hidden: Vec::new(),
            meeting_sends: Vec::new(),
            answers: Vec::new(),
            room_lists: Vec::new(),
            rooms: Vec::new(),
            rooms_refused: false,
            no_freebusy: Vec::new(),
            taken: Vec::new(),
            availability_asked: Vec::new(),
            streaming: false,
            subscriptions: Vec::new(),
            stream_for: std::time::Duration::from_secs(2),
            stream_cookies: Vec::new(),
            category_list: None,
            told: std::collections::HashMap::new(),
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
            categories: Vec::new(),
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
        // When anything in it last changed (PR_LOCAL_COMMIT_TIME_MAX): here a digest of what it
        // holds, which changes as Exchange's time does (unless told to leave it out).
        let replica = if self.no_commit_time {
            replica
        } else {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for i in self.items.iter().filter(|i| i.folder == f.id) {
                (&i.id, i.read, i.flagged, i.draft, &i.categories).hash(&mut h);
            }
            format!(
                "{replica}<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x670a\" PropertyType=\"SystemTime\"/><t:Value>2026-01-01T00:00:00.{:x}Z</t:Value></t:ExtendedProperty>",
                h.finish()
            )
        };
        // The bytes its items take (PidTagMessageSizeExtended), and on the root the mailbox's
        // limits (PidTagStorageQuotaLimit, PidTagProhibitSendQuota, PidTagProhibitReceiveQuota).
        let bytes: usize = self.items.iter().filter(|i| i.folder == f.id).map(|i| i.raw.len()).sum();
        let prop = |tag: &str, kind: &str, v: u64| {
            format!(
                "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"{tag}\" PropertyType=\"{kind}\"/><t:Value>{v}</t:Value></t:ExtendedProperty>"
            )
        };
        let mut replica = format!("{replica}{}", prop("0xe08", "Long", bytes as u64));
        if let (Some((warning, send, receive)), Some("msgfolderroot")) = (self.limits, f.known.as_deref()) {
            replica.push_str(&format!(
                "{}{}{}",
                prop("0x3ff5", "Integer", warning),
                prop("0x666e", "Integer", send),
                prop("0x666a", "Integer", receive)
            ));
        }
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
        x.push_str(&format!(
            "<t:InternetMessageId>&lt;{}&gt;</t:InternetMessageId><t:IsRead>{}</t:IsRead>{}</t:Message>",
            esc(&p.message_id),
            it.read,
            categories_xml(&it.categories)
        ));
        x
    }

    /// `GetUserConfiguration`'s answer: the master category list when Outlook made one.
    pub(super) fn user_configuration(&self, op: &El) -> String {
        let name = op.find("UserConfigurationName").and_then(|n| n.attr("Name")).unwrap_or_default();
        match self.category_list.as_ref().filter(|_| name == "CategoryList") {
            Some(xml) => format!(
                "<m:R ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode><m:UserConfiguration>\
                 <t:UserConfigurationName Name=\"CategoryList\"/><t:XmlData>{}</t:XmlData></m:UserConfiguration></m:R>",
                crate::imap::base64(xml.as_bytes())
            ),
            None => "<m:R ResponseClass=\"Error\"><m:MessageText>The specified object was not found in the store.</m:MessageText>\
                     <m:ResponseCode>ErrorItemNotFound</m:ResponseCode></m:R>"
                .into(),
        }
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

    /// The item of one occurrence of a series (`OccurrenceItemId`: the series' item and the
    /// occurrence's place in it, from 1), made the first time it's asked for, as Exchange makes
    /// an occurrence its own item once it's opened: its day worked out from the series' pattern
    /// (daily or weekly, with its interval), its other fields the series'.
    fn occurrence_item(&mut self, master: &str, index: usize) -> Option<String> {
        let id = format!("{master}:occ{index}");
        if self.pim.iter().any(|(i, _, _)| *i == id) {
            return Some(id);
        }
        let (folder, el) = self.pim.iter().find(|(i, _, _)| *i == master).map(|(_, f, e)| (f.clone(), e.clone()))?;
        let rec = el.child("Recurrence")?;
        let interval = rec.find("Interval").and_then(|i| i.text.trim().parse::<i64>().ok()).unwrap_or(1).max(1);
        let step = if rec.child("DailyRecurrence").is_some() { interval } else { interval * 7 } * 86_400;
        let start = quatrelle_jmap::utc_date(el.text_of("Start")?)?;
        let end = quatrelle_jmap::utc_date(el.text_of("End")?).unwrap_or(start + 3600);
        let shift = i64::try_from(index.checked_sub(1)?).ok()? * step;
        let when = |t: i64| {
            let (y, m, d) = quatrelle_model::dates::civil_from_days(t.div_euclid(86_400));
            let s = t.rem_euclid(86_400);
            format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
        };
        let copied: String = el
            .kids
            .iter()
            .filter(|k| matches!(k.name.as_str(), "Subject" | "Body" | "Location" | "UID" | "IsAllDayEvent" | "Organizer" | "RequiredAttendees"))
            .map(xml)
            .collect();
        let occurrence = parse(&format!(
            "<t:CalendarItem>{copied}<t:Start>{0}</t:Start><t:End>{1}</t:End><t:OriginalStart>{0}</t:OriginalStart>\
             <t:CalendarItemType>Occurrence</t:CalendarItemType></t:CalendarItem>",
            when(start + shift),
            when(end + shift)
        ));
        self.pim.push((id.clone(), folder, occurrence.kids.first()?.clone()));
        Some(id)
    }

    /// A series' occurrences that are items of their own (opened, moved), as the series lists
    /// them (`ModifiedOccurrences`).
    fn modified_occurrences(&self, master: &str) -> String {
        let prefix = format!("{master}:occ");
        let list: String = self
            .pim
            .iter()
            .filter(|(i, _, _)| i.starts_with(&prefix))
            .map(|(i, _, e)| {
                format!(
                    "<t:Occurrence><t:ItemId Id=\"{i}\" ChangeKey=\"ck2\"/><t:Start>{}</t:Start><t:End>{}</t:End><t:OriginalStart>{}</t:OriginalStart></t:Occurrence>",
                    e.text_of("Start").unwrap_or_default(),
                    e.text_of("End").unwrap_or_default(),
                    e.text_of("OriginalStart").unwrap_or_default()
                )
            })
            .collect();
        if list.is_empty() { String::new() } else { format!("<t:ModifiedOccurrences>{list}</t:ModifiedOccurrences>") }
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
mod streaming;
mod syncing;
mod tasks;

pub use server::serve;

#[cfg(test)]
mod tests;
