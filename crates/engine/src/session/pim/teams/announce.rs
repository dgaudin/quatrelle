// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What is announced of the new messages the worker reports ([`teams::Event::News`]): the engine
//! decides, the window shows. Each frame the window says whether it has the focus
//! ([`Session::teams_focus`]) and takes what is due ([`Session::take_teams_announcements`]) into
//! desktop notifications; [`Session::teams_next_announcement`] says when to look again.
//!
//! The rules, in order: the open chat looked at (Conversations shown in a window with the focus)
//! is marked read instead, whatever the settings say; then nothing with notifications off, for a
//! hidden chat or one left out of the list (Graph won't serve it, and those aren't shown), or,
//! when only one-to-one chats and mentions are asked for, for the others; each
//! message once. A chat coming on screen (the window focused again, Conversations shown again,
//! the chat opened) is marked read when it has messages unread, once each time, and its news
//! waiting their turn aren't announced: the user has seen them.
//!
//! One chat's announcements are at least [`GROUP_SECS`] apart, its news meanwhile given as one
//! ("N new messages"); in all at most [`SHOWN_PER_SPAN`] go per [`GROUP_SECS`], the rest summed
//! up in one at the start of the next span, among its own.
//!
//! What is hostile here: what the worker says of a message came from Graph (a stranger's name and
//! text): the ids are checked, names and text put on one line and cut again, and the work is
//! bounded whatever comes: [`HELD_MAX`] chats held (past it the oldest go, their news summed up),
//! [`SEEN_MAX`] messages remembered, [`QUEUE_MAX`] announcements waiting for the window.

use quatrelle_model::pim::teams::{EVENT_ITEMS, NEWS_CHARS, News, first_line};

use super::*;

/// One chat's announcements are at least this many seconds apart; its news meanwhile wait, and
/// go as one.
const GROUP_SECS: i64 = 5;
/// The most announcements given in all per [`GROUP_SECS`]; the rest go as one summary at the
/// start of the next span, counted among its own.
const SHOWN_PER_SPAN: u32 = 4;
/// The most chats held (news waiting, or announced in the last [`GROUP_SECS`]); past it the one
/// held longest goes, its news waiting summed up.
const HELD_MAX: usize = 1_000;
/// The most messages remembered as taken in; past it the oldest are forgotten.
const SEEN_MAX: usize = 2_000;
/// The most announcements waiting for the window; past it the oldest go.
const QUEUE_MAX: usize = 20;

/// A new message to announce, or several (a desktop notification).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TeamsAnnouncement {
    /// The chat it's in; empty for the summary of a burst.
    pub chat: String,
    /// Who sent it, as the chat's members name them, on one line; empty for a summary.
    pub sender: String,
    /// The chat's name; none for a one-to-one chat, or a summary.
    pub chat_name: Option<String>,
    /// The sender isn't proven to be of the user's organisation.
    pub external: bool,
    /// The sender is an application, such as a bot.
    pub app: bool,
    /// It names the user (of several, one does).
    pub mentioned: bool,
    /// Its first line, at most [`NEWS_CHARS`] characters; empty for a summary.
    pub text: String,
    /// How many new messages it stands for: past 1, "N new messages" (of its chat, or, with
    /// `chats`, of the burst).
    pub count: u32,
    /// How many chats: past 1, the summary of a burst.
    pub chats: u32,
}

impl TeamsAnnouncement {
    /// One new message, as the worker told of it, its words bounded again.
    fn of(n: News) -> TeamsAnnouncement {
        TeamsAnnouncement {
            sender: one_line(&n.sender),
            chat_name: n.chat_name.map(|c| one_line(&c)).filter(|c| !c.is_empty()),
            external: n.external,
            app: n.app,
            mentioned: n.mentions_me,
            text: first_line(&n.text, NEWS_CHARS),
            count: 1,
            chats: 1,
            chat: n.chat,
        }
    }

    /// A later message of its chat, given with it: counted, and shown in its place unless this one
    /// names the user and the later one doesn't (a mention isn't lost behind what follows it).
    fn join(&mut self, later: TeamsAnnouncement) {
        let count = self.count.saturating_add(later.count);
        if later.mentioned || !self.mentioned {
            *self = later;
        }
        self.count = count;
    }
}

