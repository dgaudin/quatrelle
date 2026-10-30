//! A client for Graph's chat API (v1.0): the signed-in user, chats, their members and
//! messages, posting and editing text, reactions, soft deletes, marking a chat read, and
//! counting the bytes of a file shared in a chat.
//!
//! Every request carries the bearer token and goes on a connection of its own. No redirect is
//! followed with the token: a Graph answer that redirects is an error, except a shared file's
//! content, whose one redirect (to SharePoint's pre-signed download address) is followed by
//! hand without it. A 429 or a 503 is waited out ([`BUSY_ROUNDS`] rounds at most, `Retry-After`
//! capped at 20 s; a post of a message only on a 429, as a 503 may come after it was made) and
//! every 429 counted in [`Stats`]. Errors say the HTTP status and Graph's
//! error code, never its message, which can quote a chat's content.

use std::cell::Cell;
use std::io::Write;

use serde_json::{Value, json};

use crate::graph::{BUSY_ROUNDS, backoff_secs, seg, utc};
use crate::http::{self, Request, Response, Url};
use crate::stream::{self, Stream};
use crate::{NetError, Result};

/// Largest JSON answer accepted.
const MAX_JSON: usize = 8 << 20;
/// Sent with every message read: reactions, message types and the like added to Graph after
/// v1.0 was published come as they are rather than as `unknownFutureValue`.
const PREFER_ENUMS: &str = "include-unknown-enum-members";
/// Chats and messages asked for per page (the most Graph gives).
const TOP: u32 = 50;
/// Pages of a chat's members followed at most.
const MEMBER_PAGES: usize = 20;
/// Longest shared file address taken.
const MAX_SHARE_URL: usize = 8 << 10;
/// What is kept of an error's body (for its code).
const KEEP_ERROR: usize = 64 << 10;
/// Longest Graph error code repeated in an error.
const MAX_CODE: usize = 64;

/// The signed-in user.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Me {
    /// The Microsoft Entra object id.
    pub id: String,
    pub display_name: String,
    /// Empty when the account has none.
    pub mail: String,
    /// The user principal name (the sign-in name).
    pub upn: String,
}

/// One page of a listing: its items as Graph gives them, and where the next page is.
#[derive(Clone, Default)]
pub struct Page {
    pub items: Vec<Value>,
    /// Graph's `@odata.nextLink`, as it came: followed only on the client's own address.
    pub next: Option<String>,
}

/// What a client has sent so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Requests sent, retries and a shared file's download included.
    pub requests: u32,
    /// Answers that were 429 (Too Many Requests).
    pub throttled: u32,
}

