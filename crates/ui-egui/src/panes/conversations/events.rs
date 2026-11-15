// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The Teams worker's events, taken a few a frame: a first copy sends thousands of rows and
//! messages, and applying them all in one frame would freeze the window. The mail's events are
//! applied at once, as before. Then the new messages the engine announces, shown on the desktop
//! ([`announce`]).
//!
//! What is hostile here: how many events the worker sends (it reports what Graph gave). The queue
//! holds at most [`PENDING_MAX`]; past it go, oldest first, the rows and messages (the worker
//! sends them again on its next pass), then the statuses a later one says again, then presences,
//! search results and pictures, and only then new messages to announce. What is said once (who
//! signed in, a copy set aside, members, a chat left, the sign-in page, a picture not to be had,
//! a message found) and the outbox's last word stay past it, up to [`KEPT_ROOM`] more, where the
//! oldest of them go too, counted in the log. A chat's outbox said again stands for the one
//! before it, as the count unsent does. What an announcement says came from Graph: the engine put
//! it on one line and cut it; here a name is cut again so its mark ("External", "App") always
//! shows, and the desktop on Linux and the BSDs escapes the markup of the text.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use quatrelle_engine::model::Event;
use quatrelle_engine::model::pim::teams;
use quatrelle_engine::{Session, TeamsAnnouncement};

use super::pictures::TeamsPictures;
use crate::{Services, i18n, widgets};

/// The most Teams events applied in a frame.
pub(crate) const TEAMS_EVENTS_A_FRAME: usize = 4;
/// The most Teams events waiting.
const PENDING_MAX: usize = 10_000;
/// How many more may wait when all of them are what is said once or the outbox's last word.
const KEPT_ROOM: usize = 1_000;
/// The most characters of a sender's name in a notification's title: cut there, so the mark after
/// it always shows.
const SENDER_CHARS: usize = 40;
/// The longest the window waits for an announcement held, in seconds: the engine's turns are
/// seconds away, and the system's clock can always take a wait this long.
const WAIT_MAX: u64 = 60;

/// The Teams events waiting to be applied, oldest first, and the pictures they brought.
#[derive(Default)]
pub(crate) struct TeamsQueue {
    pending: VecDeque<Event>,
    pub(crate) pictures: TeamsPictures,
}

fn teams_event(e: &Event) -> Option<&teams::Event> {
    match e {
        Event::Teams { event } => Some(event),
        _ => None,
    }
}

fn is_status(e: &Event) -> bool {
    matches!(teams_event(e), Some(teams::Event::Status { .. }))
}

/// What becomes of an event waiting when the queue is full: the classes go in this order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    /// Rows and messages: the worker sends them again.
    SentAgain,
    /// Where the connection stands: a later one says it again (the last always stays).
    Status,
    /// Presences, search results, pictures: said again, or not missed.
    Other,
    /// New messages to announce: only once all the above are gone.
    News,
    /// What the worker says once, and the outbox's last word (superseded, never dropped): kept
    /// past [`PENDING_MAX`], up to [`KEPT_ROOM`] more.
    Kept,
}

fn class(e: &Event) -> Class {
    match teams_event(e) {
        Some(teams::Event::Chats { .. } | teams::Event::Messages { .. }) => Class::SentAgain,
        Some(teams::Event::Status { .. }) => Class::Status,
        Some(teams::Event::Presence { .. } | teams::Event::Found { .. } | teams::Event::Picture { .. }) | None => Class::Other,
        Some(teams::Event::News { .. }) => Class::News,
        Some(
            teams::Event::Account { .. }
            | teams::Event::SetAside { .. }
            | teams::Event::Members { .. }
            | teams::Event::Removed { .. }
            | teams::Event::SignInPage { .. }
            | teams::Event::NoPicture { .. }
            | teams::Event::Placed { .. }
            | teams::Event::Outbox { .. }
            | teams::Event::Unsent { .. },
        ) => Class::Kept,
    }
}

/// The message an outbox answers for (`took`), to be handed on.
fn took_of(e: &mut Event) -> Option<&mut Option<String>> {
    match e {
        Event::Teams { event: teams::Event::Outbox { took, .. } } => Some(took),
        _ => None,
    }
}