/// A message taken in: by its id, or by its time for a chat whose messages can't be read (its
/// new last message, known from the list only).
#[derive(Clone, PartialEq, Eq, Hash)]
enum Which {
    Message(String),
    At(i64),
}

/// The messages taken in, by chat: at most [`SEEN_MAX`], the oldest forgotten first.
#[derive(Default)]
struct Seen {
    order: VecDeque<(String, Which)>,
    set: HashSet<(String, Which)>,
}

impl Seen {
    /// Whether this message is new (then remembered).
    fn insert(&mut self, chat: &str, message: &str, at: i64) -> bool {
        let which = if message.is_empty() { Which::At(at) } else { Which::Message(message.to_string()) };
        let key = (chat.to_string(), which);
        if self.set.contains(&key) {
            return false;
        }
        if self.order.len() >= SEEN_MAX
            && let Some(oldest) = self.order.pop_front()
        {
            self.set.remove(&oldest);
        }
        self.set.insert(key.clone());
        self.order.push_back(key);
        true
    }
}

/// One chat's news held: when it was last announced, and those waiting since.
struct Held {
    /// When it was last announced (or summed up), in seconds.
    last: Option<i64>,
    /// Its news waiting, as one announcement, with the order the first came in.
    waiting: Option<(u64, TeamsAnnouncement)>,
    /// The order the chat began to be held in: past [`HELD_MAX`], the oldest go.
    since: u64,
}

impl Held {
    /// When its next announcement may go: now if it wasn't announced lately (or the clock was
    /// put back), else [`GROUP_SECS`] after the last.
    fn turn(&self, now: i64) -> i64 {
        match self.last {
            Some(last) if now >= last => last.saturating_add(GROUP_SECS),
            _ => now,
        }
    }

    fn due(&self, now: i64) -> bool {
        now >= self.turn(now)
    }
}

/// What a span couldn't give, summed up for the start of the next.
struct Summary {
    chats: u32,
    count: u32,
    mentioned: bool,
    /// The first chat's announcement: given as it is when it's the only one.
    first: TeamsAnnouncement,
}

impl Summary {
    fn announcement(self) -> TeamsAnnouncement {
        if self.chats <= 1 {
            return self.first;
        }
        TeamsAnnouncement { mentioned: self.mentioned, count: self.count, chats: self.chats, ..TeamsAnnouncement::default() }
    }
}

/// The announcements held, given and remembered.
#[derive(Default)]
pub(super) struct Announcer {
    held: HashMap<String, Held>,
    /// The news taken in, counted: their order.
    taken: u64,
    /// The span running: when it began, and how many it gave.
    span: Option<(i64, u32)>,
    summary: Option<Summary>,
    seen: Seen,
    /// Given, for the window to take, oldest first.
    queue: VecDeque<TeamsAnnouncement>,
}

impl Announcer {
    /// When the span running ends: [`GROUP_SECS`] after it began (now if the clock was put back).
    fn span_end(&self, now: i64) -> Option<i64> {
        self.span.map(|(start, _)| if now >= start { start.saturating_add(GROUP_SECS) } else { now })
    }

    /// A new message that passed the rules, at `now`: given (or summed up) at once when its chat
    /// may be announced, else waiting with its chat's others.
    fn add(&mut self, a: TeamsAnnouncement, now: i64) {
        let order = self.taken;
        self.taken = self.taken.wrapping_add(1);
        if !self.held.contains_key(&a.chat) {
            self.make_room();
            self.held.insert(a.chat.clone(), Held { last: None, waiting: None, since: order });
        }
        let Some(h) = self.held.get_mut(&a.chat) else { return };
        if let Some((_, w)) = &mut h.waiting {
            w.join(a);
        } else if !h.due(now) {
            h.waiting = Some((order, a));
        } else {
            h.last = Some(now);
            self.offer(a, now);
        }
    }

    /// `chat` looked at: its news waiting aren't announced. When it was last announced stays (the
    /// next one still waits its turn).
    fn forget(&mut self, chat: &str) {
        if let Some(h) = self.held.get_mut(chat) {
            h.waiting = None;
        }
    }

