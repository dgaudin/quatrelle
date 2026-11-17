// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The fake Graph server's Teams: the signed-in user, chats, their members and messages (listed
//! in a `lastModifiedDateTime` range, posted with pictures and files, edited, reacted to,
//! soft-deleted), marking a chat read, shared files, messages' hosted images, people's photos
//! and presence, the user's OneDrive (its upload sessions, items and shares: `drive.rs`), the
//! directory and the chats created (`people.rs`), as Graph v1.0 answers them.
//!
//! Each request is counted in [`FakeTeams::hits`] and its headers kept in
//! [`FakeTeams::last_headers`] under its route name: `me`, `chats`, `members`, `messages`,
//! `post`, `edit`, `setReaction`, `unsetReaction`, `softDelete`, `markChatReadForUser`,
//! `shares`, `download`, `hosted`, `photo`, `presence`, `drive`, `uploadSession`, `upload`,
//! `item`, `deleteItem`, `invite`, `users`, `createChat`, and `unknown` for a Teams path no
//! route answers (404).

use std::collections::HashMap;

use super::*;

mod chats;
mod drive;
mod people;

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
    /// The organisation the chat was made in (its `tenantId`), when not the signed-in user's.
    pub(crate) tenant: Option<String>,
    /// When the signed-in user last read it (`viewpoint.lastMessageReadDateTime`).
    pub(crate) read_time: String,
    pub(crate) messages: Vec<FakeMessage>,
    /// When it was last renamed or its members changed (`lastUpdatedDateTime`); `None`: when it
    /// was made. A message doesn't move it.
    pub(crate) updated: Option<String>,
    /// Hidden by the user (`viewpoint.isHidden`).
    pub(crate) hidden: bool,
    /// Its `webUrl` as the list gives it: `None`, Teams' own link to it; `Some(None)`: none.
    pub(crate) link: Option<Option<String>>,
}

