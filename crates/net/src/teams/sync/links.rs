// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What the log says of the chats' links (the window's "open in Teams"), so that it tells why a
//! chat's link isn't offered: when a chat is opened, its kind, whether it can't be read, and its
//! link as the window judges it ([`is_teams_chat_link`]); after a listing, how many chats of each
//! kind, how many with a link, how many the window takes, and the hosts the links are on, said
//! again only when that changes. Never an id, a name or a message's text: kinds, counts, hosts,
//! and a path's first segment when it's a short plain word.

use std::collections::{BTreeMap, HashMap};

use quatrelle_model::ical::is_teams_chat_link;
use quatrelle_model::pim::teams::{ChatKind, MAX_CHATS};

/// The most hosts a line names; the others are counted.
const HOSTS_SAID: usize = 8;

/// The longest path segment said; a longer one (an id, a token) isn't.
const SEGMENT_SAID: usize = 16;

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
/// unreadable, and its link.
pub(super) fn opened(kind: Option<&ChatKind>, unreadable: bool, url: Option<&str>) -> String {
    let link = match judged(url) {
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
    /// A chat listed, with its link as the window will have it. At most [`MAX_CHATS`].
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
