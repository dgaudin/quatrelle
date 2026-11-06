// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Exchange Web Services: the transport for Exchange servers run by an organisation itself
//! (on-premises Exchange 2010 SP2 and later) that don't offer IMAP, behind the same
//! [`Job`](quatrelle_model::Job)/[`Event`](quatrelle_model::Event) interface as the others.
//!
//! Every request is a SOAP call over HTTPS to the server's `/EWS/Exchange.asmx`, signed in with
//! NTLMv2 ([`crate::ntlm`], on a connection kept open, as NTLM signs in the connection) or Basic
//! authentication, whichever the server offers. Mail: folders (`FindFolder`), the newest
//! messages of a folder (`FindItem`), their details (`GetItem`), MIME sources, changes
//! (`UpdateItem`, `MoveItem`, `DeleteItem`, folder operations), drafts and sending
//! (`CreateItem` from MIME) and automatic replies (`Get`/`SetUserOofSettings`). Item ids are
//! numbered the JMAP transport's way ([`jmap::State`]); an item that moves gets a new id in
//! Exchange, so it's a new message in its new folder.
//!
//! Answers are hostile input: sizes, depth and counts are capped, nothing here panics.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use quatrelle_mime as mime;
use quatrelle_model::{Flag, Flags, Folder, FolderRole, MessageKey, Op, Summary, sort_newest_first, utf7};
use quatrelle_store::{FolderState, Store};

use crate::http::{self, Conn, Request, Response};
use crate::jmap::{self, Mailbox, State, VALIDITY};
use crate::sync::{SyncOptions, SyncReport, WordQuery, fill_from_source};
use crate::{NetError, Result};
use quatrelle_model::organize::{Condition, Rule, RuleAction};
use quatrelle_model::voting;

/// The EWS address of an account whose server Autodiscover finds.
pub const AUTODISCOVER: &str = quatrelle_model::EWS_AUTODISCOVER;
/// Largest answer accepted (message sources come base64-encoded inside).
const MAX_ANSWER: usize = 96 << 20;
/// Items asked about in one request.
const BATCH: usize = 50;
const MAX_FOLDERS: usize = 5000;
/// Public folders: how deep the tree is walked, and how many are listed.
const PUBLIC_DEPTH: usize = 6;
const MAX_PUBLIC: usize = 500;
/// The folder Exchange's public folders are listed under.
pub use quatrelle_model::PUBLIC_FOLDERS;
/// Elements read from one answer at most, and how deep.
const MAX_ELEMENTS: usize = 500_000;
const MAX_DEPTH: usize = 64;

/// Messages kept of one folder at most, when all of them are asked for.
const MAX_KEPT: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scheme {
    /// Not known yet: the first answer says.
    Unknown,
    Basic,
    /// NTLM, under the name the server offered (`NTLM` or `Negotiate`).
    Ntlm(&'static str),
}

/// A connection kept open, and whether NTLM signed it in. They go together: a sign-in noted
/// apart from its connection outlived it when the connection dropped (a timeout, a server that
/// closed it), and the next request went out unsigned on a new one, its 401 read as a wrong
/// password.
struct Kept {
    conn: Conn,
    signed_in: bool,
}

/// How long Exchange may ask to be left alone (`ErrorServerBusy`'s `BackOffMilliseconds`)
/// before the request is tried again here; longer, and it's left for later.
const MAX_BACK_OFF: std::time::Duration = std::time::Duration::from_secs(60);

/// What requests cost, added up: how many, the time waiting for the server (from asking to the
/// first line of its answer), receiving the answers, and reading them; and their size. Said in
/// the log for what takes long (a calendar read), to tell the server's part from Quatrelle's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Timing {
    pub requests: u32,
    pub server: std::time::Duration,
    pub receiving: std::time::Duration,
    pub reading: std::time::Duration,
    pub bytes: usize,
}

impl Timing {
    /// Both added up.
    #[must_use]
    pub fn and(self, o: Timing) -> Timing {
        Timing {
            requests: self.requests.saturating_add(o.requests),
            server: self.server.saturating_add(o.server),
            receiving: self.receiving.saturating_add(o.receiving),
            reading: self.reading.saturating_add(o.reading),
            bytes: self.bytes.saturating_add(o.bytes),
        }
    }

    /// What was spent since `before` (taken from the same session).
    #[must_use]
    pub fn since(self, before: Timing) -> Timing {
        Timing {
            requests: self.requests.saturating_sub(before.requests),
            server: self.server.saturating_sub(before.server),
            receiving: self.receiving.saturating_sub(before.receiving),
            reading: self.reading.saturating_sub(before.reading),
            bytes: self.bytes.saturating_sub(before.bytes),
        }
    }
}

