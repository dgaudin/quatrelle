//! Microsoft Graph mail: the transport for Exchange Online (Microsoft 365, Outlook.com), behind
//! the same [`Job`](quatrelle_model::Job)/[`Event`](quatrelle_model::Event) interface as IMAP
//! and JMAP. It works where an organisation has switched IMAP off.
//!
//! Every request is HTTPS with an OAuth bearer token (`Mail.ReadWrite`, `Mail.Send`,
//! `MailboxSettings.ReadWrite`), against the documented REST API: mail folders, messages,
//! their MIME source (`$value`), `sendMail` with a MIME body, and mailbox settings for automatic
//! replies. Ids are asked for in their immutable form (`Prefer: IdType="ImmutableId"`), so a
//! message keeps its id when it moves, as JMAP's do: the numbering and folder paths are the
//! JMAP transport's ([`jmap::State`], [`jmap::folders`]). Several small requests go as one
//! `$batch` (four per batch; see [`BATCH`]).

use std::collections::HashMap;

use quatrelle_mime as mime;
use quatrelle_model::organize::{Condition, Rule, RuleAction};
use quatrelle_model::{Flag, Flags, Folder, FolderRole, MessageKey, Op, Summary, sort_newest_first, utf7, voting};
use quatrelle_store::{FolderState, Store};
use serde_json::{Value, json};

use crate::http::{self, Request, Response};
use crate::jmap::{self, Mailbox, State, VALIDITY};
use crate::sync::{SyncOptions, SyncReport, WordQuery, fill_from_source};
use crate::{NetError, Result};

/// The Graph endpoint accounts use.
pub const DEFAULT_URL: &str = "https://graph.microsoft.com/v1.0";
/// Largest JSON response accepted.
const MAX_JSON: usize = 32 << 20;
/// Largest message source downloaded.
const MAX_MIME: usize = 64 << 20;
/// Entries per `$batch`. Graph executes a batch's entries concurrently and Exchange Online's
/// per-app, per-mailbox concurrency limit is 4, so larger batches mostly came back as 429s.
const BATCH: usize = 4;
/// Attempts per request (or batch entry) while the server keeps answering 429.
const BUSY_ROUNDS: u32 = 4;
/// Upper bound in seconds on a single back-off, whatever `Retry-After` says.
const BUSY_WAIT_CAP: u64 = 20;
/// Up to this many uncached messages are fetched individually (with internet headers); above
/// it the listing's own fields are used.
const FEW: usize = 20;
/// Pages followed for one listing.
const MAX_PAGES: usize = 200;
/// Folders listed at most.
const MAX_FOLDERS: usize = 5000;
const FOLDER_FIELDS: &str = "id,displayName,parentFolderId,childFolderCount,totalItemCount,unreadItemCount";
const HEADER_FIELDS: &str = "id,isRead,isDraft,flag,subject,from,sender,toRecipients,ccRecipients,replyTo,receivedDateTime,sentDateTime,bodyPreview,hasAttachments,internetMessageId,internetMessageHeaders";
/// The voting buttons (`PidLidVerbStream`), asked for with a message's details.
const VOTING_EXPAND: &str =
    "$expand=singleValueExtendedProperties($filter=id%20eq%20'Binary%20%7B00062008-0000-0000-C000-000000000046%7D%20Id%200x8520')";
/// The choice a vote carries (`PidLidVerbResponse`), as Graph names the property.
const VERB_RESPONSE_ID: &str = "String {00062008-0000-0000-C000-000000000046} Id 0x8524";
/// `$select` for folder listings. `internetMessageHeaders` is left out on purpose: it is
/// expensive for the server to produce per item, and listing pages that include it are slow
/// enough to hit gateway timeouts. [`fill_from_source`] recovers threading later.
const LIST_FIELDS: &str = "id,subject,from,sender,toRecipients,ccRecipients,replyTo,internetMessageId,receivedDateTime,importance,hasAttachments,bodyPreview,isRead,flag,isDraft";
/// Sent on every call, including inside each `$batch` entry, where the outer header is ignored.
const PREFER: &str = "IdType=\"ImmutableId\", outlook.timezone=\"UTC\"";

/// A signed-in Graph session.
pub struct Client {
    base: String,
    auth: String,
}

/// Where a mailbox is in Graph: this account's (`/me`), or another opened in it.
fn who(owner: Option<&str>) -> String {
    owner.map_or_else(|| "/me".to_string(), |o| format!("/users/{}", seg(o)))
}

