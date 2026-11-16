// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The chats' links (the window's "open in Teams"): the one built for every chat it can be built
//! for, in Teams' own form for a conversation ([`built_chat_link`]), Graph's `webUrl` kept only
//! for a chat it can't; and what the log says of them, so that it tells why a chat's link isn't
//! offered: when a chat is opened, its kind, whether it can't be read, and that its link was
//! built, or Graph's as the window judges it ([`is_teams_chat_link`]); after a listing, how many
//! chats of each kind, how many Graph gives a link, how many of those the window takes, and the
//! hosts they are on, said again only when that changes. Never an id, a name or a message's
//! text: kinds, counts, hosts, and a path's first segment when it's a short plain word.
//!
//! What is hostile here: a chat's id (Graph's, or a copy's on disk) and the organisation's id the
//! sign-in gave. A link is built only from an id one segment of an address can carry that has a
//! Teams thread's shape, and a GUID, onto Teams' own host: nothing they hold can take the browser
//! elsewhere.

use std::collections::{BTreeMap, HashMap};

use quatrelle_model::ical::is_teams_chat_link;
use quatrelle_model::pim::teams::{ChatKind, MAX_CHATS};

use crate::graph::seg;
use crate::teams::graph::own_segment;
use crate::teams::worker::is_guid;

/// The most hosts a line names; the others are counted.
const HOSTS_SAID: usize = 8;

/// The longest path segment said; a longer one (an id, a token) isn't.
const SEGMENT_SAID: usize = 16;

/// How Teams' threads end: a group's or a meeting's, a one-to-one chat's, and older forms.
const THREADS: [&str; 4] = ["@thread.v2", "@unq.gbl.spaces", "@thread.skype", "@thread.tacv2"];

/// The `context` of Teams' own link to a chat: `{"contextType":"chat"}`, percent-encoded.
const CHAT_CONTEXT: &str = "%7B%22contextType%22%3A%22chat%22%7D";

/// A chat's link in Teams' documented form for a conversation
/// (`https://teams.microsoft.com/l/chat/<chat id>/conversations?context={"contextType":"chat"}&tenantId=<organisation>`,
/// the context percent-encoded), sent for every chat it can be built for, whatever Graph gives:
/// Graph's `webUrl` (`…/l/chat/<chat id>/0?tenantId=…`) is a link to a message, and the new Teams
/// reads it as one, message 0, which no chat has; and the chats Graph won't serve come without
/// one (decision 21). The organisation's id is kept, so that a guest's Teams picks the chat's.
/// The id is encoded as a segment of Graph's addresses is ([`seg`]). Only for an id one segment
/// can carry ([`own_segment`]: no `/`, `\`, `?` or `#`) that is a Teams thread's (`19:`,
/// something, then one of [`THREADS`]), and an organisation named by its GUID; and only a link
/// the window would open ([`is_teams_chat_link`]). Else none, and Graph's is kept.
pub(crate) fn built_chat_link(chat: &str, tenant: &str) -> Option<String> {
    let ends_a_thread = |rest: &str| {
        THREADS.iter().any(|end| {
            let at = rest.len().checked_sub(end.len()).filter(|at| *at > 0);
            at.and_then(|at| rest.get(at..)).is_some_and(|tail| tail.eq_ignore_ascii_case(end))
        })
    };
    let thread = chat.strip_prefix("19:").is_some_and(ends_a_thread);
    if !(thread && own_segment(chat) && is_guid(tenant)) {
        return None;
    }
    let url = format!("https://teams.microsoft.com/l/chat/{}/conversations?context={CHAT_CONTEXT}&tenantId={tenant}", seg(chat));
    is_teams_chat_link(&url).then_some(url)
}

/// A chat's kind as the log says it.
fn kind_name(kind: &ChatKind) -> &'static str {
    match kind {
        ChatKind::OneOnOne => "oneOnOne",
        ChatKind::Group => "group",
        ChatKind::Meeting => "meeting",
        ChatKind::Other { .. } => "unknown",
    }
}

/// A chat's link as the window judges it, and the host it's on.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Link {
    Absent,
    Accepted { host: String },
    Refused { host: String, first: String },
}

fn judged(url: Option<&str>) -> Link {
    match url {
        None => Link::Absent,
        Some(u) => {
            let (host, first) = host_and_first(u);
            if is_teams_chat_link(u) { Link::Accepted { host } } else { Link::Refused { host, first } }
        }
    }
}