impl TeamsQueue {
    /// Every Teams event of `events` moved to the queue; the others stay, in their order.
    pub(crate) fn take(&mut self, events: &mut Vec<Event>) {
        if !events.iter().any(|e| teams_event(e).is_some()) {
            return;
        }
        let (teams, others): (Vec<Event>, Vec<Event>) = std::mem::take(events).into_iter().partition(|e| teams_event(e).is_some());
        *events = others;
        let outbox = teams.iter().any(|e| matches!(teams_event(e), Some(teams::Event::Outbox { .. } | teams::Event::Unsent { .. })));
        self.pending.extend(teams);
        if outbox {
            self.supersede();
        }
        self.bound();
    }

    /// At most `most` events applied, oldest first; whether some still wait (the frame is drawn
    /// again for them). Called every frame, whatever the module.
    pub(crate) fn apply(&mut self, s: &mut Session, most: usize) -> bool {
        for _ in 0..most {
            let Some(e) = self.pending.pop_front() else { break };
            s.apply(e);
        }
        // Pictures answered: taken to be decoded every frame, so the engine's queue of them
        // never fills.
        self.pictures.take(s);
        !self.pending.is_empty()
    }

    /// A chat's outbox said again stands for the one before it (each says all of that chat's
    /// messages going out), as the count unsent does for the counts before: the earlier go. An
    /// earlier outbox that answered for a message (`took`) hands that answer to the later one
    /// when it answers for none; when both answer for one, both stay (the engine needs each).
    fn supersede(&mut self) {
        // From the newest: the outbox standing for those before it, by chat.
        let mut standing: HashMap<String, usize> = HashMap::new();
        let mut counted = false;
        let mut gone = vec![false; self.pending.len()];
        for i in (0..self.pending.len()).rev() {
            let (chat, answers) = match self.pending.get(i).and_then(teams_event) {
                Some(teams::Event::Unsent { .. }) => {
                    if let Some(g) = gone.get_mut(i) {
                        *g = counted;
                    }
                    counted = true;
                    continue;
                }
                Some(teams::Event::Outbox { chat, took, .. }) => (chat.clone(), took.is_some()),
                _ => continue,
            };
            let Some(&later) = standing.get(&chat) else {
                standing.insert(chat, i);
                continue;
            };
            let later_answers = matches!(self.pending.get(later).and_then(teams_event), Some(teams::Event::Outbox { took: Some(_), .. }));
            if answers && later_answers {
                standing.insert(chat, i);
                continue;
            }
            if answers {
                let took = self.pending.get_mut(i).and_then(took_of).and_then(Option::take);
                if let Some(slot) = self.pending.get_mut(later).and_then(took_of) {
                    *slot = took;
                }
            }
            if let Some(g) = gone.get_mut(i) {
                *g = true;
            }
        }
        let mut i = 0usize;
        self.pending.retain(|_| {
            let keep = !gone.get(i).copied().unwrap_or(false);
            i = i.saturating_add(1);
            keep
        });
    }

    /// Past [`PENDING_MAX`]: the classes go in their order, oldest first, the last status always
    /// staying; what is kept goes only past [`KEPT_ROOM`] more, oldest first, counted in the log.
    fn bound(&mut self) {
        let mut over = self.pending.len().saturating_sub(PENDING_MAX);
        for which in [Class::SentAgain, Class::Status, Class::Other, Class::News] {
            if over == 0 {
                return;
            }
            let dropped = self.drop_oldest(&mut over, |e| class(e) == which);
            if which == Class::News && dropped > 0 {
                log::warn!("teams: the queue of events was full: {dropped} of new messages to announce dropped");
            }
        }
        let mut over = self.pending.len().saturating_sub(PENDING_MAX.saturating_add(KEPT_ROOM));
        if over > 0 {
            let dropped = self.drop_oldest(&mut over, |_| true);
            log::warn!("teams: the queue of events was full: {dropped} said once dropped");
        }
    }

