// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A chat's messages in the copy: pages of at most [`PAGE`] (`msgs/<hash>/<n>.json`), in creation
//! order, newest page last. The chat's record keeps the first creation time of each page, so a
//! message edited, reacted to or deleted long after finds its page by its creation time, and no
//! map of messages is kept. A page grown past [`PAGE`] splits: one that grew at its end (new
//! messages) into full pages from its start, one that grew at its start (the history, read
//! backwards) into full pages from its end, any other in two halves.
//!
//! What comes in, and what's read back, is held to the model's caps: lists cut, a body to
//! [`MAX_BODY`] bytes as kept (its runs cut until it fits), and nothing of a deleted message but
//! its id, times and sender.

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use quatrelle_model::pim::teams::{
    Attachment, Block, ChatMessage, ChatText, MAX_ATTACHMENTS, MAX_BODY, MAX_MENTIONS, MAX_REACTION_USERS, MAX_REACTIONS, WINDOW_MAX, one_line,
};
use serde::de::{IgnoredAny, SeqAccess, Visitor};
use serde::{Deserializer, Serialize};

use super::*;

/// The most messages in a page.
pub(crate) const PAGE: usize = 100;
/// The most bytes a page of more than one message is written in: past it, it splits as a full one
/// does (reactions and bodies at their caps would make one too big to read back).
const PAGE_BYTES: usize = 8 << 20;
/// The most pages read at once: a window of the engine's.
const WINDOW_PAGES: usize = WINDOW_MAX / PAGE;
/// The number of a page not written yet: chosen when the pages settle.
const NEW_PAGE: u32 = u32::MAX;

/// A page as it's kept: the chat it's of, and its messages.
#[derive(Serialize)]
struct PageOut<'a> {
    chat: &'a str,
    messages: &'a [ChatMessage],
}

/// A page as it's read back: at most [`PAGE`] messages are taken, the rest passed over unread.
#[derive(serde::Deserialize)]
struct PageIn {
    chat: String,
    #[serde(deserialize_with = "at_most_a_page")]
    messages: Vec<ChatMessage>,
}

fn at_most_a_page<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Vec<ChatMessage>, D::Error> {
    struct Bounded;
    impl<'de> Visitor<'de> for Bounded {
        type Value = Vec<ChatMessage>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a page of messages")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Vec<ChatMessage>, A::Error> {
            let mut out = Vec::new();
            while out.len() < PAGE {
                match seq.next_element()? {
                    Some(m) => out.push(m),
                    None => return Ok(out),
                }
            }
            while seq.next_element::<IgnoredAny>()?.is_some() {}
            Ok(out)
        }
    }
    d.deserialize_seq(Bounded)
}

/// A message held to the caps; `false` when it can't be kept at all (no usable id).
fn held(m: &mut ChatMessage) -> bool {
    if !id_ok(&m.id) {
        return false;
    }
    if m.deleted.is_some() {
        *m = ChatMessage {
            id: std::mem::take(&mut m.id),
            chat: std::mem::take(&mut m.chat),
            sender: std::mem::take(&mut m.sender),
            created: m.created,
            modified: m.modified,
            edited: m.edited,
            deleted: m.deleted,
            ..ChatMessage::default()
        };
        return true;
    }
    hold_sender(&mut m.sender);
    // As Graph's reader keeps them: ids usable, texts on one line.
    m.attachments.retain(|a| match a {
        Attachment::File { id, .. } | Attachment::Hosted { id } => id_ok(id),
        Attachment::Card => true,
    });
    m.attachments.truncate(MAX_ATTACHMENTS);
    m.mentions.truncate(MAX_MENTIONS);
    for x in &mut m.mentions {
        x.text = one_line(&x.text);
        x.user_id = x.user_id.take().filter(|u| id_ok(u));
    }
    m.reactions.truncate(MAX_REACTIONS);
    for r in &mut m.reactions {
        r.users.retain(|u| id_ok(u));
        r.users.truncate(MAX_REACTION_USERS);
    }
    for run in m.body.blocks.iter_mut().flat_map(|b| &mut b.runs) {
        run.hosted = run.hosted.take().filter(|h| id_ok(h));
    }
    fit_body(&mut m.body, MAX_BODY);
    true
}

