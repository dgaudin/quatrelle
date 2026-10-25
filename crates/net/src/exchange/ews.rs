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
    /// The next change of the rules removes Outlook's own copy of them (the user said so).
    replace_outlook_rules: Cell<bool>,
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

/// What a `…ResponseMessage` said.
pub(crate) fn check_message(m: &El) -> Result<()> {
    match m.attr("ResponseClass") {
        Some("Error") => {
            let code = m.text_of("ResponseCode").unwrap_or("");
            let text = m.text_of("MessageText").unwrap_or(code);
            Err(match code {
                "ErrorItemNotFound" | "ErrorFolderNotFound" => NetError::No(format!("not on the server any more ({text})")),
                c if TRANSIENT.contains(&c) => NetError::Later(text.to_string()),
                // Outlook keeps a copy of the rules of its own in the mailbox, and Exchange won't
                // change them while it's there (`RemoveOutlookRuleBlob` would remove it, and the
                // rules Outlook runs only on its computer with it): said plainly.
                "ErrorOutlookRuleBlobExists" => NetError::No(
                    "Outlook keeps its own copy of this mailbox's rules, and Exchange won't change them while it's there. \
                     Change this rule in Outlook, or let Quatrelle replace Outlook's copy (Outlook's rules that run only on its computer would be lost)"
                        .into(),
                ),
                "ErrorAccessDenied" => NetError::No(text.to_string()),
                _ => NetError::No(text.to_string()),
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

/// The `…ResponseMessage` elements of an answer.
pub(crate) fn messages(doc: &El) -> Vec<&El> {
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
        })
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

/// An item's marks: read, flagged, a draft, its categories. Every answer whose marks are taken
/// asks for all of them (a category missing from one would be taken off).
const FLAG_FIELDS: &str = "<t:FieldURI FieldURI=\"message:IsRead\"/>\
<t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/>\
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

fn flags(it: &El) -> Flags {
    let mut keywords: Vec<String> = it
        .child("Categories")
        .map(|c| c.children("String").map(|s| quatrelle_model::keyword_for(&s.text)).filter(|k| quatrelle_model::is_keyword(k)).collect())
        .unwrap_or_default();
    keywords.sort_by_key(|k| k.to_ascii_lowercase());
    keywords.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    keywords.truncate(ITEM_CATEGORIES);
    Flags {
        seen: it.text_of("IsRead").is_some_and(|v| v == "true"),
        flagged: extended(it, "0x1090") == Some(2),
        draft: extended(it, "0x0E07").is_some_and(|f| f & 8 != 0),
        keywords,
        ..Default::default()
    }
}

/// A message's marks as the server has them now: read, flagged, a draft and its categories
/// (Exchange keeps them by name, as [`flags`] reads them); the keywords that are Quatrelle's own
/// (`$Pinned`, `$Junk`…) stay. Whether anything changed.
fn take_marks(m: &mut Flags, server: &Flags) -> bool {
    let mut next = m.clone();
    next.seen = server.seen;
    next.flagged = server.flagged;
    next.draft = server.draft;
    next.keywords.retain(|k| k.starts_with('$'));
    for k in &server.keywords {
        next.set_keyword(k, true);
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

/// A header block for the list, from an item's details (its date is the received one).
fn header_block(it: &El) -> Vec<u8> {
    let mut h = String::new();
    let clean = |s: &str| s.replace(['\r', '\n'], " ");
    if let Some(mut f) = it.child("From").and_then(|f| f.child("Mailbox")).and_then(address) {
        // A colleague's mail comes from an Exchange-internal address, kept as a name alone:
        // the sender's SMTP address (`PidTagSenderSmtpAddress`) is theirs, so a rule, a reply
        // or the address book has an address to go by.
        if let Some(name) = f.strip_suffix(" <>")
            && let Some(smtp) = extended_text(it, "0x5D01").filter(|a| a.contains('@') && !a.contains(['<', '>', '"', '\r', '\n']))
        {
            f = format!("{name} <{}>", smtp.trim());
        }
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
mod requests;
mod rules;
mod shared;
mod sizes;
mod sync;
mod xml;

pub use autodiscover::autodiscover;
use autodiscover::{alternative_mailboxes, autodiscover_public, autodiscover_smtp, autodiscover_urls};
pub use changes::{append, apply_remote, send, vote};
pub use listening::{Heard, Listener};
use sync::restriction;
pub use sync::{AheadTodo, download_ahead, fetch_body, find_lost, find_on_server, list_folders, older, read_online, search, sync_folder};
pub use xml::{El, esc, parse, unb64};
use xml::{b64, envelope};

#[cfg(test)]
mod tests;