    /// The oldest events `which` takes dropped, `over` at most (counted down), the last status
    /// kept; how many went.
    fn drop_oldest(&mut self, over: &mut usize, which: impl Fn(&Event) -> bool) -> usize {
        let last_status = self.pending.iter().rposition(is_status);
        let (mut i, mut dropped) = (0usize, 0usize);
        self.pending.retain(|e| {
            let drop = *over > 0 && Some(i) != last_status && which(e);
            if drop {
                *over = over.saturating_sub(1);
                dropped = dropped.saturating_add(1);
            }
            i = i.saturating_add(1);
            !drop
        });
        dropped
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether a Teams event waiting is one `f` takes.
    #[cfg(test)]
    pub(crate) fn holds(&self, f: impl Fn(&teams::Event) -> bool) -> bool {
        self.pending.iter().filter_map(teams_event).any(f)
    }
}

/// Teams' new messages the engine announces, as desktop notifications. Called every wake, the
/// window shown or not: the engine is told first whether the window is looked at (the open chat
/// on screen is then marked read instead of announced), and the window wakes again when the next
/// announcement held is due. `put_away`: the window is hidden in the notification area.
pub(crate) fn announce(s: &mut Session, services: &mut dyn Services, ctx: &egui::Context, put_away: bool) {
    s.teams_focus(looked_at(ctx, put_away));
    for a in s.take_teams_announcements() {
        services.notify(&title_of(&a), &body_of(&a));
    }
    if let Some(due) = s.teams_next_announcement() {
        ctx.request_repaint_after(repaint_in(due, s.clock.now));
    }
}

/// Whether the window is looked at: it has the focus, and isn't minimized, covered whole or put
/// away in the notification area (a desktop may leave the focus to a window it hides).
fn looked_at(ctx: &egui::Context, put_away: bool) -> bool {
    !put_away && ctx.input(|i| i.focused && i.viewport().visible() != Some(false))
}

/// How long to wait at `now` for what is due at `due` (Unix seconds): nothing once it's past, and
/// at most [`WAIT_MAX`].
pub(crate) fn repaint_in(due: i64, now: i64) -> Duration {
    Duration::from_secs(u64::try_from(due.saturating_sub(now)).unwrap_or(0).min(WAIT_MAX))
}

/// A notification's title: who sent it, the mark after their name ("External" first, then "App")
/// never cut, whether it names the user, and where (a group's or a meeting's name); "Teams" for
/// the summary of a burst. With no name at all, the mark alone, or the chat's name.
pub(crate) fn title_of(a: &TeamsAnnouncement) -> String {
    if a.chats > 1 {
        return tl!("Teams").to_string();
    }
    let name = widgets::shorten(&a.sender, SENDER_CHARS);
    let who = if a.external {
        tf!("{name} (External)", name = name)
    } else if a.app {
        tf!("{name} (App)", name = name)
    } else {
        name
    };
    let who = who.trim();
    if who.is_empty() {
        return a.chat_name.clone().unwrap_or_else(|| tl!("Teams").to_string());
    }
    match (&a.chat_name, a.mentioned) {
        (Some(chat), true) => tf!("{sender} mentioned you in {chat}", sender = who, chat = chat),
        (None, true) => tf!("{sender} mentioned you", sender = who),
        (Some(chat), false) => tf!("{sender} in {chat}", sender = who, chat = chat),
        (None, false) => who.to_string(),
    }
}

/// A notification's text: the message's first line ("New message" when it has none), else how
/// many came, in its chat or, for a burst, in how many chats. "New message" and "{n} new
/// messages" are the mail's words in most languages; where the mail's are a letter's (Russian,
/// Chinese), the catalogue says Teams' own (context `teams`).
pub(crate) fn body_of(a: &TeamsAnnouncement) -> String {
    if a.chats > 1 {
        return tf!("{n} new messages in {chats} conversations", n = a.count, chats = a.chats);
    }
    if a.count > 1 {
        return i18n::fmt(i18n::tc("teams", "{n} new messages"), &[("n", &a.count.to_string())]);
    }
    if a.text.is_empty() { i18n::tc("teams", "New message").to_string() } else { a.text.clone() }
}
