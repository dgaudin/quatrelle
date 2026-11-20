// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A client for Graph's chat API (v1.0): the signed-in user, chats, their members and
//! messages, posting text or HTML and editing text, reactions, soft deletes, marking a chat
//! read, a message's hosted images, people's photos and presence, and the host of the user's
//! OneDrive; its answers are read into the model here too (`read`). OneDrive's uploads, a
//! file's item and share, a post with pictures and files, and a file shared in a chat (its
//! address, its bytes counted) are in `files`; the directory and a chat's creation in `people`.
//!
//! Every request carries the bearer token and goes on a connection of its own, made and answered
//! within [`DEADLINE`] (its connection, its TLS handshake and every byte of its answer: a server
//! that trickles its answer fails it as a connection cut does, `TimedOut`). No redirect is
//! followed with the token: a Graph answer that redirects is an error, except a shared file's
//! content, whose one redirect (to SharePoint's pre-signed download address, on Microsoft's file
//! hosts: [`transfer_url`]) is followed by hand without it. A 429 or a 503 is waited out
//! ([`BUSY_ROUNDS`] rounds at most, `Retry-After` capped at 20 s; a post of a message only on a
//! 429, as a 503 may come after it was made) and every 429 counted in [`Stats`]; the Teams
//! worker turns the waits off ([`TeamsClient::set_waiting`]) and keeps to its own limiter, which
//! reads the last `Retry-After` ([`TeamsClient::retry_after`]). Errors say the HTTP status and
//! Graph's error code, never its message, which can quote a chat's content.

use std::cell::Cell;
use std::collections::HashSet;
use std::io::Write;
use std::time::{Duration, Instant};

use quatrelle_model::pim::teams::{MAX_ID, MAX_MEMBERS, Mention, PRESENCE_IDS, SEND_MAX, reactable};
use serde_json::{Value, json};

use crate::graph::{BUSY_ROUNDS, backoff_secs, seg, utc};
use crate::http::{self, Request, Response, Url};
use crate::stream::{self, Stream};
use crate::{NetError, Result};

mod files;
mod meetings;
mod people;
mod read;

pub use files::*;
pub use meetings::*;
pub use people::*;
pub use read::*;

/// Largest JSON answer accepted.
const MAX_JSON: usize = 8 << 20;
/// How long a request to Graph may take in all, from its connection to the last byte of its
/// answer: the worker hears its jobs between two requests, so this is also the longest a sign-out
/// waits on a request under way.
pub const DEADLINE: Duration = Duration::from_secs(120);
/// Sent with every chat list and message read: kinds of chat, reactions, message types and the
/// like added to Graph after v1.0 was published come as they are rather than as
/// `unknownFutureValue`.
const PREFER_ENUMS: &str = "include-unknown-enum-members";
/// Chats and messages asked for per page (the most Graph gives).
const TOP: u32 = 50;
/// Pages of a chat's members followed at most ([`TeamsClient::members`]), and by
/// [`TeamsClient::members_all`] (5,000 members at 25 a page).
const MEMBER_PAGES: usize = 20;
const MEMBER_PAGES_ALL: usize = 200;
/// The longest wait a `Retry-After` is taken for (an hour).
const MAX_RETRY_AFTER: u64 = 3600;
/// The longest host name taken (DNS's).
const MAX_HOST: usize = 253;
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
    /// `Bearer <token>`: held in memory only, never printed; replaced by [`TeamsClient::set_token`].
    auth: Cell<String>,
    stats: Cell<Stats>,
    /// The server's clock at its last answer that said it (Unix seconds).
    date: Cell<Option<i64>>,
    /// Whether a 429 or a 503 is waited out (else returned at once).
    waiting: Cell<bool>,
    /// The last 429's or 503's `Retry-After`, in seconds.
    retry: Cell<Option<u64>>,
    /// How long a request may take in all ([`DEADLINE`]; a test's shorter one).
    deadline: Cell<Duration>,
}

