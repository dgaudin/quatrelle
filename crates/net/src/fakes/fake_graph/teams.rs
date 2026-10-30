//! The fake Graph server's Teams: the signed-in user, chats, their members and messages (listed
//! in a `lastModifiedDateTime` range, posted, edited, reacted to, soft-deleted), marking a chat
//! read and shared files, as Graph v1.0 answers them.
//!
//! Each request is counted in [`FakeTeams::hits`] and its headers kept in
//! [`FakeTeams::last_headers`] under its route name: `me`, `chats`, `members`, `messages`,
//! `post`, `edit`, `setReaction`, `unsetReaction`, `softDelete`, `markChatReadForUser`,
//! `shares`, `download`, and `unknown` for a Teams path no route answers (404).

use std::collections::HashMap;

use super::*;

/// A Microsoft Entra user, as the fake knows one.
#[derive(Clone, Debug, Default)]
pub(crate) struct FakeUser {
    pub(crate) id: String,
    pub(crate) display_name: String,
    pub(crate) mail: String,
    pub(crate) upn: String,
}

impl FakeUser {
    /// A user whose address is made from the first word of the name.
    pub(crate) fn new(id: &str, name: &str) -> Self {
        let first = name.split_whitespace().next().unwrap_or("user").to_lowercase();
        let mail = format!("{first}@contoso.example");
        Self { id: id.into(), display_name: name.into(), upn: mail.clone(), mail }
    }
}

/// A chat: `kind` is Graph's `chatType` (`oneOnOne`, `group` or `meeting`).
#[derive(Clone, Debug, Default)]
pub(crate) struct FakeChat {
    pub(crate) id: String,
    pub(crate) kind: &'static str,
    /// `None` for a chat without a topic (a one-on-one chat has none).
    pub(crate) topic: Option<String>,
    pub(crate) members: Vec<FakeUser>,
    /// When the signed-in user last read it (`viewpoint.lastMessageReadDateTime`).
    pub(crate) read_time: String,
    pub(crate) messages: Vec<FakeMessage>,
}

impl FakeChat {
    pub(crate) fn new(id: &str, kind: &'static str, topic: Option<&str>, members: Vec<FakeUser>) -> Self {
        Self { id: id.into(), kind, topic: topic.map(str::to_string), members, read_time: "2026-01-01T00:00:00.000Z".into(), messages: Vec::new() }
    }
}

/// A chat message; its times are RFC 3339 strings, as Graph sends them.
#[derive(Clone, Debug, Default)]
pub(crate) struct FakeMessage {
    /// A millisecond-like number, increasing within its chat.
    pub(crate) id: String,
    /// The sender's user id (`None`: a system message).
    pub(crate) from: Option<String>,
    pub(crate) created: String,
    pub(crate) modified: String,
    pub(crate) edited: Option<String>,
    pub(crate) deleted: Option<String>,
    pub(crate) body_html: String,
    /// (reactionType, user id).
    pub(crate) reactions: Vec<(String, String)>,
    /// (contentType, contentUrl).
    pub(crate) attachments: Vec<(String, String)>,
}

impl FakeMessage {
    /// A message created (and last changed) at `at`.
    pub(crate) fn new(id: &str, from: Option<&str>, at: &str, body_html: &str) -> Self {
        Self {
            id: id.into(),
            from: from.map(str::to_string),
            created: at.into(),
            modified: at.into(),
            body_html: body_html.into(),
            ..Self::default()
        }
    }
}

/// The Teams side of the fake: what the signed-in user sees, and what the tests read back.
#[derive(Clone, Debug, Default)]
pub(crate) struct FakeTeams {
    pub(crate) me: FakeUser,
    pub(crate) tenant: String,
    pub(crate) chats: Vec<FakeChat>,
    /// Answer every n-th Teams request with 429 and `Retry-After: 1`.
    pub(crate) throttle_every: Option<u32>,
    /// Chats (their ids) where posting is refused with 403.
    pub(crate) forbid_post: Vec<String>,
    /// The next n requests of a route (by route name) answered 500, then the route works again.
    pub(crate) fail_next: HashMap<String, u32>,
    /// The next n requests of a route carried out, but answered 503 (the answer lost on the way).
    pub(crate) answer_lost: HashMap<String, u32>,
    /// Chats (their ids) whose message reads are answered 500.
    pub(crate) fail_reads: Vec<String>,
    /// Every read of a chat's messages: (chat id, its `$filter` as sent, empty when none).
    pub(crate) message_reads: Vec<(String, String)>,
    /// Shared files' bytes, by the `contentUrl` an attachment carries.
    pub(crate) shares: HashMap<String, Vec<u8>>,
    /// Requests received, per route name.
    pub(crate) hits: HashMap<String, u32>,
    /// The headers of the last request per route name, their names lowercased.
    pub(crate) last_headers: HashMap<String, Vec<(String, String)>>,
}

