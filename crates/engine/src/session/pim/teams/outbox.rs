// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Writing in Teams, as the engine keeps it: the open chat's messages and changes going out, and
//! how many messages the outbox holds in all, as the worker last said them; a message sent (or a
//! change, `changes.rs`) shown at once, before the worker has it; one to another chat that wasn't
//! made said by a notice, once.
//!
//! Only the worker keeps the outbox (`net::teams::outbox`). A message sent here shows at once,
//! without a number (`seq` 0), until the worker answers for it (`took`) or holds it: taken, it
//! shows as the worker keeps it; refused, the worker's answer carries it "Not sent" without a
//! number, and it stays in its conversation, known here alone (in memory: not after a restart),
//! until it's deleted or sent again. An edit or a deletion goes the same way. A reaction is never
//! kept "Not sent": the worker lets it go and says so once, and so does the engine, by a notice
//! wherever its chat is; its chip is Teams' own again. The open chat is on screen only while
//! Conversations is shown in a window with the focus ([`Session::teams_on_screen`]): otherwise, a
//! message, an edit or a deletion not made there is said too.
//!
//! What is hostile here: the commands' params (a text, its ranges and mentions, from the window or
//! a script), checked before anything changes. What the worker says is bounded: a chat's share
//! ([`OUTBOX_CHAT_MAX`]), the local ids ([`LOCAL_MAX`]), the notices remembered ([`SAID_MAX`]),
//! the refusals kept for chats not open ([`REFUSED_MAX`]).

use quatrelle_mime::{Typed, teams_compose_typed, teams_write_mentions};
use quatrelle_model::pim::teams::{ChatText, LOCAL_MAX, Mention, OUTBOX_CHAT_MAX, OUTBOX_MAX, Op, OutKind, OutState, Outgoing};

use super::*;

/// The most messages not sent remembered as said, or as seen in their chat.
const SAID_MAX: usize = 1_000;
/// The most messages refused by the outbox kept for the chats not open (each its share at most).
const REFUSED_MAX: usize = 1_000;
/// The refusals of `teams.send`, `teams.edit`, `teams.retry` and `teams.discard` the window knows
/// (and says in its own words): the message too long for Teams, too many waiting, the chat left,
/// a message that may be in Teams already, an edit that leaves nothing of its message.
pub const TEAMS_TOO_LONG: &str = "This message is too long for Teams";
pub const TEAMS_TOO_MANY: &str = "Too many messages are waiting";
pub const TEAMS_LEFT: &str = "you're no longer in this conversation";
pub const TEAMS_MAY_BE_IN_TEAMS: &str = "This message may already be in Teams";
pub const TEAMS_NOTHING_LEFT: &str = "there's nothing left of this message; delete it instead";
/// The most byte ranges of each style a message sent or edited takes (`teams.send`,
/// `teams.edit`): the box holds no more.
pub const TEAMS_RANGES_MAX: usize = 1_000;

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

/// A reaction not made: never kept, said once.
fn reaction_not_made(o: &Outgoing) -> bool {
    matches!(o.what, OutKind::React { .. }) && matches!(o.state, OutState::NotSent { .. })
}

/// What is newly not made in an outbox's word, by kind: each said by a notice of its own.
#[derive(Clone, Copy, Default)]
struct NotMade {
    sends: bool,
    edits: bool,
    deletions: bool,
    reactions: bool,
}

impl NotMade {
    fn add(&mut self, what: &OutKind) {
        match what {
            OutKind::Send => self.sends = true,
            OutKind::Edit { .. } => self.edits = true,
            OutKind::Delete { .. } => self.deletions = true,
            OutKind::React { .. } => self.reactions = true,
        }
    }
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
    /// answered for them (`took`: kept, or not, as when a reaction taken back cancels the one
    /// waiting) or holds them. Past [`OUTBOX_CHAT_MAX`] the oldest go. What is newly not sent is
    /// noted in `new`.
    fn take_outbox(&mut self, items: Vec<Outgoing>, took: Option<&str>, new: &mut NotMade) {
        let unnumbered: Vec<Outgoing> = std::mem::take(&mut self.outgoing)
            .into_iter()
            .filter(|o| o.seq == 0 && took != Some(o.local.as_str()) && !items.iter().any(|i| i.local == o.local))
            .collect();
        let mut shown = items;
        shown.extend(unnumbered);
        let past = shown.len().saturating_sub(OUTBOX_CHAT_MAX);
        shown.drain(..past);
        for o in &shown {
            self.note(o, new);
        }
        self.outgoing = shown;
    }