impl TeamsClient {
    /// Check the token by reading the signed-in user.
    pub fn connect(base: &str, token: &str) -> Result<(TeamsClient, Me)> {
        TeamsClient::connect_waiting(base, token, true)
    }

    /// [`Self::connect`], a 429 or a 503 waited out or not from its first request on
    /// ([`Self::set_waiting`]): the worker never waits in a request.
    pub(crate) fn connect_waiting(base: &str, token: &str, waiting: bool) -> Result<(TeamsClient, Me)> {
        let base = base.trim().trim_end_matches('/').to_string();
        let url = Url::parse(&base)?;
        if !url.https && !loopback(&url) {
            return Err(NetError::Tls("the Graph address isn't https".into()));
        }
        let c = TeamsClient {
            base,
            auth: Cell::new(format!("Bearer {token}")),
            stats: Cell::new(Stats::default()),
            date: Cell::new(None),
            waiting: Cell::new(waiting),
            retry: Cell::new(None),
            deadline: Cell::new(DEADLINE),
        };
        let v = c.json("GET", &c.at("/me?$select=id,displayName,mail,userPrincipalName"), None, false)?;
        let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().chars().take(320).collect::<String>();
        let me = Me { id: text("id"), display_name: text("displayName"), mail: text("mail"), upn: text("userPrincipalName") };
        if me.id.is_empty() {
            return Err(NetError::Protocol("Graph: the signed-in user has no id".into()));
        }
        Ok((c, me))
    }

    /// Use this access token from now on (the one in use has run out).
    pub fn set_token(&self, token: &str) {
        self.auth.set(format!("Bearer {token}"));
    }

