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

use std::cell::RefCell;
use std::collections::HashMap;

use quatrelle_mime as mime;
use quatrelle_model::{Flag, Flags, Folder, FolderRole, MessageKey, Op, Summary, sort_newest_first, utf7};
use quatrelle_store::{FolderState, Store};
use quick_xml::Reader;
use quick_xml::events::Event as X;

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
pub const PUBLIC_FOLDERS: &str = "Public Folders";
/// Elements read from one answer at most, and how deep.
const MAX_ELEMENTS: usize = 500_000;
const MAX_DEPTH: usize = 64;

/// An element of an answer: its local name, attributes, text and children.
#[derive(Clone, Debug, Default)]
pub struct El {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub kids: Vec<El>,
}

impl El {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    /// The first child called `name`.
    pub fn child(&self, name: &str) -> Option<&El> {
        self.kids.iter().find(|k| k.name == name)
    }
    pub fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a El> + 'a {
        self.kids.iter().filter(move |k| k.name == name)
    }
    /// The first element called `name` below this one (depth first).
    pub fn find(&self, name: &str) -> Option<&El> {
        self.kids.iter().find_map(|k| if k.name == name { Some(k) } else { k.find(name) })
    }
    /// Every element called `name` below this one.
    pub fn all<'a>(&'a self, name: &str, out: &mut Vec<&'a El>) {
        for k in &self.kids {
            if k.name == name {
                out.push(k);
            } else {
                k.all(name, out);
            }
        }
    }
    pub fn text_of(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.as_str())
    }
    /// The folders in an answer, whatever their kind: Exchange gives each in an element of its
    /// own (`Folder`, `ContactsFolder`, `CalendarFolder`, `TasksFolder`, `SearchFolder`).
    pub fn folders(&self) -> Vec<&El> {
        fn walk<'a>(el: &'a El, out: &mut Vec<&'a El>) {
            for k in &el.kids {
                if FOLDER_KINDS.contains(&k.name.as_str()) {
                    out.push(k);
                } else {
                    walk(k, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(self, &mut out);
        out
    }
}

/// The elements a folder comes in (EWS's array of folders).
const FOLDER_KINDS: [&str; 5] = ["Folder", "ContactsFolder", "CalendarFolder", "TasksFolder", "SearchFolder"];

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or("").to_string()
}

/// An attribute's value with its entities replaced.
fn unescape(v: &str) -> String {
    if !v.contains('&') {
        return v.to_string();
    }
    let mut out = String::with_capacity(v.len());
    let mut rest = v;
    while let Some(i) = rest.find('&') {
        out.push_str(rest.get(..i).unwrap_or(""));
        let after = rest.get(i + 1..).unwrap_or("");
        let Some(end) = after.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = after;
            continue;
        };
        let name = after.get(..end).unwrap_or("");
        let c = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            n => n
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| n.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = after.get(end + 1..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Read XML into elements (namespaces dropped: the local names are enough here).
pub fn parse(xml: &str) -> El {
    let mut r = Reader::from_str(xml);
    let mut stack: Vec<El> = vec![El { name: "#doc".into(), ..Default::default() }];
    let mut count = 0usize;
    let close = |stack: &mut Vec<El>| {
        if stack.len() > 1
            && let Some(done) = stack.pop()
            && let Some(parent) = stack.last_mut()
        {
            parent.kids.push(done);
        }
    };
    while let Ok(ev) = r.read_event() {
        let empty = matches!(ev, X::Empty(_));
        match ev {
            X::Start(e) | X::Empty(e) if stack.len() < MAX_DEPTH && count < MAX_ELEMENTS => {
                count += 1;
                let el = El {
                    name: local(e.name().as_ref()),
                    attrs: e.attributes().flatten().map(|a| (local(a.key.as_ref()), unescape(&a.value))).collect(),
                    ..Default::default()
                };
                stack.push(el);
                if empty {
                    close(&mut stack);
                }
            }
            X::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t);
                }
            }
            X::CData(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t);
                }
            }
            X::GeneralRef(g) => {
                let name = g.to_string();
                let c = match name.as_str() {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    n => n
                        .strip_prefix("#x")
                        .or_else(|| n.strip_prefix("#X"))
                        .and_then(|h| u32::from_str_radix(h, 16).ok())
                        .or_else(|| n.strip_prefix('#').and_then(|d| d.parse().ok()))
                        .and_then(char::from_u32),
                };
                if let (Some(c), Some(top)) = (c, stack.last_mut()) {
                    top.text.push(c);
                }
            }
            X::End(_) => close(&mut stack),
            X::Eof => break,
            _ => {}
        }
    }
    while stack.len() > 1 {
        close(&mut stack);
    }
    stack.pop().unwrap_or_default()
}

