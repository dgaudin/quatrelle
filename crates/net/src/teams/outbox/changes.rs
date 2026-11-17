// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The outbox's changes to messages already there: an edit, a deletion, a reaction set or taken
//! back. Each is a record of the outbox as a send is: kept in the copy, in its chat's order with
//! the sends, on the chat's send route ([`Route::Send`]), counted in the caps.
//!
//! * A change is taken when it can go: its chat in the copy and not left, an edit's HTML within
//!   [`SEND_MAX`](quatrelle_model::pim::teams::SEND_MAX) and its text not empty, a reaction an
//!   emoji ([`reactable`]), and under the caps once merged with what waits of its message: a
//!   reaction taken back before it went cancels it (neither goes), the same reaction twice is
//!   kept once, a later edit replaces one waiting, a deletion drops the edits and reactions
//!   waiting and a second deletion isn't kept. Else it is answered "Not sent" (`seq` 0).
//! * It is made when its turn comes, written "Sending" first. Graph's 204 is done, and only it:
//!   the record goes, and the change is written in the copy at Graph's time and said (an edit's
//!   words replace the old ones in the index, a deletion's leave it); the next read brings Graph's
//!   own version. A change is the same however often it's made: answered 5xx or left "Sending" by
//!   a run cut off, it goes again, [`POSTS`] times at most, then it is "Not sent" (couldn't be
//!   confirmed). Unanswered (the network away, the connection cut), it is held back a while
//!   (`held.rs`) and the worker goes on with the rest: that is a try only once Graph answered
//!   another request since (the network works, that request alone is cut), else it waits for the
//!   network as a send does. Refused (a deletion's 404 too: Graph answers so for a chat it no
//!   longer serves the user, while the others still read the message), it is "Not sent" with
//!   Teams' code, nothing written in the copy, made again only when the user asks; a read showing
//!   the message deleted lets go of it ([`gone`]).
//! * A reaction is never kept "Not sent": refused, unconfirmed or of a chat left, it leaves the
//!   outbox and is said once (a `seq` 0 item), and Teams' own state stands.
//! * A message a read shows deleted takes the changes to it still to make with it ([`gone`]).
//!
//! What is hostile here: the engine's changes (their ids checked before anything is kept), Graph's
//! answers, and the outbox read back. What it protects: the user's changes (made once their turn
//! comes, never made to a message Teams deleted, a reaction never left half done) and Teams'
//! limits (as the sends'). The log says the kind, counts, keyed hashes and HTTP statuses: never a
//! text, an id or an emoji.

use quatrelle_model::pim::teams::{Mention, Reaction, reactable, same_reaction};

use super::*;

/// What goes out, as the log names it with its chat: "a message to", "an edit in", …
pub(super) fn named(what: &OutKind) -> &'static str {
    match what {
        OutKind::Send => "a message to",
        OutKind::Edit { .. } => "an edit in",
        OutKind::Delete { .. } => "a deletion in",
        OutKind::React { .. } => "a reaction in",
    }
}

/// The message a change is to, and its creation time; none for a send.
fn target(what: &OutKind) -> Option<(&str, i64)> {
    match what {
        OutKind::Send => None,
        OutKind::Edit { message, created } | OutKind::Delete { message, created } | OutKind::React { message, created, .. } => {
            Some((message, *created))
        }
    }
}