/// Whether a message read back is within the caps (`Err` says which it breaks): the fuzz target's
/// check.
pub(super) fn within_caps(m: &ChatMessage, chat: &str) -> std::result::Result<(), String> {
    let lists = m.attachments.len() <= MAX_ATTACHMENTS
        && m.mentions.len() <= MAX_MENTIONS
        && m.reactions.len() <= MAX_REACTIONS
        && m.reactions.iter().all(|r| r.users.len() <= MAX_REACTION_USERS && r.users.iter().all(|u| id_ok(u)));
    let ids = m.attachments.iter().all(|a| match a {
        Attachment::File { id, .. } | Attachment::Hosted { id } => id_ok(id),
        Attachment::Card => true,
    }) && m.mentions.iter().all(|x| x.user_id.as_deref().is_none_or(id_ok) && one_line(&x.text) == x.text)
        && m.body.blocks.iter().flat_map(|b| &b.runs).all(|r| r.hosted.as_deref().is_none_or(id_ok))
        && sender_ok(&m.sender);
    let bare = m.deleted.is_none() || (m.body.blocks.is_empty() && m.attachments.is_empty() && m.mentions.is_empty() && m.reactions.is_empty());
    match () {
        _ if !id_ok(&m.id) => Err("a message without a usable id".into()),
        _ if m.chat != chat => Err("a message of another chat".into()),
        _ if !lists => Err("a message's lists past their caps".into()),
        _ if !ids => Err("a message with an unusable id or a name not on one line".into()),
        _ if json_len(&m.body) > MAX_BODY => Err(format!("a body of {} bytes", json_len(&m.body))),
        _ if !bare => Err("a deleted message keeps more than its id, times and sender".into()),
        _ => Ok(()),
    }
}

/// Bytes `v` takes as JSON.
fn json_len<T: Serialize + ?Sized>(v: &T) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(b.len());
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut c = Count(0);
    match serde_json::to_writer(&mut c, v) {
        Ok(()) => c.0,
        Err(_) => usize::MAX,
    }
}

/// A body cut to `max` bytes as kept (JSON): whole runs while they fit, then as much of the next
/// one's text as does, then nothing more. A run's JSON is a part of its block's, and a block's of
/// the text's, each after a comma, so what is counted is what is written.
fn fit_body(body: &mut ChatText, max: usize) {
    if json_len(body) <= max {
        return;
    }
    let mut used = json_len(&ChatText::default());
    let mut kept: Vec<Block> = Vec::new();
    'blocks: for block in std::mem::take(&mut body.blocks) {
        let shell = Block { kind: block.kind, runs: Vec::new() };
        let cost = json_len(&shell).saturating_add(usize::from(!kept.is_empty()));
        if used.saturating_add(cost) > max {
            break;
        }
        used = used.saturating_add(cost);
        kept.push(shell);
        let Some(into) = kept.last_mut() else { break };
        for run in block.runs {
            let comma = usize::from(!into.runs.is_empty());
            let cost = json_len(&run).saturating_add(comma);
            if used.saturating_add(cost) <= max {
                used = used.saturating_add(cost);
                into.runs.push(run);
                continue;
            }
            // As much of its text as fits, counted in one pass as JSON writes it.
            let mut cut = run;
            let text = std::mem::take(&mut cut.text);
            let room = max.saturating_sub(used).saturating_sub(comma).saturating_sub(json_len(&cut));
            let end = fitting_prefix(&text, room);
            if end > 0 {
                cut.text = text.get(..end).unwrap_or_default().to_string();
                into.runs.push(cut);
            }
            break 'blocks;
        }
    }
    body.blocks = kept;
    // What was counted is what is written; should it not be, the last runs go until it fits.
    while json_len(body) > max {
        match body.blocks.last_mut() {
            Some(b) if !b.runs.is_empty() => drop(b.runs.pop()),
            Some(_) => drop(body.blocks.pop()),
            None => break,
        }
    }
}