/// Text for XML.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // Not allowed in XML 1.0.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

fn envelope(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\" \
xmlns:t=\"http://schemas.microsoft.com/exchange/services/2006/types\" \
xmlns:m=\"http://schemas.microsoft.com/exchange/services/2006/messages\">\
<soap:Header><t:RequestServerVersion Version=\"Exchange2010_SP2\"/></soap:Header>\
<soap:Body>{body}</soap:Body></soap:Envelope>"
    )
}

/// Messages kept of one folder at most, when all of them are asked for.
const MAX_KEPT: usize = 100_000;

/// Base64 without line breaks.
fn b64(data: &[u8]) -> String {
    crate::imap::base64(data)
}

/// Base64 text to bytes (whitespace skipped); `None` when it isn't base64.
pub fn unb64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace()) {
        if c == b'=' {
            break;
        }
        acc = (acc << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xFF).unwrap_or(0));
        }
    }
    Some(out)
}

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

/// The address in an Autodiscover answer about a mailbox.
fn autodiscover_smtp(xml: &str) -> Option<String> {
    let doc = parse(xml);
    doc.find("AutoDiscoverSMTPAddress").map(|a| a.text.trim().to_string()).filter(|a| a.contains('@') && a.len() < 320)
}

/// The public folder hierarchy mailbox in an Autodiscover answer about the account (Exchange
/// 2013 and later: `PublicFolderInformation`'s `SmtpAddress`).
fn autodiscover_public(xml: &str) -> Option<String> {
    let doc = parse(xml);
    doc.find("PublicFolderInformation")
        .and_then(|p| p.find("SmtpAddress"))
        .map(|a| a.text.trim().to_string())
        .filter(|a| a.contains('@') && a.len() < 320)
}

/// A mailbox Autodiscover maps to the account (Outlook opens these by itself).
struct Alternative {
    name: String,
    smtp: Option<String>,
    legacy_dn: Option<String>,
}

