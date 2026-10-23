//! An in-memory Microsoft Graph mail API on localhost, for tests: mail folders (well-known
//! names, children, paging), messages (list, get, patch, move, delete, `$value`, made from
//! MIME), `sendMail`, mailbox settings and JSON batches, with a bearer token.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

pub const TOKEN: &str = "test-token";
const ROOT: &str = "root";

#[derive(Clone, Debug)]
pub struct Fo {
    pub id: String,
    pub name: String,
    pub parent: String,
    /// The well-known name (`inbox`, `sentitems`…).
    pub known: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Msg {
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

#[derive(Default)]
pub struct Data {
    pub folders: Vec<Fo>,
    pub messages: Vec<Msg>,
    /// MIME sources handed to `sendMail`.
    pub sent: Vec<Vec<u8>>,
    /// Votes sent: (subject, choice).
    pub votes: Vec<(String, String)>,
    pub replies: Value,
    /// Calendars (id, name), events (with a `calendar` field) and contacts, as Graph has them.
    pub calendars: Vec<(String, String)>,
    pub events: Vec<Value>,
    pub contacts: Vec<Value>,
    /// Contacts that carry a picture (their id), as Outlook's do.
    pub contact_photos: Vec<String>,
    /// Every request, as `METHOD path`.
    pub calls: Vec<String>,
    /// The signed-in Microsoft account has no mailbox.
    pub no_mailbox: bool,
    /// File a sent message's copy in Sent Items a moment after `sendMail`, as Graph does.
    pub file_late: bool,
    /// Other people whose calendars are shared with this account: (address, may edit). Their
    /// events have `calendar` = `user:<address>`.
    pub others: Vec<(String, bool)>,
    /// Other mailboxes this account may open (shared, or as a delegate), reached under
    /// `/users/<address>`; their folders hang under `root-<address>`.
    pub shared: Vec<String>,
    /// Whose mailbox the request being answered is about (`None`: `/me`).
    ctx: Option<String>,
    /// The user's calendar groups: (id, name, the calendars in it).
    pub groups: Vec<(String, String, Vec<String>)>,
    /// Calendars shared with the user and kept only in a group: (id, name, owner, may edit).
    pub group_calendars: Vec<(String, String, String, bool)>,
    /// Messages whose source can't be had (the server fails on it).
    pub unreadable: Vec<String>,
    /// Per-batch concurrency cap: entries past this index get a 429 with Retry-After.
    pub at_once: Option<usize>,
    /// Number of upcoming top-level requests to refuse with 429.
    pub throttle: u32,
    /// Room lists (name, address) and rooms (name, address, the list's address), as the
    /// directory has them; `places_refused`: the Places API is refused (no `Place.Read.All`),
    /// so only the older `findRoomLists`/`findRooms` answer.
    pub room_lists: Vec<(String, String)>,
    pub rooms: Vec<(String, String, String)>,
    pub places_refused: bool,
    /// Busy stretches besides the calendars' events: (address, start, end, status, subject).
    pub busy: Vec<(String, String, String, String, String)>,
    /// Mailboxes whose free/busy this account may not read.
    pub no_freebusy: Vec<String>,
    /// The `getSchedule` bodies seen.
    pub schedule_asked: Vec<Value>,
    /// The one bearer token taken, when not [`TOKEN`] (a test renews it as tokens expire).
    pub token: Option<String>,
    /// Set per request from its `Prefer` header; without `IdType="ImmutableId"` message ids
    /// are returned in the other (prefixed) form.
    asked_immutable: bool,
    /// A folder's messages listed at most this many a page (`None`: all of `$top` at once), the
    /// rest through `@odata.nextLink`.
    pub page_messages: Option<usize>,
    late: Vec<(std::time::Instant, Vec<u8>)>,
    next: u32,
}

impl Data {
    fn id(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    /// A folder of another mailbox (`parent` `None`: at its top).
    pub fn shared_folder(&mut self, owner: &str, name: &str, parent: Option<&str>, known: Option<&str>) -> String {
        let top = format!("root-{owner}");
        self.folder(name, Some(parent.unwrap_or(&top)), known)
    }

    /// Whose mailbox a folder is in (`None`: this account's).
    fn owner_of(&self, folder: &str) -> Option<String> {
        let mut at = folder.to_string();
        for _ in 0..32 {
            if let Some(o) = at.strip_prefix("root-") {
                return Some(o.to_string());
            }
            at = self.folders.iter().find(|f| f.id == at)?.parent.clone();
        }
        None
    }

    /// The top of the mailbox being answered for.
    fn top(&self) -> String {
        self.ctx.as_ref().map_or_else(|| ROOT.to_string(), |o| format!("root-{o}"))
    }

    pub fn folder(&mut self, name: &str, parent: Option<&str>, known: Option<&str>) -> String {
        let id = self.id("F");
        self.folders.push(Fo { id: id.clone(), name: name.into(), parent: parent.unwrap_or(ROOT).into(), known: known.map(str::to_string) });
        id
    }

    pub fn deliver(&mut self, folder: &str, raw: &[u8], read: bool, received: &str) -> String {
        let id = self.id("M");
        self.messages.push(Msg {
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

    pub fn message(&self, id: &str) -> Option<&Msg> {
        let id = plain_id(id);
        self.messages.iter().find(|m| m.id == id)
    }

    pub fn by_name(&self, name: &str) -> Option<&Fo> {
        self.folders.iter().find(|f| f.name == name)
    }

    /// A folder by id or well-known name, in the mailbox being answered for.
    fn resolve(&self, id: &str) -> Option<String> {
        self.folders.iter().find(|f| (f.id == id || f.known.as_deref() == Some(id)) && self.owner_of(&f.id) == self.ctx).map(|f| f.id.clone())
    }

    fn folder_json(&self, f: &Fo) -> Value {
        let inside: Vec<&Msg> = self.messages.iter().filter(|m| m.folder == f.id).collect();
        json!({
            "id": f.id, "displayName": f.name, "parentFolderId": f.parent,
            "childFolderCount": self.folders.iter().filter(|c| c.parent == f.id).count(),
            "totalItemCount": inside.len(), "unreadItemCount": inside.iter().filter(|m| !m.read).count(),
        })
    }

    fn message_json(&self, m: &Msg) -> Value {
        let p = quatrelle_mime::parse(&m.raw);
        let addr = |a: &quatrelle_model::EmailAddress| json!({"emailAddress": {"name": a.name, "address": a.email}});
        let mut v = json!({
            "id": if self.asked_immutable { m.id.clone() } else { format!("{REGULAR}{}", m.id) }, "parentFolderId": m.folder, "isRead": m.read, "isDraft": m.draft,
            "flag": {"flagStatus": if m.flagged { "flagged" } else { "notFlagged" }},
            "subject": p.as_ref().map(|p| p.subject.clone()),
            "from": p.as_ref().map(|p| addr(&p.from)),
            "toRecipients": p.as_ref().map(|p| p.to.iter().map(addr).collect::<Vec<_>>()),
            "ccRecipients": [],
            "receivedDateTime": m.received,
            "bodyPreview": p.as_ref().map(|p| p.text.chars().take(60).collect::<String>()),
            "hasAttachments": p.as_ref().is_some_and(|p| !p.attachments.is_empty()),
            "internetMessageId": p.as_ref().map(|p| format!("<{}>", p.message_id)),
        });
        if !m.voting.is_empty() {
            v["singleValueExtendedProperties"] =
                json!([{"id": "Binary {00062008-0000-0000-c000-000000000046} Id 0x8520", "value": crate::imap::base64(&m.voting)}]);
        }
        v
    }
}

/// A message's header lines, as Graph's `internetMessageHeaders` gives them.
fn header_lines(raw: &[u8]) -> Value {
    let text = String::from_utf8_lossy(raw);
    let head = text.split("\r\n\r\n").next().unwrap_or_default().replace("\r\n ", " ").replace("\r\n\t", " ");
    let lines: Vec<Value> =
        head.split("\r\n").filter_map(|l| l.split_once(':')).map(|(name, value)| json!({"name": name.trim(), "value": value.trim()})).collect();
    json!(lines)
}

/// A `$filter` condition `receivedDateTime ge <ISO 8601 UTC time>`: the time in Unix seconds
/// (`Some(None)`: that condition, with a time Graph wouldn't take; `None`: another condition).
fn received_since(cond: &str) -> Option<Option<i64>> {
    match cond.split_whitespace().collect::<Vec<_>>().as_slice() {
        [name, op, at] if name.eq_ignore_ascii_case("receivedDateTime") && op.eq_ignore_ascii_case("ge") => {
            Some(at.strip_suffix('Z').filter(|t| t.len() >= 19).and_then(|_| quatrelle_jmap::utc_date(at)))
        }
        _ => None,
    }
}

/// Whether a message meets a KQL (Graph's `$search`) or AQS (EWS's `QueryString`) query, as
/// far as Quatrelle writes them: words and `"phrases"`, `from:` `to:` `subject:` `body:`, `NOT`;
/// a word as a part of what's searched, case aside.
pub(crate) fn words_match(query: &str, raw: &[u8]) -> bool {
    let p = quatrelle_mime::parse(raw);
    let field = |f: &str| -> String {
        let Some(p) = &p else { return String::new() };
        let addr = |a: &quatrelle_model::EmailAddress| format!("{} {}", a.name, a.email);
        match f {
            "from" => addr(&p.from),
            "to" => p.to.iter().map(addr).collect::<Vec<_>>().join(" "),
            "subject" => p.subject.clone(),
            "body" => p.text.clone(),
            _ => format!("{} {} {}", addr(&p.from), p.subject, p.text),
        }
        .to_lowercase()
    };
    // Tokens, a quoted part kept whole.
    let mut tokens: Vec<String> = Vec::new();
    let (mut cur, mut quoted) = (String::new(), false);
    for c in query.chars() {
        match c {
            '"' => quoted = !quoted,
            ' ' if !quoted => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    let mut not = false;
    for t in tokens {
        if t == "NOT" {
            not = true;
            continue;
        }
        let (f, w) = match t.split_once(':') {
            Some((f, w)) if ["from", "to", "subject", "body"].contains(&f) => (f, w),
            _ => ("", t.as_str()),
        };
        if field(f).contains(&w.to_lowercase()) == not {
            return false;
        }
        not = false;
    }
    true
}

fn unbase64(body: &[u8]) -> Vec<u8> {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in body {
        let Some(v) = A.iter().position(|a| a == c) else { continue };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

pub struct Server {
    /// The API address (`…/v1.0`).
    pub base: String,
    pub data: Arc<Mutex<Data>>,
}

/// Prefix that turns a stored id into its non-immutable spelling.
const REGULAR: &str = "AAMk-";

/// Strip [`REGULAR`] if present.
fn plain_id(id: &str) -> &str {
    id.strip_prefix(REGULAR).unwrap_or(id)
}

fn respond(s: &mut impl Write, status: u16, body: &[u8], kind: &str) {
    let mut head = format!("HTTP/1.1 {status} X\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n", body.len());
    if status == 429 {
        head.push_str("Retry-After: 1\r\n");
    }
    head.push_str("Connection: close\r\n\r\n");
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

/// Start the server; it runs until the test process ends.
pub fn serve(data: Data) -> Server {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}/v1.0", l.local_addr().unwrap().port());
    let data = Arc::new(Mutex::new(data));
    let (shared, b) = (data.clone(), base.clone());
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(mut s) = conn else { continue };
            let mut reader = BufReader::new(s.try_clone().unwrap());
            let mut first = String::new();
            if reader.read_line(&mut first).unwrap_or(0) == 0 {
                continue;
            }
            let (mut len, mut authorized, mut immutable) = (0usize, false, false);
            let bearer = format!("Bearer {}", shared.lock().unwrap().token.clone().unwrap_or_else(|| TOKEN.into()));
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                authorized |= line.split_once(':').is_some_and(|(k, v)| k.eq_ignore_ascii_case("authorization") && v.trim() == bearer);
                immutable |= line.starts_with("Prefer: IdType=\"ImmutableId\"");
            }
            let mut body = vec![0u8; len];
            let _ = reader.read_exact(&mut body);
            if !authorized {
                respond(&mut s, 401, br#"{"error": {"code": "InvalidAuthenticationToken"}}"#, "application/json");
                continue;
            }
            assert!(immutable, "ids are asked for in their immutable form");
            let mut words = first.split(' ');
            let (method, target) = (words.next().unwrap_or("GET").to_string(), words.next().unwrap_or("/").to_string());
            let target = target.strip_prefix("/v1.0").unwrap_or(&target).to_string();
            let (status, out, kind) = {
                let mut d = shared.lock().unwrap();
                if d.throttle > 0 {
                    d.throttle -= 1;
                    (429, br#"{"error": {"code": "TooManyRequests"}}"#.to_vec(), "application/json")
                } else {
                    d.asked_immutable = true;
                    d.route(&method, &target, &body, &b)
                }
            };
            respond(&mut s, status, &out, kind);
        }
    });
    Server { base, data }
}

mod route;

#[cfg(test)]
mod tests;