impl FakeChat {
    pub(crate) fn new(id: &str, kind: &'static str, topic: Option<&str>, members: Vec<FakeUser>) -> Self {
        Self {
            id: id.into(),
            kind,
            topic: topic.map(str::to_string),
            members,
            tenant: None,
            read_time: "2026-01-01T00:00:00.000Z".into(),
            messages: Vec::new(),
            updated: None,
            hidden: false,
            link: None,
        }
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
    /// Its hosted images: (hosted content id, bytes).
    pub(crate) hosted: Vec<(String, Vec<u8>)>,
    /// Files of the user's OneDrive it carries: (id, name, contentUrl), listed as `reference`
    /// attachments after `attachments`.
    pub(crate) files: Vec<(String, String, String)>,
    /// Graph's `from` as given, in place of the one made from `from`: an application, someone of
    /// another organisation, or a name the members don't give.
    pub(crate) from_json: Option<Value>,
    /// The users it mentions (their ids), in the order of its `mentions`.
    pub(crate) mentions: Vec<String>,
    /// A system message (`systemEventMessage`, with its `eventDetail`).
    pub(crate) system: bool,
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
    /// The Teams requests answered before `throttle_every` starts counting.
    pub(crate) throttle_from: u32,
    /// Chats (their ids) where posting is refused with 403.
    pub(crate) forbid_post: Vec<String>,
    /// Chats (their ids) where marking read is refused with 403.
    pub(crate) forbid_mark: Vec<String>,
    /// Every `POST /chats/{id}/messages` received, whatever its answer: (the chat's id, the JSON
    /// body as it came; `Null` when it isn't JSON).
    pub(crate) posts: Vec<(String, Value)>,
    /// Every `post`, `edit`, `setReaction`, `unsetReaction` and `softDelete` received, whatever its
    /// answer: (route name, the chat's id, the message's id or empty, the JSON body as it came;
    /// `Null` when it isn't JSON).
    pub(crate) writes: Vec<(String, String, String, Value)>,
    /// Chats (their ids) where editing is refused with 403.
    pub(crate) forbid_edit: Vec<String>,
    /// Chats (their ids) where reacting and taking a reaction back are refused with 403.
    pub(crate) forbid_react: Vec<String>,
    /// A quick reaction kept as its emoji is listed by its name (`like`, `heart`, `laugh`,
    /// `surprised`, `sad`, `angry`), as Graph may.
    pub(crate) reactions_by_name: bool,
    /// The most bytes a post's `body.content` may have (`None`: 100 KiB); a longer one is
    /// answered 413 `RequestEntityTooLarge`.
    pub(crate) post_limit: Option<usize>,
    /// How the fake keeps a posted message's HTML, as Graph does its own way (wrapped, entities
    /// and spaces rewritten): what it answers and lists from then on. `None`: as it came.
    pub(crate) rewrite_posts: Option<fn(&str) -> String>,
    /// The next n requests of a route (by route name) answered 500, then the route works again.
    pub(crate) fail_next: HashMap<String, u32>,
    /// The next n requests of a route (by route name) answered 404 with nothing done, as Graph
    /// answers for a chat it no longer serves the user, then the route works again.
    pub(crate) not_found_next: HashMap<String, u32>,
    /// The next n requests of a route carried out, but answered 503 (the answer lost on the way).
    pub(crate) answer_lost: HashMap<String, u32>,
    /// The `Retry-After` (seconds) those 503s say; `None`: they say none.
    pub(crate) lost_retry_after: Option<u64>,
    /// The next n requests of a route (by route name) carried out, then the connection closed
    /// with no answer at all.
    pub(crate) cut_after: HashMap<String, u32>,
    /// Every Teams request closed with no answer at all, nothing done (the network away):
    /// counted in `hits`, never in `posts` or `writes`.
    pub(crate) cut_all: bool,
    /// Chats (their ids) whose message reads are answered 500.
    pub(crate) fail_reads: Vec<String>,
    /// Chats (their ids) whose message reads are refused with 403.
    pub(crate) forbid_reads: Vec<String>,
    /// Chats (their ids) whose members' read is refused with 403.
    pub(crate) forbid_members: Vec<String>,
    /// The chat list itself is refused with 403 (an account without Teams).
    pub(crate) forbid_list: bool,
    /// People's photos by user id; anyone else has none (404).
    pub(crate) photos: HashMap<String, Vec<u8>>,
    /// People's availability (`Available`, `Busy`, …) by user id; anyone else's is
    /// `PresenceUnknown`.
    pub(crate) presences: HashMap<String, String>,
    /// The user's OneDrive address (`webUrl`); `None`: they have none (404).
    pub(crate) drive: Option<String>,
    /// Every read of a chat's messages: (chat id, its `$filter` as sent, empty when none).
    pub(crate) message_reads: Vec<(String, String)>,
    /// Shared files' bytes, by the `contentUrl` an attachment carries.
    pub(crate) shares: HashMap<String, Vec<u8>>,
    /// Requests received, per route name.
    pub(crate) hits: HashMap<String, u32>,
    /// The headers of the last request per route name, their names lowercased.
    pub(crate) last_headers: HashMap<String, Vec<(String, String)>>,
    /// The token runs out: (route name, its n-th request, the one token taken from then on).
    pub(crate) token_expires: Option<(&'static str, u32, String)>,
    /// Whether it has run out.
    pub(crate) expired: bool,
    /// The next request of this route (by route name) is refused with 401, whatever its token:
    /// once, then the route works again.
    pub(crate) refuse_once: Option<&'static str>,
    /// The next request of this route (by route name) is answered only after this long, once: a
    /// request stalled on its way. The server goes on answering the others meanwhile.
    pub(crate) hold: Option<(&'static str, std::time::Duration)>,
    /// Set for the server by the request being answered: how long its answer is held back.
    pub(crate) holding: Option<std::time::Duration>,
    /// The fake's clock (milliseconds since 1970), when a test sets it: what its `Date` says,
    /// and what a change made now is dated from. `None`: this computer's clock.
    pub(crate) clock: Option<i64>,
    /// The next request of this route (by route name) is answered 429, once.
    pub(crate) throttle_once: Option<&'static str>,
    /// Users who are guests (by user id): as members, their roles say `guest`.
    pub(crate) guests: Vec<String>,
    /// The listing of this route (`chats` or `messages`) points every page's `@odata.nextLink`
    /// at its second page: a server going round in circles.
    pub(crate) loop_next: Option<&'static str>,
    /// The listing of this route answers its first page whatever its link asks, each time with
    /// a link it never gave before: a server that brings nothing new.
    pub(crate) repeat_page: Option<&'static str>,
    /// The next n reads of messages that follow a link (`$skiptoken`) are answered 400: Graph no
    /// longer takes the link (it was kept too long).
    pub(crate) refuse_links: u32,
    /// The links `refuse_links` refuses are answered 404 rather than 400.
    pub(crate) links_not_found: bool,
    /// The next n pages of the chat list that follow a link (`$skiptoken`) are answered 400.
    pub(crate) refuse_list_links: u32,
    /// The next n pages of the chat list that follow a link (`$skiptoken`) are answered 500.
    pub(crate) fail_list_links: u32,
    /// The chat list's `@odata.nextLink` points at another site than Graph's.
    pub(crate) list_next_elsewhere: bool,
    /// The chat list's pages are this long, whatever `$top` asks (and the last says no more).
    pub(crate) list_page: Option<usize>,
    /// The user's OneDrive: its files, upload sessions, ranges and shares.
    pub(crate) drive_files: drive::FakeDrive,
    /// The organisation's directory, as `GET /users` searches it (its guests: `guests`).
    pub(crate) directory: Vec<FakeUser>,
    /// The `$search` of the last `GET /users`, decoded, as it came.
    pub(crate) last_search: String,
    /// `POST /chats` is refused with 403.
    pub(crate) forbid_create: bool,
    /// Every `POST /chats` body received, whatever its answer.
    pub(crate) creations: Vec<Value>,
}

impl FakeTeams {
    pub(crate) fn new(me: FakeUser, tenant: &str) -> Self {
        Self { me, tenant: tenant.into(), ..Self::default() }
    }

    pub(crate) fn chat(&self, id: &str) -> Option<&FakeChat> {
        self.chats.iter().find(|c| c.id == id)
    }

    pub(crate) fn chat_mut(&mut self, id: &str) -> Option<&mut FakeChat> {
        self.chats.iter_mut().find(|c| c.id == id)
    }

    /// A chat renamed now: its topic, and its `lastUpdatedDateTime` moved.
    pub(crate) fn rename(&mut self, id: &str, topic: &str) {
        let now = rfc3339(self.now());
        if let Some(c) = self.chat_mut(id) {
            c.topic = Some(topic.to_string());
            c.updated = Some(now);
        }
    }
}

/// A time in milliseconds since 1970 as Graph writes it: `2026-10-30T09:12:34.567Z`.
pub(crate) fn rfc3339(ms: i64) -> String {
    let (secs, milli) = (ms.div_euclid(1000), ms.rem_euclid(1000));
    let (y, m, d) = quatrelle_model::dates::civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{milli:03}Z", s / 3600, s / 60 % 60, s % 60)
}

/// One answer: status, body, content type, where a redirect points, the fake's clock when it
/// answered (Unix seconds, sent as `Date`) and its `Retry-After` (a 429 always says 1); or, when
/// `cut`, none at all (the connection closed once the request is read); `stall`: its body stops
/// after so many bytes, the connection left open.
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) body: Vec<u8>,
    pub(super) kind: &'static str,
    pub(super) location: Option<String>,
    pub(super) date: Option<i64>,
    pub(super) retry_after: Option<u64>,
    pub(super) cut: bool,
    pub(super) stall: Option<usize>,
}

impl Reply {
    fn bytes(status: u16, body: Vec<u8>, kind: &'static str) -> Self {
        Self { status, body, kind, location: None, date: None, retry_after: None, cut: false, stall: None }
    }

