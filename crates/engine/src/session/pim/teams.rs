// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Teams, as the engine keeps it: where the connection stands, the rows of the list, the open
//! chat's last messages, presence, search hits and pictures waiting for the window. Only the
//! Teams worker (`net::teams`) talks to Graph and holds the copy; the engine sends it jobs and
//! applies what it reports, in windows that never grow with the copy (`window.rs`).
//!
//! What is hostile here: everything an event carries came from Graph or the copy on disk (the
//! worker checked it, the engine still bounds it: the rows at [`teams::MAX_CHATS`], the window at
//! [`teams::WINDOW_MAX`], presence, hits and pictures by count and bytes); the commands' params come
//! from the window or a script, checked before anything changes.

use std::collections::VecDeque;

use quatrelle_model::pim::teams::{self, ChatMessage, ChatRow, MAX_ID, MAX_MEMBERS, Member, Picture, Presence, Status, TeamsSettings, one_line};
use quatrelle_model::{valid_oauth_client_id, valid_oauth_tenant};

use super::*;

mod window;

#[cfg(test)]
pub(crate) use window::row_scans;

/// The most search hits kept (the worker sends at most this many).
pub const TEAMS_HITS_MAX: usize = 200;
/// The most people whose presence is kept.
pub const PRESENCE_MAX: usize = 5_000;
/// The most pictures waiting for the window, and the most bytes they hold together (one picture
/// is at most [`teams::MAX_PICTURE`]).
pub const PICTURES_MAX: usize = 64;
pub const PICTURE_BYTES_MAX: usize = 64 << 20;
/// The most pictures remembered as asked (a missing photo isn't asked again every frame).
const ASKED_MAX: usize = 5_000;

/// What a search of the conversations found: (chat id, message id; empty for a chat's name).
/// `indexing`: the index isn't complete yet; `partial`: some of the copy wasn't searched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TeamsFound {
    pub query: String,
    pub hits: Vec<(String, String)>,
    pub indexing: bool,
    pub partial: bool,
}

/// Teams in the engine: bounded windows over the worker's copy.
#[derive(Default)]
pub struct TeamsState {
    pub status: Status,
    /// The sign-in won't be remembered (no keychain): said once by the worker, kept until signed
    /// out, whatever status follows.
    pub not_remembered: bool,
    /// The rows of the list, at most [`teams::MAX_CHATS`], newest activity first.
    pub rows: Vec<ChatRow>,
    /// Each row's place in `rows`, by chat id.
    index: HashMap<String, usize>,
    /// The chat on screen.
    pub open: Option<String>,
    /// Its messages, in creation order, at most [`teams::WINDOW_MAX`].
    pub window: VecDeque<ChatMessage>,
    /// The user went back through the history since opening it: past [`teams::WINDOW_MAX`], the newest
    /// end goes (else the oldest).
    scrolled_back: bool,
    /// The newest end was dropped: messages newer than the window's last aren't taken (they'd
    /// follow a gap); opening the chat again shows them.
    pub newer_left_out: bool,
    /// The open chat's members (at most [`MAX_MEMBERS`]) and its link (`webUrl`, kept as Graph
    /// gave it: checked where it's used), as the worker last said them.
    pub members: Vec<Member>,
    pub web_url: Option<String>,
    /// The page last asked for (its `before`): asked once.
    asked_before: Option<i64>,
    /// People's presence, by user id, at most [`PRESENCE_MAX`].
    pub presence: HashMap<String, Presence>,
    /// The users whose presence was last asked for.
    visible: HashSet<String>,
    pub found: Option<TeamsFound>,
    /// The query last sent: only its answer is taken.
    searching: Option<String>,
    /// Pictures answered, oldest first, for the window to take.
    pub pictures: VecDeque<(Picture, Vec<u8>)>,
    /// The pictures asked for and not given up: only their answers are taken, and a hosted image
    /// or a photo isn't asked twice.
    asked: HashSet<Picture>,
    /// The host of the user's OneDrive, which their shared files come from.
    pub drive_host: Option<String>,
    /// A copy made for another account, set aside (its path), until the user deletes it.
    pub set_aside: Option<String>,
}

impl TeamsState {
    /// The rail's badge: unread messages of the listed (not hidden) chats; none at 0, `99+` past 99.
    pub fn unread_badge(&self) -> Option<String> {
        let n = self.rows.iter().filter(|r| !r.hidden).fold(0u64, |n, r| n.saturating_add(u64::from(r.unread)));
        match n {
            0 => None,
            1..=99 => Some(n.to_string()),
            _ => Some("99+".into()),
        }
    }

    /// Whether this chat is listed.
    pub fn has_chat(&self, chat: &str) -> bool {
        self.index.contains_key(chat)
    }

