// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Microsoft Teams chats as Quatrelle keeps them (read through Microsoft Graph by the network
//! layer): the chats and their members, the messages, the rows of the list, presence, pictures,
//! where the connection stands, and the caps on what a server can make Quatrelle hold. Pure: no
//! clock (times come in, in milliseconds since 1970) and no I/O, so it builds for wasm.

use serde::{Deserialize, Serialize};

mod job;
mod text;

pub use job::{Event, Job};
pub use text::{Block, BlockKind, ChatText, Run};

/// The store's pseudo-account the Teams copy is kept under: its file is `Teams.quatrelle`.
pub const TEAMS_ACCOUNT: &str = "Teams";
/// The most characters of a name or topic shown ([`one_line`]).
pub const NAME_CHARS: usize = 80;
/// The longest id (chat, message, user, hosted content) kept; a longer one is refused.
pub const MAX_ID: usize = 512;
/// The most bytes of a message's body read and kept.
pub const MAX_BODY: usize = 256 << 10;
/// The most attachments kept of a message.
pub const MAX_ATTACHMENTS: usize = 20;
/// The most mentions kept of a message.
pub const MAX_MENTIONS: usize = 50;
/// The most reactions kept of a message, and the most people kept for each.
pub const MAX_REACTIONS: usize = 100;
pub const MAX_REACTION_USERS: usize = 200;
/// The most members kept of a chat.
pub const MAX_MEMBERS: usize = 5_000;
/// The most chats kept.
pub const MAX_CHATS: usize = 20_000;
/// The most pages of one chat's messages read in a pass, and of the list of chats.
pub const MESSAGE_PAGES: usize = 200;
pub const CHAT_PAGES: usize = 400;
/// The most items (rows, messages, presences) one event carries.
pub const EVENT_ITEMS: usize = 50;
/// The messages an opened chat shows first, and the most the engine holds of one chat.
pub const OPEN_WINDOW: usize = 200;
pub const WINDOW_MAX: usize = 500;
/// The most hosted images read of a message, and the most bytes of one picture.
pub const HOSTED_PER_MESSAGE: usize = 20;
pub const MAX_PICTURE: usize = 10 << 20;
/// The most people whose presence is asked at once.
pub const PRESENCE_IDS: usize = 650;
/// The most words indexed of a message, the longest word indexed, and the most bytes of the index.
pub const INDEX_WORDS: usize = 2_000;
pub const INDEX_WORD_CHARS: usize = 64;
pub const INDEX_MAX: u64 = 512 << 20;
/// How far past the server's own time (its `Date`) a time it gives may be (one hour): a later one
/// would hold the next reads back.
pub const CLOCK_TOLERANCE_MS: i64 = 3_600_000;

/// The name of a chat that has none to show, in English (the interface translates it).
pub const NO_NAME: &str = "Teams conversation";

/// File › Options › Teams: whether it's on, the application and organisation signed in with, and
/// who signed in (no secret: the refresh token is in the system's keychain).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TeamsSettings {
    pub on: bool,
    /// The application (client) id registered in the organisation.
    pub client_id: String,
    /// The organisation signed in to: its id or domain, or `organizations`.
    pub tenant: String,
    pub signed_in: bool,
    /// Who signed in: display name, address, Graph object id and organisation id.
    pub name: String,
    pub address: String,
    pub object_id: String,
    pub tenant_id: String,
    /// The host of the user's OneDrive, which shared files are sent from.
    pub drive_host: Option<String>,
    /// Announce new messages, and only those sent to the user alone or naming them.
    pub notify: bool,
    pub notify_only_direct: bool,
}

impl Default for TeamsSettings {
    fn default() -> TeamsSettings {
        TeamsSettings {
            on: false,
            client_id: String::new(),
            tenant: String::new(),
            signed_in: false,
            name: String::new(),
            address: String::new(),
            object_id: String::new(),
            tenant_id: String::new(),
            drive_host: None,
            notify: true,
            notify_only_direct: false,
        }
    }
}

impl TeamsSettings {
    pub fn is_default(&self) -> bool {
        *self == TeamsSettings::default()
    }
}

/// What kind of chat it is, as Graph's `chatType` says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChatKind {
    OneOnOne,
    Group,
    Meeting,
    /// A kind Graph doesn't name (`unknownFutureValue` today), by the name it gives when that's
    /// a plain word, else `unreadable`.
    Other {
        name: String,
    },
}