    /// The `Authorization` header's value.
    fn bearer(&self) -> String {
        let auth = self.auth.take();
        self.auth.set(auth.clone());
        auth
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

    /// Whether a 429 or a 503 is waited out (`true`, the default) or returned at once as
    /// [`NetError::Closed`] (`false`: the worker's limiter waits, and is told how long by
    /// [`Self::retry_after`]).
    pub fn set_waiting(&self, on: bool) {
        self.waiting.set(on);
    }

    /// The last 429's or 503's `Retry-After` in seconds (at most an hour); `None` when it said
    /// none, or not in seconds.
    pub fn retry_after(&self) -> Option<u64> {
        self.retry.get()
    }

    /// A test's deadline for each request, in place of [`DEADLINE`].
    #[cfg(test)]
    pub(crate) fn set_deadline(&self, within: Duration) {
        self.deadline.set(within);
    }

    /// When a request begun now must be answered by.
    fn until(&self) -> Instant {
        let now = Instant::now();
        now.checked_add(self.deadline.get()).unwrap_or(now)
    }

    /// A page of the signed-in user's chats, the most recently active first, each with a
    /// preview of its last message; `next` is the previous page's [`Page::next`].
    pub fn chats(&self, next: Option<&str>) -> Result<Page> {
        let url = match next {
            Some(n) => self.own(n)?,
            None => self.at(&format!("/me/chats?$top={TOP}&$expand=lastMessagePreview&$orderby=lastMessagePreview/createdDateTime%20desc")),
        };
        page(self.json("GET", &url, None, true)?)
    }

    /// A chat's members, every page. The id must be one segment of the address ([`path_id`]),
    /// else it is refused before any request.
    pub fn members(&self, chat: &str) -> Result<Vec<Value>> {
        if !path_id(chat) {
            return Err(NetError::Refused("not a chat's id".into()));
        }
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

    /// A chat's members, every page up to [`MAX_MEMBERS`] of them, each once (by its id; one
    /// without an id is left out); the pages stop at one already followed or one that brings
    /// nobody new.
    pub fn members_all(&self, chat: &str) -> Result<Vec<Value>> {
        if !path_id(chat) {
            return Err(NetError::Refused("not a chat's id".into()));
        }
        let mut url = self.at(&format!("/chats/{}/members", seg(chat)));
        let mut out = Vec::new();
        let (mut links, mut ids) = (HashSet::new(), HashSet::new());
        for _ in 0..MEMBER_PAGES_ALL {
            let p = page(self.json("GET", &url, None, false)?)?;
            let before = out.len();
            for m in p.items {
                if out.len() >= MAX_MEMBERS {
                    break;
                }
                let id = ["userId", "anonymousGuestId", "skypeId", "azureCommunicationServicesId", "id"]
                    .iter()
                    .find_map(|k| m.get(*k).and_then(Value::as_str).filter(|i| !i.is_empty()));
                if let Some(id) = id
                    && ids.insert(id.to_string())
                {
                    out.push(m);
                }
            }
            match p.next {
                Some(n) if out.len() > before && out.len() < MAX_MEMBERS && links.insert(n.clone()) => url = self.own(&n)?,
                _ => break,
            }
        }
        Ok(out)
    }

    /// An image kept with a message (its hosted content), at most `max` bytes: the address is
    /// built here from the message's own chat, its id and the image's id, never taken from the
    /// message. Asked with `ConsistencyLevel: eventual`, which an edited or deleted message needs.
    pub fn hosted(&self, chat: &str, message: &str, id: &str, max: usize) -> Result<Vec<u8>> {
        if ![chat, message].iter().all(|p| path_id(p)) || !own_segment(id) {
            return Err(NetError::Refused("not a hosted image's id".into()));
        }
        let url = self.at(&format!("/chats/{}/messages/{}/hostedContents/{}/$value", seg(chat), seg(message), seg(id)));
        let req = Request::get(&url, max).header("Authorization", &self.bearer()).header("ConsistencyLevel", "eventual");
        let resp = self.send(&req, true)?;
        status_error(resp.status, &resp.body)?;
        Ok(resp.body)
    }

    /// Someone's photo (48 by 48), at most `max` bytes; `None` when they have none (404).
    pub fn photo(&self, user: &str, max: usize) -> Result<Option<Vec<u8>>> {
        if !own_segment(user) {
            return Err(NetError::Refused("not a user's id".into()));
        }
        let url = self.at(&format!("/users/{}/photos/48x48/$value", seg(user)));
        let resp = self.send(&Request::get(&url, max).header("Authorization", &self.bearer()), true)?;
        if resp.status == 404 {
            return Ok(None);
        }
        status_error(resp.status, &resp.body)?;
        Ok(Some(resp.body))
    }

    /// The presence of these people (Graph's `presence` objects), for the first [`PRESENCE_IDS`]
    /// of them.
    pub fn presences(&self, ids: &[String]) -> Result<Vec<Value>> {
        let ids: Vec<&String> = ids.iter().take(PRESENCE_IDS).collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let v = self.json("POST", &self.at("/communications/getPresencesByUserId"), Some(&json!({ "ids": ids })), false)?;
        Ok(page(v)?.items.into_iter().take(PRESENCE_IDS).collect())
    }

    /// The host of the user's OneDrive (`contoso-my.sharepoint.com`): `None` when they have none
    /// (404), or its address isn't an `https` one.
    pub fn drive_host(&self) -> Result<Option<String>> {
        let v = match self.json("GET", &self.at("/me/drive?$select=webUrl"), None, false) {
            Err(NetError::Protocol(m)) if m.starts_with("HTTP 404") => return Ok(None),
            r => r?,
        };
        let Some(url) = v.get("webUrl").and_then(Value::as_str).and_then(|u| Url::parse(u).ok()) else { return Ok(None) };
        let host = url.host.to_ascii_lowercase();
        let named = !host.is_empty() && host.len() <= MAX_HOST && host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.'));
        Ok((url.https && named).then_some(host))
    }

    /// A page of a chat's messages changed in `range`, the last changed first; `next` is the
    /// previous page's [`Page::next`] (it carries the range).
    pub fn messages(&self, chat: &str, range: Range<'_>, next: Option<&str>) -> Result<Page> {
        if !path_id(chat) {
            return Err(NetError::Refused("not a chat's id".into()));
        }
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

    /// Post a plain-text message in a chat: the message Graph made. The chat's id must be one
    /// segment of the address ([`own_segment`]), else it is refused before any request. Waited out
    /// on a 429 only (nothing was done); a 503 may come after the message was made, so it isn't
    /// posted again.
    pub fn send_text(&self, chat: &str, text: &str) -> Result<Value> {
        if !own_segment(chat) {
            return Err(NetError::Refused("not a chat's id".into()));
        }
        let body = json!({"body": {"contentType": "text", "content": text}});
        self.request("POST", &self.at(&format!("/chats/{}/messages", seg(chat))), Some(&body), false, false)
    }

    /// Post a message written in HTML in a chat: the message Graph made. The chat's id must be one
    /// segment of the address ([`own_segment`]) and the HTML at most [`SEND_MAX`] bytes, else it is
    /// refused before any request. `mentions` (those with a user id) go as the message's `mentions`
    /// array, only when there are some. Waited out on a 429 only, as [`Self::send_text`]. With
    /// pictures and files: [`Self::send_parts`].
    pub fn send_html(&self, chat: &str, html: &str, mentions: &[Mention]) -> Result<Value> {
        self.send_parts(chat, html, mentions, &[], &[])
    }

    /// Replace the words of a message the signed-in user sent by HTML: the chat's and the
    /// message's ids must each be one segment of the address ([`own_segment`]) and the HTML at
    /// most [`SEND_MAX`] bytes, else it is refused before any request. The body always carries
    /// `mentions` (empty to clear them). The same edit can be sent again.
    pub fn edit_html(&self, chat: &str, msg: &str, html: &str, mentions: &[Mention]) -> Result<()> {
        if !own_segment(chat) || !own_segment(msg) {
            return Err(NetError::Refused("not a chat's or a message's id".into()));
        }
        if html.len() > SEND_MAX {
            return Err(NetError::Refused(format!("a message longer than Teams takes ({SEND_MAX} bytes)")));
        }
        let body = json!({"body": {"contentType": "html", "content": html}, "mentions": mentions_json(mentions)});
        self.json("PATCH", &self.at(&format!("/chats/{}/messages/{}", seg(chat), seg(msg))), Some(&body), false).map(drop)
    }

    /// Replace a message's text (one the signed-in user sent); the ids are checked as for
    /// [`Self::edit_html`].
    pub fn edit_text(&self, chat: &str, msg: &str, text: &str) -> Result<()> {
        if !own_segment(chat) || !own_segment(msg) {
            return Err(NetError::Refused("not a chat's or a message's id".into()));
        }
        let body = json!({"body": {"contentType": "text", "content": text}});
        self.json("PATCH", &self.at(&format!("/chats/{}/messages/{}", seg(chat), seg(msg))), Some(&body), false).map(drop)
    }

    /// React to a message with an emoji ([`reactable`]: a name such as `like` is refused, before
    /// any request, as is an id that isn't one segment).
    pub fn set_reaction(&self, chat: &str, msg: &str, emoji: &str) -> Result<()> {
        self.react("setReaction", chat, msg, emoji)
    }

    /// Take a reaction back, checked as [`Self::set_reaction`].
    pub fn unset_reaction(&self, chat: &str, msg: &str, emoji: &str) -> Result<()> {
        self.react("unsetReaction", chat, msg, emoji)
    }

    fn react(&self, verb: &str, chat: &str, msg: &str, emoji: &str) -> Result<()> {
        if !own_segment(chat) || !own_segment(msg) {
            return Err(NetError::Refused("not a chat's or a message's id".into()));
        }
        if !reactable(emoji) {
            return Err(NetError::Refused("not an emoji a reaction can be".into()));
        }
        let url = self.at(&format!("/chats/{}/messages/{}/{verb}", seg(chat), seg(msg)));
        self.json("POST", &url, Some(&json!({"reactionType": emoji})), false).map(drop)
    }

    /// Delete a message the signed-in user sent (it stays listed, with its deletion time):
    /// through `/users/{user}` (the signed-in user's object id), or `/me` when `None`. Each id
    /// must be one segment of the address ([`own_segment`]), else it is refused before any request.
    pub fn soft_delete(&self, chat: &str, msg: &str, user: Option<&str>) -> Result<()> {
        if !own_segment(chat) || !own_segment(msg) || user.is_some_and(|u| !own_segment(u)) {
            return Err(NetError::Refused("not a chat's, a message's or a user's id".into()));
        }
        let who = user.map_or_else(|| "/me".to_string(), |u| format!("/users/{}", seg(u)));
        self.json("POST", &self.at(&format!("{who}/chats/{}/messages/{}/softDelete", seg(chat), seg(msg))), None, false).map(drop)
    }

    /// Mark a chat read for the signed-in user (`user`: their object id, in `tenant`); a chat id
    /// that isn't one segment of the address ([`own_segment`]) is refused before any request.
    pub fn mark_read(&self, chat: &str, user: &str, tenant: &str) -> Result<()> {
        if !own_segment(chat) {
            return Err(NetError::Refused("not a chat's id".into()));
        }
        let body = json!({"user": {"id": user, "tenantId": tenant}});
        self.json("POST", &self.at(&format!("/chats/{}/markChatReadForUser", seg(chat))), Some(&body), false).map(drop)
    }
}

// Requests.
impl TeamsClient {
    /// The full address of a path under the API.
    fn at(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// A next page's address, when it is under this client's own API address: the only one the
    /// token goes to (a link kept in the copy is checked by it before use).
    pub(crate) fn own(&self, next: &str) -> Result<String> {
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
        self.answered(&self.prepared(method, url, body, prefer)?, again).map(|(_, v)| v)
    }

    /// A request to Graph with the token, its body JSON, a JSON answer wanted.
    fn prepared(&self, method: &str, url: &str, body: Option<&Value>, prefer: bool) -> Result<Request> {
        let mut req = match body {
            Some(b) => {
                let bytes = serde_json::to_vec(b).map_err(|e| NetError::Protocol(e.to_string()))?;
                Request::post(url, "application/json", bytes, MAX_JSON)
            }
            None => Request::get(url, MAX_JSON),
        };
        req.method = method.into();
        req = req.header("Authorization", &self.bearer()).header("Accept", "application/json");
        if prefer {
            req = req.header("Prefer", PREFER_ENUMS);
        }
        Ok(req)
    }

    /// A request sent as [`Self::send`] does: its status and its JSON answer (`Null` when empty);
    /// an HTTP failure as [`status_error`] says it.
    fn answered(&self, req: &Request, again: bool) -> Result<(u16, Value)> {
        let resp = self.send(req, again)?;
        status_error(resp.status, &resp.body)?;
        if resp.body.iter().all(u8::is_ascii_whitespace) {
            return Ok((resp.status, Value::Null));
        }
        let v = serde_json::from_slice(&resp.body).map_err(|e| NetError::Protocol(format!("Graph: {e}")))?;
        Ok((resp.status, v))
    }

    /// A request, waited out while busy when [waiting](Self::set_waiting) (a 503 only when
    /// `again`: the request can be repeated): the last answer as it came.
    fn send(&self, req: &Request, again: bool) -> Result<Response> {
        let mut round = 0;
        loop {
            round += 1;
            let resp = self.once(req)?;
            if !busy(resp.status) || !self.waiting.get() || (resp.status == 503 && !again) || round >= BUSY_ROUNDS {
                return Ok(resp);
            }
            pause(backoff_secs(resp.header("Retry-After"), round));
        }
    }

    /// One request on a connection of its own, made and answered within the deadline, its answer
    /// as it came (a redirect isn't followed).
    fn once(&self, req: &Request) -> Result<Response> {
        let url = Url::parse(&req.url)?;
        let mut s = connect_within(&url, self.until())?;
        self.count();
        let resp = http::exchange(&mut s, req, &url)?;
        self.heard(&resp, true);
        Ok(resp)
    }

    /// A GET whose body is counted as it comes and not kept (only the start of an error's, for
    /// its code); a redirect's body is left unread, and a file's when not `served`. Graph's, its
    /// body unread (a shared file's address, which the worker asks for), within the deadline.
    fn download(&self, req: &Request, graph: bool, served: bool) -> Result<(Response, Tally)> {
        let url = Url::parse(&req.url)?;
        let mut s = match (graph, served) {
            (true, false) => connect_within(&url, self.until())?,
            (true, true) => connect(&url)?,
            (false, _) => self.transfer_stream(&url)?,
        };
        self.count();
        let mut tally = Tally::default();
        let sink = &mut tally;
        let head = http::exchange_into(&mut s, req, &url, move |head: &Response| {
            if busy(head.status) || (head.ok() && !served) {
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

    /// An answer in: a 429 counted, a 429's or 503's `Retry-After` kept, and Graph's clock noted.
    fn heard(&self, resp: &Response, graph: bool) {
        if resp.status == 429 {
            let mut s = self.stats.get();
            s.throttled = s.throttled.saturating_add(1);
            self.stats.set(s);
        }
        // Only the last answer counts: once one isn't busy, an older wait is forgotten.
        let wait = if busy(resp.status) { resp.header("Retry-After").and_then(|h| h.trim().parse::<u64>().ok()) } else { None };
        self.retry.set(wait.map(|s| s.min(MAX_RETRY_AFTER)));
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

/// [`connect`] with a deadline for all of its exchange: connected, TLS's handshake done, and every
/// read and write of the socket itself made by `until` (under TLS too: [`stream::tls_within`]).
fn connect_within(url: &Url, until: Instant) -> Result<Stream> {
    let sock = stream::tcp_within(&url.host, url.port, until)?;
    if url.https { stream::tls_within(sock, &url.host, until) } else { Ok(Stream::PlainWithin(stream::Timed::new(sock, until))) }
}

/// Whether a download's redirect to `to` may be followed by a client whose API is at `base`:
/// https, or plain HTTP to this computer only when the client's own API is there too (the
/// tests' servers). Graph itself never sends the download to a local port. [`transfer_url`]
/// takes plain HTTP by this rule, and https only on Microsoft's file hosts.
pub(super) fn hop_allowed(base: &str, to: &Url) -> bool {
    to.https || (loopback(to) && Url::parse(base).is_ok_and(|b| !b.https && loopback(&b)))
}

/// This computer: the only place plain HTTP is spoken to (the tests' servers).
fn loopback(url: &Url) -> bool {
    matches!(url.host.as_str(), "127.0.0.1" | "localhost" | "::1")
}

/// Whether an id can be one segment of an address: not empty, at most [`MAX_ID`] characters, no
/// control character, and not only dots (which would climb the path, encoded or not).
fn path_id(id: &str) -> bool {
    !id.is_empty() && id.chars().nth(MAX_ID).is_none() && !id.chars().any(char::is_control) && !id.chars().all(|c| c == '.')
}

/// [`path_id`], with nothing a server might take for a path or a query of its own (`/`, `\`,
/// `?`, `#`), even encoded: some servers decode `%2F`.
pub(crate) fn own_segment(id: &str) -> bool {
    path_id(id) && !id.contains(['/', '\\', '?', '#'])
}

/// The `mentions` array of a message written: those with a user id, in order, each as Graph
/// documents a `chatMessageMention` of an Entra user.
pub(crate) fn mentions_json(mentions: &[Mention]) -> Value {
    let list: Vec<Value> = mentions
        .iter()
        .filter_map(|m| {
            let user = m.user_id.as_deref()?;
            Some(json!({
                "id": m.id,
                "mentionText": m.text,
                "mentioned": {"user": {"id": user, "displayName": m.text, "userIdentityType": "aadUser"}},
            }))
        })
        .collect();
    Value::Array(list)
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
