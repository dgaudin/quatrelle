//! Microsoft Graph mail: the transport for Exchange Online (Microsoft 365, Outlook.com), behind
//! the same [`Job`](mailcraft_model::Job)/[`Event`](mailcraft_model::Event) interface as IMAP
//! and JMAP. It works where an organisation has switched IMAP off.
//!
//! Every request is HTTPS with an OAuth bearer token (`Mail.ReadWrite`, `Mail.Send`,
//! `MailboxSettings.ReadWrite`), against the documented REST API: mail folders, messages,
//! their MIME source (`$value`), `sendMail` with a MIME body, and mailbox settings for automatic
//! replies. Ids are asked for in their immutable form (`Prefer: IdType="ImmutableId"`), so a
//! message keeps its id when it moves, as JMAP's do: the numbering and folder paths are the
//! JMAP transport's ([`jmap::State`], [`jmap::folders`]). Several small requests go as one
//! JSON batch (20 at a time).

use std::collections::HashMap;

use mailcraft_mime as mime;
use mailcraft_model::organize::{Condition, Rule, RuleAction};
use mailcraft_model::{Flag, Flags, Folder, FolderRole, MessageKey, Op, Summary, sort_newest_first, utf7, voting};
use mailcraft_store::{FolderState, Store};
use serde_json::{Value, json};

use crate::http::{self, Request, Response};
use crate::jmap::{self, Mailbox, State, VALIDITY};
use crate::sync::{SyncOptions, SyncReport, fill_from_source};
use crate::{NetError, Result};

