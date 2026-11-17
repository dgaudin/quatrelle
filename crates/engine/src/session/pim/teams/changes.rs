// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Changes to the open chat's messages, as the engine keeps them: an edit, a deletion, a reaction
//! set or taken back, each handed to the worker as a send is (`outbox.rs`) and shown at once over
//! the window: the chips with the user's own reactions going out
//! ([`TeamsState::shown_reactions`]), a message's edit or deletion going out
//! ([`TeamsState::change_of`]). The window keeps Teams' own state; what goes out is drawn over it
//! until the worker no longer holds it, so a chip clicked twice, or a change Teams refused, ends
//! as Teams has it.
//!
//! What may be changed: an edit or a deletion only of the user's own message (`from.user.id` their
//! object id), a message (not a system line) not deleted; an edit only of one the box can hold
//! whole ([`TeamsState::editable`]); a reaction one of the six quick ones or one the message shows,
//! and an emoji ([`reactable`]); nothing of a message being deleted, nor in a chat left or that
//! Teams doesn't let be read. A person @mentioned is a member of the open chat of the user's
//! organisation ([`mentionable`]), or, in an edit, someone the message mentions already.
//!
//! What is hostile here: the commands' params (from the window or a script: a text, its ranges, a
//! message's id, an emoji), checked before anything changes; the window and the outbox as the
//! worker said them (bounded there: the chips at [`MAX_REACTIONS`]). What it protects: only the
//! user's own messages are changed, and what the screen shows going out is what was handed over.

use std::ops::Range;

use quatrelle_mime::{Typed, teams_compose_typed, teams_typed};
use quatrelle_model::pim::teams::{
    ChatText, MAX_MENTIONS, MAX_REACTIONS, MessageKind, NAME_CHARS, OutKind, OutState, QUICK_REACTIONS, Sender, mentionable, reactable, same_reaction,
};

use super::outbox::TEAMS_NOTHING_LEFT;
use super::*;

/// A reaction's chip as the window draws it: its emoji (as the message lists it), how many
/// reacted so and whether the user did, the user's own going out counted; and who did as Teams
/// last said (the user's own going out isn't in it: `mine` says whether they are).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShownReaction<'a> {
    pub emoji: &'a str,
    pub count: usize,
    pub mine: bool,
    pub users: &'a [String],
}

/// What a mention's params must be, said when they aren't.
const MENTION_SHAPE: &str = "a mention is a name on one line of the text, apart from the others";
/// A mention of someone who can't be mentioned here.
const NOT_A_MEMBER: &str = "a mention names a member of the conversation";

/// Whether `m` is from the user (`me`, their object id, whatever its capitals).
fn own(m: &ChatMessage, me: &str) -> bool {
    !me.is_empty() && matches!(&m.sender, Sender::User { id, .. } if id.eq_ignore_ascii_case(me))
}

/// Whether `o` is a change of the message `message` of the kind `of` picks.
fn is_change(o: &Outgoing, message: &str, of: fn(&OutKind) -> bool) -> bool {
    of(&o.what) && o.what.message() == Some(message)
}

