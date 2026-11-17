// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Teams, as the engine keeps it: where the connection stands, the rows of the list, the open
//! chat's last messages, those going out (`outbox.rs`, their files and pictures checked in
//! `files.rs`) and the changes to them going out, shown at once over them (`changes.rs`: edits,
//! deletions, reactions), the files shared asked for and saved (`files.rs`), presence, search
//! hits and pictures waiting for the window, what is announced of new messages (`announce.rs`),
//! and the Teams pages the window opens in the meetings browser's window ([`TeamsPage`]). Only
//! the Teams worker (`net::teams`) talks to Graph and holds the copy and the outbox; the engine
//! sends it jobs and applies what it reports, in windows that never grow with the copy
//! (`window.rs`).
//!
//! What is hostile here: everything an event carries came from Graph or the copy on disk (the
//! worker checked it, the engine still bounds it: the rows at [`teams::MAX_CHATS`], the window at
//! [`teams::WINDOW_MAX`], presence, hits and pictures by count and bytes); the commands' params come
//! from the window or a script, checked before anything changes. A page is handed to the window
//! only when it's Microsoft's sign-in ([`is_microsoft_sign_in`]) or a Teams chat
//! ([`is_teams_chat_link`]); the window checks it again.

use std::collections::VecDeque;

use quatrelle_model::ical::{is_microsoft_sign_in, is_teams_chat_link};
use quatrelle_model::pim::teams::{self, ChatMessage, ChatRow, MAX_ID, Member, Op, Outgoing, Picture, Presence, Status, TeamsSettings, one_line};
use quatrelle_model::{valid_oauth_client_id, valid_oauth_tenant};

use super::*;

mod announce;
mod changes;
mod files;
mod members;
mod outbox;
mod window;

pub use announce::TeamsAnnouncement;
pub use changes::ShownReaction;
pub use files::{TEAMS_FILES_KEPT, TEAMS_TOO_MANY_FILES, TeamsFileState, TeamsSaved, teams_could_run};
pub use members::ChatMembers;
pub use outbox::{
    TEAMS_ATTACH_AGAIN, TEAMS_LEFT, TEAMS_MAY_BE_IN_TEAMS, TEAMS_NOTHING_LEFT, TEAMS_RANGES_MAX, TEAMS_TOO_LONG, TEAMS_TOO_MANY,
    TEAMS_TOO_MANY_DOWNLOADS,
};

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

/// A Teams page for the window to open in the meetings browser's app window (else the default
/// browser): Microsoft's page to sign in on, or a chat in Teams. Each was checked here, and is
/// checked again by the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TeamsPage {
    /// Microsoft's page to sign in on ([`is_microsoft_sign_in`]).
    SignIn(String),
    /// A chat in Teams ([`is_teams_chat_link`]).
    Chat(String),
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
    /// The message the open chat was opened at (a search's hit, [`teams::Job::Find`]), until
    /// another chat or this one is opened again: the window scrolls to it once it holds it.
    pub go_to: Option<String>,
    /// The opening `go_to` is for: a number taken afresh at each opening at a message
    /// (the session's `fresh_id`, never twice in a run, Teams switched off or signed out between):
    /// the window goes to the message once for each, in the window that opening loaded.
    pub go_to_opening: u64,
    /// The open chat's members (at most [`teams::MAX_MEMBERS`], found by id) and its link
    /// (`webUrl`, kept as Graph gave it: checked where it's used), as the worker last said them.
    members: ChatMembers,
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
    /// Pictures the worker said can't be had, oldest first, for the window to take, at most
    /// [`PICTURES_MAX`].
    no_pictures: VecDeque<Picture>,
    /// The pictures asked for and not given up: only their answers are taken, and a picture isn't
    /// asked twice (one that can't be had stays here, never asked again in this session).
    asked: HashSet<Picture>,
    /// The host of the user's OneDrive, which their shared files come from.
    pub drive_host: Option<String>,
    /// A copy made for another account, set aside (its path), until the user deletes it.
    pub set_aside: Option<String>,
    /// The open chat's messages and changes going out: as the worker last said them, then those
    /// sent here it doesn't number (`seq` 0: not answered for yet, or refused); at most
    /// [`teams::OUTBOX_CHAT_MAX`].
    pub outgoing: Vec<Outgoing>,
    /// How many messages the outbox holds in all (waiting, being sent or not sent), as the worker
    /// last said.
    pub unsent: u32,
    /// The messages the outbox refused to take in chats not open (`seq` 0, known here alone), put
    /// back in `outgoing` when their chat opens; at most [`teams::OUTBOX_CHAT_MAX`] a chat, 1,000
    /// in all.
    refused: VecDeque<Outgoing>,
    /// The messages not sent already said by a notice, or seen in their chat.
    said: outbox::Said,
    /// The window is in the background (another application in front, or in the tray), as it
    /// last said ([`Session::teams_focus`], each frame; until then, as a window opens, it has the
    /// focus): the open chat isn't looked at.
    background: bool,
    /// The chat on screen when it was last noted ([`Session::teams_focus`], each frame, and when a
    /// chat is opened or news come): one coming on screen is marked read and its news waiting
    /// aren't announced, once for its coming.
    looked: Option<String>,
    /// What is announced of the new messages: held, given, remembered.
    announcer: announce::Announcer,
    /// Microsoft's page to sign in on, while signing in (the worker's, when it's Microsoft's): to
    /// be opened again or copied while the browser is waited for.
    pub sign_in_page: Option<String>,
    /// The page the window is to open, once ([`Session::take_teams_page`]).
    page: Option<TeamsPage>,
    /// The open chat's files asked for, the saved ones waiting for the window, the parts of the
    /// messages sent not answered for yet.
    files: files::Files,
}