/// An EWS session: the address, who signs in, and the connection (kept open for NTLM).
pub struct Client {
    url: String,
    user: String,
    password: String,
    conn: RefCell<Option<Kept>>,
    scheme: RefCell<Scheme>,
    /// The account's own address (automatic replies are asked for by it).
    email: String,
    /// The public folder mailbox requests are routed to (`X-AnchorMailbox` and
    /// `X-PublicFolderMailbox`), while reading a public folder's content.
    route: RefCell<Option<String>>,
    /// Public folder mailboxes found by Autodiscover: GUID → address.
    pf_mailboxes: RefCell<HashMap<String, String>>,
    /// Other people's addresses as the directory has them (see [`Client::address_of`]).
    addresses: Box<RefCell<HashMap<String, String>>>,
    /// The account's public folder hierarchy mailbox (Autodiscover's PublicFolderInformation),
    /// once asked: `None` inside when there is none.
    pf_hierarchy: RefCell<Option<Option<String>>>,
    /// The public folder mailboxes holding the public folders' content (their GUIDs, from the
    /// folders' PR_REPLICA_LIST): a request one of their folders or items isn't found by is
    /// tried there.
    pf_replicas: RefCell<Vec<String>>,
    /// The mailbox requests are routed to (`X-AnchorMailbox`) while working in a shared mailbox.
    anchor: RefCell<Option<String>>,
    /// Shared mailboxes added to the account by hand (their addresses).
    shared_added: RefCell<Vec<String>>,
    /// The shared mailboxes whose folders were listed: a request not found in this one is
    /// tried routed to them.
    shared_open: RefCell<Vec<String>>,
    /// The account's Autodiscover answer, once asked (`None` inside when there was none).
    discovered: RefCell<Option<Option<String>>>,
    /// The folders of the shared mailboxes listed: the mailbox each is in (its address) and the
    /// role it has there (`Other` for its top folder and the folders that aren't special).
    shared_roles: RefCell<HashMap<String, (String, FolderRole)>>,
    /// The User-Agent every request names, instead of Quatrelle's (a server that lets in only the
    /// clients it knows).
    user_agent: Option<String>,
    /// The next change of the rules removes Outlook's own copy of them (the user said so).
    replace_outlook_rules: Cell<bool>,
    /// The shared mailboxes found without a calendar this session (their addresses): not asked
    /// again at each calendar read (three on a real account, 2.6 s of the 9 the calendar took to
    /// open, 29/10).
    no_calendar: RefCell<Vec<String>>,
    /// The folders whose changes the server won't give this session (`SyncFolderItems` refused
    /// from nothing: public folders, on some servers), by the element naming them: read whole, as
    /// before, without asking again.
    changes_refused: RefCell<Vec<String>>,
    /// The server refused an appointment's fields asked by name: all their properties are asked
    /// this session, as before (see `ews_pim`'s `get_items`).
    fields_refused: Cell<bool>,
    /// How far the server's clock is ahead of this computer's, in seconds (from its first
    /// answer's `Date`): the times it gives (a new mail's) set beside this computer's.
    clock: Cell<Option<i64>>,
    /// The server's major version, once an answer said it (`ServerVersionInfo`: 15 for Exchange
    /// 2013 and later, 14 for 2010).
    server_major: Cell<Option<u32>>,
    /// What this session's requests cost, added up.
    spent: Cell<Timing>,
}

/// Another mailbox the account opens (Outlook's shared mailboxes): its address and name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedMailbox {
    pub address: String,
    pub name: String,
}

/// Most shared mailboxes an account opens.
const MAX_SHARED: usize = 20;

/// A mailbox's special folders: their distinguished ids and roles.
const SPECIAL: [(&str, FolderRole); 6] = [
    ("inbox", FolderRole::Inbox),
    ("drafts", FolderRole::Drafts),
    ("sentitems", FolderRole::Sent),
    ("deleteditems", FolderRole::Trash),
    ("junkemail", FolderRole::Junk),
    ("archive", FolderRole::Archive),
];

/// A public folder in the tree.
pub(crate) struct PublicFolder {
    pub id: String,
    /// Its path under Public Folders.
    pub path: String,
    pub class: String,
    pub total: u32,
    /// The GUID of the public folder mailbox holding its content (`PR_REPLICA_LIST`).
    pub replica: Option<String>,
}

/// A folder found walking the public folders.
struct TreeFolder {
    mailbox: Mailbox,
    class: String,
    children: bool,
    replica: Option<String>,
}

