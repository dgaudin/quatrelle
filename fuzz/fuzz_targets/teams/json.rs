// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Graph's answers about Teams (a server's, carrying what senders wrote) read into the model: a
//! chat, a member, a message and a presence, from the value itself and from each item of its
//! `value` list. Never a panic (the readers run without their guards too, so one is seen), and
//! what comes out within its caps: ids, lists, names on one line, bodies. And the answers about
//! files and people: an upload session's address, a range's answer, a file's item and its eTag, a
//! share's answer, a person of the directory; every string of the input as a transfer's address.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_model::pim::teams::{
    Attachment, Chat, ChatMessage, MAX_ATTACHMENTS, MAX_BODY, MAX_ID, MAX_MEMBERS, MAX_MENTIONS, MAX_REACTION_USERS, MAX_REACTIONS, Member,
    NAME_CHARS, REACTION_CHARS, SHARE_PEOPLE, Sender, is_bidi, is_object_id, person_ok,
};
use quatrelle_net::teams::graph::{
    MAX_ROLES, RangeAnswer, chat_of, chat_of_unguarded, drive_item_of, etag_guid, in_upload_folder, member_of, message_of, message_of_unguarded,
    next_expected_of, person_of, presence_of, range_answer_of, shared_of, transfer_url, upload_url_of,
};
use serde_json::Value;

/// The most items of a `value` list read from one input.
const ITEMS: usize = 64;
/// The most strings of one input tried as a transfer's address, and how deep they are looked for.
const STRINGS: usize = 64;
const DEPTH: usize = 4;

/// Whether an id can be one segment of an address: what Graph's ids are taken as.
fn segment_ok(id: &str) -> bool {
    id_ok(id) && !id.chars().all(|c| c == '.') && !id.contains(['/', '\\', '?', '#'])
}

fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.chars().count() <= MAX_ID && !id.chars().any(char::is_control)
}

fn line_ok(s: &str) -> bool {
    s.chars().count() <= NAME_CHARS && !s.chars().any(|c| c.is_control() || is_bidi(c))
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
        // An emoji with no control or bidi character, at most REACTION_CHARS of them.
        assert!(r.emoji.chars().count() <= REACTION_CHARS && !r.emoji.chars().any(|c| c.is_control() || is_bidi(c)), "{:?}", r.emoji);
        assert!(!["like", "heart", "laugh", "surprised", "sad", "angry"].iter().any(|n| r.emoji.eq_ignore_ascii_case(n)), "{:?}", r.emoji);
    }
    assert!(m.mentions.iter().all(|x| line_ok(&x.text) && x.user_id.as_deref().is_none_or(id_ok)));
    assert!(m.body.plain().len() <= MAX_BODY);
    for a in &m.attachments {
        if let Attachment::File { id, name, url, .. } = a {
            assert!(id_ok(id));
            assert!(!name.chars().chain(url.chars()).any(|c| c.is_control() || is_bidi(c)), "{name:?} {url:?}");
        }
    }
    check_sender(&m.sender);
    if m.deleted.is_some() || m.unshowable {
        assert!(m.body.blocks.is_empty() && m.attachments.is_empty() && m.mentions.is_empty() && m.reactions.is_empty());
    }
}

/// The answers about files and people.
fn read_files(v: &Value) {
    if let Some(p) = person_of(v) {
        assert!(person_ok(&p), "{p:?}");
    }
    if let Some(guid) = v.get("eTag").and_then(Value::as_str).and_then(etag_guid) {
        assert!(is_object_id(&guid) && guid == guid.to_ascii_lowercase(), "{guid:?}");
    }
    if let Some(item) = drive_item_of(v) {
        assert!(segment_ok(&item.id) && is_object_id(&item.guid) && !item.name.is_empty(), "{item:?}");
        assert!(item.web_dav_url.len() <= 8 << 10 && !item.web_dav_url.chars().any(|c| c.is_whitespace() || c.is_control()));
    }
    let _ = upload_url_of(v);
    let _ = in_upload_folder(v);
    for asked in [0, 2, SHARE_PEOPLE] {
        let shared = shared_of(v, asked);
        assert_eq!(shared.granted + shared.failed, asked);
    }
    let total = v.get("size").and_then(Value::as_u64).unwrap_or(5 << 20);
    for total in [total, 0, u64::MAX] {
        if let Some(next) = next_expected_of(v, total) {
            assert!(next <= total);
        }
        for (done, end) in [(false, total / 2), (true, total / 2), (false, total), (true, total)] {
            match range_answer_of(v, done, end, total) {
                Some(RangeAnswer::Next(n)) => assert!(!done && end < total && n >= end && n < total),
                Some(RangeAnswer::Done(id)) => assert!(done && end == total && segment_ok(&id), "{id:?}"),
                None => {}
            }
        }
    }
    let mut strings = Vec::new();
    strings_of(v, DEPTH, &mut strings);
    for s in strings {
        if let Ok(url) = transfer_url("https://graph.microsoft.com/v1.0", s) {
            assert!(url.https && url.port == 443 && url.host.ends_with(".sharepoint.com"), "{s:?}");
        }
    }
}

/// The strings of a value, [`STRINGS`] at most, `depth` levels down.
fn strings_of<'a>(v: &'a Value, depth: usize, out: &mut Vec<&'a str>) {
    match v {
        Value::String(s) if out.len() < STRINGS => out.push(s),
        Value::Array(a) if depth > 0 => a.iter().for_each(|x| strings_of(x, depth - 1, out)),
        Value::Object(o) if depth > 0 => o.values().for_each(|x| strings_of(x, depth - 1, out)),
        _ => {}
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
    read_files(v);
}

fuzz_target!(|data: &[u8]| {
    let Ok(v) = serde_json::from_slice::<Value>(data) else { return };
    read(&v);
    for item in v.get("value").and_then(Value::as_array).into_iter().flatten().take(ITEMS) {
        read(item);
    }
});