impl FakeTeams {
    pub(crate) fn new(me: FakeUser, tenant: &str) -> Self {
        Self { me, tenant: tenant.into(), ..Self::default() }
    }

    pub(crate) fn chat(&self, id: &str) -> Option<&FakeChat> {
        self.chats.iter().find(|c| c.id == id)
    }
}

/// A time in milliseconds since 1970 as Graph writes it: `2026-10-30T09:12:34.567Z`.
pub(crate) fn rfc3339(ms: i64) -> String {
    let (secs, milli) = (ms.div_euclid(1000), ms.rem_euclid(1000));
    let (y, m, d) = quatrelle_model::dates::civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{milli:03}Z", s / 3600, s / 60 % 60, s % 60)
}

/// One answer: status, body, content type, where a redirect points, and the fake's clock when
/// it answered (Unix seconds, sent as `Date`).
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) body: Vec<u8>,
    pub(super) kind: &'static str,
    pub(super) location: Option<String>,
    pub(super) date: Option<i64>,
}

impl Reply {
    fn json(status: u16, v: &Value) -> Self {
        Self { status, body: v.to_string().into_bytes(), kind: "application/json", location: None, date: None }
    }

    fn empty(status: u16) -> Self {
        Self { status, body: Vec::new(), kind: "application/json", location: None, date: None }
    }

    fn error(status: u16, code: &str, message: &str) -> Self {
        Self::json(status, &json!({"error": {"code": code, "message": message}}))
    }
}

/// A request's route name (the key of `hits` and `last_headers`), from its method and its
/// decoded path; `/me/chats/{id}/…` and `/users/{oid}/chats/{id}/…` are taken as `/chats/{id}/…`.
/// Every Teams-shaped path is claimed (`unknown` when no route answers it), so a Teams client
/// never reaches the mail routes.
fn route_name(method: &str, p: &[&str]) -> Option<&'static str> {
    Some(match (method, p) {
        ("GET", ["me"]) => "me",
        ("GET", ["me", "chats"]) => "chats",
        ("GET", ["chats", _, "members"]) => "members",
        ("GET", ["chats", _, "messages"]) => "messages",
        ("POST", ["chats", _, "messages"]) => "post",
        ("PATCH", ["chats", _, "messages", _]) => "edit",
        ("POST", ["chats", _, "messages", _, "setReaction"]) => "setReaction",
        ("POST", ["chats", _, "messages", _, "unsetReaction"]) => "unsetReaction",
        ("POST", ["chats", _, "messages", _, "softDelete"]) => "softDelete",
        ("POST", ["chats", _, "markChatReadForUser"]) => "markChatReadForUser",
        ("GET", ["shares", _, "driveItem", "content"]) => "shares",
        ("GET", ["download", _]) => "download",
        (_, ["chats", ..] | ["shares", ..] | ["download", ..]) => "unknown",
        _ => return None,
    })
}

/// A query's parameters, names and values decoded (`+` as a space, as a form encodes it).
fn params(query: &str) -> HashMap<String, String> {
    let dec = |s: &str| quatrelle_model::mailto::percent_decode(&s.replace('+', " "));
    query.split('&').filter(|kv| !kv.is_empty()).map(|kv| kv.split_once('=').unwrap_or((kv, ""))).map(|(k, v)| (dec(k), dec(v))).collect()
}

