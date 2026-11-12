// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Graph's JSON read into the model: chats, their members, messages and presences.
//!
//! What comes in is a server's and, inside it, the senders' own: an id longer than [`MAX_ID`]
//! characters is refused, every list is capped by the model's caps, names and topics are put on
//! one line, and a body goes through the HTML subset's reader within its caps. Nothing here
//! fetches anything: a shared file's address is kept as data, a thumbnail's is dropped, a card's
//! content isn't kept.
//!
//! The HTML reader calls a third-party parser and leaves its panics to its caller: each message,
//! and each chat's preview, is read here behind a guard of its own, so one that panics is kept as
//! a message that can't be shown (a chat without its preview) and the read goes on.

use std::panic::{AssertUnwindSafe, catch_unwind};

use quatrelle_mime::{teams_html, teams_text};
use quatrelle_model::pim::teams::{
    Attachment, Chat, ChatKind, ChatMessage, ChatText, MAX_ATTACHMENTS, MAX_BODY, MAX_ID, MAX_MEMBERS, MAX_MENTIONS, MAX_REACTION_USERS,
    MAX_REACTIONS, Member, Mention, MessageKind, Presence, Preview, Reaction, Sender, is_bidi, one_line,
};
use serde_json::Value;

/// The most roles kept of a member (a `guest` role is always among them).
pub const MAX_ROLES: usize = 8;
/// The longest word kept: a role, an identity type, a member's kind.
const MAX_WORD: usize = 64;
/// The most characters of a reaction kept (an emoji sequence or a name such as `like`).
const MAX_EMOJI: usize = 32;
/// The longest address kept.
const MAX_ADDRESS: usize = 320;
/// The most characters of a shared file's name kept.
const MAX_FILE_NAME: usize = 255;
/// The longest web address kept (a chat's, a shared file's).
const MAX_URL: usize = 8 << 10;
/// The most bytes of a preview's body read: it is shown on one line of 80 characters, and the
/// list of chats reads hundreds of them.
const PREVIEW_MAX: usize = 16 << 10;
/// How Graph writes "never" (`0001-01-01T00:00:00Z`).
const NEVER: &str = "0001-01-01";

/// A chat from the list of chats (`$expand=lastMessagePreview`, and its members when expanded):
/// `None` without a usable id. Its preview is read behind its own guard: one that panics leaves
/// the chat without a preview.
pub fn chat_of(v: &Value) -> Option<Chat> {
    chat_with(v, |chat, p| catch_unwind(AssertUnwindSafe(|| preview_of(chat, p))).ok().flatten())
}

/// [`chat_of`] without the guard on its preview, for the fuzz target (which must see a panic).
#[doc(hidden)]
pub fn chat_of_unguarded(v: &Value) -> Option<Chat> {
    chat_with(v, preview_of)
}

fn chat_with(v: &Value, preview: impl Fn(&str, &Value) -> Option<Preview>) -> Option<Chat> {
    let id = id_at(v, "id")?;
    let preview = v.get("lastMessagePreview").filter(|p| p.is_object()).and_then(|p| preview(&id, p));
    let read = v.pointer("/viewpoint/lastMessageReadDateTime").and_then(Value::as_str).filter(|t| !t.trim_start().starts_with(NEVER));
    Some(Chat {
        kind: ChatKind::of_graph(str_at(v, "chatType").unwrap_or_default()),
        topic: str_at(v, "topic").map(one_line).filter(|t| !t.is_empty()),
        members: v.get("members").and_then(Value::as_array).into_iter().flatten().filter_map(member_of).take(MAX_MEMBERS).collect(),
        last_read: read.and_then(ms_of),
        hidden: v.pointer("/viewpoint/isHidden").and_then(Value::as_bool).unwrap_or(false),
        left: false,
        unreadable: None,
        last_activity: preview.as_ref().map(|p| p.at).or_else(|| time_at(v, "createdDateTime")),
        updated: time_at(v, "lastUpdatedDateTime"),
        web_url: url_at(v, "webUrl"),
        tenant_id: id_at(v, "tenantId"),
        preview,
        id,
    })
}

/// A chat's `lastMessagePreview`: `None` without its time. A deleted message's text isn't kept.
fn preview_of(chat: &str, p: &Value) -> Option<Preview> {
    let at = time_at(p, "createdDateTime")?;
    let deleted = p.get("isDeleted").and_then(Value::as_bool).unwrap_or(false);
    let message = id_at(p, "id").unwrap_or_default();
    let text = if deleted { String::new() } else { one_line(&body_of(chat, &message, p.get("body"), PREVIEW_MAX).plain()) };
    Some(Preview { sender: sender_of(p.get("from")), text, at })
}

