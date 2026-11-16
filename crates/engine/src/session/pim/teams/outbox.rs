// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Writing in Teams, as the engine keeps it: the open chat's messages going out, and how many the
//! outbox holds in all, as the worker last said them; a message sent shown at once, before the
//! worker has it; a message to another chat that wasn't sent said by a notice, once.
//!
//! Only the worker keeps the outbox (`net::teams::outbox`). A message sent here shows at once,
//! without a number (`seq` 0), until the worker answers for it (`took`) or holds it: taken, it
//! shows as the worker keeps it; refused, the worker's answer carries it "Not sent" without a
//! number, and it stays in its conversation, known here alone (in memory: not after a restart),
//! until it's deleted or sent again. The open chat is on screen only while Conversations is
//! shown in a window with the focus ([`Session::teams_on_screen`]): otherwise, a message not sent
//! there is said too.
//!
//! What is hostile here: the commands' params (a text and its ranges, from the window or a
//! script), checked before anything changes. What the worker says is bounded: a chat's share
//! ([`OUTBOX_CHAT_MAX`]), the local ids ([`LOCAL_MAX`]), the notices remembered ([`SAID_MAX`]),
//! the refusals kept for chats not open ([`REFUSED_MAX`]).

use std::ops::Range;

use quatrelle_mime::{teams_compose, teams_write};
use quatrelle_model::pim::teams::{ChatText, LOCAL_MAX, OUTBOX_CHAT_MAX, OUTBOX_MAX, Op, OutKind, OutState, Outgoing};

use super::*;

/// The most messages not sent remembered as said, or as seen in their chat.
const SAID_MAX: usize = 1_000;
/// The most messages refused by the outbox kept for the chats not open (each its share at most).
const REFUSED_MAX: usize = 1_000;
/// The refusals of `teams.send`, `teams.retry` and `teams.discard` the window knows (and says in
/// its own words): the message too long for Teams, too many waiting, the chat left, a message
/// that may be in Teams already.
pub const TEAMS_TOO_LONG: &str = "This message is too long for Teams";
pub const TEAMS_TOO_MANY: &str = "Too many messages are waiting";
pub const TEAMS_LEFT: &str = "you're no longer in this conversation";
pub const TEAMS_MAY_BE_IN_TEAMS: &str = "This message may already be in Teams";

/// The messages not sent already said by a notice, or seen in their chat, by local id (unique to
/// each message sent, where the outbox's numbers come back): at most [`SAID_MAX`], the oldest
/// forgotten first.
#[derive(Default)]
pub(super) struct Said {
    order: VecDeque<String>,
    set: HashSet<String>,
}

impl Said {
    /// `local` remembered: whether it's new.
    fn insert(&mut self, local: &str) -> bool {
        if self.set.contains(local) {
            return false;
        }
        if self.order.len() >= SAID_MAX
            && let Some(oldest) = self.order.pop_front()
        {
            self.set.remove(&oldest);
        }
        self.set.insert(local.to_string());
        self.order.push_back(local.to_string());
        true
    }

    /// `local` going out again: said again if it isn't sent again.
    fn forget(&mut self, local: &str) {
        if self.set.remove(local) {
            self.order.retain(|l| l != local);
        }
    }
}

fn usable_local(local: &str) -> bool {
    !local.is_empty() && local.len() <= LOCAL_MAX
}

/// A message the worker refused to take: it keeps nothing of it.
fn refused(o: &Outgoing) -> bool {
    o.seq == 0 && matches!(o.state, OutState::NotSent { .. })
}

impl TeamsState {
    /// Another chat (or none) opened: the refusals of the one left kept for it, and those of the
    /// one opened shown again.
    pub(super) fn switch_outgoing(&mut self, to: Option<&str>) {
        for o in std::mem::take(&mut self.outgoing) {
            if refused(&o) {
                self.keep_refused(o);
            }
        }
        if let Some(to) = to {
            let (mine, others): (VecDeque<Outgoing>, VecDeque<Outgoing>) = std::mem::take(&mut self.refused).into_iter().partition(|o| o.chat == to);
            self.refused = others;
            self.outgoing = mine.into();
        }
    }