/// An RFC 3339 time in milliseconds since 1970: `2026-10-30T09:12:34Z`, with or without a
/// fraction, `Z` or an offset.
fn ms_of(t: &str) -> Option<i64> {
    let num = |a: usize, b: usize| t.get(a..b).filter(|x| x.bytes().all(|c| c.is_ascii_digit())).and_then(|x| x.parse::<i64>().ok());
    let seps = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')];
    if !seps.iter().all(|(at, c)| t.as_bytes().get(*at).is_some_and(|b| b.eq_ignore_ascii_case(c))) {
        return None;
    }
    let (y, mo, d, h, mi, s) = (num(0, 4)?, num(5, 7)?, num(8, 10)?, num(11, 13)?, num(14, 16)?, num(17, 19)?);
    let mut rest = t.get(19..)?;
    let mut milli = 0;
    if let Some(r) = rest.strip_prefix('.') {
        let digits = r.bytes().take_while(u8::is_ascii_digit).count();
        let frac = r.get(..digits)?;
        milli = format!("{:0<3}", frac.get(..3.min(digits))?).parse::<i64>().ok()?;
        rest = r.get(digits..)?;
    }
    let offset = match rest.as_bytes() {
        [b'Z' | b'z'] => 0,
        [sign @ (b'+' | b'-'), ..] if rest.len() == 6 && rest.get(3..4) == Some(":") => {
            let o = rest.get(1..3)?.parse::<i64>().ok()? * 3600 + rest.get(4..6)?.parse::<i64>().ok()? * 60;
            if *sign == b'+' { o } else { -o }
        }
        _ => return None,
    };
    let days = quatrelle_model::dates::days_from_civil(y, u32::try_from(mo).ok()?, u32::try_from(d).ok()?);
    Some(((days * 86_400 + h * 3600 + mi * 60 + s - offset) * 1000).saturating_add(milli))
}

/// `$top`: `None` when absent, `Err` when it isn't a number from 1 to 50.
fn top(q: &HashMap<String, String>, default: usize) -> Result<usize, Reply> {
    match q.get("$top") {
        None => Ok(default),
        Some(t) => {
            t.parse::<usize>().ok().filter(|n| (1..=50).contains(n)).ok_or_else(|| Reply::error(400, "BadRequest", "$top must be from 1 to 50."))
        }
    }
}

/// One page of a listing at `$skiptoken`, with `@odata.nextLink` for the rest.
fn page(list: Vec<Value>, top: usize, q: &HashMap<String, String>, path: &str, query: &str, base: &str) -> Reply {
    let skip: usize = q.get("$skiptoken").and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut v = json!({ "value": list.iter().skip(skip).take(top).cloned().collect::<Vec<_>>() });
    if skip.saturating_add(top) < list.len() {
        let rest: Vec<&str> =
            query.split('&').filter(|kv| !kv.is_empty() && !kv.starts_with("$skiptoken=") && !kv.starts_with("%24skiptoken=")).collect();
        let sep = if rest.is_empty() { "" } else { "&" };
        v["@odata.nextLink"] = json!(format!("{base}{path}?{}{sep}$skiptoken={}", rest.join("&"), skip + top));
    }
    Reply::json(200, &v)
}

/// A shared file's key in `shares`: the URL a `u!` share id encodes (base64url, unpadded), or
/// the id itself.
fn share_key(teams: &FakeTeams, id: &str) -> Option<String> {
    let url = id.strip_prefix("u!").map(|b| String::from_utf8_lossy(&unbase64(b.replace('-', "+").replace('_', "/").as_bytes())).to_string());
    url.into_iter().chain(std::iter::once(id.to_string())).find(|k| teams.shares.contains_key(k))
}

impl FakeTeams {
    /// A user by id: a chat member or the signed-in user.
    fn user(&self, id: &str) -> Option<&FakeUser> {
        std::iter::once(&self.me).chain(self.chats.iter().flat_map(|c| &c.members)).find(|u| u.id == id)
    }

    /// Graph's `identitySet` for a user.
    fn identity(&self, id: &str) -> Value {
        json!({"application": null, "device": null, "user": {
            "@odata.type": "#microsoft.graph.teamworkUserIdentity", "id": id,
            "displayName": self.user(id).map(|u| u.display_name.clone()), "userIdentityType": "aadUser", "tenantId": self.tenant,
        }})
    }