/// `PR_REPLICA_LIST`, asked for with public folders.
const REPLICA_LIST: &str = "<t:ExtendedFieldURI PropertyTag=\"26264\" PropertyType=\"Binary\"/>";

/// The mailbox GUID in a `PR_REPLICA_LIST` value (base64 of the GUID as ASCII, NUL-ended).
fn replica_guid(value: &str) -> Option<String> {
    let bytes = unb64(value.trim())?;
    let guid = String::from_utf8(bytes.get(..36)?.to_vec()).ok()?;
    let ok = guid.char_indices().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { c == '-' } else { c.is_ascii_hexdigit() });
    ok.then(|| guid.to_ascii_lowercase())
}

/// Why a call didn't answer: the server asked to wait this long, or it failed.
enum Busy {
    Wait(std::time::Duration),
    Failed(NetError),
}

/// Refusals that say the server can't now, not that it won't: tried again later rather than
/// dropped.
const TRANSIENT: [&str; 7] = [
    "ErrorServerBusy",
    "ErrorInternalServerTransientError",
    "ErrorMailboxStoreUnavailable",
    "ErrorMailboxMoveInProgress",
    "ErrorConnectionFailed",
    "ErrorTimeoutExpired",
    "ErrorBatchProcessingStopped",
];

/// Whether Exchange said it's busy (`ErrorServerBusy`, as a fault or in a response message),
/// and for how long it asks to be left alone (its `BackOffMilliseconds`; a few seconds when it
/// doesn't say).
fn busy(doc: &El) -> Option<std::time::Duration> {
    let fault = doc.find("Fault").and_then(|f| f.find("detail")).filter(|d| d.text_of("ResponseCode").is_some_and(|c| c.trim() == "ErrorServerBusy"));
    let message = || messages(doc).into_iter().find(|m| m.text_of("ResponseCode").is_some_and(|c| c.trim() == "ErrorServerBusy"));
    let found = fault.or_else(message)?;
    let mut values = Vec::new();
    found.all("Value", &mut values);
    let ms = values.iter().find(|v| v.attr("Name") == Some("BackOffMilliseconds")).and_then(|v| v.text.trim().parse::<u64>().ok()).unwrap_or(5_000);
    Some(std::time::Duration::from_millis(ms.max(500)))
}

/// The most fields a refusal is said to name, and the longest name said.
const FIELDS_NAMED: usize = 3;
const FIELD_NAME_MAX: usize = 80;

/// The fields a refusal's `MessageXml` names (`FieldURI`, `IndexedFieldURI`, `ExtendedFieldURI`
/// as [`extended_name`] says it), the first few, in its order. The server's words: only what
/// looks like a name ([`plain_name`]).
fn fields_named(m: &El) -> Vec<String> {
    let mut names = Vec::new();
    // Depth first, without recursion (an answer's elements are capped in number and depth
    // already: see `parse`).
    let mut next: Vec<&El> = m.child("MessageXml").map(|x| x.kids.iter().rev().collect()).unwrap_or_default();
    while let Some(el) = next.pop()
        && names.len() < FIELDS_NAMED
    {
        let name = match el.name.as_str() {
            "FieldURI" | "IndexedFieldURI" => el.attr("FieldURI").filter(|n| plain_name(n)).map(str::to_string),
            "ExtendedFieldURI" => extended_name(el),
            _ => None,
        };
        names.extend(name.filter(|n| plain_name(n)));
        next.extend(el.kids.iter().rev());
    }
    names
}

/// An extended field's name: its tag, or its name, or its id — after its property set when one
/// is given (`Appointment:33315`), as calendar properties are named. Its parts are each a
/// [`plain_name`], or it has none.
fn extended_name(el: &El) -> Option<String> {
    if let Some(n) = el.attr("PropertyTag").or_else(|| el.attr("PropertyName")) {
        return plain_name(n).then(|| n.to_string());
    }
    let id = el.attr("PropertyId").filter(|i| plain_name(i))?;
    match el.attr("DistinguishedPropertySetId").or_else(|| el.attr("PropertySetId")) {
        Some(set) => plain_name(set).then(|| format!("{set}:{id}")),
        None => Some(id.to_string()),
    }
}

/// Not too long, and nothing but ASCII letters, digits and `:_-.`: what a refusal's text says of
/// the server's names, a field's (`calendar:Organizer`, `0x1090`) or the code's
/// (`ErrorInvalidPropertySet`).
fn plain_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= FIELD_NAME_MAX && n.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-' | b'.'))
}