impl TeamsState {
    /// The rail's badge: unread messages of the listed chats ([`TeamsState::listed`]); none at 0,
    /// `99+` past 99.
    pub fn unread_badge(&self, show_unreadable: bool) -> Option<String> {
        let n = self.listed(show_unreadable).fold(0u64, |n, r| n.saturating_add(u64::from(r.unread)));
        match n {
            0 => None,
            1..=99 => Some(n.to_string()),
            _ => Some("99+".into()),
        }
    }

    /// The open chat's link, when it's a Teams chat link "Open in Teams" may open.
    pub fn teams_link(&self) -> Option<&str> {
        self.web_url.as_deref().filter(|u| is_teams_chat_link(u))
    }

    /// `page` for the window to open: the last asked for, once.
    pub(crate) fn open_page(&mut self, page: TeamsPage) {
        self.page = Some(page);
    }

    /// Whether this chat is listed.
    pub fn has_chat(&self, chat: &str) -> bool {
        self.index.contains_key(chat)
    }

    /// A chat's row, by its id.
    pub fn row(&self, chat: &str) -> Option<&ChatRow> {
        self.index.get(chat).and_then(|&i| self.rows.get(i))
    }

    /// The open chat's members.
    pub fn members(&self) -> &ChatMembers {
        &self.members
    }

    /// The open chat's member with this id, whatever its capitals.
    pub fn member(&self, id: &str) -> Option<&Member> {
        self.members.get(id)
    }

    /// The listed rows, in the list's order: not hidden, and those whose messages Graph refuses
    /// only when the user shows them (File › Options › Teams).
    pub fn listed(&self, show_unreadable: bool) -> impl Iterator<Item = &ChatRow> {
        self.rows.iter().filter(move |r| TeamsState::is_listed(r, show_unreadable))
    }

    /// Whether a row is listed ([`TeamsState::listed`]): a search's hits go by it too.
    pub fn is_listed(r: &ChatRow, show_unreadable: bool) -> bool {
        !r.hidden && (show_unreadable || !r.unreadable)
    }

    /// `chat` (or none) now on screen: its window begins empty, filled by what the worker sends,
    /// and no message to go to. Its messages going out are said again by the worker; those it
    /// refused, known here alone, stay with their chat (the same chat opened again keeps them
    /// all).
    pub(crate) fn open_chat(&mut self, chat: Option<String>) {
        if self.open != chat {
            self.switch_outgoing(chat.as_deref());
            self.files.chat_left();
        }
        self.open = chat;
        self.window.clear();
        self.members.clear();
        self.web_url = None;
        self.scrolled_back = false;
        self.newer_left_out = false;
        self.go_to = None;
        self.asked_before = None;
        // The images of the chat left aren't waited for any more.
        self.asked.retain(|p| !matches!(p, Picture::Hosted { .. }));
        self.no_pictures.retain(|p| !matches!(p, Picture::Hosted { .. }));
    }