    /// Where `o` stands, noted: in `new` when it's newly not sent (neither said nor seen so far).
    fn note(&mut self, o: &Outgoing, new: &mut NotMade) {
        if matches!(o.state, OutState::NotSent { .. }) {
            if self.said.insert(&o.local) {
                new.add(&o.what);
            }
        } else {
            self.said.forget(&o.local);
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
    /// [`teams::Event::Outbox`]: the open chat's messages and changes going out shown as the worker
    /// says them; another chat's refusals kept for it. A message, an edit or a deletion newly not
    /// made in a chat not on screen (not open, or open while another module is shown or the window
    /// is in the background) is said by a notice (once for the event and its kind, whatever their
    /// number). A reaction not made is never kept, here or for its chat: it's said wherever its
    /// chat is, once.
    pub(super) fn teams_outbox(&mut self, chat: &str, items: Vec<Outgoing>, took: Option<String>) {
        if chat.is_empty() || chat.len() > MAX_ID {
            return;
        }
        let items = items.into_iter().filter(|o| o.chat == chat && usable_local(&o.local)).take(OUTBOX_MAX);
        let (not_made, items): (Vec<Outgoing>, Vec<Outgoing>) = items.partition(reaction_not_made);
        let mut new = NotMade::default();
        for o in &not_made {
            self.teams.note(o, &mut new);
        }
        let open = self.teams.open.as_deref() == Some(chat);
        if open {
            // Shown at once and not answered for yet: gone with it.
            self.teams.outgoing.retain(|o| o.seq != 0 || !not_made.iter().any(|n| n.local == o.local));
            let mut seen = NotMade::default();
            self.teams.take_outbox(items, took.as_deref(), &mut seen);
            if !self.teams_on_screen(chat) {
                new = NotMade { reactions: new.reactions, ..seen };
            }
        } else {
            for o in &items {
                self.teams.note(o, &mut new);
            }
            for o in items.into_iter().filter(refused) {
                self.teams.keep_refused(o);
            }
        }
        self.teams_not_made(chat, new);
    }

    /// What wasn't made in `chat`, said: a notice for each kind.
    fn teams_not_made(&mut self, chat: &str, new: NotMade) {
        let NotMade { sends, edits, deletions, reactions } = new;
        if !(sends || edits || deletions || reactions) {
            return;
        }
        // A chat not listed (or without a name) is named as Teams is.
        let name = self.teams.row(chat).map(|r| one_line(&r.name)).filter(|n| !n.is_empty()).unwrap_or_else(|| "Teams".into());
        let named = [("chat", name.as_str())];
        if sends {
            self.notice_with("A message to {chat} wasn't sent", &named, true);
        }
        if edits {
            self.notice_with("Your edit in {chat} wasn't made", &named, true);
        }
        if deletions {
            self.notice_with("Your deletion in {chat} wasn't made", &named, true);
        }
        if reactions {
            self.notice_with("Teams didn't take your reaction in {chat}", &named, true);
        }
    }

    /// What the user typed in `chat`'s box (`typed`: its text, the byte ranges of it in bold and
    /// in italic, the people @mentioned), sent: shown at once when the chat is open, and handed to
    /// the worker. Its local id is returned. Refused when there's nothing to send, judged on what
    /// would be sent (a text of spaces, blank lines or control characters is nothing), and when a
    /// mention isn't a name on one line naming a member of the open chat of the user's
    /// organisation (a chat not open takes none: its members aren't known here).
    pub fn teams_send(&mut self, chat: &str, typed: &Typed) -> Result<String, CmdError> {
        self.teams.mentions_ok(chat, typed, &self.prefs.teams.tenant_id, &[])?;
        let (body, mentions) = teams_compose_typed(typed);
        if body.blocks.is_empty() {
            return Err(CmdError::Params("there's nothing to send".into()));
        }
        self.teams_queue(chat, OutKind::Send, body, mentions)
    }

    /// `what` to go to `chat` (a message `body` with its `mentions`, or a change of one of its
    /// messages): a listed chat the user is still in and Teams lets be read, the HTML within
    /// [`teams::SEND_MAX`], under the outbox's caps (the open chat's share, then all of it). Shown
    /// at once when the chat is open, and handed to the worker as the operation of its kind; an
    /// edit or a deletion lets go of its message's edits not made first (refused, it lets go of
    /// nothing). A message sent from older messages opens the chat again at its newest first: it
    /// shows after them. Its local id is returned.
    pub(super) fn teams_queue(&mut self, chat: &str, what: OutKind, body: ChatText, mentions: Vec<Mention>) -> Result<String, CmdError> {
        let row = self.teams.row(chat).ok_or_else(|| CmdError::Params("that conversation isn't listed".into()))?;
        if row.left {
            return Err(CmdError::Disabled(TEAMS_LEFT.into()));
        }
        if row.unreadable {
            return Err(CmdError::Disabled("Teams doesn't let this conversation be written to here".into()));
        }
        if teams_write_mentions(&body, &mentions).is_err() {
            return Err(CmdError::Params(TEAMS_TOO_LONG.into()));
        }
        let open = self.teams.open.as_deref() == Some(chat);
        let all = usize::try_from(self.teams.unsent).unwrap_or(usize::MAX);
        if (open && self.teams.outgoing.len() >= OUTBOX_CHAT_MAX) || all >= OUTBOX_MAX {
            return Err(CmdError::Disabled(TEAMS_TOO_MANY.into()));
        }
        // Taken: an edit or a deletion takes the place of the message's edits not made.
        if open && let OutKind::Edit { message, .. } | OutKind::Delete { message, .. } = &what {
            for seq in self.teams.drop_edits_not_made(message) {
                self.push_job(Job::Teams { job: teams::Job::Discard { seq } });
            }
        }
        let chat = chat.to_string();
        // A change is to a message of the window as it is: it stays.
        if open && self.teams.newer_left_out && what == OutKind::Send {
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
                mentions: mentions.clone(),
                what: what.clone(),
                state: OutState::Waiting,
            });
        }
        let l = local.clone();
        let op = match what {
            OutKind::Send => Op::Send { chat, local: l, body, mentions },
            OutKind::Edit { message, created } => Op::Edit { chat, local: l, message, created, body, mentions },
            OutKind::Delete { message, created } => Op::Delete { chat, local: l, message, created },
            OutKind::React { message, created, emoji, on } => Op::React { chat, local: l, message, created, emoji, on },
        };
        self.push_job(Job::Teams { job: teams::Job::Op { op } });
        Ok(local)
    }

    /// A message (or a change) of the open chat that wasn't sent, sent again: one the worker keeps
    /// is asked of it again (shown waiting at once); one it refused goes as a new one of its kind,
    /// its text the same (left where it was if that's refused). An edit isn't sent again while its
    /// message is being deleted: it would come after the deletion. Its local id is returned.
    pub(crate) fn teams_retry(&mut self, seq: u64, local: Option<&str>) -> Result<String, CmdError> {
        let i = self.teams.outgoing_at(seq, local)?;
        let what = self.teams.outgoing.get(i).map(|o| &o.what);
        if seq == 0 && matches!(what, Some(OutKind::Edit { message, .. }) if self.teams.deleting(message)) {
            return Err(CmdError::Disabled("this message is being deleted".into()));
        }
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
        match self.teams_queue(&o.chat, o.what.clone(), o.body.clone(), o.mentions.clone()) {
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