    fn json(status: u16, v: &Value) -> Self {
        Self::bytes(status, v.to_string().into_bytes(), "application/json")
    }

    fn empty(status: u16) -> Self {
        Self::bytes(status, Vec::new(), "application/json")
    }

    /// No answer: the connection is closed.
    fn cut() -> Self {
        Self { cut: true, ..Self::empty(0) }
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
        ("GET", ["chats", _, "messages", _, "hostedContents", _, "$value"]) => "hosted",
        ("GET", ["users", _, "photos", "48x48", "$value"]) => "photo",
        ("POST", ["communications", "getPresencesByUserId"]) => "presence",
        ("GET", ["me", "drive"]) => "drive",
        ("POST", ["me", "drive", "root:", .., "createUploadSession"]) => "uploadSession",
        ("PUT" | "GET" | "DELETE", ["upload", _]) => "upload",
        ("GET", ["me", "drive", "items", _]) => "item",
        ("DELETE", ["me", "drive", "items", _]) => "deleteItem",
        ("POST", ["me", "drive", "items", _, "invite"]) => "invite",
        ("GET", ["users"]) => "users",
        ("POST", ["chats"]) => "createChat",
        (
            _,
            ["chats", ..]
            | ["shares", ..]
            | ["download", ..]
            | ["upload", ..]
            | ["me", "drive", ..]
            | ["communications", ..]
            | ["users", _, "photos", ..],
        ) => "unknown",
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

/// The URL a `u!` share id encodes (base64url, unpadded).
fn share_url(id: &str) -> Option<String> {
    id.strip_prefix("u!").map(|b| String::from_utf8_lossy(&unbase64(b.replace('-', "+").replace('_', "/").as_bytes())).to_string())
}

/// A shared file's key in `shares`: the URL a `u!` share id encodes, or the id itself.
fn share_key(teams: &FakeTeams, id: &str) -> Option<String> {
    share_url(id).into_iter().chain(std::iter::once(id.to_string())).find(|k| teams.shares.contains_key(k))
}

impl FakeTeams {
    /// A shared file's bytes by its share id: a file of `shares`, or one of the user's OneDrive
    /// by its `webDavUrl` (a file sent in a chat).
    fn shared_bytes(&self, id: &str) -> Option<Vec<u8>> {
        if let Some(key) = share_key(self, id) {
            return self.shares.get(&key).cloned();
        }
        let url = share_url(id)?;
        self.drive_files.files.iter().find(|f| self.web_dav_url(f) == url).map(|f| f.bytes.clone())
    }

    /// A user by id: a chat member or the signed-in user.
    fn user(&self, id: &str) -> Option<&FakeUser> {
        std::iter::once(&self.me).chain(self.chats.iter().flat_map(|c| &c.members)).find(|u| u.id == id)
    }

    /// A message's `from`: the one it was given, else its sender's identity (none: a system
    /// message's).
    fn sender_json(&self, m: &FakeMessage) -> Value {
        m.from_json.clone().unwrap_or_else(|| m.from.as_deref().map_or(Value::Null, |f| self.identity(f)))
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
        let clock = self.clock.unwrap_or_else(|| {
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0))
        });
        let held = self.chats.iter().flat_map(|c| {
            c.messages
                .iter()
                .flat_map(|m| [Some(&m.created), Some(&m.modified), m.edited.as_ref(), m.deleted.as_ref()].into_iter().flatten())
                .chain(std::iter::once(&c.read_time))
        });
        held.filter_map(|t| ms_of(t)).map(|t| t + 1).fold(clock, i64::max)
    }

