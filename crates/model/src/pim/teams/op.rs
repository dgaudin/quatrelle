// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What Quatrelle does in Teams on the user's behalf (a message sent, a chat marked read), the
//! messages going out as the window shows them, and the new messages the worker reports for the
//! engine to announce, with the caps on each.

use serde::{Deserialize, Serialize};

use super::ChatText;

/// The most messages the outbox holds, and the most of one chat's: past them a message isn't taken
/// ([`Unsent::TooMany`]).
pub const OUTBOX_MAX: usize = 500;
pub const OUTBOX_CHAT_MAX: usize = 50;
/// The most bytes of HTML one message sent is written in (80 KiB; its pictures apart).
pub const SEND_MAX: usize = 80 << 10;
/// The longest local id of a message going out ([`Op::Send`]).
pub const LOCAL_MAX: usize = 64;
/// The most characters of a new message's text an announcement shows ([`super::first_line`]).
pub const NEWS_CHARS: usize = 200;

/// What the user did that Teams is to be told: a message sent (kept in the outbox until Teams has
/// it) or a chat read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Op {
    /// Send `body` to `chat`; `local` is the engine's id for it (at most [`LOCAL_MAX`]
    /// characters), which the worker's answers name until Teams gives the message its own.
    Send { chat: String, local: String, body: ChatText },
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
    pub state: OutState,
}

/// Where a message going out stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum OutState {
    /// In the outbox, its turn to come.
    Waiting,
    /// Being sent, or sent without an answer and being checked.
    Sending,
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