/// A change to a message: kept as the next of the outbox, merged with what waits of its message,
/// when it can go; else answered "Not sent" with nothing kept. Either way the chat's outbox is
/// said, naming it. Ids the engine never makes are dropped unanswered.
pub(super) fn take(w: &mut Worker, chat: String, local: String, what: OutKind, body: ChatText, mentions: Vec<Mention>) {
    if !id_ok(&chat) || !local_ok(&local) || !what.message().is_some_and(id_ok) {
        return log::warn!("teams: a change to a message without usable ids was dropped");
    }
    let op = OpRecord { seq: 0, local, chat, body, mentions, what, state: OpState::Waiting, tries: 0, after: None, parts: Vec::new() };
    // Switched off or signed out meanwhile: nothing can keep it.
    let Some(copy) = &w.copy else { return refuse(w, &[], op, Unsent::Refused) };
    let mut ops = copy.ops();
    let runs: usize = op.body.blocks.iter().map(|b| b.runs.len()).sum();
    let why = match &op.what {
        _ if copy.chat(&op.chat).is_none_or(|r| r.chat.left) => Some(Unsent::Left),
        OutKind::Edit { .. } if runs > TEAMS_RUNS || teams_write_mentions(&op.body, &op.mentions).is_err() => Some(Unsent::TooLong),
        OutKind::Edit { .. } if op.body.plain().is_empty() => Some(Unsent::Refused),
        OutKind::React { emoji, .. } if !reactable(emoji) => Some(Unsent::Refused),
        _ => None,
    };
    if let Some(why) = why {
        return refuse(w, &ops, op, why);
    }
    // The caps count what the outbox will hold: a change that only cancels one waiting, or
    // takes the place of others, is taken however full it is.
    let (drops, keep) = merged(&ops, &op);
    let ours = ops.iter().filter(|o| o.chat == op.chat).count().saturating_sub(drops.len());
    let full = copy.op_count().is_none_or(|n| n.saturating_sub(drops.len()) >= OUTBOX_MAX) || ours >= OUTBOX_CHAT_MAX;
    if keep && full {
        return refuse(w, &ops, op, Unsent::TooMany);
    }
    let (chat, local, name) = (op.chat.clone(), op.local.clone(), named(&op.what));
    // Kept first, then what it replaces let go: a failure leaves a change more, never one less.
    if keep {
        let op = OpRecord { seq: copy.next_seq(), ..op };
        if let Err(e) = copy.put_op(&op) {
            log::error!("teams: {name} conversation {} couldn't be kept: {}", w.hashed(&chat), kind(&NetError::Store(e)));
            return refuse(w, &ops, op, Unsent::Refused);
        }
        ops.push(op);
    }
    for seq in &drops {
        match copy.remove_op(*seq) {
            Ok(()) => ops.retain(|o| o.seq != *seq),
            Err(e) => log::error!("teams: a change waiting couldn't be let go: {}", kind(&NetError::Store(e))),
        }
    }
    let then = match (keep, drops.len()) {
        (true, 0) => "taken".to_string(),
        (true, n) => format!("taken in the place of {n} waiting"),
        (false, 0) => "waiting already".to_string(),
        (false, _) => "cancelling the one waiting".to_string(),
    };
    log::info!("teams: {name} conversation {} {then}; {} in the outbox", w.hashed(&chat), ops.len());
    said(w, &ops, &chat, Some(local), Vec::new());
}

/// What taking `new` does to the changes waiting of its message (in its chat, "Waiting" only):
/// the records it lets go, and whether it is kept itself. A reaction is weighed against the last
/// of its kind waiting: two can wait (the first was being made when the second came, then went
/// back to waiting), and the last says where the chip stands.
fn merged(ops: &[OpRecord], new: &OpRecord) -> (Vec<u64>, bool) {
    let Some(message) = new.what.message() else { return (Vec::new(), true) };
    let waiting: Vec<&OpRecord> =
        ops.iter().filter(|o| o.chat == new.chat && o.state == OpState::Waiting && o.what.message() == Some(message)).collect();
    let seqs = |keep: fn(&OutKind) -> bool| waiting.iter().filter(|o| keep(&o.what)).map(|o| o.seq).collect::<Vec<u64>>();
    match &new.what {
        OutKind::React { emoji, on, .. } => {
            let same = waiting.iter().rev().find_map(|o| match &o.what {
                OutKind::React { emoji: e, on: was, .. } if same_reaction(e, emoji) => Some((o.seq, *was)),
                _ => None,
            });
            match same {
                // Taken back before it went (or set back): neither goes.
                Some((seq, was)) if was != *on => (vec![seq], false),
                // The same again: once is enough.
                Some(_) => (Vec::new(), false),
                None => (Vec::new(), true),
            }
        }
        OutKind::Edit { .. } => (seqs(|k| matches!(k, OutKind::Edit { .. })), true),
        OutKind::Delete { .. } if !seqs(|k| matches!(k, OutKind::Delete { .. })).is_empty() => (Vec::new(), false),
        OutKind::Delete { .. } => (seqs(|k| matches!(k, OutKind::Edit { .. } | OutKind::React { .. })), true),
        OutKind::Send => (Vec::new(), true),
    }
}