    /// The open chat's members, in parts (`first`: they begin here), and its link; another chat's
    /// aren't taken. A member without a usable id is left out, and past [`teams::MAX_MEMBERS`]
    /// the rest.
    fn take_members(&mut self, chat: &str, members: Vec<Member>, web_url: Option<String>, first: bool) {
        if self.open.as_deref() != Some(chat) {
            return;
        }
        if first {
            self.members.clear();
            // Longer than a browser is handed: no link (it's checked again where it's used).
            self.web_url = web_url.filter(|u| u.len() <= 2048);
        }
        self.members.extend(members.into_iter().filter(|m| !m.user_id.is_empty() && m.user_id.len() <= MAX_ID));
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

    /// A picture to ask for: whether to (each is asked once; an image from elsewhere, the user's
    /// click, again once it came, never once it can't be had).
    pub(crate) fn ask_picture(&mut self, picture: &Picture) -> bool {
        if self.asked.len() >= ASKED_MAX {
            self.asked.clear();
        }
        self.asked.insert(picture.clone())
    }

    /// Whether `picture` was asked for and not answered yet, or answered that it can't be had
    /// (the window doesn't offer it again).
    pub fn is_asked(&self, picture: &Picture) -> bool {
        self.asked.contains(picture)
    }

    /// The pictures answered, for the window to take. An image kept with a message, once taken,
    /// may be asked for again: the window lets images go past its room and asks for them again
    /// when they're back on screen (a photo isn't: the window keeps photos for the session).
    pub fn take_pictures(&mut self) -> Vec<(Picture, Vec<u8>)> {
        let taken: Vec<(Picture, Vec<u8>)> = self.pictures.drain(..).collect();
        for (p, _) in &taken {
            if matches!(p, Picture::Hosted { .. }) {
                self.asked.remove(p);
            }
        }
        taken
    }

    /// The pictures the worker said can't be had, for the window to take (shown as not
    /// available). They stay asked: none is asked again in this session.
    pub fn take_no_pictures(&mut self) -> Vec<Picture> {
        self.no_pictures.drain(..).collect()
    }

    fn take_no_picture(&mut self, picture: Picture) {
        if !self.asked.contains(&picture) || self.no_pictures.contains(&picture) {
            return;
        }
        self.no_pictures.push_back(picture);
        while self.no_pictures.len() > PICTURES_MAX {
            // Never seen by the window: it may be asked again (the worker answers at once).
            if let Some(dropped) = self.no_pictures.pop_front() {
                self.asked.remove(&dropped);
            }
        }
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

    /// `chat` (or none) on screen: its window filled again by the worker, and a chat with messages
    /// unread that the user is still in and can read here marked read (its row stays as it is
    /// until the worker says it's read).
    pub(crate) fn teams_show(&mut self, chat: Option<String>) {
        self.teams_show_at(chat, None);
    }

    /// [`Session::teams_show`], at the chat's message `at` when one is given (a search's hit):
    /// the worker is asked to find it ([`teams::Job::Find`]) instead of the chat's newest, and the
    /// window goes to it once it holds it.
    pub(crate) fn teams_show_at(&mut self, chat: Option<String>, at: Option<String>) {
        let unread = chat.as_deref().is_some_and(|c| self.teams.to_mark_read(c));
        self.teams.open_chat(chat.clone());
        let at = at.filter(|_| chat.is_some());
        if at.is_some() {
            self.teams.go_to_opening = self.fresh_id();
        }
        self.teams.go_to.clone_from(&at);
        // On screen now: its news waiting aren't announced, and it's marked read below, once.
        self.teams_came_on_screen();
        let mark = chat.clone().filter(|_| unread);
        let job = match (chat, at) {
            (Some(chat), Some(message)) => teams::Job::Find { chat, message },
            (chat, _) => teams::Job::Open { chat },
        };
        self.push_job(Job::Teams { job });
        if let Some(chat) = mark {
            self.teams_mark_read(chat);
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
            show_unreadable: t.show_unreadable,
            ..TeamsSettings::default()
        };
        let set_aside = self.teams.set_aside.take();
        self.teams = TeamsState { set_aside, ..TeamsState::default() };
        self.teams_fallback();
    }

    /// The Teams page the window is to open, once.
    pub fn take_teams_page(&mut self) -> Option<TeamsPage> {
        self.teams.page.take()
    }

    /// The worker's page to sign in on: kept and opened only while signing in, and only when it's
    /// Microsoft's own (a page elsewhere is never handed to a browser).
    fn teams_sign_in_page(&mut self, url: String) {
        if self.teams.status != Status::SigningIn {
            return log::info!("teams: a sign-in page that came after the sign-in was passed over");
        }
        if !is_microsoft_sign_in(&url) {
            return log::warn!("teams: a sign-in page that isn't Microsoft's was not opened");
        }
        self.teams.open_page(TeamsPage::SignIn(url.clone()));
        self.teams.sign_in_page = Some(url);
    }

    /// What the Teams worker reports. Off, nothing is taken (what was on its way when it was
    /// switched off); signed out, only where the connection stands, who signed in, and the page to
    /// sign in on.
    pub fn teams_apply(&mut self, e: teams::Event) {
        if !self.prefs.teams.on {
            return;
        }
        let signed_in = self.prefs.teams.signed_in;
        match e {
            teams::Event::People { .. } | teams::Event::Created { .. } | teams::Event::NotCreated { .. } => {}
            teams::Event::Status { status } => self.teams_status(status),
            teams::Event::Account { name, address, object_id, tenant_id, drive_host } => {
                if object_id.is_empty() || object_id.len() > MAX_ID || tenant_id.len() > MAX_ID {
                    return log::warn!("teams: an account without a usable id was passed over");
                }
                let drive_host = drive_host.filter(|h| h.len() <= 253 && h.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.')));
                self.teams_account_changed(&object_id, &tenant_id);
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
            teams::Event::SignInPage { url } => self.teams_sign_in_page(url),
            _ if !signed_in => {}
            teams::Event::Chats { rows } => self.teams.upsert_rows(rows),
            teams::Event::Messages { chat, messages, .. } => self.teams.take_messages(&chat, messages),
            teams::Event::Presence { presences } => self.teams.merge_presence(presences),
            teams::Event::Found { query, hits, indexing, partial } => self.teams.found(query, hits, indexing, partial),
            teams::Event::Picture { picture, bytes } => self.teams.take_picture(picture, bytes),
            teams::Event::Removed { chat } => self.teams.left(&chat),
            teams::Event::Members { chat, members, web_url, first } => self.teams.take_members(&chat, members, web_url, first),
            teams::Event::Outbox { chat, items, took } => self.teams_outbox(&chat, items, took),
            teams::Event::Unsent { count } => self.teams.unsent = count,
            teams::Event::News { news } => self.teams_news(news),
            teams::Event::Placed { chat, message, newest } => self.teams.placed(&chat, &message, newest),
            teams::Event::File { chat, message, attachment, outcome } => self.teams_file(&chat, &message, &attachment, outcome),
            // A picture not to be had in this run: for the window to show so, and not asked again.
            teams::Event::NoPicture { picture } => self.teams.take_no_picture(picture),
        }
    }

    /// Another account signed in (another user, or the same id in another organisation): nothing
    /// of the last one's chats stays on screen, and what was asked for it is asked again. Where
    /// the connection stands, a sign-in not remembered and a copy set aside stay (the worker says
    /// that copy was set aside after it says who signed in).
    fn teams_account_changed(&mut self, object_id: &str, tenant_id: &str) {
        let was = &self.prefs.teams;
        let same_user = was.object_id.is_empty() || was.object_id.eq_ignore_ascii_case(object_id);
        // An organisation not known before (settings of an earlier version) doesn't tell.
        let same_tenant = was.tenant_id.is_empty() || tenant_id.is_empty() || was.tenant_id.eq_ignore_ascii_case(tenant_id);
        if same_user && same_tenant {
            return;
        }
        let set_aside = self.teams.set_aside.take();
        let status = std::mem::take(&mut self.teams.status);
        let not_remembered = self.teams.not_remembered;
        self.teams = TeamsState { set_aside, status, not_remembered, ..TeamsState::default() };
    }

    fn teams_status(&mut self, status: Status) {
        // The sign-in over (or never begun): its page is forgotten, and not opened if it waits.
        if status != Status::SigningIn {
            self.teams.sign_in_page = None;
            if matches!(self.teams.page, Some(TeamsPage::SignIn(_))) {
                self.teams.page = None;
            }
        }
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