/// A link's host, lowercase, when it's a plain one, and its path's first segment when that's a
/// short plain word: what the log may say of it.
fn host_and_first(url: &str) -> (String, String) {
    let rest = ["https://", "http://"].iter().find_map(|s| url.get(..s.len()).filter(|h| h.eq_ignore_ascii_case(s)).and_then(|_| url.get(s.len()..)));
    let Some(rest) = rest else { return ("(not a web address)".into(), String::new()) };
    let end = rest.find(['/', '?', '#', '\\']).unwrap_or(rest.len());
    let authority = rest.get(..end).unwrap_or_default();
    let host = authority.rsplit_once(':').filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit())).map_or(authority, |(h, _)| h);
    let plain = !host.is_empty() && host.len() <= 253 && host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    let host = if plain { host.to_ascii_lowercase() } else { "(not a plain host)".into() };
    let path = if rest.as_bytes().get(end) == Some(&b'/') { rest.get(end + 1..).unwrap_or_default() } else { "" };
    let segment = path.split(['/', '?', '#']).next().unwrap_or_default();
    let word = segment.len() <= SEGMENT_SAID && segment.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    (host, if word { segment.to_string() } else { "(withheld)".into() })
}

/// The line said when a chat is opened: its kind (none when the copy hasn't it), whether it's
/// unreadable, and its link: `built` when one was built ([`built_chat_link`]), else Graph's as
/// the window judges it.
pub(super) fn opened(kind: Option<&ChatKind>, unreadable: bool, url: Option<&str>, built: bool) -> String {
    let link = match judged(url) {
        _ if built => "built".to_string(),
        Link::Absent => "absent".to_string(),
        Link::Accepted { .. } => "ok".to_string(),
        Link::Refused { host, first } => format!("refused (host {host}, path /{first})"),
    };
    let unreadable = if unreadable { ", unreadable" } else { "" };
    format!("teams: a conversation opened: {}{unreadable}; its link: {link}", kind.map_or("unknown", kind_name))
}

/// The kind and link of each chat listed this session (by its keyed hash), and the figures last
/// said of them.
#[derive(Default)]
pub(crate) struct Links {
    chats: HashMap<u64, (&'static str, Link)>,
    said: Option<String>,
}

impl Links {
    /// A chat listed, with the link Graph gives it, judged as the window would (what Graph gives,
    /// not the link built in its place). At most [`MAX_CHATS`].
    pub(super) fn note(&mut self, key: u64, kind: &ChatKind, url: Option<&str>) {
        if self.chats.len() < MAX_CHATS || self.chats.contains_key(&key) {
            self.chats.insert(key, (kind_name(kind), judged(url)));
        }
    }

    /// A listing is over: its figures, when they aren't those last said.
    pub(super) fn changed(&mut self) -> Option<String> {
        let line = self.figures();
        if self.said.as_deref() == Some(line.as_str()) {
            return None;
        }
        self.said = Some(line.clone());
        Some(line)
    }

    fn figures(&self) -> String {
        let mut kinds: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new();
        let mut hosts: BTreeMap<&str, usize> = BTreeMap::new();
        for (kind, link) in self.chats.values() {
            let n = kinds.entry(kind).or_default();
            n.0 += 1;
            let host = match link {
                Link::Absent => None,
                Link::Accepted { host } => {
                    n.2 += 1;
                    Some(host)
                }
                Link::Refused { host, .. } => Some(host),
            };
            if let Some(host) = host {
                n.1 += 1;
                *hosts.entry(host.as_str()).or_default() += 1;
            }
        }
        let kinds: Vec<String> = kinds.iter().map(|(k, (n, linked, ok))| format!("{k} {n} ({linked} with a link, {ok} accepted)")).collect();
        let mut by_count: Vec<(&str, usize)> = hosts.into_iter().collect();
        by_count.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let mut named: Vec<String> = by_count.iter().take(HOSTS_SAID).map(|(h, n)| format!("{h} {n}")).collect();
        if by_count.len() > HOSTS_SAID {
            named.push(format!("{} more hosts", by_count.len() - HOSTS_SAID));
        }
        let kinds = if kinds.is_empty() { "none".to_string() } else { kinds.join(", ") };
        let hosts = if named.is_empty() { "none".to_string() } else { named.join(", ") };
        format!("teams: links of the conversations listed: {kinds}; hosts: {hosts}")
    }
}