    /// What its `Date` says: the clock a test set, else [`Self::now`].
    fn date(&self) -> i64 {
        self.clock.unwrap_or_else(|| self.now())
    }

    /// One Teams request, by route name, on paths already decoded and normalized.
    fn answer(&mut self, name: &str, p: &[&str], query: &str, raw_path: &str, input: &Value, base: &str) -> Reply {
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
            if self.refuse_list_links > 0 && q.contains_key("$skiptoken") {
                self.refuse_list_links -= 1;
                return Reply::error(400, "BadRequest", "The skip token is no longer valid.");
            }
            if self.fail_list_links > 0 && q.contains_key("$skiptoken") {
                self.fail_list_links -= 1;
                return Reply::error(500, "InternalServerError", "Failed, as the test asked.");
            }
            return self.chats(&q, raw_path, query, base);
        }
        if let Some(r) = self.people(name, p, &q, input) {
            return r;
        }
        if let Some(r) = self.drive_route(name, p, &q, raw_path, input, base) {
            return r;
        }
        if name == "createChat" {
            return self.create_chat(input, base);
        }
        if let Some(r) = self.share_route(name, p, base) {
            return r;
        }
        self.chat_route(name, p, query, raw_path, input, base)
    }
}

impl FakeTeams {
    /// The routes about people and the user's drive rather than a chat: photos, presence,
    /// OneDrive; `None` for the others.
    fn people(&self, name: &str, p: &[&str], q: &HashMap<String, String>, input: &Value) -> Option<Reply> {
        Some(match name {
            "photo" => match p.get(1).and_then(|u| self.photos.get(*u)) {
                Some(bytes) => Reply::bytes(200, bytes.clone(), "image/jpeg"),
                None => Reply::error(404, "ImageNotFound", "The photo wasn't found."),
            },
            "presence" => {
                let Some(ids) = input.get("ids").and_then(Value::as_array) else { return Some(Reply::error(400, "BadRequest", "ids are wanted.")) };
                if ids.len() > 650 {
                    return Some(Reply::error(400, "BadRequest", "At most 650 ids."));
                }
                let value: Vec<Value> = ids
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|id| {
                        let availability = self.presences.get(id).map_or("PresenceUnknown", String::as_str);
                        json!({"id": id, "availability": availability, "activity": availability})
                    })
                    .collect();
                Reply::json(200, &json!({ "value": value }))
            }
            "drive" => match &self.drive {
                Some(url) if q.get("$select").is_some_and(|s| s == "webUrl") => Reply::json(200, &json!({ "webUrl": url })),
                Some(url) => Reply::json(200, &json!({ "id": "drive-1", "driveType": "business", "webUrl": url })),
                None => Reply::error(404, "itemNotFound", "The user has no drive."),
            },
            _ => return None,
        })
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
        let hit = t.hits.entry(name.to_string()).or_default();
        *hit += 1;
        let hit = *hit;
        t.last_headers.insert(name.to_string(), headers.to_vec());
        if t.cut_all {
            return Some(Reply::cut());
        }
        let mut authorized = authorized;
        if let Some((route, nth, renewed)) = t.token_expires.clone() {
            t.expired |= route == name && hit >= nth;
            if t.expired {
                let bearer = format!("Bearer {renewed}");
                authorized = headers.iter().any(|(k, v)| k == "authorization" && *v == bearer);
            }
        }
        if t.refuse_once == Some(name) {
            t.refuse_once = None;
            authorized = false;
        }
        if name == "upload" {
            // An upload session's address carries its own authorisation: a token is refused there.
            authorized = !headers.iter().any(|(k, _)| k == "authorization");
        }
        t.holding = t.hold.take_if(|(route, _)| *route == name).map(|(_, wait)| wait);
        let input: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        if name == "post" {
            t.posts.push((p.get(1).copied().unwrap_or_default().to_string(), input.clone()));
        }
        if name == "invite" {
            t.drive_files.invites.push((p.get(3).copied().unwrap_or_default().to_string(), input.clone()));
        }
        if name == "createChat" {
            t.creations.push(input.clone());
        }
        if matches!(name, "post" | "edit" | "setReaction" | "unsetReaction" | "softDelete") {
            let (chat, msg) = (p.get(1).copied().unwrap_or_default(), p.get(3).copied().unwrap_or_default());
            t.writes.push((name.to_string(), chat.to_string(), msg.to_string(), input.clone()));
        }
        let mut reply = match t.refusal(name, authorized, someone_else) {
            Some(r) => r,
            None => {
                let done = match name {
                    "upload" => t.upload(method, &p, headers, body),
                    "users" => t.users(query, headers),
                    _ => t.answer(name, &p, query, raw_path, &input, base),
                };
                if let Some(n) = t.cut_after.get_mut(name).filter(|n| **n > 0) {
                    *n -= 1;
                    Reply::cut()
                } else if let Some(n) = t.answer_lost.get_mut(name).filter(|n| **n > 0) {
                    *n -= 1;
                    Reply { retry_after: t.lost_retry_after, ..Reply::error(503, "ServiceUnavailable", "Done, but the answer was lost.") }
                } else {
                    done
                }
            }
        };
        reply.date = Some(t.date().div_euclid(1000));
        Some(reply)
    }
}