/// How many bytes of `text` (ending where a character does) fit in `room` bytes once written in
/// a JSON string, as serde_json escapes it: a quote, a backslash and the five short escapes two bytes, any
/// other control six, the rest as in UTF-8.
fn fitting_prefix(text: &str, room: usize) -> usize {
    let mut cost = 0usize;
    for (i, c) in text.char_indices() {
        let one = match c {
            '"' | '\\' | '\u{8}' | '\u{c}' | '\n' | '\r' | '\t' => 2,
            '\0'..='\u{1f}' => 6,
            c => c.len_utf8(),
        };
        cost = cost.saturating_add(one);
        if cost > room {
            return i;
        }
    }
    text.len()
}

/// A page of the copy while messages go in: its first creation time, its number, its messages
/// once read, whether it changed, and where new messages went in it (before all it had, after,
/// or among them).
struct Slot {
    first: i64,
    key: u32,
    msgs: Option<Vec<ChatMessage>>,
    dirty: bool,
    at_start: bool,
    at_end: bool,
    inside: bool,
}

impl Slot {
    fn kept(first: i64, key: u32) -> Slot {
        Slot { first, key, msgs: None, dirty: false, at_start: false, at_end: false, inside: false }
    }

    /// Which way it fills as it splits, being page `p` of pages up to `last`: as its new messages
    /// went (after the others: new ones; before them: history), else by where it is.
    fn side(&self, p: usize, last: usize) -> Side {
        match (self.at_start, self.at_end, self.inside) {
            (true, false, false) => Side::Back,
            (false, true, false) => Side::Front,
            _ if p == last => Side::Front,
            _ if p == 0 => Side::Back,
            _ => Side::Halves,
        }
    }
}

/// Where a message goes in a page: by creation time, then id.
fn place(msgs: &[ChatMessage], m: &ChatMessage) -> usize {
    msgs.partition_point(|x| (x.created, x.id.as_str()) < (m.created, m.id.as_str()))
}

impl Copy {
    /// A page's messages: those of `chat`, held to the caps, in creation order. A page that can't
    /// be read is empty.
    fn page(&self, chat: &str, hash: &str, key: u32) -> Vec<ChatMessage> {
        let Some(bytes) = self.read(&format!("msgs/{hash}/{key}.json")) else { return Vec::new() };
        let Ok(page) = serde_json::from_slice::<PageIn>(&bytes) else {
            log::warn!("Teams copy: a damaged page passed over");
            return Vec::new();
        };
        if page.chat != chat {
            return Vec::new();
        }
        let mut msgs: Vec<ChatMessage> = page.messages.into_iter().filter(|m| m.chat == chat).filter_map(|mut m| held(&mut m).then_some(m)).collect();
        msgs.sort_by(|a, b| (a.created, &a.id).cmp(&(b.created, &b.id)));
        msgs
    }

    /// The messages of a chat's last `n` pages (at most a window's worth), in creation order.
    pub(crate) fn last_pages(&self, chat: &str, n: usize) -> Vec<ChatMessage> {
        let Some(r) = self.chat(chat) else { return Vec::new() };
        let hash = chat_hash(chat);
        let from = r.page_keys.len().saturating_sub(n.min(WINDOW_PAGES));
        r.page_keys.get(from..).unwrap_or_default().iter().flat_map(|k| self.page(chat, &hash, *k)).collect()
    }