/// Turn an HTTP failure into the error the worker knows how to handle.
fn check(resp: &Response) -> Result<()> {
    status_error(resp.status, &resp.body)
}

/// What a Microsoft account without a mailbox Graph can reach gets (a personal account made
/// with a Gmail or other address, a mailbox on premises).
pub(crate) const NO_MAILBOX: &str = "this Microsoft account has no Outlook.com or Microsoft 365 mailbox that Quatrelle can reach. \
If its mail is with another provider (such as Gmail), add it with that provider's settings instead";

fn status_error(status: u16, body: &[u8]) -> Result<()> {
    let error = serde_json::from_slice::<Value>(body).ok().and_then(|v| v.get("error").cloned());
    let field = |k: &str| error.as_ref().and_then(|e| e.get(k)).and_then(Value::as_str).unwrap_or_default().to_string();
    let detail = || {
        let message = field("message");
        if message.is_empty() { field("code") } else { message }
    };
    if matches!(field("code").as_str(), "MailboxNotEnabledForRESTAPI" | "MailboxNotSupportedForRESTAPI") {
        return Err(NetError::No(NO_MAILBOX.into()));
    }
    match status {
        200..=299 => Ok(()),
        401 => Err(NetError::Auth("Microsoft refused the sign-in; sign in again".into())),
        429 | 500..=599 => Err(NetError::Closed(format!("HTTP {status}"))),
        s => Err(NetError::No(format!("HTTP {s} {}", detail()).trim().to_string())),
    }
}

fn json_body(body: &[u8]) -> Result<Value> {
    serde_json::from_slice(body).map_err(|e| NetError::Protocol(format!("Graph: {e}")))
}

fn seg(id: &str) -> String {
    http::pct(id)
}

impl Client {
    /// Check the token works (and that the address is the mailbox's).
    pub fn connect(base: &str, token: &str) -> Result<Client> {
        let base = base.trim().trim_end_matches('/').to_string();
        let url = http::Url::parse(&base)?;
        if !url.https && !matches!(url.host.as_str(), "127.0.0.1" | "localhost") {
            return Err(NetError::Tls("the Graph address isn't https".into()));
        }
        let c = Client { base, auth: format!("Bearer {token}") };
        c.get("/me/mailFolders/inbox?$select=id")?;
        Ok(c)
    }

    /// Carry a new access token from now on (the old one ran out).
    pub fn set_token(&mut self, token: &str) {
        self.auth = format!("Bearer {token}");
    }

    /// The mail folders, with roles for the well-known ones; the Outbox and Exchange's own
    /// bookkeeping folders are left out.
    pub fn mailboxes(&self) -> Result<Vec<Mailbox>> {
        self.mailboxes_of(None)
    }

    /// Another mailbox opened in the account (a shared mailbox, or someone's who made this
    /// account a delegate): a folder named after its address, its mail folders under it. A
    /// delegate who may only open some folders gets those of the usual ones it can.
    pub fn shared_mailbox(&self, owner: &str) -> Result<Vec<Mailbox>> {
        let me = who(Some(owner));
        let root = self.get(&format!("{me}/mailFolders/msgfolderroot?$select=id")).ok().and_then(|v| v.get("id")?.as_str().map(str::to_string));
        let mut out = match self.mailboxes_of(Some(owner)) {
            Ok(list) => list,
            Err(e) => {
                let mut some = Vec::new();
                for (name, role) in [("inbox", FolderRole::Inbox), ("sentitems", FolderRole::Sent), ("drafts", FolderRole::Drafts)] {
                    if let Ok(f) = self.get(&format!("{me}/mailFolders/{name}?$select={FOLDER_FIELDS}")) {
                        let count = |k: &str| f.get(k).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
                        let Some(id) = f.get("id").and_then(Value::as_str).map(str::to_string) else { continue };
                        some.push(Mailbox {
                            name: f.get("displayName").and_then(Value::as_str).unwrap_or(name).chars().take(255).collect(),
                            parent: None,
                            role,
                            total: count("totalItemCount"),
                            unread: count("unreadItemCount"),
                            id,
                        });
                    }
                }
                if some.is_empty() {
                    return Err(e);
                }
                some
            }
        };
        let root = root.unwrap_or_else(|| format!("root:{owner}"));
        let ids: Vec<String> = out.iter().map(|m| m.id.clone()).collect();
        for m in &mut out {
            if m.parent.as_ref().is_none_or(|p| !ids.contains(p)) {
                m.parent = Some(root.clone());
            }
        }
        out.insert(0, Mailbox { id: root, name: owner.to_string(), parent: None, role: FolderRole::Other, total: 0, unread: 0 });
        Ok(out)
    }