/// The messages to list, by `lastModifiedDateTime` (an RFC 3339 time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Range<'a> {
    /// Changed after this time.
    After(&'a str),
    /// Changed before this time.
    Before(&'a str),
}

/// A signed-in Graph session for Teams chats. Not shared between threads (its counters are
/// plain cells).
pub struct TeamsClient {
    /// The API address, without a trailing `/`.
    base: String,
    /// `Bearer <token>`: held in memory only, never printed.
    auth: String,
    stats: Cell<Stats>,
    /// The server's clock at its last answer that said it (Unix seconds).
    date: Cell<Option<i64>>,
}

impl TeamsClient {
    /// Check the token by reading the signed-in user.
    pub fn connect(base: &str, token: &str) -> Result<(TeamsClient, Me)> {
        let base = base.trim().trim_end_matches('/').to_string();
        let url = Url::parse(&base)?;
        if !url.https && !loopback(&url) {
            return Err(NetError::Tls("the Graph address isn't https".into()));
        }
        let c = TeamsClient { base, auth: format!("Bearer {token}"), stats: Cell::new(Stats::default()), date: Cell::new(None) };
        let v = c.json("GET", &c.at("/me?$select=id,displayName,mail,userPrincipalName"), None, false)?;
        let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().chars().take(320).collect::<String>();
        let me = Me { id: text("id"), display_name: text("displayName"), mail: text("mail"), upn: text("userPrincipalName") };
        if me.id.is_empty() {
            return Err(NetError::Protocol("Graph: the signed-in user has no id".into()));
        }
        Ok((c, me))
    }

    /// What this client has sent so far.
    pub fn stats(&self) -> Stats {
        self.stats.get()
    }

    /// The server's clock (Unix seconds) at its last answer that gave it (`Date`): what times
    /// on the server are measured against, never this computer's clock.
    pub fn server_time(&self) -> Option<i64> {
        self.date.get()
    }

    /// A page of the signed-in user's chats, the most recently active first, each with a
    /// preview of its last message; `next` is the previous page's [`Page::next`].
    pub fn chats(&self, next: Option<&str>) -> Result<Page> {
        let url = match next {
            Some(n) => self.own(n)?,
            None => self.at(&format!("/me/chats?$top={TOP}&$expand=lastMessagePreview&$orderby=lastMessagePreview/createdDateTime%20desc")),
        };
        page(self.json("GET", &url, None, false)?)
    }

    /// A chat's members, every page.
    pub fn members(&self, chat: &str) -> Result<Vec<Value>> {
        let mut url = self.at(&format!("/chats/{}/members", seg(chat)));
        let mut out = Vec::new();
        for _ in 0..MEMBER_PAGES {
            let p = page(self.json("GET", &url, None, false)?)?;
            out.extend(p.items);
            match p.next {
                Some(n) => url = self.own(&n)?,
                None => return Ok(out),
            }
        }
        Ok(out)
    }

    /// A page of a chat's messages changed in `range`, the last changed first; `next` is the
    /// previous page's [`Page::next`] (it carries the range).
    pub fn messages(&self, chat: &str, range: Range<'_>, next: Option<&str>) -> Result<Page> {
        let url = match next {
            Some(n) => self.own(n)?,
            None => {
                let (op, at) = match range {
                    Range::After(t) => ("gt", t),
                    Range::Before(t) => ("lt", t),
                };
                let at = filter_time(at).ok_or_else(|| NetError::Refused("a message range needs an RFC 3339 time".into()))?;
                let filter = http::pct(&format!("lastModifiedDateTime {op} {at}"));
                self.at(&format!("/chats/{}/messages?$filter={filter}&$top={TOP}&$orderby=lastModifiedDateTime%20desc", seg(chat)))
            }
        };
        page(self.json("GET", &url, None, true)?)
    }

    /// Post a plain-text message in a chat: the message Graph made. Waited out on a 429 only
    /// (nothing was done); a 503 may come after the message was made, so it isn't posted again.
    pub fn send_text(&self, chat: &str, text: &str) -> Result<Value> {
        let body = json!({"body": {"contentType": "text", "content": text}});
        self.request("POST", &self.at(&format!("/chats/{}/messages", seg(chat))), Some(&body), false, false)
    }

    /// Replace a message's text (one the signed-in user sent).
    pub fn edit_text(&self, chat: &str, msg: &str, text: &str) -> Result<()> {
        let body = json!({"body": {"contentType": "text", "content": text}});
        self.json("PATCH", &self.at(&format!("/chats/{}/messages/{}", seg(chat), seg(msg))), Some(&body), false).map(drop)
    }

    /// React to a message (`like`, `heart`… or an emoji).
    pub fn set_reaction(&self, chat: &str, msg: &str, emoji: &str) -> Result<()> {
        self.react("setReaction", chat, msg, emoji)
    }

    /// Take a reaction back.
    pub fn unset_reaction(&self, chat: &str, msg: &str, emoji: &str) -> Result<()> {
        self.react("unsetReaction", chat, msg, emoji)
    }

    fn react(&self, verb: &str, chat: &str, msg: &str, emoji: &str) -> Result<()> {
        let url = self.at(&format!("/chats/{}/messages/{}/{verb}", seg(chat), seg(msg)));
        self.json("POST", &url, Some(&json!({"reactionType": emoji})), false).map(drop)
    }

    /// Delete a message the signed-in user sent (it stays listed, with its deletion time):
    /// through `/users/{user}` (the signed-in user's object id), or `/me` when `None`.
    pub fn soft_delete(&self, chat: &str, msg: &str, user: Option<&str>) -> Result<()> {
        let who = user.map_or_else(|| "/me".to_string(), |u| format!("/users/{}", seg(u)));
        self.json("POST", &self.at(&format!("{who}/chats/{}/messages/{}/softDelete", seg(chat), seg(msg))), None, false).map(drop)
    }

    /// Mark a chat read for the signed-in user (`user`: their object id, in `tenant`).
    pub fn mark_read(&self, chat: &str, user: &str, tenant: &str) -> Result<()> {
        let body = json!({"user": {"id": user, "tenantId": tenant}});
        self.json("POST", &self.at(&format!("/chats/{}/markChatReadForUser", seg(chat))), Some(&body), false).map(drop)
    }

    /// Download a file shared in a chat, by the `contentUrl` its attachment carries, counting
    /// its bytes and keeping none: Graph's `/shares/u!…/driveItem/content`, then its one
    /// redirect followed without the token. More than `max` bytes is refused.
    pub fn download_shared(&self, content_url: &str, max: usize) -> Result<usize> {
        let content_url = content_url.trim();
        if content_url.is_empty() || content_url.len() > MAX_SHARE_URL {
            return Err(NetError::Refused("not a shared file's address".into()));
        }
        let share = crate::imap::base64(content_url.as_bytes()).trim_end_matches('=').replace('/', "_").replace('+', "-");
        let first = self.at(&format!("/shares/u!{share}/driveItem/content"));
        let req = Request::get(&first, max).header("Authorization", &self.auth);
        let mut round = 0;
        let (head, tally) = loop {
            round += 1;
            let (head, tally) = self.download(&req, true)?;
            if !busy(head.status) || round >= BUSY_ROUNDS {
                break (head, tally);
            }
            pause(backoff_secs(head.header("Retry-After"), round));
        };
        if head.ok() {
            // Graph served the file itself.
            return Ok(tally.bytes);
        }
        let Some(location) = redirect(&head) else {
            status_error(head.status, &tally.kept)?;
            return Err(NetError::Protocol(format!("HTTP {}", head.status)));
        };
        let to = Url::parse(&first)?.join(location)?;
        if !to.https && !loopback(&to) {
            return Err(NetError::Tls("the shared file's download isn't https".into()));
        }
        // A request of its own: the token stays with Graph.
        let (head, tally) = self.download(&Request::get(&format!("{}{}", to.origin(), to.path), max), false)?;
        if redirect(&head).is_some() {
            return Err(NetError::Protocol("the shared file's download redirected again".into()));
        }
        if !head.ok() {
            return Err(NetError::Protocol(format!("HTTP {}", head.status)));
        }
        Ok(tally.bytes)
    }
}

// Requests.
impl TeamsClient {
    /// The full address of a path under the API.
    fn at(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// A next page's address, when it is under this client's own API address.
    fn own(&self, next: &str) -> Result<String> {
        let inside = next.starts_with(&format!("{}/", self.base)) && !next.chars().any(|c| c.is_whitespace() || c.is_control());
        if inside && Url::parse(next).is_ok() {
            Ok(next.to_string())
        } else {
            Err(NetError::Protocol("Graph: a next page outside its address".into()))
        }
    }

    /// A request to Graph with the token, waited out while busy: its JSON answer (`Null` when
    /// empty).
    fn json(&self, method: &str, url: &str, body: Option<&Value>, prefer: bool) -> Result<Value> {
        self.request(method, url, body, prefer, true)
    }

    /// [`Self::json`], a 503 waited out only when `again` (the request can be repeated).
    fn request(&self, method: &str, url: &str, body: Option<&Value>, prefer: bool, again: bool) -> Result<Value> {
        let mut req = match body {
            Some(b) => {
                let bytes = serde_json::to_vec(b).map_err(|e| NetError::Protocol(e.to_string()))?;
                Request::post(url, "application/json", bytes, MAX_JSON)
            }
            None => Request::get(url, MAX_JSON),
        };
        req.method = method.into();
        req = req.header("Authorization", &self.auth).header("Accept", "application/json");
        if prefer {
            req = req.header("Prefer", PREFER_ENUMS);
        }
        let mut round = 0;
        let resp = loop {
            round += 1;
            let resp = self.once(&req)?;
            if !busy(resp.status) || (resp.status == 503 && !again) || round >= BUSY_ROUNDS {
                break resp;
            }
            pause(backoff_secs(resp.header("Retry-After"), round));
        };
        status_error(resp.status, &resp.body)?;
        if resp.body.iter().all(u8::is_ascii_whitespace) {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&resp.body).map_err(|e| NetError::Protocol(format!("Graph: {e}")))
    }

    /// One request on a connection of its own, its answer as it came (a redirect isn't followed).
    fn once(&self, req: &Request) -> Result<Response> {
        let url = Url::parse(&req.url)?;
        let mut s = connect(&url)?;
        self.count();
        let resp = http::exchange(&mut s, req, &url)?;
        self.heard(&resp, true);
        Ok(resp)
    }

    /// A GET whose body is counted as it comes and not kept (only the start of an error's, for
    /// its code); a redirect's body is left unread.
    fn download(&self, req: &Request, graph: bool) -> Result<(Response, Tally)> {
        let url = Url::parse(&req.url)?;
        let mut s = connect(&url)?;
        self.count();
        let mut tally = Tally::default();
        let sink = &mut tally;
        let head = http::exchange_into(&mut s, req, &url, move |head: &Response| {
            if busy(head.status) {
                return Ok(None);
            }
            if head.ok()
                && let Some(len) = head.header("Content-Length").and_then(|l| l.trim().parse::<u64>().ok())
                && len > u64::try_from(req.max).unwrap_or(u64::MAX)
            {
                return Err(too_large(req.max));
            }
            sink.keep = !head.ok();
            Ok(Some(sink))
        })
        .map_err(|e| match e {
            NetError::Protocol(m) if m.starts_with("the download is larger") => too_large(req.max),
            e => e,
        })?;
        self.heard(&head, graph);
        Ok((head, tally))
    }

    /// One more request sent.
    fn count(&self) {
        let mut s = self.stats.get();
        s.requests = s.requests.saturating_add(1);
        self.stats.set(s);
    }

    /// An answer in: a 429 counted, and Graph's clock noted.
    fn heard(&self, resp: &Response, graph: bool) {
        if resp.status == 429 {
            let mut s = self.stats.get();
            s.throttled = s.throttled.saturating_add(1);
            self.stats.set(s);
        }
        if graph && let Some(t) = resp.date() {
            self.date.set(Some(t));
        }
    }
}

/// Where a download goes: its bytes counted, and only the start of an error's body kept.
#[derive(Default)]
struct Tally {
    bytes: usize,
    keep: bool,
    kept: Vec<u8>,
}

impl Write for Tally {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes = self.bytes.saturating_add(buf.len());
        if self.keep {
            let room = KEEP_ERROR.saturating_sub(self.kept.len()).min(buf.len());
            self.kept.extend_from_slice(buf.get(..room).unwrap_or_default());
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn too_large(max: usize) -> NetError {
    NetError::Protocol(format!("the shared file is larger than {max} bytes"))
}

/// A connection to a URL's server: TLS for `https`.
fn connect(url: &Url) -> Result<Stream> {
    let sock = stream::tcp(&url.host, url.port)?;
    if url.https { stream::tls(sock, &url.host) } else { Ok(Stream::Plain(sock)) }
}

/// This computer: the only place plain HTTP is spoken to (the tests' servers).
fn loopback(url: &Url) -> bool {
    matches!(url.host.as_str(), "127.0.0.1" | "localhost" | "::1")
}

/// Whether an answer asks to try again later.
fn busy(status: u16) -> bool {
    matches!(status, 429 | 503)
}

fn pause(secs: u64) {
    std::thread::sleep(std::time::Duration::from_secs(secs));
}

/// Where a redirect sends a GET, if the answer is one.
fn redirect(head: &Response) -> Option<&str> {
    head.header("Location").filter(|_| matches!(head.status, 301 | 302 | 303 | 307 | 308))
}

/// A listing's page from Graph's answer.
fn page(v: Value) -> Result<Page> {
    let items = match v.get("value") {
        Some(Value::Array(a)) => a.clone(),
        _ => return Err(NetError::Protocol("Graph: a listing without its values".into())),
    };
    let next = v.get("@odata.nextLink").and_then(Value::as_str).map(str::to_string);
    Ok(Page { items, next })
}

/// An HTTP failure as an error: the status and Graph's error code (when it looks like one),
/// never the error's message, which can quote what the user wrote.
pub(crate) fn status_error(status: u16, body: &[u8]) -> Result<()> {
    if (200..300).contains(&status) {
        return Ok(());
    }
    let code = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/code").and_then(Value::as_str).map(str::to_string))
        .filter(|c| !c.is_empty() && c.len() <= MAX_CODE && c.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')));
    let said = match code {
        Some(c) => format!("HTTP {status} {c}"),
        None => format!("HTTP {status}"),
    };
    Err(match status {
        401 => NetError::Auth(format!("Microsoft refused the sign-in ({said}); sign in again")),
        429 | 500..=599 => NetError::Closed(said),
        _ => NetError::Protocol(said),
    })
}

/// An RFC 3339 time (`2026-10-30T11:00:00.123+01:00`) as `$filter` takes it: UTC with a `Z`,
/// its milliseconds kept (`2026-10-30T10:00:00.123Z`). Made from the numbers read, so nothing
/// of the input reaches the query but them.
pub(super) fn filter_time(t: &str) -> Option<String> {
    let t = t.trim();
    let b = t.as_bytes();
    let shape = (0..19).all(|i| match i {
        4 | 7 => b.get(i) == Some(&b'-'),
        10 => b.get(i).is_some_and(|c| c.eq_ignore_ascii_case(&b'T')),
        13 | 16 => b.get(i) == Some(&b':'),
        _ => b.get(i).is_some_and(u8::is_ascii_digit),
    });
    if !shape {
        return None;
    }
    let mut rest = t.get(19..)?;
    let mut ms = 0;
    if let Some(r) = rest.strip_prefix('.') {
        let digits = r.bytes().take_while(u8::is_ascii_digit).count();
        if !(1..=9).contains(&digits) {
            return None;
        }
        ms = format!("{:0<3}", r.get(..digits.min(3))?).parse::<u32>().ok()?;
        rest = r.get(digits..)?;
    }
    let zoned = matches!(rest, "Z" | "z")
        || (rest.len() == 6
            && matches!(rest.as_bytes().first(), Some(b'+' | b'-'))
            && rest.get(3..4) == Some(":")
            && rest.bytes().enumerate().all(|(i, c)| i == 0 || i == 3 || c.is_ascii_digit()));
    if !zoned {
        return None;
    }
    let secs = crate::jmap::utc_date(t).filter(|s| (0..=253_402_300_799).contains(s))?;
    let day = utc(secs);
    Some(if ms == 0 { day } else { format!("{}.{ms:03}Z", day.trim_end_matches('Z')) })
}