/// What a `…ResponseMessage` said: a refusal in the server's words, with the fields it named
/// (Exchange names the field it refused in its `MessageXml`, which its text doesn't say). It
/// reads the server's answer, hostile like any: public for the fuzz target.
pub fn check_message(m: &El) -> Result<()> {
    match m.attr("ResponseClass") {
        Some("Error") => {
            let code = m.text_of("ResponseCode").unwrap_or("");
            let said = m.text_of("MessageText").unwrap_or(code);
            let names = fields_named(m).join(", ");
            let text = match code.trim() {
                _ if names.is_empty() => said.to_string(),
                c if plain_name(c) => format!("{said} ({c}: {names})"),
                _ => format!("{said} ({names})"),
            };
            Err(match code {
                "ErrorItemNotFound" | "ErrorFolderNotFound" => NetError::No(format!("not on the server any more ({text})")),
                c if TRANSIENT.contains(&c) => NetError::Later(text),
                // Outlook keeps a copy of the rules of its own in the mailbox, and Exchange won't
                // change them while it's there (`RemoveOutlookRuleBlob` would remove it, and the
                // rules Outlook runs only on its computer with it): said plainly.
                "ErrorOutlookRuleBlobExists" => NetError::No(
                    "Outlook keeps its own copy of this mailbox's rules, and Exchange won't change them while it's there. \
                     Change this rule in Outlook, or let Quatrelle replace Outlook's copy (Outlook's rules that run only on its computer would be lost)"
                        .into(),
                ),
                "ErrorAccessDenied" => NetError::No(text),
                // From an address the account may not send from.
                "ErrorSendAsDenied" => NetError::No(
                    "this account isn't allowed to send from that address: Exchange's administrator gives Send As or Send on Behalf on it".into(),
                ),
                _ => NetError::No(text),
            })
        }
        _ => Ok(()),
    }
}

/// The response message of an operation: the one inside `ResponseMessages`, or — as some servers
/// (Exchange 2016 among them) answer — the response element itself, carrying `ResponseClass`.
pub(crate) fn operation<'a>(doc: &'a El, element: &str) -> Result<&'a El> {
    messages(doc).first().copied().or_else(|| doc.find(element)).ok_or_else(|| NetError::Protocol("an empty answer".into()))
}

/// The picture a `GetUserPhoto` answer carries. Exchange 2016 answers with the response element
/// itself (`ResponseClass`, `PictureData` and all), not inside the `ResponseMessages` list every
/// other operation uses; a server that does wrap it is read too.
fn photo_of(doc: &El) -> Result<(String, Vec<u8>)> {
    let m =
        messages(doc).first().copied().or_else(|| doc.find("GetUserPhotoResponse")).ok_or_else(|| NetError::Protocol("an empty answer".into()))?;
    check_message(m)?;
    let data = m.find("PictureData").map(|c| c.text.trim()).unwrap_or_default();
    if data.is_empty() {
        return Err(NetError::No("no picture for this person".into()));
    }
    let bytes = unb64(data).ok_or_else(|| NetError::Protocol("the picture isn't base64".into()))?;
    if bytes.is_empty() || bytes.len() > crate::MAX_PICTURE {
        return Err(NetError::No("no picture for this person".into()));
    }
    Ok((crate::picture_media(&bytes).to_string(), bytes))
}

/// The `…ResponseMessage` elements of an answer, as the server sent it (hostile like any): public
/// for the fuzz target.
pub fn messages(doc: &El) -> Vec<&El> {
    let mut out = Vec::new();
    if let Some(body) = doc.find("Body") {
        let mut all = Vec::new();
        body.all("ResponseMessages", &mut all);
        for rm in all {
            out.extend(rm.kids.iter());
        }
    }
    out
}

impl Client {
    /// A session for `url` (checked with the first request).
    pub fn new(url: &str, user: &str, password: &str, email: &str) -> Result<Client> {
        let u = http::Url::parse(url)?;
        let local = matches!(u.host.as_str(), "127.0.0.1" | "localhost" | "::1");
        if !u.https && !local {
            return Err(NetError::Protocol("Exchange is reached over https only".into()));
        }
        Ok(Client {
            url: url.to_string(),
            user: user.to_string(),
            password: password.to_string(),
            conn: RefCell::new(None),
            scheme: RefCell::new(Scheme::Unknown),
            email: email.to_string(),
            route: RefCell::new(None),
            pf_mailboxes: RefCell::new(HashMap::new()),
            addresses: Box::new(RefCell::new(HashMap::new())),
            pf_hierarchy: RefCell::new(None),
            pf_replicas: RefCell::new(Vec::new()),
            anchor: RefCell::new(None),
            shared_added: RefCell::new(Vec::new()),
            shared_open: RefCell::new(Vec::new()),
            discovered: RefCell::new(None),
            shared_roles: RefCell::new(HashMap::new()),
            replace_outlook_rules: Cell::new(false),
            user_agent: None,
            no_calendar: RefCell::new(Vec::new()),
            changes_refused: RefCell::new(Vec::new()),
            fields_refused: Cell::new(false),
            clock: Cell::new(None),
            server_major: Cell::new(None),
            spent: Cell::new(Timing::default()),
        })
    }