    /// The mail folders of a mailbox (`owner`: another one opened in the account).
    fn mailboxes_of(&self, owner: Option<&str>) -> Result<Vec<Mailbox>> {
        let me = who(owner);
        let roles = [
            ("inbox", FolderRole::Inbox),
            ("drafts", FolderRole::Drafts),
            ("sentitems", FolderRole::Sent),
            ("deleteditems", FolderRole::Trash),
            ("junkemail", FolderRole::Junk),
            ("archive", FolderRole::Archive),
        ];
        let hidden = ["outbox", "conversationhistory", "syncissues", "recoverableitemsdeletions"];
        let wk: Vec<(&str, String, Option<Value>)> =
            roles.iter().map(|(n, _)| *n).chain(hidden).map(|n| ("GET", format!("{me}/mailFolders/{n}?$select=id"), None)).collect();
        let ids: Vec<Option<String>> = self
            .batch(&wk)?
            .into_iter()
            .map(|(s, b)| if s == 200 { b.get("id").and_then(Value::as_str).map(str::to_string) } else { None })
            .collect();
        let role_of: HashMap<String, FolderRole> = roles.iter().zip(&ids).filter_map(|((_, r), id)| Some((id.clone()?, *r))).collect();
        let skip: Vec<String> = ids.iter().skip(roles.len()).flatten().cloned().collect();

        let mut out: Vec<Mailbox> = Vec::new();
        let mut pending: Vec<Value> = self.all(&format!("{me}/mailFolders?$top=100&$select={FOLDER_FIELDS}"), MAX_FOLDERS)?;
        while let Some(f) = pending.pop() {
            let Some(id) = f.get("id").and_then(Value::as_str).map(str::to_string) else { continue };
            if skip.contains(&id) || out.iter().any(|m| m.id == id) || out.len() >= MAX_FOLDERS {
                continue;
            }
            if f.get("childFolderCount").and_then(Value::as_u64).unwrap_or(0) > 0 {
                pending.extend(self.all(&format!("{me}/mailFolders/{}/childFolders?$top=100&$select={FOLDER_FIELDS}", seg(&id)), MAX_FOLDERS)?);
            }
            let count = |k: &str| f.get(k).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
            out.push(Mailbox {
                name: f.get("displayName").and_then(Value::as_str).unwrap_or_default().chars().take(255).collect(),
                parent: f.get("parentFolderId").and_then(Value::as_str).map(str::to_string),
                role: role_of.get(&id).copied().unwrap_or(FolderRole::Other),
                total: count("totalItemCount"),
                unread: count("unreadItemCount"),
                id,
            });
        }
        Ok(out)
    }

    /// The newest `window` messages of a folder with their list details ([`LIST_FIELDS`]), only
    /// those received since `since` (Unix seconds) when the age of mail kept is limited, and the
    /// folder's (total, unread).
    fn newest(&self, folder: &str, window: usize, since: Option<i64>, owner: Option<&str>) -> Result<(Vec<Value>, (u32, u32))> {
        let me = who(owner);
        let page = window.clamp(1, 1000);
        // Graph sorts a filtered listing only by a property its filter names first: the received
        // time is both. A time before 1970 asks for all of it.
        let since = since.filter(|t| *t > 0);
        let filter = since.map(|t| format!("$filter={}&", http::pct(&format!("receivedDateTime ge {}", utc(t))))).unwrap_or_default();
        let mut entries = self.all(
            &format!("{me}/mailFolders/{}/messages?{filter}$top={page}&$orderby=receivedDateTime%20desc&$select={LIST_FIELDS}", seg(folder)),
            window.min(100_000),
        )?;
        // Measured here as the server measures it, should it list older mail all the same: one
        // whose received time can't be read is older than any (its summary would have none).
        entries.retain(|m| m["id"].is_string() && since.is_none_or(|t| received(m).is_some_and(|r| r >= t)));
        let ids = entries;
        let f = self.get(&format!("{me}/mailFolders/{}?$select=totalItemCount,unreadItemCount", seg(folder)))?;
        let count = |k: &str| f.get(k).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
        Ok((ids, (count("totalItemCount"), count("unreadItemCount"))))
    }

