// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Graph's answers about Teams (a server's, carrying what senders wrote) read into the model: a
//! chat, a member, a message and a presence, from the value itself and from each item of its
//! `value` list. Never a panic (the readers run without their guards too, so one is seen), and
//! what comes out within its caps: ids, lists, names on one line, bodies.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_model::pim::teams::{
    Attachment, Chat, ChatMessage, MAX_ATTACHMENTS, MAX_BODY, MAX_ID, MAX_MEMBERS, MAX_MENTIONS, MAX_REACTION_USERS, MAX_REACTIONS, Member,
    NAME_CHARS, Sender,
};
use quatrelle_net::teams::graph::{MAX_ROLES, chat_of, chat_of_unguarded, member_of, message_of, message_of_unguarded, presence_of};
use serde_json::Value;

/// The most items of a `value` list read from one input.
const ITEMS: usize = 64;

fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.chars().count() <= MAX_ID && !id.chars().any(char::is_control)
}

fn line_ok(s: &str) -> bool {
    s.chars().count() <= NAME_CHARS && !s.chars().any(|c| c.is_control() || quatrelle_model::pim::teams::is_bidi(c))
}

fn check_member(m: &Member) {
    assert!(id_ok(&m.user_id) && line_ok(&m.name) && m.roles.len() <= MAX_ROLES, "{m:?}");
    assert!(m.tenant_id.as_deref().is_none_or(id_ok));
}

fn check_sender(s: &Sender) {
    match s {
        Sender::User { id, name, tenant_id, .. } => assert!(id_ok(id) && line_ok(name) && tenant_id.as_deref().is_none_or(id_ok)),
        Sender::App { id, name } => assert!(id_ok(id) && line_ok(name)),
        Sender::Nobody => {}
    }
}

fn check_chat(c: &Chat) {
    assert!(id_ok(&c.id) && c.members.len() <= MAX_MEMBERS);
    assert!(c.topic.as_deref().is_none_or(line_ok));
    c.members.iter().for_each(check_member);
    if let Some(p) = &c.preview {
        assert!(line_ok(&p.text), "{:?}", p.text);
        check_sender(&p.sender);
    }
}

fn check_message(m: &ChatMessage) {
    assert!(m.unshowable || id_ok(&m.id), "{:?}", m.id);
    assert!(m.id.is_empty() || id_ok(&m.id));
    assert!(m.attachments.len() <= MAX_ATTACHMENTS && m.mentions.len() <= MAX_MENTIONS && m.reactions.len() <= MAX_REACTIONS);
    assert!(m.reactions.iter().all(|r| r.users.len() <= MAX_REACTION_USERS && r.users.iter().all(|u| id_ok(u))));
    // A quick reaction is kept as its emoji, never by its name, and one kind of reaction is one
    // (an emoji with or without its presentation selector included).
    let bare: Vec<String> = m.reactions.iter().map(|r| r.emoji.chars().filter(|&c| c != '\u{FE0F}').collect()).collect();
    for (i, b) in bare.iter().enumerate() {
        assert!(!bare.iter().skip(i + 1).any(|o| o == b), "{:?}", m.reactions);
    }
    for r in &m.reactions {
        assert!(!["like", "heart", "laugh", "surprised", "sad", "angry"].iter().any(|n| r.emoji.eq_ignore_ascii_case(n)), "{:?}", r.emoji);
    }
    assert!(m.mentions.iter().all(|x| line_ok(&x.text) && x.user_id.as_deref().is_none_or(id_ok)));
    assert!(m.body.plain().len() <= MAX_BODY);
    for a in &m.attachments {
        if let Attachment::File { id, .. } = a {
            assert!(id_ok(id));
        }
    }
    check_sender(&m.sender);
    if m.deleted.is_some() || m.unshowable {
        assert!(m.body.blocks.is_empty() && m.attachments.is_empty() && m.mentions.is_empty() && m.reactions.is_empty());
    }
}

fn read(v: &Value) {
    for c in [chat_of_unguarded(v), chat_of(v)].iter().flatten() {
        check_chat(c);
    }
    if let Some(m) = member_of(v) {
        check_member(&m);
    }
    check_message(&message_of_unguarded("19:f@thread.v2", v));
    check_message(&message_of("19:f@thread.v2", v));
    if let Some((id, _)) = presence_of(v) {
        assert!(id_ok(&id));
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(v) = serde_json::from_slice::<Value>(data) else { return };
    read(&v);
    for item in v.get("value").and_then(Value::as_array).into_iter().flatten().take(ITEMS) {
        read(item);
    }
});