    /// What is due at `now`: the summary when a span begins, then the chats whose news waited
    /// their turn, in the order they came; the news of the chat `looked_at` aren't. A chat neither
    /// waiting nor announced lately is let go.
    fn release(&mut self, now: i64, looked_at: Option<&str>) {
        if let Some(chat) = looked_at {
            self.forget(chat);
        }
        if self.span_end(now).is_some_and(|end| now >= end) {
            self.span = None;
        }
        if self.span.is_none()
            && let Some(summary) = self.summary.take()
        {
            self.offer(summary.announcement(), now);
        }
        let mut due: Vec<(u64, String)> = self
            .held
            .iter()
            .filter(|(_, h)| h.due(now))
            .filter_map(|(chat, h)| h.waiting.as_ref().map(|(order, _)| (*order, chat.clone())))
            .collect();
        due.sort_unstable();
        for (_, chat) in due {
            let Some(h) = self.held.get_mut(&chat) else { continue };
            let Some((_, a)) = h.waiting.take() else { continue };
            h.last = Some(now);
            self.offer(a, now);
        }
        self.held.retain(|_, h| h.waiting.is_some() || !h.due(now));
    }

    /// `a` given at `now` while the span has room (one begins if none runs), else summed up for
    /// the next.
    fn offer(&mut self, a: TeamsAnnouncement, now: i64) {
        let (_, given) = self.span.get_or_insert((now, 0));
        if *given < SHOWN_PER_SPAN {
            *given += 1;
            if self.queue.len() >= QUEUE_MAX {
                self.queue.pop_front();
            }
            self.queue.push_back(a);
        } else {
            self.sum_up(a);
        }
    }

    fn sum_up(&mut self, a: TeamsAnnouncement) {
        match &mut self.summary {
            Some(s) => {
                s.chats = s.chats.saturating_add(1);
                s.count = s.count.saturating_add(a.count);
                s.mentioned |= a.mentioned;
            }
            None => self.summary = Some(Summary { chats: 1, count: a.count, mentioned: a.mentioned, first: a }),
        }
    }

    /// Room for one more chat: at [`HELD_MAX`], the one held longest goes, its news waiting
    /// summed up (none is lost).
    fn make_room(&mut self) {
        while self.held.len() >= HELD_MAX {
            let Some(oldest) = self.held.iter().min_by_key(|(_, h)| h.since).map(|(chat, _)| chat.clone()) else { return };
            if let Some((_, a)) = self.held.remove(&oldest).and_then(|h| h.waiting) {
                self.sum_up(a);
            }
        }
    }

    /// When something held is due: now for what waits for the window, the span's end for the
    /// summary, a chat's turn for its news waiting.
    fn next_due(&self, now: i64) -> Option<i64> {
        let queued = (!self.queue.is_empty()).then_some(now);
        let summary = self.summary.as_ref().map(|_| self.span_end(now).unwrap_or(now));
        let chats = self.held.values().filter(|h| h.waiting.is_some()).map(|h| h.turn(now)).min();
        [queued, summary, chats].into_iter().flatten().min()
    }
}

impl TeamsState {
    /// Whether `chat` may be marked read in Teams from here: listed, the user still in it, and
    /// Teams letting it be read here (one it doesn't keeps its unread mark there).
    pub(super) fn may_mark_read(&self, chat: &str) -> bool {
        self.row(chat).is_some_and(|r| !r.left && !r.unreadable)
    }

    /// Whether `chat` has messages unread, as its row says, that may be marked read from here.
    pub(super) fn to_mark_read(&self, chat: &str) -> bool {
        self.row(chat).is_some_and(|r| r.unread > 0) && self.may_mark_read(chat)
    }

    /// How many chats the announcements hold.
    #[cfg(test)]
    pub(crate) fn announcements_held(&self) -> usize {
        self.announcer.held.len()
    }
}

impl Session {
    /// Whether `chat` is on screen, looked at: open in Conversations, in a window with the focus.
    /// With another module shown, or the window in the background (in the tray, another
    /// application in front), it isn't: its news are announced and a message not sent there is
    /// said, as any other chat's.
    pub(super) fn teams_on_screen(&self, chat: &str) -> bool {
        self.module == Module::Conversations && !self.teams.background && self.teams.open.as_deref() == Some(chat)
    }