/// A change made, its record written "Sending" already, on its chat's send route: done on
/// Graph's 204; waiting again when Graph didn't take it (the sign-in refused, a 429, waited out
/// whole); "Not sent" when refused (any other 4xx, a deletion's 404 among them; 413: too long);
/// made again when an answer didn't say (a 5xx: [`POSTS`] tries in all), after the worker's pause
/// or Graph's `Retry-After`, then "Not sent" (couldn't be confirmed). No answer at all: held back
/// ([`Held::unanswered`]); a try as a 5xx is once Graph answered another request since, with no
/// pause (the network answers), else waiting for the network, its tries as they were, after the
/// worker's pause. A reaction not made leaves the outbox.
pub(super) fn make(w: &mut Worker, head: &Head, mut op: OpRecord, now: Instant) {
    let (route, chat, me) = (Route::Send(head.key), head.chat.clone(), w.settings.object_id.clone());
    let made = match &op.what {
        OutKind::Edit { message, .. } => {
            let Ok(html) = teams_write_mentions(&op.body, &op.mentions) else {
                op.state = OpState::NotSent { reason: Unsent::TooLong, code: None };
                return settle(w, &op);
            };
            w.request(route, now, |c| c.edit_html(&chat, message, &html, &op.mentions))
        }
        OutKind::Delete { message, .. } => w.request(route, now, |c| c.soft_delete(&chat, message, Some(me.as_str()))),
        OutKind::React { message, emoji, on: true, .. } => w.request(route, now, |c| c.set_reaction(&chat, message, emoji)),
        OutKind::React { message, emoji, on: false, .. } => w.request(route, now, |c| c.unset_reaction(&chat, message, emoji)),
        // Sends are posted (`post`), never made here.
        OutKind::Send => return,
    };
    let e = match made {
        Ok(()) => {
            w.outbox.held.forget(op.seq);
            return done(w, &op);
        }
        Err(e) => e,
    };
    // Unanswered, or answered 5xx: it may have been made, and is made again (the same change).
    // With no answer at all nothing was said of it: a try only once Graph answered another request
    // since (the network works), else it waits for the network. Any answer forgets that.
    let state = after_post(&e);
    let again = state == OpState::Unknown;
    let unanswered = again && of_the_network(&e);
    let counts = unanswered && w.outbox.held.unanswered(op.seq, w.answered, now);
    if !unanswered {
        w.outbox.held.forget(op.seq);
    }
    op.state = match state {
        OpState::Unknown if unanswered && !counts => OpState::Waiting,
        OpState::Unknown => {
            op.tries = op.tries.saturating_add(1);
            if op.tries < POSTS { OpState::Waiting } else { OpState::NotSent { reason: Unsent::Unconfirmed, code: None } }
        }
        state => state,
    };
    let then = match &op.state {
        OpState::NotSent { .. } if matches!(op.what, OutKind::React { .. }) => "let go",
        OpState::NotSent { .. } => "not made",
        _ if counts => "a try, the network answering: made again after a longer wait",
        _ if unanswered => "made again once the network answers",
        _ if again => "made again after a wait",
        _ => "it waits",
    };
    log::warn!("teams: {} conversation {} failed ({}); {then}", named(&op.what), w.hashed(&chat), kind(&e));
    match op.state.clone() {
        OpState::NotSent { reason, code } if matches!(op.what, OutKind::React { .. }) => let_go(w, op, reason, code),
        _ => settle(w, &op),
    }
    if busy(&e) {
        w.busy_wait(now);
    } else if again && !counts {
        w.pause(&e, now);
    }
}

/// Teams made the change `op` is: its record goes; the change is written in the copy at Graph's
/// time (the message as Graph will list it, its next read bringing Graph's own version) and the
/// message said; the outbox is said. A message the copy doesn't hold, or no time from Graph:
/// nothing written, the next read brings it.
fn done(w: &mut Worker, op: &OpRecord) {
    let (at, me) = (server_ms(w), w.settings.object_id.clone());
    let (mut ops, mut written) = (Vec::new(), Vec::new());
    if let Some(copy) = &w.copy {
        if let Err(e) = copy.remove_op(op.seq) {
            log::error!("teams: a change made couldn't leave the outbox: {}", kind(&NetError::Store(e)));
        }
        ops = copy.ops();
        let message = target(&op.what).and_then(|(id, created)| copy.message_at(&op.chat, id, created));
        if let (Some(m), Some(at)) = (message, at) {
            match copy.upsert(&op.chat, &[applied(m, op, at, &me)]) {
                Ok(changed) => written = changed,
                Err(e) => log::warn!("teams: a change made couldn't be kept in the copy: {}", kind(&NetError::Store(e))),
            }
        }
    }
    say_messages(w, &op.chat, &written, false, false);
    log::info!("teams: {} conversation {} was made", named(&op.what), w.hashed(&op.chat));
    said(w, &ops, &op.chat, None, Vec::new());
}