    /// A refusal kept for its chat, not open: once; past its chat's share or [`REFUSED_MAX`] in
    /// all, the oldest goes.
    fn keep_refused(&mut self, o: Outgoing) {
        if self.refused.iter().any(|r| r.local == o.local) {
            return;
        }
        if self.refused.iter().filter(|r| r.chat == o.chat).count() >= OUTBOX_CHAT_MAX
            && let Some(oldest) = self.refused.iter().position(|r| r.chat == o.chat)
        {
            self.refused.remove(oldest);
        }
        if self.refused.len() >= REFUSED_MAX {
            self.refused.pop_front();
        }
        self.refused.push_back(o);
    }

    /// The worker's word on the open chat's messages going out (`items`, all of them, in order):
    /// they replace those shown, followed by those sent here it doesn't number, unless it has just
    /// answered for them (`took`) or holds them. Past [`OUTBOX_CHAT_MAX`] the oldest go. Whether
    /// one is newly not sent.
    fn take_outbox(&mut self, items: Vec<Outgoing>, took: Option<&str>) -> bool {
        let unnumbered: Vec<Outgoing> = std::mem::take(&mut self.outgoing)
            .into_iter()
            .filter(|o| o.seq == 0 && took != Some(o.local.as_str()) && !items.iter().any(|i| i.local == o.local))
            .collect();
        let mut shown = items;
        shown.extend(unnumbered);
        let past = shown.len().saturating_sub(OUTBOX_CHAT_MAX);
        shown.drain(..past);
        let mut new = false;
        for o in &shown {
            new |= self.note(o);
        }
        self.outgoing = shown;
        new
    }

    /// Where `o` stands, noted: whether it's newly not sent (neither said nor seen so far).
    fn note(&mut self, o: &Outgoing) -> bool {
        if matches!(o.state, OutState::NotSent { .. }) {
            self.said.insert(&o.local)
        } else {
            self.said.forget(&o.local);
            false
        }
    }

    /// The open chat's message going out numbered `seq`; by its `local` id for one the worker
    /// doesn't number.
    fn outgoing_at(&self, seq: u64, local: Option<&str>) -> Result<usize, CmdError> {
        let found = match (seq, local) {
            (0, Some(local)) => self.outgoing.iter().position(|o| o.seq == 0 && o.local == local),
            (0, None) => None,
            _ => self.outgoing.iter().position(|o| o.seq == seq),
        };
        found.ok_or_else(|| CmdError::Params("that message isn't going out in the open conversation".into()))
    }

    /// The message going out at `i` taken away.
    fn take_outgoing(&mut self, i: usize) -> Option<Outgoing> {
        (i < self.outgoing.len()).then(|| self.outgoing.remove(i))
    }
}

impl Session {
    /// [`teams::Event::Outbox`]: the open chat's messages going out shown as the worker says them;
    /// another chat's refusals kept for it. A message newly not sent in a chat not on screen (not
    /// open, or open while another module is shown or the window is in the background) is said by
    /// a notice (once for the event, whatever their number).
    pub(super) fn teams_outbox(&mut self, chat: &str, items: Vec<Outgoing>, took: Option<String>) {
        if chat.is_empty() || chat.len() > MAX_ID {
            return;
        }
        let items: Vec<Outgoing> = items.into_iter().filter(|o| o.chat == chat && usable_local(&o.local)).take(OUTBOX_MAX).collect();
        if self.teams.open.as_deref() == Some(chat) {
            if self.teams.take_outbox(items, took.as_deref()) && !self.teams_on_screen(chat) {
                self.teams_not_sent(chat);
            }
            return;
        }
        let mut new = false;
        for o in &items {
            new |= self.teams.note(o);
        }
        for o in items.into_iter().filter(refused) {
            self.teams.keep_refused(o);
        }
        if new {
            self.teams_not_sent(chat);
        }
    }

    /// A message to `chat` not sent, said.
    fn teams_not_sent(&mut self, chat: &str) {
        // A chat not listed (or without a name) is named as Teams is.
        let name = self.teams.row(chat).map(|r| one_line(&r.name)).filter(|n| !n.is_empty()).unwrap_or_else(|| "Teams".into());
        self.notice_with("A message to {chat} wasn't sent", &[("chat", &name)], true);
    }

    /// What the user typed in `chat`'s box (`text`, with the byte ranges of it in bold and in
    /// italic), sent: shown at once when the chat is open, and handed to the worker. Its local id
    /// is returned. Refused when there's nothing to send, judged on what would be sent (a text of
    /// spaces, blank lines or control characters is nothing).
    pub fn teams_send(&mut self, chat: &str, text: &str, bold: Vec<Range<usize>>, italic: Vec<Range<usize>>) -> Result<String, CmdError> {
        let body = teams_compose(text, &bold, &italic);
        if body.blocks.is_empty() {
            return Err(CmdError::Params("there's nothing to send".into()));
        }
        self.teams_queue(chat, body)
    }