    /// A chat's row, by its id.
    pub fn row(&self, chat: &str) -> Option<&ChatRow> {
        self.index.get(chat).and_then(|&i| self.rows.get(i))
    }

    /// The listed rows (not hidden), in the list's order.
    pub fn listed(&self) -> impl Iterator<Item = &ChatRow> {
        self.rows.iter().filter(|r| !r.hidden)
    }

    /// `chat` (or none) now on screen: its window begins empty, filled by what the worker sends.
    pub(crate) fn open_chat(&mut self, chat: Option<String>) {
        self.open = chat;
        self.window.clear();
        self.members.clear();
        self.web_url = None;
        self.scrolled_back = false;
        self.newer_left_out = false;
        self.asked_before = None;
        // The images of the chat left aren't waited for any more.
        self.asked.retain(|p| !matches!(p, Picture::Hosted { .. }));
    }

    /// The open chat's members, in parts (`first`: they begin here), and its link; another chat's
    /// aren't taken. A member without a usable id is left out, and past [`MAX_MEMBERS`] the rest.
    fn take_members(&mut self, chat: &str, members: Vec<Member>, web_url: Option<String>, first: bool) {
        if self.open.as_deref() != Some(chat) {
            return;
        }
        if first {
            self.members.clear();
            // Longer than a browser is handed: no link (it's checked again where it's used).
            self.web_url = web_url.filter(|u| u.len() <= 2048);
        }
        let room = MAX_MEMBERS.saturating_sub(self.members.len());
        self.members.extend(members.into_iter().filter(|m| !m.user_id.is_empty() && m.user_id.len() <= MAX_ID).take(room));
    }

    /// The page before the window to ask for, unless it was asked already.
    pub(crate) fn older_page(&mut self) -> Option<i64> {
        let before = self.window.front()?.created;
        if self.asked_before == Some(before) {
            return None;
        }
        self.asked_before = Some(before);
        self.scrolled_back = true;
        Some(before)
    }

    /// The users now shown: whether that's another set than the one asked for last.
    pub(crate) fn now_visible(&mut self, users: HashSet<String>) -> bool {
        if users == self.visible {
            return false;
        }
        self.visible = users;
        true
    }

    /// A picture to ask for: whether to (a hosted image or a photo is asked once; an image from
    /// elsewhere is the user's click, asked each time).
    pub(crate) fn ask_picture(&mut self, picture: &Picture) -> bool {
        if self.asked.len() >= ASKED_MAX {
            self.asked.clear();
        }
        self.asked.insert(picture.clone()) || matches!(picture, Picture::Outside { .. })
    }

    pub(crate) fn search(&mut self, query: Option<String>) {
        if query.is_none() {
            self.found = None;
        }
        self.searching = query;
    }

    fn found(&mut self, query: String, mut hits: Vec<(String, String)>, indexing: bool, partial: bool) {
        if self.searching.as_deref() != Some(query.as_str()) {
            return;
        }
        // A chat's id, and a message's (empty for the chat's name).
        hits.retain(|(chat, message)| !chat.is_empty() && chat.len() <= MAX_ID && message.len() <= MAX_ID);
        hits.truncate(TEAMS_HITS_MAX);
        self.found = Some(TeamsFound { query, hits, indexing, partial });
    }

    fn merge_presence(&mut self, presences: Vec<(String, Presence)>) {
        for (user, p) in presences {
            if user.is_empty() || user.len() > MAX_ID {
                continue;
            }
            if self.presence.len() >= PRESENCE_MAX && !self.presence.contains_key(&user) {
                // Full: only the people shown now are kept.
                let visible = &self.visible;
                self.presence.retain(|u, _| visible.contains(u));
                if self.presence.len() >= PRESENCE_MAX {
                    continue;
                }
            }
            self.presence.insert(user, p);
        }
    }

    fn take_picture(&mut self, picture: Picture, bytes: Vec<u8>) {
        if !self.asked.contains(&picture) || bytes.len() > teams::MAX_PICTURE {
            return;
        }
        if matches!(picture, Picture::Outside { .. }) {
            self.asked.remove(&picture);
        }
        self.pictures.push_back((picture, bytes));
        let mut held: usize = self.pictures.iter().map(|(_, b)| b.len()).sum();
        while self.pictures.len() > PICTURES_MAX || held > PICTURE_BYTES_MAX {
            let Some((dropped, b)) = self.pictures.pop_front() else { break };
            held = held.saturating_sub(b.len());
            // Never seen by the window: it may be asked again.
            self.asked.remove(&dropped);
        }
    }
}