    /// The page before the messages from `created` on: the messages of the newest page that
    /// begins before it, those created before it, in creation order.
    pub(crate) fn before(&self, chat: &str, created: i64) -> Vec<ChatMessage> {
        let Some(r) = self.chat(chat) else { return Vec::new() };
        let at = r.pages.partition_point(|&p| p < created);
        let Some(key) = at.checked_sub(1).and_then(|i| r.page_keys.get(i)) else { return Vec::new() };
        let mut msgs = self.page(chat, &chat_hash(chat), *key);
        msgs.retain(|m| m.created < created);
        msgs
    }

    /// Messages of a chat put in the copy, by (chat, message id): a new one where its creation
    /// time puts it, a known one replaced (an edit replaces the body, a deletion leaves only its
    /// id, times and sender, and its hosted images go). Messages of another chat, or without a
    /// usable id, are passed over; of an id twice, the last counts. Only the pages that change are
    /// written, with the chat's record when its pages do, in one go. Returns what changed, as
    /// kept. A chat not in the copy keeps nothing.
    pub(crate) fn upsert(&self, chat: &str, messages: &[ChatMessage]) -> Result<Vec<ChatMessage>> {
        let Some(mut r) = self.chat(chat) else {
            log::warn!("Teams copy: {} messages of a chat not in the copy passed over", messages.len());
            return Ok(Vec::new());
        };
        let hash = chat_hash(chat);
        let mut incoming: Vec<ChatMessage> = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        for m in messages.iter().filter(|m| m.chat == chat) {
            let mut m = m.clone();
            if !held(&mut m) {
                continue;
            }
            match seen.get(&m.id) {
                Some(&i) => {
                    if let Some(slot) = incoming.get_mut(i) {
                        *slot = m;
                    }
                }
                None => {
                    seen.insert(m.id.clone(), incoming.len());
                    incoming.push(m);
                }
            }
        }
        if incoming.is_empty() {
            return Ok(Vec::new());
        }
        if r.pages.is_empty() {
            // Pages left by a record that lost them, or by a write cut short: not this layout's.
            self.store.remove_account_files(TEAMS_ACCOUNT, &format!("msgs/{hash}"))?;
        }
        let mut slots: Vec<Slot> = r.pages.iter().zip(&r.page_keys).map(|(first, key)| Slot::kept(*first, *key)).collect();
        let mut changed = Vec::new();
        let mut forget: Vec<(String, String)> = Vec::new();
        for m in incoming {
            match self.find(&mut slots, chat, &hash, &m) {
                Some((p, i)) => {
                    let Some(slot) = slots.get_mut(p) else { continue };
                    let Some(msgs) = slot.msgs.as_mut() else { continue };
                    let Some(old) = msgs.get(i) else { continue };
                    if *old == m {
                        continue;
                    }
                    if m.deleted.is_some() {
                        forget.extend(hosted_ids(old).into_iter().map(|h| (m.id.clone(), h)));
                    }
                    slot.dirty = true;
                    if old.created == m.created {
                        if let Some(o) = msgs.get_mut(i) {
                            *o = m.clone();
                        }
                    } else {
                        // Its creation time moved (it shouldn't): taken out, and put where it goes now.
                        msgs.remove(i);
                        if let Some(f) = msgs.first() {
                            slot.first = f.created;
                        }
                        self.insert(&mut slots, chat, &hash, m.clone());
                    }
                    changed.push(m);
                }
                None => {
                    self.insert(&mut slots, chat, &hash, m.clone());
                    changed.push(m);
                }
            }
        }
        if changed.is_empty() {
            return Ok(changed);
        }
        let Settled { firsts, keys, mut writes, gone } = settle(slots, chat, &r.page_keys)?;
        if firsts != r.pages || keys != r.page_keys {
            r.pages = firsts;
            r.page_keys = keys;
            writes.push(self.chat_file(&r)?);
        }
        // What a deleted message showed goes first: should its page fail to be written, the
        // deletion comes again with the next read, and nothing of it is left meanwhile.
        for (message, hosted) in forget {
            self.forget_hosted(chat, &message, &hosted)?;
        }
        self.store.write_account_files(TEAMS_ACCOUNT, &writes)?;
        for key in gone {
            self.store.remove_account_files(TEAMS_ACCOUNT, &format!("msgs/{hash}/{key}.json"))?;
        }
        Ok(changed)
    }