    /// Another session like this one, on a connection of its own (NTLM signs in a connection),
    /// to ask several things of the server at once: what this one has learnt (how to sign in,
    /// other people's addresses, where shared mailboxes and public folders are, where requests
    /// are routed now) goes with it.
    pub(crate) fn sibling(&self) -> Client {
        Client {
            url: self.url.clone(),
            user: self.user.clone(),
            password: self.password.clone(),
            conn: RefCell::new(None),
            scheme: self.scheme.clone(),
            email: self.email.clone(),
            route: self.route.clone(),
            pf_mailboxes: self.pf_mailboxes.clone(),
            addresses: self.addresses.clone(),
            pf_hierarchy: self.pf_hierarchy.clone(),
            pf_replicas: self.pf_replicas.clone(),
            anchor: self.anchor.clone(),
            shared_added: self.shared_added.clone(),
            shared_open: self.shared_open.clone(),
            discovered: self.discovered.clone(),
            shared_roles: self.shared_roles.clone(),
            replace_outlook_rules: Cell::new(false),
            user_agent: self.user_agent.clone(),
            no_calendar: self.no_calendar.clone(),
            changes_refused: self.changes_refused.clone(),
            fields_refused: self.fields_refused.clone(),
            clock: self.clock.clone(),
            server_major: self.server_major.clone(),
            spent: Cell::new(Timing::default()),
        }
    }

    /// Whether the server refused an appointment's fields asked by name this session.
    pub(crate) fn fields_refused(&self) -> bool {
        self.fields_refused.get()
    }

    /// The server refused an appointment's fields asked by name: all their properties from now on.
    pub(crate) fn note_fields_refused(&self) {
        self.fields_refused.set(true);
    }

    /// How far the server's clock is ahead of this computer's, in seconds, once it answered.
    pub fn clock_ahead(&self) -> Option<i64> {
        self.clock.get()
    }

    /// What this session's requests have cost so far.
    pub(crate) fn spent(&self) -> Timing {
        self.spent.get()
    }

    /// Requests made elsewhere (a sibling's) counted as this session's.
    pub(crate) fn count_spent(&self, t: Timing) {
        self.spent.set(self.spent.get().and(t));
    }

    /// The same session naming itself `agent` to the server (`None`, or nothing usable: as
    /// Quatrelle).
    #[must_use]
    pub fn with_agent(mut self, agent: Option<&str>) -> Client {
        self.user_agent = agent.and_then(quatrelle_model::user_agent);
        self
    }
}

/// A well-known folder (`inbox`, `msgfolderroot`…) of this account's mailbox, or of another one
/// opened in it.
fn distinguished(name: &str, owner: Option<&str>) -> String {
    match owner {
        Some(o) => format!(
            "<t:DistinguishedFolderId Id=\"{name}\"><t:Mailbox><t:EmailAddress>{}</t:EmailAddress></t:Mailbox></t:DistinguishedFolderId>",
            esc(o)
        ),
        None => format!("<t:DistinguishedFolderId Id=\"{name}\"/>"),
    }
}

/// Drop folders whose parent was dropped, repeating until every remaining parent is listed.
fn keep_rooted(out: &mut Vec<Mailbox>) {
    loop {
        let ids: std::collections::HashSet<String> = out.iter().map(|m| m.id.clone()).collect();
        let before = out.len();
        out.retain(|m| m.parent.as_ref().is_none_or(|p| ids.contains(p)));
        if out.len() == before {
            break;
        }
    }
}

/// The `PidTagAttributeHidden` property ([MS-OXPROPS]), asked for with each folder.
const HIDDEN: &str = "<t:ExtendedFieldURI PropertyTag=\"0x10f4\" PropertyType=\"Boolean\"/>";

/// Whether a listed folder carries that property set to true: Exchange's own bookkeeping
/// folders (Working Set, Yammer Root, Conversation Action Settings…), which Outlook hides.
fn is_hidden(f: &El) -> bool {
    f.children("ExtendedProperty").any(|p| {
        p.child("ExtendedFieldURI").and_then(|u| u.attr("PropertyTag")).is_some_and(|t| is_tag(t, "0x10f4"))
            && p.text_of("Value").is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
    })
}