    /// A time later than every time the fake holds and not before the clock: what is changed
    /// now is always seen as newer.
    fn now(&self) -> i64 {
        let clock = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0));
        let held = self.chats.iter().flat_map(|c| {
            c.messages
                .iter()
                .flat_map(|m| [Some(&m.created), Some(&m.modified), m.edited.as_ref(), m.deleted.as_ref()].into_iter().flatten())
                .chain(std::iter::once(&c.read_time))
        });
        held.filter_map(|t| ms_of(t)).map(|t| t + 1).fold(clock, i64::max)
    }

    fn message_json(&self, chat: &FakeChat, m: &FakeMessage) -> Value {
        json!({
            "id": m.id, "replyToId": null, "etag": ms_of(&m.modified).unwrap_or_default().to_string(), "messageType": "message",
            "createdDateTime": m.created, "lastModifiedDateTime": m.modified, "lastEditedDateTime": m.edited, "deletedDateTime": m.deleted,
            "subject": null, "summary": null, "chatId": chat.id, "importance": "normal", "locale": "en-us", "webUrl": null,
            "channelIdentity": null, "policyViolation": null, "eventDetail": null,
            "from": m.from.as_deref().map(|f| self.identity(f)),
            "body": {"contentType": "html", "content": if m.deleted.is_some() { "" } else { m.body_html.as_str() }},
            "attachments": m.attachments.iter().enumerate().map(|(i, (kind, url))| json!({
                "id": format!("{}-{i}", m.id), "contentType": kind, "contentUrl": url, "content": null,
                "name": url.rsplit('/').next(), "thumbnailUrl": null, "teamsAppId": null,
            })).collect::<Vec<_>>(),
            "mentions": [],
            "reactions": m.reactions.iter().map(|(kind, user)| json!({
                "reactionType": kind, "displayName": null, "reactionContentUrl": null, "createdDateTime": m.modified, "user": self.identity(user),
            })).collect::<Vec<_>>(),
        })
    }

    /// The newest message not deleted.
    fn newest(chat: &FakeChat) -> Option<&FakeMessage> {
        chat.messages.iter().filter(|m| m.deleted.is_none()).max_by_key(|m| ms_of(&m.created))
    }

    fn chat_json(&self, c: &FakeChat, preview: bool) -> Value {
        let created = c.messages.iter().map(|m| m.created.as_str()).min_by_key(|t| ms_of(t)).unwrap_or("2026-01-01T00:00:00.000Z");
        let updated = c.messages.iter().map(|m| m.modified.as_str()).max_by_key(|t| ms_of(t)).unwrap_or(created);
        let mut v = json!({
            "id": c.id, "topic": c.topic, "createdDateTime": created, "lastUpdatedDateTime": updated, "chatType": c.kind,
            "webUrl": format!("https://teams.microsoft.com/l/chat/{}/0?tenantId={}", c.id, self.tenant), "tenantId": self.tenant,
            "onlineMeetingInfo": null, "viewpoint": {"isHidden": false, "lastMessageReadDateTime": c.read_time},
        });
        if preview {
            v["lastMessagePreview"] = Self::newest(c).map_or(Value::Null, |m| {
                json!({
                    "id": m.id, "createdDateTime": m.created, "isDeleted": false, "messageType": "message", "eventDetail": null,
                    "body": {"contentType": "html", "content": m.body_html}, "from": m.from.as_deref().map(|f| self.identity(f)),
                })
            });
        }
        v
    }

    fn chats(&self, q: &HashMap<String, String>, path: &str, query: &str, base: &str) -> Reply {
        let top = match top(q, 20) {
            Ok(t) => t,
            Err(e) => return e,
        };
        let preview = q.get("$expand").is_some_and(|e| e.split(',').any(|x| x.trim() == "lastMessagePreview"));
        let mut chats: Vec<&FakeChat> = self.chats.iter().collect();
        if q.get("$orderby").is_some_and(|o| o.starts_with("lastMessagePreview/createdDateTime")) {
            chats.sort_by_key(|c| std::cmp::Reverse(Self::newest(c).and_then(|m| ms_of(&m.created))));
        }
        page(chats.into_iter().map(|c| self.chat_json(c, preview)).collect(), top, q, path, query, base)
    }

    /// A chat's messages in a `lastModifiedDateTime` range (`gt` and/or `lt`, joined by `and`).
    fn messages(&self, chat: &FakeChat, q: &HashMap<String, String>, path: &str, query: &str, base: &str) -> Reply {
        let top = match top(q, 20) {
            Ok(t) => t,
            Err(e) => return e,
        };
        let refused = || Reply::error(400, "BadRequest", "A lastModifiedDateTime range is wanted.");
        let Some(filter) = q.get("$filter") else { return refused() };
        let mut range: Vec<(bool, i64)> = Vec::new();
        for cond in filter.split(" and ") {
            match cond.split_whitespace().collect::<Vec<_>>().as_slice() {
                [field, op, at] if field.eq_ignore_ascii_case("lastModifiedDateTime") && ["gt", "lt"].contains(op) => match ms_of(at) {
                    Some(t) => range.push((*op == "gt", t)),
                    None => return refused(),
                },
                _ => return refused(),
            }
        }
        let by_modified = q.get("$orderby").is_some_and(|o| o.starts_with("lastModifiedDateTime"));
        let key = |m: &FakeMessage| ms_of(if by_modified { &m.modified } else { &m.created });
        let mut list: Vec<&FakeMessage> = chat
            .messages
            .iter()
            .filter(|m| ms_of(&m.modified).is_some_and(|t| range.iter().all(|(gt, at)| if *gt { t > *at } else { t < *at })))
            .collect();
        list.sort_by_key(|m| std::cmp::Reverse(key(m)));
        page(list.into_iter().map(|m| self.message_json(chat, m)).collect(), top, q, path, query, base)
    }

    /// One Teams request, by route name, on paths already decoded and normalized.
    fn answer(&mut self, name: &str, p: &[&str], query: &str, raw_path: &str, input: &Value, base: &str) -> Reply {
        let not_found = || Reply::error(404, "NotFound", "Not found.");
        let q = params(query);
        if name == "me" {
            let full = json!({"id": self.me.id, "displayName": self.me.display_name, "mail": self.me.mail, "userPrincipalName": self.me.upn});
            let Some(select) = q.get("$select") else { return Reply::json(200, &full) };
            let mut v = serde_json::Map::new();
            for k in select.split(',').map(str::trim) {
                if let Some(x) = full.get(k) {
                    v.insert(k.to_string(), x.clone());
                }
            }
            return Reply::json(200, &Value::Object(v));
        }
        if name == "chats" {
            return self.chats(&q, raw_path, query, base);
        }
        if name == "shares" || name == "download" {
            let Some(key) = p.get(1).and_then(|id| share_key(self, id)) else { return not_found() };
            if name == "shares" {
                let token = p.get(1).copied().unwrap_or_default();
                return Reply { status: 302, body: Vec::new(), kind: "text/plain", location: Some(format!("{base}/download/{token}")), date: None };
            }
            let bytes = self.shares.get(&key).cloned().unwrap_or_default();
            return Reply { status: 200, body: bytes, kind: "application/octet-stream", location: None, date: None };
        }
        let Some(ci) = p.get(1).and_then(|id| self.chats.iter().position(|c| c.id == *id)) else { return not_found() };
        let now = rfc3339(self.now());
        let mine = self.me.id.clone();
        let tenant = self.tenant.clone();
        let forbidden = self.forbid_post.iter().any(|f| Some(f.as_str()) == p.get(1).copied());
        let Some(chat) = self.chats.get_mut(ci) else { return not_found() };
        match name {
            "members" => {
                let members: Vec<Value> = chat
                    .members
                    .iter()
                    .map(|u| {
                        json!({
                            "@odata.type": "#microsoft.graph.aadUserConversationMember", "id": format!("{}#{}", chat.id, u.id), "roles": ["owner"],
                            "displayName": u.display_name, "userId": u.id, "email": u.mail, "tenantId": tenant,
                            "visibleHistoryStartDateTime": "0001-01-01T00:00:00Z",
                        })
                    })
                    .collect();
                Reply::json(200, &json!({ "value": members }))
            }
            "messages" => {
                let filter = q.get("$filter").cloned().unwrap_or_default();
                self.message_reads.push((chat.id.clone(), filter));
                if self.fail_reads.contains(&chat.id) {
                    return Reply::error(500, "InternalServerError", "This chat's messages fail, as the test asked.");
                }
                let Some(chat) = self.chats.get(ci).cloned() else { return not_found() };
                self.messages(&chat, &q, raw_path, query, base)
            }
            "post" if forbidden => Reply::error(403, "Forbidden", "Posting in this chat is not allowed."),
            "post" => {
                let after = chat.messages.iter().filter_map(|m| m.id.parse::<i64>().ok()).max().map_or(0, |m| m + 1);
                let id = ms_of(&now).unwrap_or_default().max(after).to_string();
                let html = input.pointer("/body/content").and_then(Value::as_str).unwrap_or_default();
                chat.messages.push(FakeMessage::new(&id, Some(&mine), &now, html));
                let chat = chat.clone();
                let made = chat.messages.last().map(|m| self.message_json(&chat, m)).unwrap_or(Value::Null);
                Reply::json(201, &made)
            }
            "markChatReadForUser" => {
                let user = &input["user"];
                if user["id"].as_str() != Some(mine.as_str()) || user["tenantId"].as_str() != Some(tenant.as_str()) {
                    return Reply::error(400, "BadRequest", "The user is not the signed-in user.");
                }
                chat.read_time = now;
                Reply::empty(204)
            }
            _ => {
                let Some(m) = p.get(3).and_then(|mid| chat.messages.iter_mut().find(|m| m.id == *mid)) else { return not_found() };
                let kind = input["reactionType"].as_str().unwrap_or_default().to_string();
                match name {
                    "edit" => {
                        if let Some(html) = input.pointer("/body/content").and_then(Value::as_str) {
                            m.body_html = html.to_string();
                        }
                        m.edited = Some(now.clone());
                    }
                    "setReaction" | "unsetReaction" if kind.is_empty() => return Reply::error(400, "BadRequest", "A reactionType is wanted."),
                    "setReaction" => {
                        if !m.reactions.iter().any(|(k, u)| *k == kind && *u == mine) {
                            m.reactions.push((kind, mine));
                        }
                    }
                    "unsetReaction" => m.reactions.retain(|(k, u)| !(*k == kind && *u == mine)),
                    "softDelete" => m.deleted = Some(now.clone()),
                    _ => return not_found(),
                }
                m.modified = now;
                Reply::empty(204)
            }
        }
    }
}