    /// Where a known message is: (page, place in it), among the pages its creation time may be in
    /// (the one that begins before it, and those that begin at it).
    fn find(&self, slots: &mut [Slot], chat: &str, hash: &str, m: &ChatMessage) -> Option<(usize, usize)> {
        let below = slots.partition_point(|s| s.first < m.created);
        let upto = slots.partition_point(|s| s.first <= m.created);
        for p in below.saturating_sub(1)..upto {
            let slot = slots.get_mut(p)?;
            let key = slot.key;
            let msgs = slot.msgs.get_or_insert_with(|| self.page(chat, hash, key));
            if let Some(i) = msgs.iter().position(|x| x.id == m.id) {
                return Some((p, i));
            }
        }
        None
    }

    /// A new message put in the newest page that begins at or before it (the first, when it's
    /// older than them all).
    fn insert(&self, slots: &mut Vec<Slot>, chat: &str, hash: &str, m: ChatMessage) {
        if slots.is_empty() {
            slots.push(Slot { msgs: Some(Vec::new()), ..Slot::kept(m.created, NEW_PAGE) });
        }
        let p = slots.partition_point(|s| s.first <= m.created).saturating_sub(1);
        let Some(slot) = slots.get_mut(p) else { return };
        let key = slot.key;
        let msgs = slot.msgs.get_or_insert_with(|| self.page(chat, hash, key));
        let (at, had) = (place(msgs, &m), msgs.len());
        msgs.insert(at, m);
        slot.dirty = true;
        match at {
            _ if had == 0 => {}
            0 => slot.at_start = true,
            _ if at == had => slot.at_end = true,
            _ => slot.inside = true,
        }
        if let Some(f) = msgs.first() {
            slot.first = f.created;
        }
    }
}

/// The pages once messages went in.
struct Settled {
    /// Each page's first creation time and number, oldest page first.
    firsts: Vec<i64>,
    keys: Vec<u32>,
    /// The pages to write.
    writes: Vec<(String, Vec<u8>)>,
    /// The numbers of the pages left empty.
    gone: Vec<u32>,
}

/// The pages once messages went in: changed ones split where they're full and numbered (a page
/// keeps its number, a new one takes the lowest free one), with their bytes.
fn settle(slots: Vec<Slot>, chat: &str, old_keys: &[u32]) -> Result<Settled> {
    let hash = chat_hash(chat);
    let mut used: BTreeSet<u32> = old_keys.iter().copied().collect();
    let last = slots.len().saturating_sub(1);
    let mut out = Settled { firsts: Vec::new(), keys: Vec::new(), writes: Vec::new(), gone: Vec::new() };
    for (p, mut slot) in slots.into_iter().enumerate() {
        let (true, Some(msgs)) = (slot.dirty, slot.msgs.take()) else {
            out.firsts.push(slot.first);
            out.keys.push(slot.key);
            continue;
        };
        if msgs.is_empty() {
            if slot.key != NEW_PAGE {
                out.gone.push(slot.key);
            }
            continue;
        }
        let side = slot.side(p, last);
        for (i, (part, bytes)) in pieces(msgs, side, chat)?.into_iter().enumerate() {
            let key = if i == 0 && slot.key != NEW_PAGE { slot.key } else { lowest_free(&mut used)? };
            out.firsts.push(part.first().map_or(slot.first, |m| m.created));
            out.keys.push(key);
            out.writes.push((format!("msgs/{hash}/{key}.json"), bytes));
        }
    }
    Ok(out)
}