    /// Of these messages, those still in a folder, with their flags (through `$batch`).
    fn still_in(&self, folder: &str, ids: &[String]) -> Result<Vec<(String, Flags)>> {
        let reqs: Vec<(&str, String, Option<Value>)> =
            ids.iter().map(|id| ("GET", format!("/me/messages/{}?$select=id,isRead,isDraft,flag,parentFolderId", seg(id)), None)).collect();
        Ok(self
            .batch(&reqs)?
            .into_iter()
            .filter(|(s, m)| *s == 200 && m.get("parentFolderId").and_then(Value::as_str).is_none_or(|p| p == folder))
            .filter_map(|(_, m)| Some((m.get("id")?.as_str()?.to_string(), flags(&m))))
            .collect())
    }

    /// Messages' details, for the list.
    fn headers(&self, ids: &[String], owner: Option<&str>) -> Result<Vec<Value>> {
        let me = who(owner);
        let reqs: Vec<(&str, String, Option<Value>)> =
            ids.iter().map(|id| ("GET", format!("{me}/messages/{}?$select={HEADER_FIELDS}&{VOTING_EXPAND}", seg(id)), None)).collect();
        Ok(self.batch(&reqs)?.into_iter().filter(|(s, _)| *s == 200).map(|(_, b)| b).collect())
    }

    /// A message's MIME source.
    pub fn download(&self, id: &str, owner: Option<&str>) -> Result<Vec<u8>> {
        let r = Request::get(&format!("{}{}/messages/{}/$value", self.base, who(owner), seg(id)), MAX_MIME)
            .header("Authorization", &self.auth)
            .header("Prefer", "IdType=\"ImmutableId\"");
        let resp = send_through_limits(&r)?;
        check(&resp)?;
        Ok(resp.body)
    }

    /// A message from its MIME source, as a draft (Graph files it in Drafts), moved to `folder`
    /// when that's somewhere else. Its id.
    pub fn import(&self, raw: &[u8], folder: Option<&str>, owner: Option<&str>) -> Result<String> {
        let me = who(owner);
        let v = self.send_raw("POST", &format!("{me}/messages"), "text/plain", crate::imap::base64(raw).into_bytes())?;
        let id = v.get("id").and_then(Value::as_str).ok_or_else(|| NetError::Protocol("Graph: the message wasn't made".into()))?.to_string();
        if let Some(f) = folder {
            let parent = v.get("parentFolderId").and_then(Value::as_str).unwrap_or_default();
            if parent != f {
                let moved = self.post(&format!("{me}/messages/{}/move", seg(&id)), &json!({"destinationId": f}))?;
                return Ok(moved.get("id").and_then(Value::as_str).unwrap_or(&id).to_string());
            }
        }
        Ok(id)
    }

    /// Answer a message's voting buttons with `choice`: a reply to it (so it stays in the
    /// conversation), its subject the choice's, carrying the choice (`PidLidVerbResponse`) for the
    /// sender's mail client to count; Exchange files the copy in Sent Items.
    pub fn vote(&self, id: &str, choice: &str, subject: &str, owner: Option<&str>) -> Result<()> {
        let me = who(owner);
        let reply = self.post(&format!("{me}/messages/{}/createReply", seg(id)), &json!({}))?;
        let rid = reply.get("id").and_then(Value::as_str).ok_or_else(|| NetError::Protocol("Graph: the reply wasn't made".into()))?.to_string();
        let change = json!({
            "subject": voting::response_subject(choice, subject),
            "singleValueExtendedProperties": [{"id": VERB_RESPONSE_ID, "value": choice}],
        });
        let bytes = serde_json::to_vec(&change).map_err(|e| NetError::Protocol(e.to_string()))?;
        self.send_raw("PATCH", &format!("{me}/messages/{}", seg(&rid)), "application/json", bytes)?;
        self.send_raw("POST", &format!("{me}/messages/{}/send", seg(&rid)), "application/json", Vec::new()).map(drop).map_err(|e| match e {
            NetError::No(m) | NetError::Bad(m) => NetError::Smtp(m),
            e => e,
        })
    }

    /// Send a MIME message; Exchange files the copy in Sent Items.
    pub fn send_mime(&self, raw: &[u8]) -> Result<()> {
        self.send_raw("POST", "/me/sendMail", "text/plain", crate::imap::base64(raw).into_bytes()).map(|_| ()).map_err(|e| match e {
            NetError::No(m) | NetError::Bad(m) => NetError::Smtp(m),
            e => e,
        })
    }