/// The read, flag and draft state, asked for with every item.
/// A change in a folder, as `SyncFolderItems` tells it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Created(String, Flags),
    Updated(String, Flags),
    Deleted(String),
    Read(String, bool),
}

/// Changes asked for at a time (Exchange answers 512 at most).
const CHANGES_PAGE: usize = 512;

/// An item's marks: read, flagged or its follow-up complete (`PidTagFlagStatus`, and
/// `PidTagFlagCompleteTime` the time it was done), replied to or forwarded
/// (`PidTagLastVerbExecuted`, and `PidTagLastVerbExecutionTime` when), a draft, its categories.
/// Every answer whose marks are taken asks for all of them (a category missing from one would be
/// taken off).
const FLAG_FIELDS: &str = "<t:FieldURI FieldURI=\"message:IsRead\"/>\
<t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/>\
<t:ExtendedFieldURI PropertyTag=\"0x1091\" PropertyType=\"SystemTime\"/>\
<t:ExtendedFieldURI PropertyTag=\"0x1081\" PropertyType=\"Integer\"/>\
<t:ExtendedFieldURI PropertyTag=\"0x1082\" PropertyType=\"SystemTime\"/>\
<t:ExtendedFieldURI PropertyTag=\"0x0E07\" PropertyType=\"Integer\"/>\
<t:FieldURI FieldURI=\"item:Categories\"/>";
/// Most categories taken from one item.
const ITEM_CATEGORIES: usize = 50;
const HEADER_FIELDS: &str = "<t:FieldURI FieldURI=\"item:Subject\"/><t:FieldURI FieldURI=\"message:From\"/>\
<t:FieldURI FieldURI=\"message:ToRecipients\"/><t:FieldURI FieldURI=\"message:CcRecipients\"/>\
<t:FieldURI FieldURI=\"message:ReplyTo\"/><t:FieldURI FieldURI=\"item:DateTimeReceived\"/>\
<t:FieldURI FieldURI=\"item:DateTimeSent\"/><t:FieldURI FieldURI=\"message:InternetMessageId\"/>\
<t:FieldURI FieldURI=\"item:HasAttachments\"/><t:FieldURI FieldURI=\"item:Size\"/>\
<t:ExtendedFieldURI DistinguishedPropertySetId=\"Common\" PropertyId=\"34080\" PropertyType=\"Binary\"/>\
<t:ExtendedFieldURI PropertyTag=\"0x5D01\" PropertyType=\"String\"/>";

/// The voting buttons on an item (`PidLidVerbStream`, asked for with its details).
fn voting_choices(it: &El) -> Vec<String> {
    let id = u32::from(voting::VERB_STREAM);
    it.children("ExtendedProperty")
        .find(|p| {
            p.child("ExtendedFieldURI").and_then(|u| u.attr("PropertyId")).is_some_and(|v| {
                let v = v.trim();
                v.parse::<u32>().ok().or_else(|| v.strip_prefix("0x").and_then(|h| u32::from_str_radix(h, 16).ok())) == Some(id)
            })
        })
        .and_then(|p| p.text_of("Value"))
        .and_then(|v| unb64(v.trim()))
        .map(|b| voting::choices(&b))
        .unwrap_or_default()
}

/// Whether a property tag as the server writes it is `tag`: Exchange answers in its own
/// spelling (`0xe08` for `0x0E08`, in hex for one asked in decimal).
pub(crate) fn is_tag(said: &str, tag: &str) -> bool {
    fn num(t: &str) -> Option<u32> {
        let t = t.trim();
        t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).map_or_else(|| t.parse().ok(), |h| u32::from_str_radix(h, 16).ok())
    }
    match (num(said), num(tag)) {
        (Some(a), Some(b)) => a == b,
        _ => said.eq_ignore_ascii_case(tag),
    }
}

/// An extended property's value.
fn extended(it: &El, tag: &str) -> Option<i64> {
    it.children("ExtendedProperty")
        .find(|p| p.child("ExtendedFieldURI").and_then(|u| u.attr("PropertyTag")).is_some_and(|t| is_tag(t, tag)))
        .and_then(|p| p.text_of("Value"))
        .and_then(|v| v.trim().parse().ok())
}

/// An extended property's value as text.
fn extended_text<'a>(it: &'a El, tag: &str) -> Option<&'a str> {
    it.children("ExtendedProperty")
        .find(|p| p.child("ExtendedFieldURI").and_then(|u| u.attr("PropertyTag")).is_some_and(|t| is_tag(t, tag)))
        .and_then(|p| p.text_of("Value"))
}