impl TeamsState {
    /// `m`'s chips: its reactions, with the user's own going out (`me` their id) drawn over them,
    /// the last asked of each kind (a reaction not made is none): one set counts the user in, one
    /// taken back counts them out. A kind the message lacks that the user sets comes last (nobody
    /// else in it); a kind left with nobody isn't shown. At most [`MAX_REACTIONS`].
    pub fn shown_reactions<'a>(&'a self, m: &'a ChatMessage, me: &str) -> Vec<ShownReaction<'a>> {
        // The user's wishes going out, by kind: the last of each stands (and whether a chip took it).
        let mut wishes: Vec<(&'a str, bool, bool)> = Vec::new();
        for o in &self.outgoing {
            let OutKind::React { message, emoji, on, .. } = &o.what else { continue };
            if *message != m.id || matches!(o.state, OutState::NotSent { .. }) {
                continue;
            }
            match wishes.iter_mut().find(|(e, ..)| same_reaction(e, emoji)) {
                Some(wish) => wish.1 = *on,
                None => wishes.push((emoji.as_str(), *on, false)),
            }
        }
        let mut shown = Vec::new();
        for r in m.reactions.iter().take(MAX_REACTIONS) {
            let had = !me.is_empty() && r.users.iter().any(|u| u.eq_ignore_ascii_case(me));
            let (count, mine) = match wishes.iter_mut().find(|(e, _, taken)| !*taken && same_reaction(e, &r.emoji)) {
                Some((_, on, taken)) => {
                    *taken = true;
                    let count = match (*on, had) {
                        (true, false) => r.users.len().saturating_add(1),
                        (false, true) => r.users.len().saturating_sub(1),
                        _ => r.users.len(),
                    };
                    (count, *on)
                }
                None => (r.users.len(), had),
            };
            if count > 0 {
                shown.push(ShownReaction { emoji: &r.emoji, count, mine, users: &r.users });
            }
        }
        let added = wishes.iter().filter(|(_, on, taken)| *on && !*taken);
        shown.extend(added.map(|&(emoji, ..)| ShownReaction { emoji, count: 1, mine: true, users: &[] }));
        shown.truncate(MAX_REACTIONS);
        shown
    }

    /// The edit or deletion of the open chat's message `message` going out, as the window draws
    /// it under the message ("Editing…", "Not deleted"…): the last of them, a deletion before any
    /// edit.
    pub fn change_of(&self, message: &str) -> Option<&Outgoing> {
        let last = |of: fn(&OutKind) -> bool| self.outgoing.iter().rev().find(|o| is_change(o, message, of));
        last(|k| matches!(k, OutKind::Delete { .. })).or_else(|| last(|k| matches!(k, OutKind::Edit { .. })))
    }

    /// Whether `m` may be edited here, `me` being the user's id: their own message (not a system
    /// line), not deleted nor unshowable, without an attachment, whose text the box holds whole
    /// ([`TeamsState::typed_for`]), not being deleted.
    pub fn editable(&self, m: &ChatMessage, me: &str) -> bool {
        own(m, me)
            && m.kind == MessageKind::Message
            && m.deleted.is_none()
            && !m.unshowable
            && m.attachments.is_empty()
            && !self.deleting(&m.id)
            && self.typed_for(m).is_some()
    }

    /// What the box loads to edit `m`: the text of its edit going out when there is one (the
    /// last), else its own ([`teams_typed`]); none when the box can't hold it whole.
    pub fn typed_for(&self, m: &ChatMessage) -> Option<Typed> {
        match self.edit_of(&m.id) {
            Some(o) => teams_typed(&o.body, &o.mentions),
            None => teams_typed(&m.body, &m.mentions),
        }
    }

    /// The window's newest message from the user (`me` their id) not deleted: the one ↑ edits.
    /// None while the last isn't known: newer messages left out of the window, or a message of the
    /// user's to the chat going out (not "Not sent": it isn't in the window yet, and ↑ right after
    /// a send is meant for it, not the one before).
    pub fn last_own(&self, me: &str) -> Option<&ChatMessage> {
        let sending = self.outgoing.iter().any(|o| o.what == OutKind::Send && !matches!(o.state, OutState::NotSent { .. }));
        if self.newer_left_out || sending {
            return None;
        }
        self.window.iter().rev().find(|m| own(m, me) && m.deleted.is_none())
    }

    /// Whether a deletion of the open chat's message `message` is going out (in any state).
    pub(super) fn deleting(&self, message: &str) -> bool {
        self.outgoing.iter().any(|o| is_change(o, message, |k| matches!(k, OutKind::Delete { .. })))
    }

    /// The last edit of the open chat's message `message` going out.
    fn edit_of(&self, message: &str) -> Option<&Outgoing> {
        self.outgoing.iter().rev().find(|o| is_change(o, message, |k| matches!(k, OutKind::Edit { .. })))
    }

    /// The open chat's message `message`'s edits that weren't made, let go: a newer edit or a
    /// deletion of it takes their place (sent again later, one would put older words back, or
    /// stay "Not edited" under a message deleted). The numbers of those the worker keeps are
    /// returned, for it to let go of them too; one it refused at the door was known here alone.
    pub(super) fn drop_edits_not_made(&mut self, message: &str) -> Vec<u64> {
        let not_made = |o: &Outgoing| is_change(o, message, |k| matches!(k, OutKind::Edit { .. })) && matches!(o.state, OutState::NotSent { .. });
        let kept: Vec<u64> = self.outgoing.iter().filter(|o| o.seq != 0 && not_made(o)).map(|o| o.seq).collect();
        self.outgoing.retain(|o| !not_made(o));
        kept
    }

    /// The open chat `chat`'s message `message`, in the window: a change needs it there (else the
    /// state it was offered in no longer holds).
    fn shown_message(&self, chat: &str, message: &str) -> Result<&ChatMessage, CmdError> {
        let open = self.open.as_deref() == Some(chat);
        self.window.iter().find(|m| open && m.id == message).ok_or_else(|| CmdError::Disabled("that message isn't in the open conversation".into()))
    }

    /// `typed`'s mentions, checked: at most [`MAX_MENTIONS`], each a range of its text that is a
    /// name on one line as it stands (as the box puts a member's name: [`one_line`]), apart from
    /// the others, and a user id; each naming a member of the open chat `chat` of the
    /// organisation `tenant` ([`mentionable`]), or one of `known` (an edit's: who the message
    /// mentions already). A chat not open takes none: its members aren't known here. Each mention
    /// that passes is sent as one ([`teams_compose_typed`] keeps it).
    pub(super) fn mentions_ok(&self, chat: &str, typed: &Typed, tenant: &str, known: &[&str]) -> Result<(), CmdError> {
        let shape = || CmdError::Params(MENTION_SHAPE.into());
        if typed.mentions.len() > MAX_MENTIONS {
            return Err(shape());
        }
        let open = self.open.as_deref() == Some(chat);
        let mut ranges: Vec<&Range<usize>> = Vec::with_capacity(typed.mentions.len());
        for (range, user) in &typed.mentions {
            // A name is at most NAME_CHARS characters: a longer range isn't one (nor looked at).
            let name = typed.text.get(range.clone()).filter(|t| !t.is_empty() && t.len() <= NAME_CHARS * 4 && one_line(t) == *t);
            if name.is_none() || user.is_empty() || user.len() > MAX_ID || user.chars().any(char::is_control) {
                return Err(shape());
            }
            let member = open && self.member(user).is_some_and(|m| mentionable(m, tenant));
            if !member && !known.iter().any(|k| k.eq_ignore_ascii_case(user)) {
                return Err(CmdError::Params(NOT_A_MEMBER.into()));
            }
            ranges.push(range);
        }
        ranges.sort_by_key(|r| r.start);
        if ranges.windows(2).any(|w| w.first().zip(w.get(1)).is_some_and(|(a, b)| b.start < a.end)) {
            return Err(shape());
        }
        Ok(())
    }
}

impl Session {
    /// [`teams::Job::Op`] of [`Op::Edit`]: the open chat's message `message` given the words the
    /// box holds (`typed`), shown at once under it. Only the user's own message the box can hold
    /// ([`TeamsState::editable`]); its mentions members of the organisation or people it mentions
    /// already; refused when nothing is left of it once composed ([`TEAMS_NOTHING_LEFT`]: delete
    /// it instead). Its local id is returned.
    pub fn teams_edit(&mut self, chat: &str, message: &str, typed: &Typed) -> Result<String, CmdError> {
        let t = &self.teams;
        let m = t.shown_message(chat, message)?;
        if !t.editable(m, &self.prefs.teams.object_id) {
            return Err(CmdError::Disabled("only one's own message, as the box holds it, can be edited".into()));
        }
        let pending = t.edit_of(message).map(|o| o.mentions.as_slice()).unwrap_or_default();
        let known: Vec<&str> = m.mentions.iter().chain(pending).filter_map(|x| x.user_id.as_deref()).collect();
        t.mentions_ok(chat, typed, &self.prefs.teams.tenant_id, &known)?;
        let created = m.created;
        let (body, mentions) = teams_compose_typed(typed);
        if body.blocks.is_empty() {
            return Err(CmdError::Params(TEAMS_NOTHING_LEFT.into()));
        }
        self.teams_queue(chat, OutKind::Edit { message: message.to_string(), created }, body, mentions)
    }