impl ChatKind {
    pub fn of_graph(chat_type: &str) -> ChatKind {
        match chat_type {
            "oneOnOne" => ChatKind::OneOnOne,
            "group" => ChatKind::Group,
            "meeting" => ChatKind::Meeting,
            other => {
                let plain = !other.is_empty() && other.len() <= 32 && other.bytes().all(|b| b.is_ascii_alphanumeric());
                ChatKind::Other { name: if plain { other.to_string() } else { "unreadable".to_string() } }
            }
        }
    }
}

/// A member of a chat.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub user_id: String,
    pub name: String,
    pub address: String,
    /// Its `@odata.type` without `#microsoft.graph.` (`aadUserConversationMember`,
    /// `anonymousGuestConversationMember`, …).
    pub kind: String,
    pub tenant_id: Option<String>,
    /// `owner`, `guest`, …
    pub roles: Vec<String>,
}

/// Who sent a message: a user (with what Graph says of them when it does), an application such as
/// a bot, or no one (a system message).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "from", rename_all = "camelCase")]
pub enum Sender {
    User {
        id: String,
        name: String,
        /// Graph's `userIdentityType` (`aadUser`, `anonymousGuest`, …).
        identity_type: Option<String>,
        tenant_id: Option<String>,
    },
    App {
        id: String,
        name: String,
    },
    #[default]
    Nobody,
}

/// The last message of a chat as the list of chats gives it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preview {
    pub sender: Sender,
    /// On one line.
    pub text: String,
    /// Milliseconds since 1970.
    pub at: i64,
}

/// A chat. Times are milliseconds since 1970.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chat {
    pub id: String,
    pub kind: ChatKind,
    pub topic: Option<String>,
    pub members: Vec<Member>,
    /// When the user last read it; none: never.
    pub last_read: Option<i64>,
    pub hidden: bool,
    /// No longer in the user's list (left, or deleted): kept to read.
    pub left: bool,
    /// Its messages are refused: when that was last tried.
    pub unreadable: Option<i64>,
    pub preview: Option<Preview>,
    pub last_activity: Option<i64>,
    /// Graph's `lastUpdatedDateTime`: moves when it's renamed or its members change.
    pub updated: Option<i64>,
    pub web_url: Option<String>,
    pub tenant_id: Option<String>,
}

/// A message, or an event of the chat (someone added, the topic changed).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MessageKind {
    #[default]
    Message,
    System,
}

/// A person named in a message: `id` is the one the body's `<at id>` gives.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mention {
    pub id: u32,
    pub text: String,
    pub user_id: Option<String>,
}

/// A reaction and who reacted so (user ids).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    pub emoji: String,
    pub users: Vec<String>,
}

/// What a message carries besides its text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Attachment {
    /// A shared file (OneDrive, SharePoint): its address, never fetched without the user's action.
    File { id: String, name: String, url: String, size: Option<u64> },
    /// An image kept with the message (its hosted content).
    Hosted { id: String },
    /// A card, not shown.
    Card,
}

/// A message. Times are milliseconds since 1970.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub chat: String,
    pub sender: Sender,
    pub created: i64,
    /// Graph's `lastModifiedDateTime` (a reaction moves it too).
    pub modified: i64,
    pub edited: Option<i64>,
    pub deleted: Option<i64>,
    pub kind: MessageKind,
    pub body: ChatText,
    pub mentions: Vec<Mention>,
    pub reactions: Vec<Reaction>,
    pub attachments: Vec<Attachment>,
    /// It couldn't be read: shown as a message that can't be.
    pub unshowable: bool,
}

/// What the list shows of a chat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatRow {
    pub id: String,
    pub name: String,
    pub kind: ChatKind,
    pub last_line: String,
    /// The last message is from someone outside the organisation.
    pub last_external: bool,
    pub at: Option<i64>,
    pub unread: u32,
    pub hidden: bool,
    pub left: bool,
    pub unreadable: bool,
    /// A member is from outside the organisation.
    pub external: bool,
    pub members: u32,
    /// The other person's user id, in a one-to-one chat.
    pub person: Option<String>,
}

/// Whether someone is there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "presence", rename_all = "camelCase")]
pub enum Presence {
    Available,
    Busy,
    Away,
    DoNotDisturb,
    Offline,
    #[default]
    Unknown,
}