    /// Whether the window has the focus, as it says each frame. The open chat come on screen since
    /// the last frame, whichever way (the window focused again, from another application or the
    /// tray, or Conversations shown again): marked read when it has messages unread, as the user
    /// now sees them (its row stays as it is until the worker says it's read), and its news
    /// waiting not announced.
    pub fn teams_focus(&mut self, focused: bool) {
        self.teams.background = !focused;
        if let Some(chat) = self.teams_came_on_screen().filter(|c| self.teams.to_mark_read(c)) {
            self.teams_mark_read(chat);
        }
    }

    /// The chat on screen noted: one come on screen since it was last noted has its news waiting
    /// forgotten, and is returned (for its caller to mark it read, once for its coming).
    pub(super) fn teams_came_on_screen(&mut self) -> Option<String> {
        let on = self.teams.open.as_deref().filter(|c| self.teams_on_screen(c));
        if on == self.teams.looked.as_deref() {
            return None;
        }
        let on = on.map(str::to_string);
        self.teams.looked.clone_from(&on);
        let chat = on?;
        self.teams.announcer.forget(&chat);
        Some(chat)
    }

    pub(super) fn teams_mark_read(&mut self, chat: String) {
        self.push_job(Job::Teams { job: teams::Job::Op { op: Op::MarkRead { chat } } });
    }

    /// [`teams::Event::News`]: the open chat looked at is marked read (once for the event, its
    /// coming on screen included) whatever the settings say, and its news aren't announced; the
    /// others' are, as the settings say, each message once.
    pub(super) fn teams_news(&mut self, news: Vec<News>) {
        let now = self.clock.now;
        let t = &self.prefs.teams;
        let (notify, only_direct, show_unreadable) = (t.notify, t.notify_only_direct, t.show_unreadable);
        let came = self.teams_came_on_screen().filter(|c| self.teams.to_mark_read(c));
        self.teams.announcer.release(now, self.teams.looked.as_deref());
        let mut looked_at = None;
        for n in news.into_iter().take(EVENT_ITEMS) {
            if n.chat.is_empty() || n.chat.len() > MAX_ID || n.message.len() > MAX_ID {
                continue;
            }
            if self.teams_on_screen(&n.chat) {
                looked_at = Some(n.chat);
                continue;
            }
            // Hidden, or left out of the list (Graph won't serve it, and those aren't shown).
            let hidden = n.hidden || self.teams.row(&n.chat).is_some_and(|r| !TeamsState::is_listed(r, show_unreadable));
            if !notify || hidden || (only_direct && !n.direct && !n.mentions_me) {
                continue;
            }
            if self.teams.announcer.seen.insert(&n.chat, &n.message, n.at) {
                self.teams.announcer.add(TeamsAnnouncement::of(n), now);
            }
        }
        if let Some(chat) = came.or(looked_at.filter(|c| self.teams.may_mark_read(c))) {
            self.teams_mark_read(chat);
        }
    }

    /// The announcements due, for the window to show (it takes them each frame). None while
    /// Teams is off, signed out, or its notifications are: what was held is then forgotten.
    pub fn take_teams_announcements(&mut self) -> Vec<TeamsAnnouncement> {
        if !self.teams_announces() {
            self.teams.announcer = Announcer::default();
            return Vec::new();
        }
        // The chat on screen now, even if the window hasn't said so yet this frame: what waited
        // there isn't announced.
        let shown = self.teams.open.as_deref().is_some_and(|c| self.teams_on_screen(c));
        let looked_at = self.teams.open.as_deref().filter(|_| shown);
        self.teams.announcer.release(self.clock.now, looked_at);
        self.teams.announcer.queue.drain(..).collect()
    }

    /// When the next announcement held is due, in Unix seconds (the window looks again then).
    pub fn teams_next_announcement(&self) -> Option<i64> {
        if !self.teams_announces() {
            return None;
        }
        self.teams.announcer.next_due(self.clock.now)
    }

    fn teams_announces(&self) -> bool {
        let t = &self.prefs.teams;
        t.on && t.signed_in && t.notify
    }
}