/// A member of a chat (`conversationMember`): `None` without a usable id. Its kind is its
/// `@odata.type` without `#microsoft.graph.`, or empty when it isn't a plain word (so it isn't
/// taken for one of the organisation).
pub fn member_of(v: &Value) -> Option<Member> {
    // Each kind of member names its id its own way.
    let user_id = ["userId", "anonymousGuestId", "skypeId", "azureCommunicationServicesId"].iter().find_map(|k| id_at(v, k))?;
    let kind = str_at(v, "@odata.type").map(|t| t.strip_prefix("#microsoft.graph.").unwrap_or(t)).filter(|t| plain_word(t)).unwrap_or_default();
    Some(Member {
        user_id,
        name: one_line(str_at(v, "displayName").unwrap_or_default()),
        address: clean(str_at(v, "email").unwrap_or_default(), MAX_ADDRESS),
        kind: kind.to_string(),
        tenant_id: id_at(v, "tenantId"),
        roles: roles_of(v.get("roles")),
    })
}

/// A member's roles, at most [`MAX_ROLES`]: a `guest` role is kept even past them, as it is what
/// tells a guest from a member of the organisation.
fn roles_of(v: Option<&Value>) -> Vec<String> {
    let mut roles: Vec<String> = Vec::new();
    let mut guest = false;
    for role in items(v).filter_map(Value::as_str).map(str::trim) {
        guest |= role.eq_ignore_ascii_case("guest");
        let shown = !role.is_empty() && role.chars().nth(MAX_WORD).is_none() && !role.chars().any(|c| c.is_control() || is_bidi(c));
        if shown && roles.len() < MAX_ROLES {
            roles.push(role.to_string());
        }
    }
    if guest && !roles.iter().any(|r| r.eq_ignore_ascii_case("guest")) {
        roles.truncate(MAX_ROLES.saturating_sub(1));
        roles.push("guest".to_string());
    }
    roles
}

/// A message of `chat` (`chatMessage`), read behind its own guard: one whose reading panics comes
/// back as a message that can't be shown, with its id, times and sender when they were read, else
/// its id only. One without a usable id can't be shown either (its id is then empty).
pub fn message_of(chat: &str, v: &Value) -> ChatMessage {
    let head = match catch_unwind(AssertUnwindSafe(|| head_of(chat, v))) {
        Ok(head) => head,
        Err(_) => {
            let id = catch_unwind(AssertUnwindSafe(|| id_at(v, "id"))).ok().flatten().unwrap_or_default();
            return ChatMessage { id, chat: chat.to_string(), unshowable: true, ..ChatMessage::default() };
        }
    };
    if head.unshowable || head.deleted.is_some() {
        return head;
    }
    let kept = head.clone();
    catch_unwind(AssertUnwindSafe(move || fill(head, v))).unwrap_or(ChatMessage { unshowable: true, ..kept })
}

/// [`message_of`] without its guards, for the fuzz target (which must see a panic).
#[doc(hidden)]
pub fn message_of_unguarded(chat: &str, v: &Value) -> ChatMessage {
    let head = head_of(chat, v);
    if head.unshowable || head.deleted.is_some() {
        return head;
    }
    fill(head, v)
}

/// What is known of a message before its content: its id, times, sender and kind. A message with
/// `eventDetail` set is a system message whatever its `messageType` says.
fn head_of(chat: &str, v: &Value) -> ChatMessage {
    let id = id_at(v, "id");
    let created = time_at(v, "createdDateTime");
    let modified = time_at(v, "lastModifiedDateTime");
    let system = v.get("eventDetail").is_some_and(|e| !e.is_null()) || str_at(v, "messageType") == Some("systemEventMessage");
    ChatMessage {
        unshowable: id.is_none(),
        id: id.unwrap_or_default(),
        chat: chat.to_string(),
        sender: sender_of(v.get("from")),
        created: created.or(modified).unwrap_or(0),
        modified: modified.or(created).unwrap_or(0),
        edited: time_at(v, "lastEditedDateTime"),
        deleted: time_at(v, "deletedDateTime"),
        kind: if system { MessageKind::System } else { MessageKind::Message },
        ..ChatMessage::default()
    }
}

/// A message's content: its body, mentions, reactions and attachments, each within its cap.
fn fill(mut m: ChatMessage, v: &Value) -> ChatMessage {
    m.body = body_of(&m.chat, &m.id, v.get("body"), MAX_BODY);
    m.mentions = items(v.get("mentions")).filter_map(mention_of).take(MAX_MENTIONS).collect();
    m.reactions = reactions_of(v.get("reactions"));
    m.attachments = items(v.get("attachments")).filter_map(attachment_of).take(MAX_ATTACHMENTS).collect();
    m
}

/// A body (`itemBody`) as text, at most `max` bytes of it read: `text` as plain text, anything
/// else as Teams' HTML. Not guarded: its callers are.
fn body_of(chat: &str, message: &str, body: Option<&Value>, max: usize) -> ChatText {
    #[cfg(test)]
    forced_panic();
    let content = cut(body.and_then(|b| str_at(b, "content")).unwrap_or_default(), max);
    match body.and_then(|b| str_at(b, "contentType")) {
        Some(t) if t.eq_ignore_ascii_case("text") => teams_text(content),
        _ => teams_html(content, chat, message),
    }
}