    /// `body` to go to `chat`: a listed chat the user is still in and Teams lets be read, its
    /// HTML within [`teams::SEND_MAX`], under the outbox's caps (the open chat's share, then all
    /// of it). Sent from older messages, the chat is opened again at its newest first: the message
    /// shows after them.
    fn teams_queue(&mut self, chat: &str, body: ChatText) -> Result<String, CmdError> {
        let row = self.teams.row(chat).ok_or_else(|| CmdError::Params("that conversation isn't listed".into()))?;
        if row.left {
            return Err(CmdError::Disabled(TEAMS_LEFT.into()));
        }
        if row.unreadable {
            return Err(CmdError::Disabled("Teams doesn't let this conversation be written to here".into()));
        }
        if teams_write(&body).is_err() {
            return Err(CmdError::Params(TEAMS_TOO_LONG.into()));
        }
        let open = self.teams.open.as_deref() == Some(chat);
        let all = usize::try_from(self.teams.unsent).unwrap_or(usize::MAX);
        if (open && self.teams.outgoing.len() >= OUTBOX_CHAT_MAX) || all >= OUTBOX_MAX {
            return Err(CmdError::Disabled(TEAMS_TOO_MANY.into()));
        }
        let chat = chat.to_string();
        if open && self.teams.newer_left_out {
            self.teams_show(Some(chat.clone()));
        }
        // Counted on from the launch's second × 1,000, as drafts' ids are: a run launched in
        // another second (the command line beside the app) sending in the same second doesn't give
        // the same id.
        let n = self.fresh_id();
        let local = format!("{}-{n}", self.clock.now);
        if open {
            self.teams.outgoing.push(Outgoing {
                seq: 0,
                local: local.clone(),
                chat: chat.clone(),
                body: body.clone(),
                mentions: Vec::new(),
                what: OutKind::Send,
                state: OutState::Waiting,
            });
        }
        self.push_job(Job::Teams { job: teams::Job::Op { op: Op::Send { chat, local: local.clone(), body, mentions: Vec::new() } } });
        Ok(local)
    }

    /// A message of the open chat that wasn't sent, sent again: one the worker keeps is asked of
    /// it again (shown waiting at once); one it refused goes as a new message, its text the same
    /// (left where it was if that's refused). Its local id is returned.
    pub(crate) fn teams_retry(&mut self, seq: u64, local: Option<&str>) -> Result<String, CmdError> {
        let i = self.teams.outgoing_at(seq, local)?;
        let Some(o) = self.teams.outgoing.get_mut(i).filter(|o| matches!(o.state, OutState::NotSent { .. })) else {
            return Err(CmdError::Disabled("only a message that wasn't sent can be sent again".into()));
        };
        if seq != 0 {
            o.state = OutState::Waiting;
            let local = o.local.clone();
            self.teams.said.forget(&local);
            self.push_job(Job::Teams { job: teams::Job::Retry { seq } });
            return Ok(local);
        }
        let Some(o) = self.teams.take_outgoing(i) else { return Err(CmdError::Params("that message isn't going out".into())) };
        match self.teams_queue(&o.chat, o.body.clone()) {
            Ok(local) => Ok(local),
            Err(e) => {
                let at = i.min(self.teams.outgoing.len());
                self.teams.outgoing.insert(at, o);
                Err(e)
            }
        }
    }

    /// A message of the open chat going out, deleted: gone from it at once, and from the outbox
    /// when the worker keeps it. Refused while it may be in Teams already: being sent, or sent
    /// from here and not answered for yet.
    pub(crate) fn teams_discard(&mut self, seq: u64, local: Option<&str>) -> Result<(), CmdError> {
        let i = self.teams.outgoing_at(seq, local)?;
        let going = self.teams.outgoing.get(i).is_some_and(|o| o.state == OutState::Sending || (o.seq == 0 && o.state == OutState::Waiting));
        if going {
            return Err(CmdError::Disabled(TEAMS_MAY_BE_IN_TEAMS.into()));
        }
        self.teams.take_outgoing(i);
        if seq != 0 {
            self.push_job(Job::Teams { job: teams::Job::Discard { seq } });
        }
        Ok(())
    }
}