/// The Graph endpoint accounts use.
pub const DEFAULT_URL: &str = "https://graph.microsoft.com/v1.0";
/// Largest JSON response accepted.
const MAX_JSON: usize = 32 << 20;
/// Largest message source downloaded.
const MAX_MIME: usize = 64 << 20;
/// Requests in one batch (Graph's limit).
const BATCH: usize = 20;
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
pub(crate) const NO_MAILBOX: &str = "this Microsoft account has no Outlook.com or Microsoft 365 mailbox that MailCraft can reach. \
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

    fn request(&self, mut r: Request) -> Result<Value> {
        r = r
            .header("Authorization", &self.auth)
            .header("Accept", "application/json")
            .header("Prefer", "IdType=\"ImmutableId\", outlook.timezone=\"UTC\"");
        let resp = http::send(&r)?;
        check(&resp)?;
        if resp.body.iter().all(u8::is_ascii_whitespace) {
            return Ok(Value::Null);
        }
        json_body(&resp.body)
    }

    pub(crate) fn get(&self, path: &str) -> Result<Value> {
        self.request(Request::get(&format!("{}{path}", self.base), MAX_JSON))
    }

    pub(crate) fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let bytes = serde_json::to_vec(body).map_err(|e| NetError::Protocol(e.to_string()))?;
        self.request(Request::post(&format!("{}{path}", self.base), "application/json", bytes, MAX_JSON))
    }

    pub(crate) fn send_raw(&self, method: &str, path: &str, content_type: &str, body: Vec<u8>) -> Result<Value> {
        let mut r = Request::post(&format!("{}{path}", self.base), content_type, body, MAX_JSON);
        r.method = method.into();
        self.request(r)
    }

    /// Every item of a listing, following `@odata.nextLink` (on the same server only).
    pub(crate) fn all(&self, path: &str, limit: usize) -> Result<Vec<Value>> {
        let origin = http::Url::parse(&self.base)?.origin();
        let mut out = Vec::new();
        let mut url = format!("{}{path}", self.base);
        for _ in 0..MAX_PAGES {
            let v = self.request(Request::get(&url, MAX_JSON))?;
            out.extend(v.get("value").and_then(Value::as_array).cloned().unwrap_or_default());
            if out.len() >= limit {
                out.truncate(limit);
                break;
            }
            match v.get("@odata.nextLink").and_then(Value::as_str) {
                Some(next) if http::Url::parse(next).is_ok_and(|u| u.origin() == origin) => url = next.to_string(),
                _ => break,
            }
        }
        Ok(out)
    }

    /// Run requests as JSON batches; each answer's (status, body), in order.
    fn batch(&self, requests: &[(&str, String, Option<Value>)]) -> Result<Vec<(u16, Value)>> {
        let mut out = Vec::with_capacity(requests.len());
        for (n, chunk) in requests.chunks(BATCH).enumerate() {
            let reqs: Vec<Value> = chunk
                .iter()
                .enumerate()
                .map(|(i, (method, url, body))| {
                    let mut r = json!({"id": (i + 1).to_string(), "method": method, "url": url});
                    if let Some(b) = body {
                        r["body"] = b.clone();
                        r["headers"] = json!({"Content-Type": "application/json"});
                    }
                    r
                })
                .collect();
            let v = self.post("/$batch", &json!({ "requests": reqs }))?;
            let answers = v.get("responses").and_then(Value::as_array).cloned().unwrap_or_default();
            for i in 0..chunk.len() {
                let id = (i + 1).to_string();
                let a = answers.iter().find(|a| a.get("id").and_then(Value::as_str) == Some(id.as_str()));
                let status = a.and_then(|a| a.get("status")).and_then(Value::as_u64).and_then(|s| u16::try_from(s).ok()).unwrap_or(0);
                let body = a.and_then(|a| a.get("body")).cloned().unwrap_or(Value::Null);
                if status == 0 {
                    return Err(NetError::Protocol(format!("Graph: no answer in batch {n}")));
                }
                out.push((status, body));
            }
        }
        Ok(out)
    }

    /// Run requests as batches and fail on the first refusal.
    fn batch_ok(&self, requests: &[(&str, String, Option<Value>)]) -> Result<()> {
        for (status, body) in self.batch(requests)? {
            // Already gone is fine for a delete or a move.
            if status == 404 {
                continue;
            }
            let bytes = serde_json::to_vec(&body).unwrap_or_default();
            status_error(status, &bytes)?;
        }
        Ok(())
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

    /// The newest `limit` messages of a folder with their flags, and its (total, unread).
    fn newest(&self, folder: &str, limit: usize, owner: Option<&str>) -> Result<(Vec<(String, Flags)>, (u32, u32))> {
        let me = who(owner);
        let list = self.all(
            &format!(
                "{me}/mailFolders/{}/messages?$top={}&$orderby=receivedDateTime%20desc&$select=id,isRead,isDraft,flag",
                seg(folder),
                limit.clamp(1, 1000)
            ),
            limit.min(100_000),
        )?;
        let ids = list.iter().filter_map(|m| Some((m.get("id")?.as_str()?.to_string(), flags(m)))).collect();
        let f = self.get(&format!("{me}/mailFolders/{}?$select=totalItemCount,unreadItemCount", seg(folder)))?;
        let count = |k: &str| f.get(k).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
        Ok((ids, (count("totalItemCount"), count("unreadItemCount"))))
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
        let resp = http::send(&r)?;
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

    /// The automatic replies.
    pub fn vacation(&self) -> Result<mailcraft_model::job::Vacation> {
        let v = self.get("/me/mailboxSettings/automaticRepliesSetting")?;
        let status = v.get("status").and_then(Value::as_str).unwrap_or("disabled").to_ascii_lowercase();
        let day = |k: &str| {
            v.get(k).and_then(|d| d.get("dateTime")).and_then(Value::as_str).and_then(|d| d.get(..10)).filter(|d| mailcraft_model::job::is_day(d))
        };
        let scheduled = status == "scheduled";
        let reply = |k: &str| {
            let html = v.get(k).and_then(Value::as_str).unwrap_or_default();
            if html.contains('<') { mime::html_to_text(html).trim().to_string() } else { html.trim().to_string() }
        };
        let (text, external) = (reply("internalReplyMessage"), reply("externalReplyMessage"));
        let outside = match v.get("externalAudience").and_then(Value::as_str).unwrap_or("all").to_ascii_lowercase().as_str() {
            "none" => "none",
            "contactsonly" => "known",
            _ => "",
        };
        Ok(mailcraft_model::job::Vacation {
            enabled: status != "disabled",
            subject: String::new(),
            external: (!external.is_empty() && external != text).then_some(external),
            outside: outside.to_string(),
            text,
            from: day("scheduledStartDateTime").filter(|_| scheduled).map(str::to_string),
            until: day("scheduledEndDateTime").filter(|_| scheduled).map(str::to_string),
        })
    }

    pub fn set_vacation(&self, v: &mailcraft_model::job::Vacation) -> Result<()> {
        let html = |t: &str| t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\n', "<br>");
        let dated = v.from.is_some() || v.until.is_some();
        let audience = match v.outside.as_str() {
            "none" => "none",
            "known" => "contactsOnly",
            _ => "all",
        };
        let mut s = json!({
            "status": if !v.enabled { "disabled" } else if dated { "scheduled" } else { "alwaysEnabled" },
            "externalAudience": audience,
            "internalReplyMessage": html(&v.text),
            "externalReplyMessage": html(v.external_text()),
        });
        if v.enabled && dated {
            // Whole days: from the start of the first to the end of the last (open ends far off).
            let from = v.from.clone().unwrap_or_else(|| "2000-01-01".into());
            let until = v.until.clone().unwrap_or_else(|| "2099-12-31".into());
            s["scheduledStartDateTime"] = json!({"dateTime": format!("{from}T00:00:00"), "timeZone": "UTC"});
            s["scheduledEndDateTime"] = json!({"dateTime": format!("{until}T23:59:59"), "timeZone": "UTC"});
        }
        let body = serde_json::to_vec(&json!({ "automaticRepliesSetting": s })).map_err(|e| NetError::Protocol(e.to_string()))?;
        self.send_raw("PATCH", "/me/mailboxSettings", "application/json", body).map(|_| ())
    }

    /// The ids of the Inbox rules Graph keeps, by name.
    pub fn message_rule_ids(&self) -> Result<Vec<(String, String)>> {
        let v = self.get("/me/mailFolders/inbox/messageRules?$select=id,displayName&$top=500")?;
        Ok(v.get("value")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(|r| Some((r.get("displayName")?.as_str()?.to_string(), r.get("id")?.as_str()?.to_string()))).collect())
            .unwrap_or_default())
    }

    /// Create, change and remove the Inbox rules Graph runs, from MailCraft's rules, as
    /// [`ews::Client::set_inbox_rules`](crate::ews::Client::set_inbox_rules) does.
    pub fn set_message_rules(&self, wanted: &[(Rule, Option<String>)], delete: &[String]) -> Result<Vec<(String, String)>> {
        if wanted.is_empty() && delete.is_empty() {
            return Ok(Vec::new());
        }
        let before: Vec<String> = self.message_rule_ids()?.into_iter().map(|(_, id)| id).collect();
        for (i, (r, folder)) in wanted.iter().enumerate() {
            let body = message_rule_json(r, folder.as_deref(), i + 1);
            match r.remote_id.as_deref().filter(|id| !id.is_empty()) {
                Some(id) => {
                    let path = format!("/me/mailFolders/inbox/messageRules/{}", seg(id));
                    self.send_raw("PATCH", &path, "application/json", serde_json::to_vec(&body).map_err(|e| NetError::Protocol(e.to_string()))?)?;
                }
                None => {
                    self.post("/me/mailFolders/inbox/messageRules", &body)?;
                }
            }
        }
        for id in delete.iter().filter(|id| !id.is_empty()) {
            let path = format!("/me/mailFolders/inbox/messageRules/{}", seg(id));
            self.send_raw("DELETE", &path, "application/json", Vec::new())?;
        }
        let after = self.message_rule_ids()?;
        let mut out = Vec::new();
        for (r, _) in wanted {
            let id = match r.remote_id.as_deref().filter(|id| !id.is_empty()) {
                Some(id) => Some(id.to_string()),
                None => after.iter().find(|(name, id)| name == &r.name && !before.contains(id)).map(|(_, id)| id.clone()),
            };
            match id {
                Some(id) => out.push((r.name.clone(), id)),
                None => log::warn!("the rule {} was sent to Graph but isn't listed there", r.name),
            }
        }
        Ok(out)
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

/// A rule as Graph's `messageRules` hold it. Only the parts Exchange can express are written;
/// the engine never sends the others (`Rule::server_blockers`).
fn message_rule_json(r: &Rule, folder: Option<&str>, sequence: usize) -> Value {
    let mut conditions = serde_json::Map::new();
    let mut subjects: Vec<String> = Vec::new();
    let mut senders: Vec<String> = Vec::new();
    let mut recipients: Vec<String> = Vec::new();
    for c in &r.conditions {
        match c {
            Condition::Subject(t) => subjects.push(t.clone()),
            Condition::From(t) => senders.push(t.clone()),
            Condition::To(t) => recipients.push(t.clone()),
            Condition::HasAttachment => {
                conditions.insert("hasAttachments".into(), Value::Bool(true));
            }
            Condition::AnyText(_) | Condition::Bulk => {}
        }
    }
    if !subjects.is_empty() {
        conditions.insert("subjectContains".into(), json!(subjects));
    }
    if !senders.is_empty() {
        conditions.insert("senderContains".into(), json!(senders));
    }
    if !recipients.is_empty() {
        conditions.insert("recipientContains".into(), json!(recipients));
    }
    let mut actions = serde_json::Map::new();
    for a in &r.actions {
        match a {
            RuleAction::MoveTo(_) | RuleAction::MoveToRole(_) => {
                if let Some(folder) = folder {
                    actions.insert("moveToFolder".into(), Value::String(folder.to_string()));
                }
            }
            RuleAction::MarkRead => {
                actions.insert("markAsRead".into(), Value::Bool(true));
            }
            RuleAction::Categorize(c) => {
                let list = actions.entry("assignCategories").or_insert_with(|| json!([]));
                if let Some(list) = list.as_array_mut() {
                    list.push(Value::String(c.clone()));
                }
            }
            RuleAction::Delete => {
                actions.insert("delete".into(), Value::Bool(true));
            }
            RuleAction::Flag | RuleAction::Pin => {}
        }
    }
    if r.stop {
        actions.insert("stopProcessingRules".into(), Value::Bool(true));
    }
    json!({
        "displayName": r.name,
        "sequence": sequence,
        "isEnabled": r.enabled,
        "conditions": Value::Object(conditions),
        "actions": Value::Object(actions),
    })
}

/// Refresh the folder list (and the folder ids behind it).
pub fn list_folders(c: &Client, st: &mut State, account: &str, shared: &[String]) -> Result<Vec<Folder>> {
    let mut boxes = c.mailboxes()?;
    // Other mailboxes opened here, each under a folder named after it. One this account can't
    // open (no right to it, or gone) is left out; the others still list.
    let mut owner_of_box: HashMap<String, String> = HashMap::new();
    for owner in shared {
        match c.shared_mailbox(owner) {
            Ok(theirs) => {
                for b in &theirs {
                    owner_of_box.insert(b.id.clone(), owner.clone());
                }
                boxes.extend(theirs);
            }
            Err(e) => log::warn!("{owner}'s mailbox: {e}"),
        }
    }
    let mut found = jmap::folders(account, &boxes);
    st.mailboxes = found.iter().map(|(f, id)| (f.path.clone(), id.clone())).collect();
    st.owners = found.iter().filter_map(|(f, id)| Some((f.path.clone(), owner_of_box.get(id)?.clone()))).collect();
    jmap::name_special(&mut found, &boxes);
    Ok(found.into_iter().map(|(f, _)| f).collect())
}

/// Bring one folder's cache up to date.
pub fn sync_folder(c: &Client, st: &mut State, store: &Store, account: &str, folder: &str, opts: &SyncOptions) -> Result<SyncReport> {
    let id = st.mailbox(folder)?.to_string();
    let owner = st.owner(folder).map(str::to_string);
    let owner = owner.as_deref();
    let (newest, (total, unread)) = c.newest(&id, opts.window, owner)?;
    let mut fs = store.load_folder(account, folder)?;
    let mut report = SyncReport::default();
    if fs.validity != VALIDITY && (fs.validity != 0 || !fs.messages.is_empty()) {
        store.clear_folder(account, folder)?;
        fs = FolderState::default();
        report.reset = true;
    }
    fs.validity = VALIDITY;
    let wanted: HashMap<u32, Flags> = newest.iter().map(|(id, f)| (st.number(id), f.clone())).collect();
    let (kept, gone): (Vec<Summary>, Vec<Summary>) = std::mem::take(&mut fs.messages).into_iter().partition(|m| wanted.contains_key(&m.key.uid));
    for m in &gone {
        store.remove_body(&m.key);
    }
    report.removed = gone.len();
    fs.messages = kept;
    for m in &mut fs.messages {
        // Graph has no answered flag or categories here: keep those as they are.
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
    for m in c.headers(&missing, owner)? {
        let Some(id) = m.get("id").and_then(Value::as_str) else { continue };
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: st.number(id) };
        if fs.find(key.uid).is_some() {
            continue;
        }
        let received = m.get("receivedDateTime").and_then(Value::as_str).and_then(jmap::utc_date).unwrap_or(0);
        let mut s = mime::summary_from_header(&header_block(&m), key, flags(&m), 0, received);
        s.preview = m.get("bodyPreview").and_then(Value::as_str).map(mime::preview).unwrap_or_default();
        s.has_attachments = m.get("hasAttachments").and_then(Value::as_bool).unwrap_or(false);
        s.voting = voting_choices(&m);
        fs.messages.push(s);
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

fn prefetch(c: &Client, st: &State, store: &Store, account: &str, folder: &str, fs: &mut FolderState, opts: &SyncOptions) -> Result<()> {
    let wanted: Vec<(u32, String)> = fs
        .messages
        .iter()
        .take(opts.prefetch)
        // Graph lists no sizes: mail with attachments waits until it's opened.
        .filter(|m| !m.has_attachments && !store.has_body(&m.key))
        .filter_map(|m| Some((m.key.uid, st.id(m.key.uid)?.to_string())))
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }
    let mut failures = crate::sync::AheadFailures::new();
    for (n, id) in &wanted {
        let raw = match c.download(id, st.owner(folder)) {
            Ok(raw) => {
                failures.ok();
                raw
            }
            Err(e) => {
                failures.failed(&format!("{account}/{folder}: message {n}"), e)?;
                continue;
            }
        };
        if raw.len() > usize::try_from(opts.prefetch_max_bytes).unwrap_or(usize::MAX) {
            continue;
        }
        let Some(m) = fs.find_mut(*n) else { continue };
        store.write_body(&m.key, &raw)?;
        fill_from_source(m, &raw);
    }
    store.save_folder(account, folder, fs)?;
    Ok(())
}

/// Download one message's source (and cache it).
pub fn fetch_body(c: &Client, st: &State, store: &Store, key: &MessageKey) -> Result<Vec<u8>> {
    if let Some(raw) = store.read_body(key) {
        return Ok(raw);
    }
    let id = st.id(key.uid).filter(|_| key.validity == VALIDITY).ok_or_else(|| NetError::No("the message is no longer on the server".into()))?;
    let raw = c.download(id, st.owner(&key.folder))?;
    store.write_body(key, &raw)?;
    let mut fs = store.load_folder(&key.account, &key.folder)?;
    if let Some(m) = fs.find_mut(key.uid) {
        fill_from_source(m, &raw);
        store.save_folder(&key.account, &key.folder, &fs)?;
    }
    Ok(raw)
}

/// The voting buttons on a message (its `PidLidVerbStream`, when asked for).
fn voting_choices(m: &Value) -> Vec<String> {
    m.get("singleValueExtendedProperties")
        .and_then(Value::as_array)
        .and_then(|props| props.iter().find(|p| p.get("id").and_then(Value::as_str).is_some_and(|id| id.to_ascii_lowercase().ends_with("id 0x8520"))))
        .and_then(|p| p.get("value").and_then(Value::as_str))
        .and_then(|v| crate::ews::unb64(v.trim()))
        .map(|b| voting::choices(&b))
        .unwrap_or_default()
}

/// Vote on a message (see [`Client::vote`]).
pub fn vote(c: &Client, st: &State, key: &MessageKey, choice: &str, subject: &str) -> Result<()> {
    let id = st.id(key.uid).filter(|_| key.validity == VALIDITY).ok_or_else(|| NetError::No("the message is no longer on the server".into()))?;
    c.vote(id, choice, subject, st.owner(&key.folder))
}

/// Apply one change on the server.
pub fn apply_remote(c: &Client, st: &State, op: &Op) -> Result<()> {
    // Each message by its id, in its own mailbox (another one opened here is under `/users`).
    let ids = |keys: &[MessageKey]| -> Vec<(String, String)> {
        keys.iter().filter(|k| k.validity == VALIDITY).filter_map(|k| Some((st.id(k.uid)?.to_string(), k.folder.clone()))).collect()
    };
    let at = |folder: &str| who(st.owner(folder));
    let patch_all = |keys: &[MessageKey], patch: Value| -> Result<()> {
        let reqs: Vec<(&str, String, Option<Value>)> =
            ids(keys).into_iter().map(|(id, f)| ("PATCH", format!("{}/messages/{}", at(&f), seg(&id)), Some(patch.clone()))).collect();
        c.batch_ok(&reqs)
    };
    match op {
        Op::SetFlag { keys, flag: Flag::Seen, on } => patch_all(keys, json!({"isRead": on})),
        Op::SetFlag { keys, flag: Flag::Flagged, on } => patch_all(keys, json!({"flag": {"flagStatus": if *on { "flagged" } else { "notFlagged" }}})),
        // Answered and forwarded are Exchange's to set; categories stay on this computer.
        Op::SetFlag { .. } | Op::SetKeyword { .. } => Ok(()),
        Op::Move { keys, to } => {
            let dest = st.mailbox(to)?.to_string();
            let reqs: Vec<(&str, String, Option<Value>)> = ids(keys)
                .into_iter()
                .filter(|(_, from)| from != to)
                .map(|(id, f)| ("POST", format!("{}/messages/{}/move", at(&f), seg(&id)), Some(json!({"destinationId": dest}))))
                .collect();
            c.batch_ok(&reqs)
        }
        Op::Expunge { keys } => {
            let reqs: Vec<(&str, String, Option<Value>)> =
                ids(keys).into_iter().map(|(id, f)| ("DELETE", format!("{}/messages/{}", at(&f), seg(&id)), None)).collect();
            c.batch_ok(&reqs)
        }
        Op::CreateFolder { path } => {
            let (parent, name) = split_path(st, path)?;
            let me = at(path);
            let url = parent.map_or_else(|| format!("{me}/mailFolders"), |p| format!("{me}/mailFolders/{}/childFolders", seg(&p)));
            c.post(&url, &json!({"displayName": name})).map(|_| ())
        }
        Op::RenameFolder { from, to } => {
            let id = st.mailbox(from)?.to_string();
            let (old_parent, _) = split_path(st, from)?;
            let (parent, name) = split_path(st, to)?;
            let mut id = id;
            if parent != old_parent {
                let dest = parent.clone().unwrap_or_else(|| "msgfolderroot".into());
                let moved = c.post(&format!("{}/mailFolders/{}/move", at(from), seg(&id)), &json!({"destinationId": dest}))?;
                id = moved.get("id").and_then(Value::as_str).unwrap_or(&id).to_string();
            }
            let body = serde_json::to_vec(&json!({"displayName": name})).map_err(|e| NetError::Protocol(e.to_string()))?;
            c.send_raw("PATCH", &format!("{}/mailFolders/{}", at(from), seg(&id)), "application/json", body).map(|_| ())
        }
        Op::DeleteFolder { path } => {
            let id = st.mailbox(path)?.to_string();
            c.send_raw("DELETE", &format!("{}/mailFolders/{}", at(path), seg(&id)), "application/json", Vec::new()).map(|_| ())
        }
        Op::EmptyFolder { path } => c.empty(st.mailbox(path)?, st.owner(path)),
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
    c.import(raw, Some(st.mailbox(folder)?), st.owner(folder))
}

/// Send a message. The MIME source names the recipients for Exchange, so anyone only on the
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
    fn errors() {
        assert!(matches!(status_error(401, b""), Err(NetError::Auth(_))));
        assert!(matches!(status_error(503, b""), Err(NetError::Closed(_))));
        let e = status_error(403, br#"{"error": {"code": "ErrorAccessDenied", "message": "Access is denied."}}"#);
        assert!(matches!(e, Err(NetError::No(m)) if m == "HTTP 403 Access is denied."));
        assert!(Client::connect("http://graph.example.com/v1.0", "t").is_err(), "https only, except on this computer");
    }
}
