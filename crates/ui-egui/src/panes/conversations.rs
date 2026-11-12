// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Conversations: Teams' chats, read in Quatrelle. Only the Teams worker talks to Graph and keeps
//! the copy; the engine holds bounded windows of it (`Session::teams`), and the window takes the
//! worker's events a few a frame ([`TeamsQueue`]) so a burst of them never holds a frame up.
//! The module is shown with Teams on and signed in (File › Options › Teams).

mod events;

pub(crate) use events::{TEAMS_EVENTS_A_FRAME, TeamsQueue};

/// What Conversations keeps between frames.
#[derive(Clone, Debug, Default)]
pub struct ConversationsUi {
    /// The search box's text.
    pub query: String,
    /// The search box is to get the keyboard.
    pub focus_search: bool,
    /// The one-to-one users last told to the engine as shown (`teams.visible`).
    pub seen_users: Vec<String>,
}