/// An extended property's text, named as Quatrelle's own are: by its property set (a GUID, in
/// either case) and its `PropertyName`.
pub(crate) fn extended_named<'a>(it: &'a El, set: &str, name: &str) -> Option<&'a str> {
    it.children("ExtendedProperty")
        .find(|p| {
            p.child("ExtendedFieldURI")
                .is_some_and(|u| u.attr("PropertySetId").is_some_and(|s| s.trim().eq_ignore_ascii_case(set)) && u.attr("PropertyName") == Some(name))
        })
        .and_then(|p| p.text_of("Value"))
}

fn flags(it: &El) -> Flags {
    let mut keywords: Vec<String> = it
        .child("Categories")
        .map(|c| c.children("String").map(|s| quatrelle_model::keyword_for(&s.text)).filter(|k| quatrelle_model::is_keyword(k)).collect())
        .unwrap_or_default();
    keywords.sort_by_key(|k| k.to_ascii_lowercase());
    keywords.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    keywords.truncate(ITEM_CATEGORIES);
    // Outlook's Mark Complete: status 1 (2 is flagged), and when it was done.
    let status = extended(it, "0x1090");
    if status == Some(1) {
        keywords.push(quatrelle_model::FOLLOW_UP_COMPLETE.to_string());
        keywords.sort_by_key(|k| k.to_ascii_lowercase());
    }
    let completed = extended_text(it, "0x1091").and_then(jmap::utc_date).filter(|t| *t > 0 && status == Some(1)).unwrap_or(0);
    // The last thing done with it, as Outlook writes it ([MS-OXOMSG] `PidTagLastVerbExecuted`):
    // 102 replied to the sender, 103 to all, 104 forwarded; another (a vote) is neither. Only
    // the last is kept, so a reply followed by a forward shows the forward, as Outlook does.
    let verb = extended(it, "0x1081");
    let answered = matches!(verb, Some(VERB_REPLY | VERB_REPLY_ALL));
    let forwarded = verb == Some(VERB_FORWARD);
    let acted = extended_text(it, "0x1082").filter(|_| answered || forwarded).map_or(0, verb_time);
    Flags {
        seen: it.text_of("IsRead").is_some_and(|v| v == "true"),
        flagged: status == Some(2),
        answered,
        forwarded,
        draft: extended(it, "0x0E07").is_some_and(|f| f & 8 != 0),
        keywords,
        completed,
        acted,
        ..Default::default()
    }
}

/// When it was done (`PidTagLastVerbExecutionTime`), in Unix seconds: a time that isn't one,
/// before 1970, or MAPI's "none" (4501) is none (0).
pub(crate) fn verb_time(text: &str) -> i64 {
    let none = quatrelle_model::dates::days_from_civil(4501, 1, 1).saturating_mul(86_400);
    jmap::utc_date(text).filter(|t| *t > 0 && *t < none).unwrap_or(0)
}

/// The verb and the icon Outlook writes on the original of a reply (`forward`: a forward):
/// `PidTagLastVerbExecuted` 102 or 104 and `PidTagIconIndex` 0x105 or 0x106 ([MS-OXOMSG]). The
/// engine doesn't say a reply was to all (103): it's written as a reply to the sender, which
/// Outlook shows alike.
pub(crate) fn replied_verb(forward: bool) -> (i64, i64) {
    if forward { (VERB_FORWARD, 0x106) } else { (VERB_REPLY, 0x105) }
}

/// `PidTagLastVerbExecuted` for a reply to the sender ([MS-OXOMSG]; `NOTEIVERB_REPLYTOSENDER`).
pub(crate) const VERB_REPLY: i64 = 102;
/// … for a reply to all (`NOTEIVERB_REPLYTOALL`).
pub(crate) const VERB_REPLY_ALL: i64 = 103;
/// … for a forward (`NOTEIVERB_FORWARD`).
pub(crate) const VERB_FORWARD: i64 = 104;

/// The marks of every message in an answer, as [`flags`] reads them (the fuzz target reads
/// them so).
pub fn marks_listed(doc: &El) -> Vec<Flags> {
    let mut items = Vec::new();
    doc.all("Message", &mut items);
    items.into_iter().map(flags).collect()
}