/// The mailboxes an Autodiscover answer maps to the account: `AlternativeMailbox` entries for
/// mailboxes it may open (`Delegate`, `TeamMailbox`), not its archive.
fn alternative_mailboxes(xml: &str) -> Vec<Alternative> {
    let doc = parse(xml);
    let mut alts = Vec::new();
    doc.all("AlternativeMailbox", &mut alts);
    alts.iter()
        .filter(|a| a.text_of("Type").is_some_and(|t| matches!(t.trim(), "Delegate" | "TeamMailbox")))
        .map(|a| {
            let text = |k: &str| a.text_of(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty() && v.len() < 500);
            Alternative {
                name: text("DisplayName").unwrap_or_default(),
                smtp: text("SmtpAddress").filter(|v| v.contains('@')),
                legacy_dn: text("LegacyDN"),
            }
        })
        .take(MAX_SHARED)
        .collect()
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
fn check_message(m: &El) -> Result<()> {
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

/// A rule's id: the element's attribute in the schema, its text from a server that keeps the
/// older shape (Exchange 2016 does).
fn rule_id(r: &El) -> String {
    r.child("RuleId").map(|x| x.attr("Id").map(str::to_string).unwrap_or_else(|| x.text.trim().to_string())).unwrap_or_default()
}

/// A rule as the server keeps it, in Quatrelle's terms when every part of it can be said there.
fn server_rule(r: &El) -> quatrelle_model::organize::ForeignRule {
    use quatrelle_model::organize::{Condition, ForeignRule, Rule, RuleAction};
    let id = rule_id(r);
    let name = r.text_of("DisplayName").unwrap_or("").trim().to_string();
    let enabled = r.text_of("IsEnabled") != Some("false");
    let strings = |el: &El| -> Vec<String> { el.children("String").map(|s| s.text.trim().to_string()).filter(|s| !s.is_empty()).collect() };
    let quoted = |v: &[String]| v.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(", ");
    let mut conditions = Vec::new();
    let mut actions = Vec::new();
    let mut stop = false;
    let mut runnable = true;
    let (mut when, mut then) = (Vec::new(), Vec::new());
    if let Some(c) = r.child("Conditions") {
        for k in &c.kids {
            match k.name.as_str() {
                "ContainsSubjectStrings" => {
                    let v = strings(k);
                    when.push(format!("subject {}", quoted(&v)));
                    match v.len() {
                        1 => conditions.push(Condition::Subject(v[0].clone())),
                        0 => runnable = false,
                        _ => conditions.push(Condition::SubjectAny(v)),
                    }
                }
                "ContainsSenderStrings" => {
                    let v = strings(k);
                    when.push(format!("sender {}", quoted(&v)));
                    match v.len() {
                        1 => conditions.push(Condition::From(v[0].clone())),
                        0 => runnable = false,
                        _ => conditions.push(Condition::FromAny(v)),
                    }
                }
                "ContainsRecipientStrings" => {
                    let v = strings(k);
                    when.push(format!("recipient {}", quoted(&v)));
                    match v.len() {
                        1 => conditions.push(Condition::To(v[0].clone())),
                        0 => runnable = false,
                        _ => conditions.push(Condition::ToAny(v)),
                    }
                }
                // "From these people" / "sent to these people": addresses as Exchange keeps
                // them, which Quatrelle reads by their address.
                "FromAddresses" | "SentToAddresses" => {
                    let who = k.name == "FromAddresses";
                    let people: Vec<String> = k
                        .children("Address")
                        .filter_map(|a| a.text_of("EmailAddress").map(|e| e.trim().to_string()))
                        .filter(|e| e.contains('@'))
                        .collect();
                    when.push(format!("{} {}", if who { "from" } else { "sent to" }, quoted(&people)));
                    match (who, people.len()) {
                        (true, 1) => conditions.push(Condition::From(people[0].clone())),
                        (false, 1) => conditions.push(Condition::To(people[0].clone())),
                        (true, 0) | (false, 0) => runnable = false,
                        (true, _) => conditions.push(Condition::FromAny(people)),
                        (false, _) => conditions.push(Condition::ToAny(people)),
                    }
                }
                "HasAttachments" if k.text.trim() == "true" => {
                    conditions.push(Condition::HasAttachment);
                    when.push("an attachment".into());
                }
                "ContainsBodyStrings" => {
                    // Quatrelle's rules look at the subject and the addresses, not the body.
                    when.push(format!("body {}", quoted(&strings(k))));
                    runnable = false;
                }
                other => {
                    when.push(other.to_string());
                    runnable = false;
                }
            }
        }
    }
    if let Some(a) = r.child("Actions") {
        for k in &a.kids {
            match k.name.as_str() {
                "Delete" if k.text.trim() == "true" => {
                    actions.push(RuleAction::Delete);
                    then.push("delete".to_string());
                }
                "MarkAsRead" if k.text.trim() == "true" => {
                    actions.push(RuleAction::MarkRead);
                    then.push("mark as read".to_string());
                }
                "AssignCategories" => {
                    for c in strings(k) {
                        actions.push(RuleAction::Categorize(c.clone()));
                        then.push(format!("categorise \"{c}\""));
                    }
                }
                "MoveToFolder" => {
                    // The folder by the id the server knows it by; the account's folders turn it
                    // into a path on the way in (see `worker::path_of_folder_id`).
                    let id = k.child("FolderId").and_then(|f| f.attr("Id")).unwrap_or("").to_string();
                    if id.is_empty() {
                        runnable = false;
                        then.push("move".to_string());
                    } else {
                        actions.push(RuleAction::MoveTo(format!("id:{id}")));
                        then.push("move to one of its folders".to_string());
                    }
                }
                "StopProcessingRules" if k.text.trim() == "true" => stop = true,
                other => {
                    then.push(other.to_string());
                    runnable = false;
                }
            }
        }
    }
    let when = if when.is_empty() { "every message".to_string() } else { when.join(" + ") };
    let then = if then.is_empty() { "nothing".to_string() } else { then.join(", ") };
    let summary = if stop { format!("{when} → {then}, then stops") } else { format!("{when} → {then}") };
    let rule = runnable.then(|| Rule {
        name: name.clone(),
        enabled,
        account: None,
        conditions,
        match_all: true,
        actions,
        stop,
        server: true,
        remote_id: Some(id.clone()),
        priority: r.text_of("Priority").and_then(|p| p.trim().parse().ok()),
    });
    ForeignRule { account: String::new(), id, name, enabled, runnable, rule, summary }
}

/// An element as EWS XML, its names in the `t:` namespace (what a rule sent back must look like).
fn xml_of(el: &El) -> String {
    let attrs: String = el.attrs.iter().filter(|(k, _)| *k != "xmlns" && !k.contains(':')).map(|(k, v)| format!(" {k}=\"{}\"", esc(v))).collect();
    let kids: String = el.kids.iter().map(xml_of).collect();
    format!("<t:{}{attrs}>{}{}</t:{}>", el.name, esc(&el.text), kids, el.name)
}

/// The response message of an operation: the one inside `ResponseMessages`, or — as some servers
/// (Exchange 2016 among them) answer — the response element itself, carrying `ResponseClass`.
fn operation<'a>(doc: &'a El, element: &str) -> Result<&'a El> {
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

/// The most rules Exchange answers with (a mailbox's rule list is small; a hostile one isn't
/// unbounded).
const MAX_RULES: usize = 500;

/// A rule test's values, as Exchange writes a list of them.
fn rule_strings(element: &str, values: &[String]) -> String {
    let list: String = values.iter().filter(|v| !v.trim().is_empty()).map(|v| format!("<t:String>{}</t:String>", esc(v))).collect();
    if list.is_empty() { String::new() } else { format!("<t:{element}>{list}</t:{element}>") }
}

/// A rule as Exchange's `InboxRules` hold it, for `CreateRuleOperation` / `SetRuleOperation`.
/// Only the parts Exchange can express are written; the engine never sends the others.
///
/// Written as the schema has it (`RulePredicatesType`, `RuleActionsType`): each test and action
/// once, in the schema's order, and the yes-or-no ones saying `true`. Exchange refuses anything
/// else with the whole update: `<t:Delete/>` (an empty boolean) or a sender test after a subject
/// test was enough for every rule sent from here to be turned down. The same test twice (two
/// "from" tests) becomes one list, which Exchange matches when any of it does.
fn inbox_rule_xml(r: &Rule, folder: Option<&str>, priority: usize, id: Option<&str>) -> String {
    let mut x = String::from("<t:Rule>");
    if let Some(id) = id {
        x.push_str(&format!("<t:RuleId Id=\"{0}\">{0}</t:RuleId>", esc(id)));
    }
    x.push_str(&format!("<t:DisplayName>{}</t:DisplayName>", esc(&r.name)));
    // Its place among the server's rules: the one it had, so that changing it leaves the others
    // where they are (a new rule goes last).
    x.push_str(&format!("<t:Priority>{}</t:Priority>", r.priority.unwrap_or(priority as u32)));
    x.push_str(&format!("<t:IsEnabled>{}</t:IsEnabled>", r.enabled));
    let (mut senders, mut recipients, mut subjects, mut attachments) = (Vec::new(), Vec::new(), Vec::new(), false);
    for c in &r.conditions {
        match c {
            Condition::From(t) => senders.push(t.clone()),
            Condition::To(t) => recipients.push(t.clone()),
            Condition::Subject(t) => subjects.push(t.clone()),
            Condition::HasAttachment => attachments = true,
            // A list: any of them matches (Exchange's own "from these people").
            Condition::FromAny(v) => senders.extend(v.iter().cloned()),
            Condition::ToAny(v) => recipients.extend(v.iter().cloned()),
            Condition::SubjectAny(v) => subjects.extend(v.iter().cloned()),
            // Exchange knows neither: the engine keeps those rules at home.
            Condition::AnyText(_) | Condition::Bulk => {}
        }
    }
    let unique = |v: &mut Vec<String>| {
        let mut seen = std::collections::HashSet::new();
        v.retain(|t| seen.insert(t.trim().to_lowercase()));
    };
    for v in [&mut senders, &mut recipients, &mut subjects] {
        unique(v);
    }
    x.push_str("<t:Conditions>");
    x.push_str(&rule_strings("ContainsRecipientStrings", &recipients));
    x.push_str(&rule_strings("ContainsSenderStrings", &senders));
    x.push_str(&rule_strings("ContainsSubjectStrings", &subjects));
    if attachments {
        x.push_str("<t:HasAttachments>true</t:HasAttachments>");
    }
    x.push_str("</t:Conditions><t:Actions>");
    let mut categories: Vec<String> = r
        .actions
        .iter()
        .filter_map(|a| match a {
            RuleAction::Categorize(c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    unique(&mut categories);
    x.push_str(&rule_strings("AssignCategories", &categories));
    let has = |want: fn(&RuleAction) -> bool| r.actions.iter().any(want);
    if has(|a| matches!(a, RuleAction::Delete)) {
        x.push_str("<t:Delete>true</t:Delete>");
    }
    if has(|a| matches!(a, RuleAction::MarkRead)) {
        x.push_str("<t:MarkAsRead>true</t:MarkAsRead>");
    }
    if let Some(folder) = folder.filter(|_| has(|a| matches!(a, RuleAction::MoveTo(_) | RuleAction::MoveToRole(_)))) {
        x.push_str(&format!("<t:MoveToFolder><t:FolderId Id=\"{}\"/></t:MoveToFolder>", esc(folder)));
    }
    if r.stop {
        x.push_str("<t:StopProcessingRules>true</t:StopProcessingRules>");
    }
    x.push_str("</t:Actions></t:Rule>");
    x
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
        })
    }
}

/// Where Autodiscover is asked (POX, MS-OXDSCLI): the domain's `autodiscover.` host, then the
/// domain itself.
fn autodiscover_urls(email: &str) -> Vec<String> {
    let domain = email.rsplit_once('@').map(|(_, d)| d.trim().to_ascii_lowercase()).unwrap_or_default();
    if domain.is_empty() || !domain.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        return Vec::new();
    }
    vec![format!("https://autodiscover.{domain}/autodiscover/autodiscover.xml"), format!("https://{domain}/autodiscover/autodiscover.xml")]
}