/// Who sent it (an `identitySet`): a user, else an application, else no one.
fn sender_of(from: Option<&Value>) -> Sender {
    let Some(from) = from else { return Sender::Nobody };
    if let Some(u) = from.get("user")
        && let Some(id) = id_at(u, "id")
    {
        let identity_type = str_at(u, "userIdentityType").filter(|t| plain_word(t)).map(str::to_string);
        return Sender::User { id, name: one_line(str_at(u, "displayName").unwrap_or_default()), identity_type, tenant_id: id_at(u, "tenantId") };
    }
    if let Some(a) = from.get("application")
        && let Some(id) = id_at(a, "id")
    {
        return Sender::App { id, name: one_line(str_at(a, "displayName").unwrap_or_default()) };
    }
    Sender::Nobody
}

/// A mention (`chatMessageMention`): `None` without its number.
fn mention_of(v: &Value) -> Option<Mention> {
    let id = v.get("id")?.as_u64().and_then(|i| u32::try_from(i).ok())?;
    Some(Mention {
        id,
        text: one_line(str_at(v, "mentionText").unwrap_or_default()),
        user_id: v.pointer("/mentioned/user").and_then(|u| id_at(u, "id")),
    })
}

/// Reactions (one `chatMessageReaction` per person and kind) grouped by kind, as they came: at
/// most [`MAX_REACTIONS`] kinds of [`MAX_REACTION_USERS`] people each. One without a person is
/// left out.
fn reactions_of(v: Option<&Value>) -> Vec<Reaction> {
    let mut out: Vec<Reaction> = Vec::new();
    for r in items(v) {
        let emoji = clean(str_at(r, "reactionType").unwrap_or_default(), MAX_EMOJI);
        let Some(user) = r.pointer("/user/user").and_then(|u| id_at(u, "id")) else { continue };
        if emoji.trim().is_empty() {
            continue;
        }
        let at = match out.iter().position(|x| x.emoji == emoji) {
            Some(i) => i,
            None if out.len() < MAX_REACTIONS => {
                out.push(Reaction { emoji, users: Vec::new() });
                out.len().saturating_sub(1)
            }
            None => continue,
        };
        if let Some(x) = out.get_mut(at)
            && x.users.len() < MAX_REACTION_USERS
            && !x.users.contains(&user)
        {
            x.users.push(user);
        }
    }
    out
}

/// An attachment (`chatMessageAttachment`): a shared file (`reference`) by its id, name and
/// address, never fetched here, its thumbnail dropped; anything else a card, its content not kept.
fn attachment_of(v: &Value) -> Option<Attachment> {
    if str_at(v, "contentType") != Some("reference") {
        return Some(Attachment::Card);
    }
    Some(Attachment::File {
        id: id_at(v, "id")?,
        name: clean(str_at(v, "name").unwrap_or_default(), MAX_FILE_NAME),
        url: url_at(v, "contentUrl").unwrap_or_default(),
        size: None,
    })
}

/// Someone's presence (`presence`): their id and availability.
pub fn presence_of(v: &Value) -> Option<(String, Presence)> {
    Some((id_at(v, "id")?, Presence::of_graph(str_at(v, "availability").unwrap_or_default())))
}

/// An RFC 3339 time in milliseconds since 1970.
pub(crate) fn ms_of(t: &str) -> Option<i64> {
    let secs = crate::jmap::utc_date(t)?;
    let frac =
        t.trim().get(19..).and_then(|r| r.strip_prefix('.')).map_or("", |r| r.get(..r.bytes().take_while(u8::is_ascii_digit).count()).unwrap_or(""));
    let ms = format!("{:0<3}", frac.get(..frac.len().min(3)).unwrap_or("")).parse::<i64>().ok()?;
    secs.checked_mul(1000)?.checked_add(ms)
}

fn items(v: Option<&Value>) -> impl Iterator<Item = &Value> {
    v.and_then(Value::as_array).into_iter().flatten()
}

fn str_at<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn time_at(v: &Value, key: &str) -> Option<i64> {
    str_at(v, key).and_then(ms_of)
}

/// An id: not empty, at most [`MAX_ID`] characters, no control character.
fn id_at(v: &Value, key: &str) -> Option<String> {
    str_at(v, key).filter(|id| !id.is_empty() && id.chars().nth(MAX_ID).is_none() && !id.chars().any(char::is_control)).map(str::to_string)
}

/// A web address, as it came: at most [`MAX_URL`] bytes, no space or control character.
fn url_at(v: &Value, key: &str) -> Option<String> {
    str_at(v, key).filter(|u| !u.is_empty() && u.len() <= MAX_URL && !u.chars().any(|c| c.is_whitespace() || c.is_control())).map(str::to_string)
}

/// A word made of ASCII letters and digits, at most [`MAX_WORD`] of them.
fn plain_word(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_WORD && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// `s` without control or bidi characters, at most `max` characters of it.
fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|&c| !c.is_control() && !is_bidi(c)).take(max).collect()
}

/// `s` cut to at most `max` bytes, where a character starts.
fn cut(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    s.get(..end).unwrap_or_default()
}

#[cfg(test)]
thread_local! {
    /// Set by a test: the next bodies read panic, as a third-party parser's bug would.
    pub(crate) static PANIC_ON_BODY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn forced_panic() {
    if PANIC_ON_BODY.with(std::cell::Cell::get) {
        panic!("a panic the test asked for");
    }
}