    /// The picture the directory keeps for an address (`/users/{id}/photo/$value`): its bytes
    /// and type. `No` when the person has none.
    pub fn user_photo(&self, email: &str) -> Result<(String, Vec<u8>)> {
        self.photo(&format!("/users/{}/photo/$value", seg(email)))
    }

    /// The picture a contact carries (`/me/contacts/{id}/photo/$value`): its bytes and type.
    /// `No` when the contact has none.
    pub fn contact_photo(&self, id: &str) -> Result<(String, Vec<u8>)> {
        self.photo(&format!("/me/contacts/{}/photo/$value", seg(id)))
    }

    /// A picture endpoint's answer: its bytes and type.
    fn photo(&self, path: &str) -> Result<(String, Vec<u8>)> {
        let r = Request::get(&format!("{}{path}", self.base), MAX_JSON)
            .header("Authorization", &self.auth)
            .header("Accept", "image/jpeg, image/png, image/gif")
            .header("Prefer", "IdType=\"ImmutableId\"");
        let resp = http::send(&r)?;
        if !resp.ok() {
            return Err(NetError::No("no picture for this person".into()));
        }
        let data = resp.body;
        if data.is_empty() || data.len() > crate::MAX_PICTURE {
            return Err(NetError::No("no picture for this person".into()));
        }
        Ok((crate::picture_media(&data).to_string(), data))
    }

    /// Delete every message in a folder, a batch at a time.
    fn empty(&self, folder: &str, owner: Option<&str>) -> Result<()> {
        let me = who(owner);
        for _ in 0..1000 {
            let ids = self.all(&format!("{me}/mailFolders/{}/messages?$top=100&$select=id", seg(folder)), 100)?;
            let reqs: Vec<(&str, String, Option<Value>)> = ids
                .iter()
                .filter_map(|m| m.get("id").and_then(Value::as_str))
                .map(|id| ("DELETE", format!("{me}/messages/{}", seg(id)), None))
                .collect();
            if reqs.is_empty() {
                return Ok(());
            }
            self.batch_ok(&reqs)?;
        }
        Err(NetError::No("the folder didn't empty".into()))
    }
}

/// When the server received a message (`receivedDateTime`), in Unix seconds.
fn received(m: &Value) -> Option<i64> {
    m.get("receivedDateTime").and_then(Value::as_str).and_then(jmap::utc_date)
}

