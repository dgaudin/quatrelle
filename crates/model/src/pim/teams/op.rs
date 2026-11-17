// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What Quatrelle does in Teams on the user's behalf (a message sent, edited or deleted, a reaction
//! set or taken back, a chat marked read), what is going out as the window shows it, and the new
//! messages the worker reports for the engine to announce, with the caps on each.

use serde::{Deserialize, Serialize};

use super::{ChatText, Mention, OutPart, Part};

/// The most messages the outbox holds, and the most of one chat's: past them a message isn't taken
/// ([`Unsent::TooMany`]).
pub const OUTBOX_MAX: usize = 500;
pub const OUTBOX_CHAT_MAX: usize = 50;
/// The most bytes of HTML one message sent is written in (80 KiB; its pictures apart).
pub const SEND_MAX: usize = 80 << 10;
/// The most characters of a reaction's emoji.
pub const REACTION_CHARS: usize = 32;
/// The reader's six quick reactions: thumbs up, heart, laughing, surprised, sad, angry.
pub const QUICK_REACTIONS: [&str; 6] = ["\u{1F44D}", "\u{2764}\u{FE0F}", "\u{1F606}", "\u{1F62E}", "\u{1F622}", "\u{1F621}"];

/// Whether `emoji` is something a reaction can be sent as: 1 to [`REACTION_CHARS`] characters, none
/// an ASCII letter (a name such as `like`), a control, a bidi character or white space.
pub fn reactable(emoji: &str) -> bool {
    let n = emoji.chars().count();
    (1..=REACTION_CHARS).contains(&n) && !emoji.chars().any(|c| c.is_ascii_alphabetic() || c.is_control() || super::is_bidi(c) || c.is_whitespace())
}

/// Whether two reactions are the same emoji, the variation selector U+FE0F apart.
pub fn same_reaction(a: &str, b: &str) -> bool {
    let bare = |s: &str| s.chars().filter(|c| *c != '\u{FE0F}').collect::<String>();
    bare(a) == bare(b)
}

/// The longest local id of a message going out ([`Op::Send`]).
pub const LOCAL_MAX: usize = 64;
/// The most characters of a new message's text an announcement shows ([`super::first_line`]).
pub const NEWS_CHARS: usize = 200;

/// What the user did that Teams is to be told: a message sent, edited or deleted, or a reaction set
/// or taken back (each kept in the outbox until Teams has it, the changes merged with what waits
/// of their message), or a chat read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Op {
    /// Send `body` to `chat`; `local` is the engine's id for it (at most [`LOCAL_MAX`]
    /// characters), which the worker's answers name until Teams gives the message its own.
    Send {
        chat: String,
        local: String,
        body: ChatText,
        #[serde(default)]
        mentions: Vec<Mention>,
        /// The files and pictures that go with it.
        #[serde(default)]
        parts: Vec<Part>,
    },
    /// Replace the words of the message `message` (created at `created`, in milliseconds, for the
    /// worker to find its page) of `chat` by `body`.
    Edit {
        chat: String,
        local: String,
        message: String,
        created: i64,
        body: ChatText,
        #[serde(default)]
        mentions: Vec<Mention>,
    },
    /// Delete the message `message` of `chat`.
    Delete { chat: String, local: String, message: String, created: i64 },
    /// Set (`on`) or take back the user's reaction `emoji` on the message `message` of `chat`.
    React { chat: String, local: String, message: String, created: i64, emoji: String, on: bool },
    /// Mark `chat` read up to now.
    MarkRead { chat: String },
}

/// A message going out, as the conversation shows it until Teams has it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outgoing {
    /// Its place in the outbox; 0 for one the worker doesn't keep (not taken yet, or refused).
    pub seq: u64,
    /// The engine's id for it ([`Op::Send`]).
    pub local: String,
    pub chat: String,
    pub body: ChatText,
    /// The mentions of `body` (a send or an edit).
    #[serde(default)]
    pub mentions: Vec<Mention>,
    /// What it is: a message to send, or a change to one already there.
    #[serde(default)]
    pub what: OutKind,
    /// The files and pictures going with it, as the conversation shows them.
    #[serde(default)]
    pub parts: Vec<OutPart>,
    pub state: OutState,
}

/// What an item of the outbox does.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "what", rename_all = "camelCase")]
pub enum OutKind {
    /// A new message.
    #[default]
    Send,
    /// A new text for a message; `created` is its creation time (ms), for the worker to find its page.
    Edit { message: String, created: i64 },
    /// A message deleted.
    Delete { message: String, created: i64 },
    /// A reaction set (`on`) or taken back.
    React { message: String, created: i64, emoji: String, on: bool },
}

impl OutKind {
    /// The message it changes; none for a send.
    pub fn message(&self) -> Option<&str> {
        match self {
            OutKind::Send => None,
            OutKind::Edit { message, .. } | OutKind::Delete { message, .. } | OutKind::React { message, .. } => Some(message),
        }
    }
}

/// Where a message going out stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum OutState {
    /// In the outbox, its turn to come.
    Waiting,
    /// Being sent, or sent without an answer and being checked.
    Sending,
    /// Its files going up.
    Uploading,
    /// Not sent, and never sent again unless the user asks; `code` is Teams' answer when it gave
    /// one (`HTTP 403 Forbidden`, at most 64 characters).
    NotSent { reason: Unsent, code: Option<String> },
}

/// Why a message wasn't sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "unsent", rename_all = "camelCase")]
pub enum Unsent {
    /// Teams refused it.
    Refused,
    /// It's longer than Teams takes ([`SEND_MAX`]).
    TooLong,
    /// Too many messages are waiting ([`OUTBOX_MAX`], [`OUTBOX_CHAT_MAX`]).
    TooMany,
    /// The chat was left, or deleted, before it went.
    Left,
    /// Teams didn't answer, and whether it arrived couldn't be confirmed.
    Unconfirmed,
    /// A file was gone, changed or not a file when its turn came.
    FileGone,
    /// A file or picture is past its cap ([`FILE_MAX`], [`PICTURE_INLINE_MAX`]).
    FileTooBig,
    /// The file couldn't be shared with everyone in the chat.
    NotShared,
}

/// A new message to announce, as the worker (which holds every chat's members) tells of it; the
/// engine decides whether and how it's announced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct News {
    pub chat: String,
    /// Its id; empty for the new last message of a chat whose messages can't be read (known
    /// from the list of chats only).
    pub message: String,
    /// Who sent it, by the name the chat's members give them (else the message's), on one line.
    pub sender: String,
    /// The sender isn't proven to be of the user's organisation.
    pub external: bool,
    /// The sender is an application, such as a bot.
    pub app: bool,
    /// It names the user.
    pub mentions_me: bool,
    /// The chat's name; none for a one-to-one chat.
    pub chat_name: Option<String>,
    /// It's sent to the user alone (a one-to-one chat).
    pub direct: bool,
    /// The chat is hidden in the user's list.
    pub hidden: bool,
    /// Its first line ([`super::first_line`], at most [`NEWS_CHARS`] characters).
    pub text: String,
    /// When it was sent, in milliseconds since 1970.
    pub at: i64,
}
