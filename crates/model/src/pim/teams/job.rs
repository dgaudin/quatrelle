// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What the engine asks of the Teams worker, and what it answers: carried by
//! [`crate::Job::Teams`] and [`crate::Event::Teams`].

use serde::{Deserialize, Serialize};

use super::{ChatMessage, ChatRow, Picture, Presence, Status, TeamsSettings};

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
}