/// Unix seconds as an ISO 8601 UTC time (`2026-10-23T09:00:00Z`), as `$filter` takes them.
fn utc(t: i64) -> String {
    let (y, m, d) = quatrelle_model::dates::civil_from_days(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}

/// A message's flags as the model knows them.
fn flags(m: &Value) -> Flags {
    let mut names: Vec<&str> = Vec::new();
    if m.get("isRead").and_then(Value::as_bool) == Some(true) {
        names.push("\\Seen");
    }
    if m.get("isDraft").and_then(Value::as_bool) == Some(true) {
        names.push("\\Draft");
    }
    if m.get("flag").and_then(|f| f.get("flagStatus")).and_then(Value::as_str) == Some("flagged") {
        names.push("\\Flagged");
    }
    Flags::from_imap(names)
}

/// `{"emailAddress": {"name", "address"}}` as a header value.
fn address(v: &Value) -> Option<String> {
    let e = v.get("emailAddress")?;
    let addr = e.get("address").and_then(Value::as_str).unwrap_or_default().trim();
    if addr.is_empty() || addr.contains(['\r', '\n', '<', '>']) {
        return None;
    }
    let name = e.get("name").and_then(Value::as_str).unwrap_or_default().replace(['"', '\\', '\r', '\n'], "");
    Some(if name.trim().is_empty() || name == addr { addr.to_string() } else { format!("\"{}\" <{addr}>", name.trim()) })
}

fn addresses(v: Option<&Value>) -> String {
    v.and_then(Value::as_array).map(|a| a.iter().take(500).filter_map(address).collect::<Vec<_>>().join(", ")).unwrap_or_default()
}

/// A header block for a message's details: its own internet headers when Graph has them, else
/// one made from its fields.
fn header_block(m: &Value) -> Vec<u8> {
    let mut out = String::new();
    let own = m.get("internetMessageHeaders").and_then(Value::as_array).filter(|h| !h.is_empty());
    match own {
        Some(headers) => {
            for h in headers.iter().take(500) {
                let (Some(name), Some(value)) = (h.get("name").and_then(Value::as_str), h.get("value").and_then(Value::as_str)) else { continue };
                if name.is_empty() || name.contains([':', '\r', '\n', ' ']) || value.contains(['\r', '\n']) {
                    continue;
                }
                out.push_str(&format!("{name}: {value}\r\n"));
            }
        }
        None => {
            let clean = |s: &str| s.replace(['\r', '\n'], " ");
            for (name, value) in [
                ("From", m.get("from").and_then(address).or_else(|| m.get("sender").and_then(address)).unwrap_or_default()),
                ("To", addresses(m.get("toRecipients"))),
                ("Cc", addresses(m.get("ccRecipients"))),
                ("Reply-To", addresses(m.get("replyTo"))),
                ("Subject", clean(m.get("subject").and_then(Value::as_str).unwrap_or_default())),
                ("Message-ID", clean(m.get("internetMessageId").and_then(Value::as_str).unwrap_or_default())),
                ("Importance", importance_header(m)),
            ] {
                if !value.trim().is_empty() {
                    out.push_str(&format!("{name}: {value}\r\n"));
                }
            }
        }
    }
    out.push_str("\r\n");
    out.into_bytes()
}

/// The `Importance:` value for Graph's `importance` property (`normal` gives none).
fn importance_header(m: &Value) -> String {
    let level = m["importance"].as_str().unwrap_or_default();
    if level.eq_ignore_ascii_case("high") || level.eq_ignore_ascii_case("low") {
        let mut c = level.chars();
        c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
    } else {
        String::new()
    }
}

mod requests;
mod rules;
mod sync;

use requests::send_through_limits;
pub(crate) use sync::download_ahead;
pub use sync::{append, apply_remote, fetch_body, find_on_server, list_folders, send, sync_folder, vote};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_from_fields() {
        let m = json!({
            "from": {"emailAddress": {"name": "Zoe \"Z\" Park", "address": "zoe@studio.example"}},
            "toRecipients": [{"emailAddress": {"name": "", "address": "me@studio.example"}}, {"emailAddress": {"name": "bad", "address": "x\r\nBcc: evil"}}],
            "subject": "Hello\r\nX-Injected: 1",
            "internetMessageId": "<m1@x>",
        });
        let h = String::from_utf8(header_block(&m)).unwrap();
        assert!(h.starts_with("From: \"Zoe Z Park\" <zoe@studio.example>\r\nTo: me@studio.example\r\n"), "{h}");
        assert!(h.contains("Subject: Hello  X-Injected: 1\r\n"), "no header injection: {h}");
        assert!(!h.contains("evil"));
        let s = mime::summary_from_header(&header_block(&m), MessageKey::default(), Flags::default(), 0, 5);
        assert_eq!((s.from.email.as_str(), s.message_id.as_str()), ("zoe@studio.example", "m1@x"));
        let own = json!({"internetMessageHeaders": [{"name": "Subject", "value": "Own"}, {"name": "Bad Name", "value": "x"}]});
        assert_eq!(String::from_utf8(header_block(&own)).unwrap(), "Subject: Own\r\n\r\n");
        let f = flags(&json!({"isRead": true, "flag": {"flagStatus": "flagged"}, "isDraft": false}));
        assert!(f.seen && f.flagged && !f.draft);
    }

    #[test]
    fn times_for_a_filter() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        let t = jmap::utc_date("2026-10-23T09:05:07Z").unwrap();
        assert_eq!(utc(t), "2026-10-23T09:05:07Z");
        assert_eq!(received(&json!({"receivedDateTime": "2026-10-23T09:05:07Z"})), Some(t));
        assert_eq!(received(&json!({"receivedDateTime": "soon"})), None);
    }

    #[test]
    fn errors() {
        assert!(matches!(status_error(401, b""), Err(NetError::Auth(_))));
        assert!(matches!(status_error(503, b""), Err(NetError::Closed(_))));
        let e = status_error(403, br#"{"error": {"code": "ErrorAccessDenied", "message": "Access is denied."}}"#);
        assert!(matches!(e, Err(NetError::No(m)) if m == "HTTP 403 Access is denied."));
        assert!(Client::connect("http://graph.example.com/v1.0", "t").is_err(), "https only, except on this computer");
    }
}