/// The application (client) id and directory (tenant) id a local build was made with
/// (`QUATRELLE_TEAMS_CLIENT_ID`, `QUATRELLE_TEAMS_TENANT`), each empty unless it could be one: a
/// published build carries none.
pub(crate) fn built_in() -> (String, String) {
    let client = option_env!("QUATRELLE_TEAMS_CLIENT_ID").map(str::trim).filter(|c| valid_oauth_client_id(c)).unwrap_or_default();
    let tenant = option_env!("QUATRELLE_TEAMS_TENANT").map(str::trim).filter(|t| valid_oauth_tenant(t)).unwrap_or_default();
    (client.to_string(), tenant.to_string())
}

/// File › Options › Teams at first: off, with the ids of a local build.
pub(crate) fn default_settings() -> TeamsSettings {
    let (client_id, tenant) = built_in();
    TeamsSettings { client_id, tenant, ..TeamsSettings::default() }
}

impl Session {
    /// Teams at start (and once the settings are read): on and signed in, the worker goes on with
    /// the sign-in kept. Off or signed out, the Conversations module isn't shown: Mail is.
    pub fn teams_start(&mut self) {
        if self.prefs.teams.on && self.prefs.teams.signed_in {
            self.jobs.push(Job::Teams { job: teams::Job::Start { settings: self.prefs.teams.clone() } });
        } else {
            self.teams_fallback();
        }
    }

    /// The Conversations module left for Mail while Teams is off or signed out.
    pub(crate) fn teams_fallback(&mut self) {
        if self.module == Module::Conversations && !(self.prefs.teams.on && self.prefs.teams.signed_in) {
            self.module = Module::Mail;
        }
    }

    /// Teams switched off: the worker stops, nothing of the copy is kept in memory.
    pub(crate) fn teams_off(&mut self) {
        self.prefs.teams.on = false;
        // The copy set aside stays offered for deletion (the worker deletes it, on or off).
        let set_aside = self.teams.set_aside.take();
        self.teams = TeamsState { set_aside, ..TeamsState::default() };
        self.jobs.push(Job::Teams { job: teams::Job::Stop });
        self.teams_fallback();
    }

    /// Signed out (asked, or said by the worker): who it was forgotten, nothing of the copy kept
    /// (a copy set aside stays offered for deletion), and Mail shown instead of Conversations.
    pub(crate) fn teams_signed_out(&mut self) {
        let t = &mut self.prefs.teams;
        *t = TeamsSettings {
            on: t.on,
            client_id: std::mem::take(&mut t.client_id),
            tenant: std::mem::take(&mut t.tenant),
            notify: t.notify,
            notify_only_direct: t.notify_only_direct,
            ..TeamsSettings::default()
        };
        let set_aside = self.teams.set_aside.take();
        self.teams = TeamsState { set_aside, ..TeamsState::default() };
        self.teams_fallback();
    }

    /// What the Teams worker reports. Off, nothing is taken (what was on its way when it was
    /// switched off); signed out, only where the connection stands and who signed in.
    pub fn teams_apply(&mut self, e: teams::Event) {
        if !self.prefs.teams.on {
            return;
        }
        let signed_in = self.prefs.teams.signed_in;
        match e {
            teams::Event::Status { status } => self.teams_status(status),
            teams::Event::Account { name, address, object_id, tenant_id, drive_host } => {
                if object_id.is_empty() || object_id.len() > MAX_ID || tenant_id.len() > MAX_ID {
                    return log::warn!("teams: an account without a usable id was passed over");
                }
                let drive_host = drive_host.filter(|h| h.len() <= 253 && h.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.')));
                let t = &mut self.prefs.teams;
                t.signed_in = true;
                t.name = one_line(&name);
                t.address = one_line(&address);
                t.object_id = object_id;
                t.tenant_id = tenant_id;
                t.drive_host.clone_from(&drive_host);
                self.teams.drive_host = drive_host;
            }
            teams::Event::SetAside { path } => self.teams.set_aside = Some(path),
            _ if !signed_in => {}
            teams::Event::Chats { rows } => self.teams.upsert_rows(rows),
            teams::Event::Messages { chat, messages, .. } => self.teams.take_messages(&chat, messages),
            teams::Event::Presence { presences } => self.teams.merge_presence(presences),
            teams::Event::Found { query, hits, indexing, partial } => self.teams.found(query, hits, indexing, partial),
            teams::Event::Picture { picture, bytes } => self.teams.take_picture(picture, bytes),
            teams::Event::Removed { chat } => self.teams.left(&chat),
            teams::Event::Members { chat, members, web_url, first } => self.teams.take_members(&chat, members, web_url, first),
        }
    }

    fn teams_status(&mut self, status: Status) {
        match status {
            Status::SignedOut => {
                self.teams_signed_out();
            }
            Status::NoKeychain => {
                self.teams.not_remembered = true;
                self.teams.status = Status::NoKeychain;
            }
            // The sign-in no longer holds: the copy stays readable, and opens again at the next
            // start.
            other => self.teams.status = other,
        }
    }
}