impl Data {
    /// A Teams request answered, or `None` when it isn't one (or the fake has no Teams). Each is
    /// counted in `hits` and its headers kept in `last_headers`, and dated by the fake's clock; a
    /// download needs no token and is never throttled (it is another server than Graph).
    pub(super) fn teams_request(
        &mut self,
        method: &str,
        target: &str,
        headers: &[(String, String)],
        authorized: bool,
        body: &[u8],
        base: &str,
    ) -> Option<Reply> {
        let t = self.teams.as_mut()?;
        let (raw_path, query) = target.split_once('?').unwrap_or((target, ""));
        let decoded: Vec<String> = raw_path.trim_start_matches('/').split('/').map(quatrelle_model::mailto::percent_decode).collect();
        let mut p: Vec<&str> = decoded.iter().map(String::as_str).collect();
        let mut someone_else = false;
        match p.as_slice() {
            ["me", "chats", _, ..] => p = p.split_off(1),
            ["users", oid, "chats", ..] => {
                someone_else = *oid != t.me.id;
                p = p.split_off(2);
            }
            _ => {}
        }
        let name = route_name(method, &p)?;
        *t.hits.entry(name.to_string()).or_default() += 1;
        t.last_headers.insert(name.to_string(), headers.to_vec());
        let mut reply = match t.refusal(name, authorized, someone_else) {
            Some(r) => r,
            None => {
                let input: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
                let done = t.answer(name, &p, query, raw_path, &input, base);
                match t.answer_lost.get_mut(name).filter(|n| **n > 0) {
                    Some(n) => {
                        *n -= 1;
                        Reply::error(503, "ServiceUnavailable", "Done, but the answer was lost.")
                    }
                    None => done,
                }
            }
        };
        reply.date = Some(t.now().div_euclid(1000));
        Some(reply)
    }
}

impl FakeTeams {
    /// Why a request isn't answered, if it isn't: throttled, no token, someone else's chats, or
    /// a failure asked for in `fail_next`. A download is never refused (it is another server).
    fn refusal(&mut self, name: &str, authorized: bool, someone_else: bool) -> Option<Reply> {
        if name == "download" {
            return None;
        }
        let counted: u32 = self.hits.iter().filter(|(k, _)| *k != "download").map(|(_, n)| *n).sum();
        if self.throttle_every.is_some_and(|n| n > 0 && counted.is_multiple_of(n)) {
            return Some(Reply::error(429, "TooManyRequests", "Too many requests."));
        }
        if !authorized {
            return Some(Reply::error(401, "InvalidAuthenticationToken", "Access token is empty."));
        }
        if someone_else {
            return Some(Reply::error(403, "Forbidden", "Only the signed-in user's chats."));
        }
        if let Some(n) = self.fail_next.get_mut(name).filter(|n| **n > 0) {
            *n -= 1;
            return Some(Reply::error(500, "InternalServerError", "Failed, as the test asked."));
        }
        None
    }
}