/// `m` as the change `op` leaves it, made at `at` (Graph's time) by the user `me`: an edit's body,
/// mentions and times; a deletion's time (the copy keeps nothing more of it); the user's reaction
/// added or taken back (a kind no one has left goes).
fn applied(mut m: ChatMessage, op: &OpRecord, at: i64, me: &str) -> ChatMessage {
    m.modified = at;
    match &op.what {
        OutKind::Edit { .. } => {
            m.body = op.body.clone();
            m.mentions = op.mentions.clone();
            m.edited = Some(at);
        }
        OutKind::Delete { .. } => m.deleted = Some(at),
        OutKind::React { emoji, on, .. } if !me.is_empty() => {
            let mine = |u: &String| u.eq_ignore_ascii_case(me);
            let kind = m.reactions.iter().position(|r| same_reaction(&r.emoji, emoji));
            match (kind.and_then(|i| m.reactions.get_mut(i)), on) {
                (Some(r), true) if !r.users.iter().any(mine) => r.users.push(me.to_string()),
                (Some(_), true) => {}
                (None, true) => m.reactions.push(Reaction { emoji: emoji.clone(), users: vec![me.to_string()] }),
                (_, false) => {
                    for r in m.reactions.iter_mut().filter(|r| same_reaction(&r.emoji, emoji)) {
                        r.users.retain(|u| !mine(u));
                    }
                    m.reactions.retain(|r| !(same_reaction(&r.emoji, emoji) && r.users.is_empty()));
                }
            }
        }
        OutKind::React { .. } | OutKind::Send => {}
    }
    m
}

/// A reaction not made: it leaves the outbox and is said once, "Not sent" for `reason` (`seq` 0),
/// after its chat's others.
fn let_go(w: &mut Worker, op: OpRecord, reason: Unsent, code: Option<String>) {
    let mut ops = Vec::new();
    if let Some(copy) = &w.copy {
        if let Err(e) = copy.remove_op(op.seq) {
            log::error!("teams: a reaction not made couldn't leave the outbox: {}", kind(&NetError::Store(e)));
        }
        ops = copy.ops();
    }
    let chat = op.chat.clone();
    let item = OpRecord { seq: 0, state: OpState::NotSent { reason, code }, ..op }.outgoing();
    said(w, &ops, &chat, None, vec![item]);
}

/// The copy opened: a change left "Sending" by a run cut off goes again, its try counted (the
/// [`POSTS`]th made it "Not sent", couldn't be confirmed); a reaction past its tries leaves the
/// outbox. What left, to be said.
pub(super) fn opened(w: &Worker, ops: &mut Vec<OpRecord>) -> Vec<Outgoing> {
    let Some(copy) = &w.copy else { return Vec::new() };
    let (mut again, mut gone) = (0usize, Vec::new());
    ops.retain_mut(|op| {
        if op.what == OutKind::Send || op.state != OpState::Sending {
            return true;
        }
        op.tries = op.tries.saturating_add(1);
        let last = op.tries >= POSTS;
        if last && matches!(op.what, OutKind::React { .. }) {
            if let Err(e) = copy.remove_op(op.seq) {
                log::error!("teams: a reaction not made couldn't leave the outbox: {}", kind(&NetError::Store(e)));
                return true;
            }
            let not_made = OpState::NotSent { reason: Unsent::Unconfirmed, code: None };
            gone.push(OpRecord { seq: 0, state: not_made, ..op.clone() }.outgoing());
            return false;
        }
        op.state = if last { OpState::NotSent { reason: Unsent::Unconfirmed, code: None } } else { OpState::Waiting };
        match copy.put_op(op) {
            Ok(()) => again = again.saturating_add(1),
            Err(e) => log::error!("teams: a change's state couldn't be kept: {}", kind(&NetError::Store(e))),
        }
        true
    });
    if again > 0 || !gone.is_empty() {
        log::info!(
            "teams: {} changes were being made when the last run stopped: {again} go again or aren't made, {} reactions are let go",
            again.saturating_add(gone.len()),
            gone.len()
        );
    }
    gone
}

/// Messages of `chat` a read shows `deleted`: the changes to them still in the outbox, in any
/// state but "Sending" (being made), go with them, and the chat's outbox is said when one did.
pub(crate) fn gone(w: &mut Worker, chat: &str, deleted: &[&str]) {
    let Some(copy) = &w.copy else { return };
    if deleted.is_empty() || copy.op_count() == Some(0) {
        return;
    }
    let mut let_go = 0usize;
    for op in copy.ops() {
        if op.chat != chat || op.state == OpState::Sending || !op.what.message().is_some_and(|m| deleted.contains(&m)) {
            continue;
        }
        match copy.remove_op(op.seq) {
            Ok(()) => let_go = let_go.saturating_add(1),
            Err(e) => log::error!("teams: a change to a message deleted couldn't be let go: {}", kind(&NetError::Store(e))),
        }
    }
    if let_go > 0 {
        log::info!("teams: {let_go} changes to messages deleted in conversation {} were let go", w.hashed(chat));
        changed(w, chat);
    }
}