impl Presence {
    /// From Graph's `availability`.
    pub fn of_graph(availability: &str) -> Presence {
        match availability {
            "Available" | "AvailableIdle" => Presence::Available,
            "Busy" | "BusyIdle" => Presence::Busy,
            "Away" | "BeRightBack" => Presence::Away,
            "DoNotDisturb" => Presence::DoNotDisturb,
            "Offline" => Presence::Offline,
            _ => Presence::Unknown,
        }
    }
}

/// A picture to show: an image kept with a message, a user's photo, or an image from elsewhere
/// (fetched only when the user asks).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "camelCase")]
pub enum Picture {
    Hosted { chat: String, message: String, hosted: String },
    Photo { user: String },
    Outside { url: String },
}

/// Where the connection to Teams stands.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Status {
    #[default]
    SignedOut,
    SigningIn,
    /// The first copy: `done` of `of` chats.
    Copying {
        done: u32,
        of: u32,
    },
    UpToDate,
    Offline,
    SignInAgain,
    /// The organisation didn't grant this permission.
    PermissionMissing {
        scope: String,
    },
    /// Signed in, but the system has no keychain to remember it in.
    NoKeychain,
    Failed {
        why: String,
    },
}

/// Whether a bidi control drops out of a name: embeddings, overrides and isolates, and the marks.
fn is_bidi(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}')
}

/// A name or topic as it's shown: on one line (each run of whitespace one space), without control
/// or bidi characters, cut to [`NAME_CHARS`] characters ending in `…`.
pub fn one_line(text: &str) -> String {
    let mut out = String::new();
    let mut chars = 0usize;
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            space = true;
            continue;
        }
        if c.is_control() || is_bidi(c) {
            continue;
        }
        if space && chars > 0 {
            out.push(' ');
            chars += 1;
        }
        space = false;
        out.push(c);
        chars += 1;
        if chars > NAME_CHARS {
            break;
        }
    }
    if chars > NAME_CHARS {
        let cut = out.char_indices().nth(NAME_CHARS - 1).map_or(out.len(), |(i, _)| i);
        out.truncate(cut);
        out.truncate(out.trim_end().len());
        out.push('…');
    }
    out
}

/// Whether a member is of the organisation `tenant`, as far as Graph proves it: an organisation
/// account (`aadUserConversationMember`) of that tenant (ignoring case), not a guest. Anything
/// unproven is outside.
pub fn is_internal(m: &Member, tenant: &str) -> bool {
    m.kind == "aadUserConversationMember"
        && !tenant.is_empty()
        && m.tenant_id.as_deref().is_some_and(|t| t.eq_ignore_ascii_case(tenant))
        && !m.roles.iter().any(|r| r.eq_ignore_ascii_case("guest"))
}

/// Whether a message's sender is of the organisation `tenant`: by their membership of the chat
/// when they're a member, else by what the message says (an `aadUser` of that tenant). An
/// application or no one isn't.
pub fn sender_internal(s: &Sender, members: &[Member], tenant: &str) -> bool {
    let Sender::User { id, identity_type, tenant_id, .. } = s else { return false };
    if let Some(m) = members.iter().find(|m| m.user_id == *id) {
        return is_internal(m, tenant);
    }
    identity_type.as_deref() == Some("aadUser") && !tenant.is_empty() && tenant_id.as_deref().is_some_and(|t| t.eq_ignore_ascii_case(tenant))
}

/// The name a chat is listed by, `me` being the user's own id: a one-to-one chat's other person; a
/// group's topic, else its other members' first names; any other's topic; else [`NO_NAME`].
pub fn row_name(chat: &Chat, me: &str) -> String {
    let topic = chat.topic.as_deref().map(one_line).filter(|t| !t.is_empty());
    let others = || chat.members.iter().filter(|m| !m.user_id.eq_ignore_ascii_case(me));
    let name = match chat.kind {
        ChatKind::OneOnOne => others().map(|m| one_line(&m.name)).find(|n| !n.is_empty()).or(topic),
        ChatKind::Group => topic.or_else(|| {
            let mut names = String::new();
            for first in others().filter_map(|m| one_line(&m.name).split(' ').next().map(str::to_string)).filter(|n| !n.is_empty()) {
                if !names.is_empty() {
                    names.push_str(", ");
                }
                names.push_str(&first);
                // Enough to fill the line: the rest would be cut.
                if names.chars().count() > NAME_CHARS {
                    break;
                }
            }
            Some(one_line(&names)).filter(|n| !n.is_empty())
        }),
        ChatKind::Meeting | ChatKind::Other { .. } => topic,
    };
    name.unwrap_or_else(|| NO_NAME.to_string())
}

#[cfg(test)]
mod tests;