    /// [`teams::Job::Op`] of [`Op::Delete`]: the open chat's message `message` deleted, shown at
    /// once. Only the user's own message, not a system line, not deleted, and not being deleted
    /// already. Its local id is returned.
    pub fn teams_delete(&mut self, chat: &str, message: &str) -> Result<String, CmdError> {
        let t = &self.teams;
        let m = t.shown_message(chat, message)?;
        if !own(m, &self.prefs.teams.object_id) || m.kind != MessageKind::Message || m.deleted.is_some() || t.deleting(message) {
            return Err(CmdError::Disabled("only one's own message can be deleted, once".into()));
        }
        let created = m.created;
        self.teams_queue(chat, OutKind::Delete { message: message.to_string(), created }, ChatText::default(), Vec::new())
    }

    /// [`teams::Job::Op`] of [`Op::React`]: the user's reaction `emoji` on the open chat's
    /// message `message` set, or taken back when it's theirs now (as the chips show it,
    /// [`TeamsState::shown_reactions`]), shown at once. A reaction is an emoji ([`reactable`]), one
    /// of the six quick ones or one the message shows; a message deleted, unshowable or being
    /// deleted takes none. Its local id is returned, and whether it's set.
    pub fn teams_react(&mut self, chat: &str, message: &str, emoji: &str) -> Result<(String, bool), CmdError> {
        let t = &self.teams;
        let m = t.shown_message(chat, message)?;
        if m.deleted.is_some() || m.unshowable || t.deleting(message) {
            return Err(CmdError::Disabled("this message takes no reaction".into()));
        }
        // Checked before it's compared with anything: an emoji is short.
        if !reactable(emoji) {
            return Err(CmdError::Disabled("a reaction is an emoji".into()));
        }
        let shown = t.shown_reactions(m, &self.prefs.teams.object_id);
        let chip = shown.iter().find(|r| same_reaction(r.emoji, emoji));
        if chip.is_none() && !QUICK_REACTIONS.contains(&emoji) {
            return Err(CmdError::Disabled("only a quick reaction, or one the message shows, can be added".into()));
        }
        let on = !chip.is_some_and(|r| r.mine);
        let created = m.created;
        let what = OutKind::React { message: message.to_string(), created, emoji: emoji.to_string(), on };
        let local = self.teams_queue(chat, what, ChatText::default(), Vec::new())?;
        Ok((local, on))
    }
}
