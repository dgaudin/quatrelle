// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What the engine asks of the Teams worker, and what it answers: carried by
//! [`crate::Job::Teams`] and [`crate::Event::Teams`].

use serde::{Deserialize, Serialize};

use super::{ChatMessage, ChatRow, Member, News, Op, Outgoing, Picture, Presence, Status, TeamsSettings};

/// A job for the Teams worker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "teams", rename_all = "camelCase")]
pub enum Job {
    /// Start (or go on) with these settings: signed in, the copy opened and kept up to date.
    Start { settings: TeamsSettings },
    /// Sign in through the browser with the application `client_id`, to organisation `tenant`.
    SignIn { client_id: String, tenant: String },
    /// Forget the sign-in; `delete_copy`: delete the local copy too.
    SignOut { delete_copy: bool },
    /// Stop: Teams was switched off.
    Stop,
    /// The chat now open (none: the list only), read more often.
    Open { chat: Option<String> },
    /// The chat's messages created before `before` (milliseconds since 1970).
    Page { chat: String, before: i64 },
    /// The users now shown, whose presence is wanted.
    Visible { users: Vec<String> },
    /// Look `query` up in the chats' names and the copy's messages.
    Search { query: String },
    /// Read a picture; answered by [`Event::Picture`].
    Picture { picture: Picture },
    /// Delete the copy [`Event::SetAside`] said was set aside at `path` (the user's choice); a
    /// path that isn't one the worker sets a copy aside under is refused.
    DeleteSetAside { path: String },
    /// An operation for the outbox: a send, kept until Teams has it ([`Event::Outbox`] tells
    /// where it stands), or a chat to mark read.
    Op { op: Op },
    /// Send again the message `seq` that wasn't sent; nothing for one in any other state.
    Retry { seq: u64 },
    /// Delete the message `seq` from the outbox; refused while it's being sent.
    Discard { seq: u64 },
    /// Open `chat` at its message `message` (a search's hit): answered by that message's page
    /// and those after it, then [`Event::Placed`].
    Find { chat: String, message: String },
}

/// What the Teams worker reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "teams", rename_all = "camelCase")]
pub enum Event {
    Status {
        status: Status,
    },
    /// Who signed in.
    Account {
        name: String,
        address: String,
        object_id: String,
        tenant_id: String,
        drive_host: Option<String>,
    },
    /// Rows of the list, new or changed.
    Chats {
        rows: Vec<ChatRow>,
    },
    /// A chat's messages: `fresh`, the newest (else older ones, [`Job::Page`]); `older`: there are
    /// older ones still.
    Messages {
        chat: String,
        messages: Vec<ChatMessage>,
        fresh: bool,
        older: bool,
    },
    /// The open chat's members and its link (`webUrl`), from the copy: when it's opened, and
    /// again when they change. A chat of more than [`super::EVENT_ITEMS`] members comes in
    /// several events; `first`: its members begin here (those said before are replaced).
    Members {
        chat: String,
        members: Vec<Member>,
        web_url: Option<String>,
        first: bool,
    },
    /// People's presence, by user id.
    Presence {
        presences: Vec<(String, Presence)>,
    },
    /// What [`Job::Search`] found: (chat id, message id; empty for a chat's name);
    /// `indexing`: the index isn't complete yet; `partial`: some of the copy wasn't searched.
    Found {
        query: String,
        hits: Vec<(String, String)>,
        indexing: bool,
        partial: bool,
    },
    /// A picture [`Job::Picture`] asked for.
    Picture {
        picture: Picture,
        bytes: Vec<u8>,
    },
    /// A chat gone from the copy.
    Removed {
        chat: String,
    },
    /// A copy made for another account (or one that didn't say whose it was) was in the way: it
    /// was set aside unread, at `path`, and is the user's to keep or to delete.
    SetAside {
        path: String,
    },
    /// The messages going out to `chat`, all of them, in order (said when they change and when
    /// the chat is opened); `took`: the engine's id ([`Outgoing::local`]) of the message the
    /// worker has just answered for, kept or refused.
    Outbox {
        chat: String,
        items: Vec<Outgoing>,
        took: Option<String>,
    },
    /// How many messages the outbox holds in all: waiting, being sent or not sent.
    Unsent {
        count: u32,
    },
    /// New messages from others, for the engine to announce.
    News {
        news: Vec<News>,
    },
    /// The page to sign in to Teams on, for the browser: opened only when it's Microsoft's own
    /// ([`crate::ical::is_microsoft_sign_in`]).
    SignInPage {
        url: String,
    },
    /// A picture [`Job::Picture`] asked for that can't be had while Quatrelle runs: not asked
    /// again until it's restarted.
    NoPicture {
        picture: Picture,
    },
    /// What [`Job::Find`] found: `message`'s page and those after it were said in `chat`'s
    /// [`Event::Messages`]; `newest`: they reach its newest message.
    Placed {
        chat: String,
        message: String,
        newest: bool,
    },
}
