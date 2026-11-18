// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What the worker says of new messages for the engine to announce ([`News`]): it alone holds
//! every chat's members, so it says who sent each one as they name them, whether that sender is
//! External or an application, whether the message names the user, and the chat's name, kind and
//! hidden mark. Whether and how it's announced is the engine's.
//!
//! * A message is news when an incremental read past the chat's own start brings it (never the
//!   first copy nor its history), new to the copy (an edit of, a reaction to or a deletion of one
//!   it held never is, its newest one too, whose time the watermark stands on), made after that
//!   read began (less the overlap of 2 s), someone else's (never the user's, whether a read or the
//!   outbox's check reads it back), a message rather than a system line, and neither deleted nor
//!   unshowable.
//! * A chat Graph won't serve: its new last message as the list gives it, once the copy's first
//!   listing is over and when its time passes the copy's (its id unknown, so empty). A sender the
//!   members don't have waits for them to be read again, as a read's messages do.
//!
//! Nothing is said until the user's own id is known, as the rows wait for it: without it the user
//! would count among a chat's others (its name), and their own messages among the news.
//!
//! What is hostile here: names and texts (on one line, without control or bidi characters, cut:
//! [`one_line`], [`first_line`]), and who a sender says they are. What it protects: a sender is
//! named by the members by id (the message's name only for one they don't have), External unless
//! the members prove them of the organisation ([`sender_internal`]: one they don't have is External,
//! whatever the message says), and an application is never a person.

use super::reads::theirs;
use super::*;
use quatrelle_model::pim::teams::{
    ChatKind, ChatMessage, EVENT_ITEMS, Event, MessageKind, NEWS_CHARS, News, first_line, one_line, row_name, sender_internal,
};

/// The messages a read of `chat` brought that are new to the copy (`since`: where the read began,
/// less the overlap): those that are news said, in events of at most [`EVENT_ITEMS`].
pub(crate) fn say_news(w: &Worker, chat: &str, messages: &[ChatMessage], since: i64) {
    let me = w.settings.object_id.as_str();
    let mut fresh = messages.iter().filter(|m| is_news(m, me, since)).peekable();
    if me.is_empty() || fresh.peek().is_none() {
        return;
    }
    let Some(r) = w.copy.as_ref().and_then(|c| c.chat(chat)) else { return };
    let tenant = w.settings.tenant_id.as_str();
    let news = fresh
        .map(|m| News {
            message: m.id.clone(),
            mentions_me: names_me(m, me),
            text: first_line(&m.body.plain(), NEWS_CHARS),
            at: m.created,
            ..about(&r.chat, &m.sender, me, tenant)
        })
        .collect();
    say(w, news);
}

/// Whether a message a read brought is news: made after `since`, someone else's than the user's
/// (`me`), a message, neither deleted nor unshowable.
fn is_news(m: &ChatMessage, me: &str, since: i64) -> bool {
    m.created > since && m.kind == MessageKind::Message && m.deleted.is_none() && !m.unshowable && theirs(&m.sender, me)
}

/// Whether a message names the user (`me`): a mention of their id, ASCII case aside.
fn names_me(m: &ChatMessage, me: &str) -> bool {
    !me.is_empty() && m.mentions.iter().any(|x| x.user_id.as_deref().is_some_and(|id| id.eq_ignore_ascii_case(me)))
}

/// What an announcement of a message of `c` from `sender` says besides the message itself: who,
/// as the members name them (else as the message does); External unless proven of the
/// organisation `tenant`; App for an application; the chat's name (none for a one-to-one chat),
/// kind and hidden mark.
fn about(c: &Chat, sender: &Sender, me: &str, tenant: &str) -> News {
    let (name, external, app) = match sender {
        Sender::User { id, name, .. } => {
            let own = c.members.iter().find(|m| m.user_id.eq_ignore_ascii_case(id)).map(|m| one_line(&m.name)).filter(|n| !n.is_empty());
            (own.unwrap_or_else(|| one_line(name)), !sender_internal(sender, &c.members, tenant), false)
        }
        Sender::App { name, .. } => (one_line(name), false, true),
        Sender::Nobody => (String::new(), true, false),
    };
    let direct = c.kind == ChatKind::OneOnOne;
    News {
        chat: c.id.clone(),
        message: String::new(),
        sender: name,
        external,
        app,
        mentions_me: false,
        chat_name: (!direct).then(|| row_name(c, me)),
        direct,
        hidden: c.hidden,
        text: String::new(),
        at: 0,
    }
}

/// Whether a chat Graph won't serve has a new last message to announce, as the list gives it
/// (`listed`, over the copy's `stored`): someone else's, later than the copy's last, once the
/// copy's first listing is over.
pub(super) fn new_preview(w: &Worker, stored: &ChatRecord, listed: &Chat) -> bool {
    let Some(p) = &listed.preview else { return false };
    let me = w.settings.object_id.as_str();
    let was = stored.chat.preview.as_ref().map(|p| p.at);
    stored.chat.unreadable.is_some()
        && !me.is_empty()
        && theirs(&p.sender, me)
        && was.is_none_or(|was| p.at > was)
        && w.copy.as_ref().is_some_and(|c| c.listed())
}

/// A chat's last message as the list gives it, as an announcement says it (its id isn't known).
pub(super) fn of_preview(w: &Worker, c: &Chat) -> Option<News> {
    let p = c.preview.as_ref()?;
    let (me, tenant) = (w.settings.object_id.as_str(), w.settings.tenant_id.as_str());
    Some(News { text: first_line(&p.text, NEWS_CHARS), at: p.at, ..about(c, &p.sender, me, tenant) })
}

/// A chat's work is past its members: when its new last message waited for them (a chat Graph
/// won't serve, a sender they didn't have), it is said now, its sender judged by the members as
/// the copy holds them (read again, or as they were should that have failed).
pub(super) fn after_members(w: &Worker, task: &Task) {
    if !task.preview {
        return;
    }
    let news = w.copy.as_ref().and_then(|c| c.chat(&task.chat)).and_then(|r| of_preview(w, &r.chat));
    say(w, news.into_iter().collect());
}

/// News said, in events of at most [`EVENT_ITEMS`].
pub(super) fn say(w: &Worker, news: Vec<News>) {
    for part in news.chunks(EVENT_ITEMS) {
        w.say(Event::News { news: part.to_vec() });
    }
}