/// A message's marks as the server has them now: read, flagged or complete (and when), a draft
/// and its categories (Exchange keeps them by name, as [`flags`] reads them); the keywords that
/// are Quatrelle's own (`$Pinned`, `$Junk`…) stay. Replied to or forwarded, the last of them and
/// when, once the server says one; until then (a reply made here by a version that didn't tell
/// Exchange, or not told yet) the arrow kept here stays. Whether anything changed.
fn take_marks(m: &mut Flags, server: &Flags) -> bool {
    use quatrelle_model::FOLLOW_UP_COMPLETE;
    let mut next = m.clone();
    next.seen = server.seen;
    next.flagged = server.flagged;
    next.draft = server.draft;
    next.keywords.retain(|k| k.starts_with('$') && !k.eq_ignore_ascii_case(FOLLOW_UP_COMPLETE));
    for k in &server.keywords {
        next.set_keyword(k, true);
    }
    next.completed = server.completed;
    if server.answered || server.forwarded {
        next.answered = server.answered;
        next.forwarded = server.forwarded;
        next.acted = server.acted;
    }
    let changed = next != *m;
    *m = next;
    changed
}

/// A mailbox element as an address (an Exchange-internal address keeps the name only).
fn address(m: &El) -> Option<String> {
    let name = m.text_of("Name").unwrap_or("").replace('"', "");
    let email = m.text_of("EmailAddress").filter(|_| m.text_of("RoutingType").is_none_or(|r| r.eq_ignore_ascii_case("SMTP"))).unwrap_or("");
    if email.contains('@') {
        Some(if name.is_empty() { email.to_string() } else { format!("\"{name}\" <{email}>") })
    } else if name.is_empty() {
        None
    } else {
        Some(format!("\"{name}\" <>"))
    }
}

fn addresses(it: &El, name: &str) -> String {
    it.child(name).map(|l| l.children("Mailbox").filter_map(address).collect::<Vec<_>>().join(", ")).unwrap_or_default()
}

/// Who an item is from, as an address. A colleague's mail comes from an Exchange-internal address,
/// kept as a name alone: the sender's SMTP address (`PidTagSenderSmtpAddress`) is theirs, so a
/// rule, a reply or the address book has an address to go by.
fn sender(it: &El) -> Option<String> {
    let f = it.child("From").and_then(|f| f.child("Mailbox")).and_then(address)?;
    if let Some(name) = f.strip_suffix(" <>")
        && let Some(smtp) = extended_text(it, "0x5D01").filter(|a| a.contains('@') && !a.contains(['<', '>', '"', '\r', '\n']))
    {
        return Some(format!("{name} <{}>", smtp.trim()));
    }
    Some(f)
}

/// A header block for the list, from an item's details (its date is the received one).
fn header_block(it: &El) -> Vec<u8> {
    let mut h = String::new();
    let clean = |s: &str| s.replace(['\r', '\n'], " ");
    if let Some(f) = sender(it) {
        h.push_str(&format!("From: {}\r\n", clean(&f)));
    }
    for (field, name) in [("ToRecipients", "To"), ("CcRecipients", "Cc"), ("ReplyTo", "Reply-To")] {
        let list = addresses(it, field);
        if !list.is_empty() {
            h.push_str(&format!("{name}: {}\r\n", clean(&list)));
        }
    }
    if let Some(s) = it.text_of("Subject") {
        h.push_str(&format!("Subject: {}\r\n", mime_word(&clean(s))));
    }
    if let Some(id) = it.text_of("InternetMessageId") {
        h.push_str(&format!("Message-ID: {}\r\n", clean(id)));
    }
    h.push_str("\r\n");
    h.into_bytes()
}

/// A subject as an RFC 2047 word when it isn't ASCII.
fn mime_word(s: &str) -> String {
    if s.is_ascii() { s.to_string() } else { format!("=?UTF-8?B?{}?=", b64(s.as_bytes())) }
}

mod autodiscover;
mod changes;
mod directory;
mod listening;
mod mailbox;
mod reminders;
mod requests;
mod rules;
mod shared;
mod sizes;
mod sync;
mod text_first;
mod xml;

#[cfg(test)]
pub(crate) use autodiscover::TEST_BASES;
use autodiscover::{alternative_mailboxes, autodiscover_public, autodiscover_smtp, autodiscover_urls};
pub use autodiscover::{autodiscover, autodiscover_as};
pub use changes::{append, apply_remote, send, vote};
pub use listening::{Heard, Listener};
pub use reminders::{Listed, MAX_REMINDERS, reminders_listed, server_major};
use sync::restriction;
pub use sync::{
    AheadTodo, download_ahead, fetch_body, find_by_message_id, find_lost, find_on_server, list_folders, older, read_online, search, sync_folder,
    sync_folder_then,
};
pub use text_first::{fetch_text, text_first};
pub use xml::{El, esc, parse, unb64};
use xml::{b64, envelope, envelope_as};

#[cfg(test)]
mod tests;