impl FakeTeams {
    /// Why a request isn't answered, if it isn't: throttled, no token, someone else's chats, or
    /// a failure asked for in `fail_next` or `not_found_next`. A download is never refused, an
    /// upload never throttled, and refused only a token (they are other servers than Graph).
    fn refusal(&mut self, name: &str, authorized: bool, someone_else: bool) -> Option<Reply> {
        if name == "download" {
            return None;
        }
        let graph = name != "upload";
        if graph && self.throttle_once == Some(name) {
            self.throttle_once = None;
            return Some(Reply::error(429, "TooManyRequests", "Too many requests."));
        }
        if self.forbid_list && name == "chats" && authorized {
            return Some(Reply::error(403, "Forbidden", "This account has no Teams chats."));
        }
        let counted: u32 = self.hits.iter().filter(|(k, _)| !matches!(k.as_str(), "download" | "upload")).map(|(_, n)| *n).sum();
        if graph && counted > self.throttle_from && self.throttle_every.is_some_and(|n| n > 0 && counted.is_multiple_of(n)) {
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
        if let Some(n) = self.not_found_next.get_mut(name).filter(|n| **n > 0) {
            *n -= 1;
            return Some(Reply::error(404, "NotFound", "Not found, as the test asked."));
        }
        None
    }
}