/// The EWS address in an Autodiscover answer (the outside one first), or the address to ask
/// about instead (a redirect).
fn autodiscover_answer(xml: &str) -> (Option<String>, Option<String>) {
    let doc = parse(xml);
    let Some(account) = doc.find("Account") else { return (None, None) };
    if account.text_of("Action") == Some("redirectAddr") {
        return (None, account.text_of("RedirectAddr").map(|r| r.trim().to_string()));
    }
    let protocols: Vec<&El> = account.children("Protocol").collect();
    let url = ["EXPR", "EXCH", "WEB"]
        .iter()
        .find_map(|kind| protocols.iter().find(|p| p.text_of("Type") == Some(kind)).and_then(|p| p.text_of("EwsUrl")))
        .map(|u| u.trim().to_string())
        .filter(|u| u.starts_with("https://") || (cfg!(test) && u.starts_with("http://127.0.0.1")));
    (url, None)
}

/// Find a mailbox's EWS address with Autodiscover, signed in as the account. `base`: where to
/// ask instead of the usual addresses (tests).
pub fn autodiscover(email: &str, user: &str, password: &str, base: Option<&str>) -> Result<String> {
    let mut address = email.to_string();
    for _ in 0..3 {
        let body = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><Autodiscover xmlns=\"http://schemas.microsoft.com/exchange/autodiscover/outlook/requestschema/2006\">\
             <Request><EMailAddress>{}</EMailAddress><AcceptableResponseSchema>http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a</AcceptableResponseSchema></Request></Autodiscover>",
            esc(&address)
        );
        let urls = base.map_or_else(|| autodiscover_urls(&address), |b| vec![b.to_string()]);
        let mut redirect = None;
        let mut last = NetError::No("Autodiscover found no Exchange server for this address".into());
        for url in urls {
            let c = match Client::new(&url, user, password, email) {
                Ok(c) => c,
                Err(e) => {
                    last = e;
                    continue;
                }
            };
            match c.post(body.clone().into_bytes()) {
                Ok(resp) if resp.ok() => match autodiscover_answer(&String::from_utf8_lossy(&resp.body)) {
                    (Some(url), _) => return Ok(url),
                    (None, Some(r)) => {
                        redirect = Some(r);
                        break;
                    }
                    _ => {}
                },
                Ok(resp) => last = NetError::Protocol(format!("Autodiscover answered HTTP {}", resp.status)),
                Err(NetError::Auth(m)) => return Err(NetError::Auth(m)),
                Err(e) => last = e,
            }
        }
        match redirect {
            Some(r) if r != address && r.contains('@') => address = r,
            _ => return Err(last),
        }
    }
    Err(NetError::No("Autodiscover sent Quatrelle round in circles".into()))
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
        p.child("ExtendedFieldURI").and_then(|u| u.attr("PropertyTag")).is_some_and(|t| t.eq_ignore_ascii_case("0x10f4"))
            && p.text_of("Value").is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
    })
}

