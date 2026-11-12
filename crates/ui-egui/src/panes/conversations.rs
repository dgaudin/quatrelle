// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Conversations: Teams' chats, read in Quatrelle. Only the Teams worker talks to Graph and keeps
//! the copy; the engine holds bounded windows of it (`Session::teams`), and the window takes the
//! worker's events a few a frame ([`TeamsQueue`]) so a burst of them never holds a frame up.
//! The module is shown with Teams on and signed in (File › Options › Teams); its list
//! ([`list`]) is where the folders are in Mail.

use std::collections::HashSet;

mod events;
mod list;

pub(crate) use events::{TEAMS_EVENTS_A_FRAME, TeamsQueue};
pub(crate) use list::list;
#[cfg(test)]
pub(crate) use list::picture_of;

/// What Conversations keeps between frames.
#[derive(Clone, Debug, Default)]
pub struct ConversationsUi {
    /// The search box's text.
    pub query: String,
    /// The search box is to get the keyboard.
    pub focus_search: bool,
    /// The one-to-one users last told to the engine as shown (`teams.visible`), sorted.
    pub seen_users: Vec<String>,
    /// When the search typed is to go (the box's clock, in seconds): a moment after the last key.
    pub search_due: Option<f64>,
    /// The users whose photo was asked for (each once).
    pub photos_asked: HashSet<String>,
    /// The account `seen_users` and `photos_asked` were for.
    pub asked_for: String,
    /// The conversation opened from the keyboard is to be scrolled into view.
    pub scroll_to_open: bool,
    /// Where the list was scrolled to and how tall it was, at the last frame.
    pub view: (f32, f32),
}

/// What a row's picture is: a person's (their photo once it came, else their initials), a
/// group's icon, a meeting's, or a chat's initials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RowPicture {
    Person { user: String, initials: String },
    Initials(String),
    Group,
    Meeting,
}