/// The lowest page number not taken, taken.
fn lowest_free(used: &mut BTreeSet<u32>) -> Result<u32> {
    let mut k = 0u32;
    while used.contains(&k) {
        k = k.checked_add(1).filter(|k| *k < MAX_PAGE_KEY).ok_or_else(|| refused("a chat with too many pages"))?;
    }
    used.insert(k);
    Ok(k)
}

/// Which way a full page fills as it splits.
#[derive(Clone, Copy)]
enum Side {
    /// The newest page: full pages from its start, what's left last.
    Front,
    /// The oldest: full pages from its end, what's left first.
    Back,
    /// Any other: two halves, again until each fits.
    Halves,
}

/// A page's messages as the pages they make, each with its bytes. No page's edge falls between
/// two messages made in the same millisecond (only a run of them longer than a page is parted):
/// the page before a window is found by creation time alone.
fn pieces(msgs: Vec<ChatMessage>, side: Side, chat: &str) -> Result<Vec<(Vec<ChatMessage>, Vec<u8>)>> {
    let mut parts: Vec<Vec<ChatMessage>> = Vec::new();
    let mut rest = msgs;
    match side {
        Side::Front => {
            while rest.len() > PAGE {
                let tail = rest.split_off(edge_down(&rest, PAGE));
                parts.push(std::mem::replace(&mut rest, tail));
            }
            parts.push(rest);
        }
        Side::Back => {
            while rest.len() > PAGE {
                let tail = rest.split_off(edge_up(&rest, rest.len() - PAGE));
                parts.push(tail);
            }
            parts.push(rest);
            parts.reverse();
        }
        Side::Halves => parts.push(rest),
    }
    let mut out = Vec::new();
    let mut todo: Vec<Vec<ChatMessage>> = parts.into_iter().rev().collect();
    while let Some(mut part) = todo.pop() {
        let bytes = serde_json::to_vec(&PageOut { chat, messages: &part }).map_err(|source| StoreError::Json { path: "msgs".into(), source })?;
        if part.len() > 1 && (part.len() > PAGE || bytes.len() > PAGE_BYTES) {
            let half = part.split_off(edge_near(&part, part.len() / 2));
            todo.push(half);
            todo.push(part);
            continue;
        }
        out.push((part, bytes));
    }
    Ok(out)
}

/// Whether an edge at `k` parts two messages made in the same millisecond.
fn parts_a_run(msgs: &[ChatMessage], k: usize) -> bool {
    match (k.checked_sub(1).and_then(|j| msgs.get(j)), msgs.get(k)) {
        (Some(a), Some(b)) => a.created == b.created,
        _ => false,
    }
}

/// An edge at `k` (inside `msgs`, `0 < k < len`) moved back to where the run of equal creation
/// times it falls in begins; `k` itself when that run begins the page.
fn edge_down(msgs: &[ChatMessage], k: usize) -> usize {
    let mut j = k;
    while j > 0 && parts_a_run(msgs, j) {
        j -= 1;
    }
    if j == 0 { k } else { j }
}

/// An edge at `k` moved on to where its run ends; `k` itself when that run ends the page.
fn edge_up(msgs: &[ChatMessage], k: usize) -> usize {
    let mut j = k;
    while j < msgs.len() && parts_a_run(msgs, j) {
        j += 1;
    }
    if j >= msgs.len() { k } else { j }
}

/// The edge nearest `k` that parts no run; `k` itself when there's none.
fn edge_near(msgs: &[ChatMessage], k: usize) -> usize {
    let (down, up) = (edge_down(msgs, k), edge_up(msgs, k));
    match (!parts_a_run(msgs, down), !parts_a_run(msgs, up)) {
        (true, true) if k.saturating_sub(down) <= up.saturating_sub(k) => down,
        (true, true) => up,
        (true, false) => down,
        (false, true) => up,
        (false, false) => k,
    }
}