/// The read, flag and draft state, asked for with every item.
const FLAG_FIELDS: &str = "<t:FieldURI FieldURI=\"message:IsRead\"/>\
<t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/>\
<t:ExtendedFieldURI PropertyTag=\"0x0E07\" PropertyType=\"Integer\"/>";
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

/// An extended property's value.
fn extended(it: &El, tag: &str) -> Option<i64> {
    it.children("ExtendedProperty")
        .find(|p| p.child("ExtendedFieldURI").and_then(|u| u.attr("PropertyTag")).is_some_and(|t| t.eq_ignore_ascii_case(tag)))
        .and_then(|p| p.text_of("Value"))
        .and_then(|v| v.trim().parse().ok())
}

/// An extended property's value as text.
fn extended_text<'a>(it: &'a El, tag: &str) -> Option<&'a str> {
    it.children("ExtendedProperty")
        .find(|p| p.child("ExtendedFieldURI").and_then(|u| u.attr("PropertyTag")).is_some_and(|t| t.eq_ignore_ascii_case(tag)))
        .and_then(|p| p.text_of("Value"))
}

fn flags(it: &El) -> Flags {
    Flags {
        seen: it.text_of("IsRead").is_some_and(|v| v == "true"),
        flagged: extended(it, "0x1090") == Some(2),
        draft: extended(it, "0x0E07").is_some_and(|f| f & 8 != 0),
        ..Default::default()
    }
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

/// The query in Exchange's own syntax (`QueryString`, its Advanced Query Syntax): the words, and
/// the fields Quatrelle's queries name that Exchange knows too. A clause it has no word for
/// (`is:unread`, `keyword:…`) is left to the local index rather than sent as nonsense.
fn aqs_of(query: &str) -> String {
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
fn restriction(q: &WordQuery) -> String {
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

fn item_ids(ids: &[String]) -> String {
    ids.iter().map(|id| format!("<t:ItemId Id=\"{}\"/>", esc(id))).collect()
}

fn folder_ref(id: Option<&str>) -> String {
    match id {
        Some(id) => format!("<t:FolderId Id=\"{}\"/>", esc(id)),
        None => "<t:DistinguishedFolderId Id=\"msgfolderroot\"/>".into(),
    }
}

/// Apply one change on the server.
/// Vote on a message (see [`Client::vote`]).
pub fn vote(c: &Client, st: &State, key: &MessageKey, choice: &str, subject: &str) -> Result<()> {
    let id = st.id(key.uid).filter(|_| key.validity == VALIDITY).ok_or_else(|| NetError::No("the message is no longer on the server".into()))?;
    c.vote(id, choice, subject)
}

pub fn apply_remote(c: &Client, st: &State, op: &Op) -> Result<()> {
    let ids = |keys: &[MessageKey]| -> Vec<(String, String)> {
        keys.iter().filter(|k| k.validity == VALIDITY).filter_map(|k| Some((st.id(k.uid)?.to_string(), k.folder.clone()))).collect()
    };
    let update = |keys: &[MessageKey], change: &str| -> Result<()> {
        for chunk in ids(keys).chunks(BATCH) {
            let changes: String = chunk
                .iter()
                .map(|(id, _)| format!("<t:ItemChange><t:ItemId Id=\"{}\"/><t:Updates>{change}</t:Updates></t:ItemChange>", esc(id)))
                .collect();
            c.call_all(&format!(
                "<m:UpdateItem MessageDisposition=\"SaveOnly\" ConflictResolution=\"AlwaysOverwrite\"><m:ItemChanges>{changes}</m:ItemChanges></m:UpdateItem>"
            ))?;
        }
        Ok(())
    };
    match op {
        Op::SetFlag { keys, flag: Flag::Seen, on } => update(
            keys,
            &format!("<t:SetItemField><t:FieldURI FieldURI=\"message:IsRead\"/><t:Message><t:IsRead>{on}</t:IsRead></t:Message></t:SetItemField>"),
        ),
        Op::SetFlag { keys, flag: Flag::Flagged, on: true } => update(
            keys,
            "<t:SetItemField><t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/><t:Message><t:ExtendedProperty>\
             <t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/><t:Value>2</t:Value></t:ExtendedProperty></t:Message></t:SetItemField>",
        ),
        Op::SetFlag { keys, flag: Flag::Flagged, on: false } => {
            update(keys, "<t:DeleteItemField><t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/></t:DeleteItemField>")
        }
        // Answered and forwarded are Exchange's to set; categories stay on this computer.
        Op::SetFlag { .. } | Op::SetKeyword { .. } => Ok(()),
        Op::Move { keys, to } => {
            let dest = st.mailbox(to)?.to_string();
            let moving: Vec<String> = ids(keys).into_iter().filter(|(_, from)| from != to).map(|(id, _)| id).collect();
            for chunk in moving.chunks(BATCH) {
                c.call_all(&format!(
                    "<m:MoveItem><m:ToFolderId>{}</m:ToFolderId><m:ItemIds>{}</m:ItemIds></m:MoveItem>",
                    folder_ref(Some(&dest)),
                    item_ids(chunk)
                ))?;
            }
            Ok(())
        }
        Op::Expunge { keys } => {
            let gone: Vec<String> = ids(keys).into_iter().map(|(id, _)| id).collect();
            for chunk in gone.chunks(BATCH) {
                c.call_all(&format!("<m:DeleteItem DeleteType=\"HardDelete\"><m:ItemIds>{}</m:ItemIds></m:DeleteItem>", item_ids(chunk)))?;
            }
            Ok(())
        }
        Op::CreateFolder { path } => {
            let (parent, name) = split_path(st, path)?;
            c.call_one(&format!(
                "<m:CreateFolder><m:ParentFolderId>{}</m:ParentFolderId><m:Folders><t:Folder><t:FolderClass>IPF.Note</t:FolderClass>\
                 <t:DisplayName>{}</t:DisplayName></t:Folder></m:Folders></m:CreateFolder>",
                folder_ref(parent.as_deref()),
                esc(&name)
            ))
            .map(drop)
        }
        Op::RenameFolder { from, to } => {
            let mut id = st.mailbox(from)?.to_string();
            let (old_parent, _) = split_path(st, from)?;
            let (parent, name) = split_path(st, to)?;
            if parent != old_parent {
                let m = c.call_one(&format!(
                    "<m:MoveFolder><m:ToFolderId>{}</m:ToFolderId><m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:MoveFolder>",
                    folder_ref(parent.as_deref()),
                    esc(&id)
                ))?;
                if let Some(new) = m.find("FolderId").and_then(|f| f.attr("Id")) {
                    id = new.to_string();
                }
            }
            c.call_one(&format!(
                "<m:UpdateFolder><m:FolderChanges><t:FolderChange><t:FolderId Id=\"{}\"/><t:Updates><t:SetFolderField>\
                 <t:FieldURI FieldURI=\"folder:DisplayName\"/><t:Folder><t:DisplayName>{}</t:DisplayName></t:Folder></t:SetFolderField>\
                 </t:Updates></t:FolderChange></m:FolderChanges></m:UpdateFolder>",
                esc(&id),
                esc(&name)
            ))
            .map(drop)
        }
        Op::DeleteFolder { path } => {
            let id = st.mailbox(path)?.to_string();
            c.call_one(&format!(
                "<m:DeleteFolder DeleteType=\"MoveToDeletedItems\"><m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:DeleteFolder>",
                esc(&id)
            ))
            .map(drop)
        }
        Op::EmptyFolder { path } => {
            let id = st.mailbox(path)?.to_string();
            c.call_one(&format!(
                "<m:EmptyFolder DeleteType=\"HardDelete\" DeleteSubFolders=\"false\"><m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:EmptyFolder>",
                esc(&id)
            ))
            .map(drop)
        }
    }
}

/// The parent folder id (`None` at the top) and the leaf name of a folder path.
fn split_path(st: &State, path: &str) -> Result<(Option<String>, String)> {
    match path.rsplit_once('/') {
        Some((parent, leaf)) => Ok((Some(st.mailbox(parent)?.to_string()), utf7::decode(leaf))),
        None => Ok((None, utf7::decode(path))),
    }
}

/// Store a draft in a folder.
pub fn append(c: &Client, st: &State, folder: &str, raw: &[u8]) -> Result<String> {
    c.create(raw, st.mailbox(folder)?, true)
}

/// Send a message. Exchange reads the recipients from the MIME source, so anyone only on the
/// envelope (Bcc) goes into a `Bcc:` header, which Exchange removes before delivery.
pub fn send(c: &Client, raw: &[u8], recipients: &[String]) -> Result<()> {
    let shown: Vec<String> = mime::parse(raw).map(|p| p.to.iter().chain(&p.cc).map(|a| a.email.to_ascii_lowercase()).collect()).unwrap_or_default();
    let bcc: Vec<&str> = recipients.iter().map(String::as_str).filter(|r| !shown.contains(&r.to_ascii_lowercase())).collect();
    if bcc.is_empty() {
        return c.send_mime(raw);
    }
    let mut with = format!("Bcc: {}\r\n", bcc.join(", ")).into_bytes();
    with.extend_from_slice(raw);
    c.send_mime(&with)
}

mod mailbox;
mod requests;
mod shared;

#[cfg(test)]
mod tests;
