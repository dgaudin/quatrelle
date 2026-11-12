// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What the engine is sent: a chat's row of the list when it changed, and messages, each event
//! carrying at most [`EVENT_ITEMS`]. Rows wait until the user's own id is known: without it, the
//! user would count among a chat's others (its name, its External mark).
//!
//! A row says who as the chat's members do: its name ([`row_name`]), "External" when a member
//! isn't proven of the organisation, and its last line's sender judged by
//! [`sender_internal`] (a sender the members don't have was looked for among them before).

use super::reads::theirs;
use super::*;
use quatrelle_model::pim::teams::{ChatKind, ChatMessage, ChatRow, EVENT_ITEMS, Event, is_internal, one_line, row_name, sender_internal};
use sha2::{Digest, Sha256};

/// A chat whose row may have changed: sent at the end of the pass, or once [`EVENT_ITEMS`] wait.
pub(super) fn dirty(w: &mut Worker, chat: &str) {
    if !id_ok(chat) {
        return;
    }
    if w.sync.dirtied.insert(chat.to_string()) {
        w.sync.dirty.push(chat.to_string());
    }
    if w.sync.dirty.len() >= EVENT_ITEMS {
        flush(w);
    }
}

/// The rows waiting that changed since last sent, in events of at most [`EVENT_ITEMS`]. Nothing
/// until the user's own id is known.
pub(super) fn flush(w: &mut Worker) {
    let (me, tenant) = (w.settings.object_id.clone(), w.settings.tenant_id.clone());
    if me.is_empty() || w.sync.dirty.is_empty() {
        return;
    }
    let chats = std::mem::take(&mut w.sync.dirty);
    w.sync.dirtied.clear();
    let mut rows = Vec::new();
    for chat in chats {
        let Some(r) = w.copy.as_ref().and_then(|c| c.chat(&chat)) else { continue };
        let row = row_of(&r, &me, &tenant);
        let (key, sum) = (w.key_of(&chat), row_hash(&row));
        let shown = &mut w.sync.shown;
        if shown.get(&key) == Some(&sum) {
            continue;
        }
        if shown.len() < MAX_CHATS || shown.contains_key(&key) {
            shown.insert(key, sum);
        }
        rows.push(row);
    }
    for part in rows.chunks(EVENT_ITEMS) {
        w.say(Event::Chats { rows: part.to_vec() });
    }
}

/// A chat's messages sent, in creation order as given: `fresh`, news to announce; `older`, a page
/// before the window.
pub(super) fn say_messages(w: &Worker, chat: &str, messages: &[ChatMessage], fresh: bool, older: bool) {
    for part in messages.chunks(EVENT_ITEMS) {
        w.say(Event::Messages { chat: chat.to_string(), messages: part.to_vec(), fresh, older });
    }
}

/// What the list shows of a chat, `me` being the user and `tenant` their organisation. Its unread
/// count is the copy's; when the copy has none of them, its preview says whether it's unread (a
/// chat Graph doesn't serve has only that).
fn row_of(r: &ChatRecord, me: &str, tenant: &str) -> ChatRow {
    let c = &r.chat;
    let others = || c.members.iter().filter(|m| !m.user_id.eq_ignore_ascii_case(me));
    let preview_unread = c.preview.as_ref().is_some_and(|p| theirs(&p.sender, me) && c.last_read.is_none_or(|t| p.at > t));
    let unread = if r.unread.is_empty() { u32::from(preview_unread) } else { u32::try_from(r.unread.len()).unwrap_or(u32::MAX) };
    ChatRow {
        id: c.id.clone(),
        name: row_name(c, me),
        kind: c.kind.clone(),
        last_line: last_line(c, me),
        last_external: c.preview.as_ref().is_some_and(|p| !matches!(p.sender, Sender::Nobody) && !sender_internal(&p.sender, &c.members, tenant)),
        at: c.last_activity,
        unread,
        hidden: c.hidden,
        left: c.left,
        unreadable: c.unreadable.is_some(),
        external: others().any(|m| !is_internal(m, tenant)),
        members: u32::try_from(c.members.len()).unwrap_or(u32::MAX),
        person: if c.kind == ChatKind::OneOnOne { others().next().map(|m| m.user_id.clone()) } else { None },
    }
}

/// A chat's last line: its last message's text, after its sender's first name in a chat of more
/// than two ("Paul: it's fixed"), the name a member has when the sender is one. On one line.
fn last_line(c: &Chat, me: &str) -> String {
    let Some(p) = &c.preview else { return String::new() };
    let who = match &p.sender {
        _ if c.kind == ChatKind::OneOnOne => None,
        Sender::User { id, name, .. } if !id.eq_ignore_ascii_case(me) => {
            Some(c.members.iter().find(|m| m.user_id == *id).map_or(name.as_str(), |m| m.name.as_str()))
        }
        Sender::App { name, .. } => Some(name.as_str()),
        _ => None,
    };
    let first = who.and_then(|n| one_line(n).split(' ').next().map(str::to_string)).filter(|n| !n.is_empty());
    match first {
        Some(first) => one_line(&format!("{first}: {}", p.text)),
        None => p.text.clone(),
    }
}

/// A row's fingerprint: what tells the engine has it already.
fn row_hash(row: &ChatRow) -> u64 {
    let sum = Sha256::digest(serde_json::to_vec(row).unwrap_or_default());
    sum.iter().take(8).fold(0, |n, b| (n << 8) | u64::from(*b))
}
