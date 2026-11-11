// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! How the check names what it counts: kinds of chat and of identity as Graph names them, the
//! shapes of chat ids, and counts by name. Only Graph's names and Teams' own forms of id are
//! said, never a name, an address or an id.

use serde_json::Value;

/// Names said at most in a count by name, and names kept at most (the others are counted
/// under "more").
const NAMES_SAID: usize = 8;
const NAMES_KEPT: usize = 64;
/// Longest kind said as Graph names it.
const MAX_KIND: usize = 32;
/// Longest prefix of a chat id said (`19`, `48`).
const MAX_PREFIX: usize = 4;
/// The domains Teams' chat ids end with: any other is said as `another`.
const ID_DOMAINS: [&str; 4] = ["thread.v2", "thread.skype", "thread.tacv2", "unq.gbl.spaces"];
/// Teams' own threads of one person (`48:notes`): any other is said as `...`.
const OWN_THREADS: [&str; 5] = ["notes", "calllogs", "mentions", "annotations", "drafts"];

/// A kind (of chat, of identity) as Graph names it (`unknownFutureValue`), said only when it
/// looks like one of its names: `none` when there is none, `unreadable` otherwise.
pub(crate) fn kind_name(kind: Option<&str>) -> String {
    match kind {
        None | Some("") => "none".into(),
        Some(k) if k.len() <= MAX_KIND && k.bytes().all(|b| b.is_ascii_alphanumeric()) => k.into(),
        Some(_) => "unreadable".into(),
    }
}

/// A chat id by its shape alone: its numeric prefix and the domain Teams ends it with
/// (`19:...unq.gbl.spaces`), or one of Teams' own threads (`48:notes`); never what is unique
/// to it.
pub(crate) fn id_shape(id: Option<&str>) -> String {
    let Some(id) = id else { return "none".into() };
    let Some((prefix, rest)) = id.split_once(':') else { return "unreadable".into() };
    if prefix.is_empty() || prefix.len() > MAX_PREFIX || !prefix.bytes().all(|b| b.is_ascii_digit()) {
        return "unreadable".into();
    }
    match rest.rsplit_once('@') {
        Some((_, domain)) => format!("{prefix}:...{}", ID_DOMAINS.iter().find(|d| **d == domain).copied().unwrap_or("another")),
        None => match OWN_THREADS.iter().find(|t| **t == rest) {
            Some(t) => format!("{prefix}:{t}"),
            None => format!("{prefix}:..."),
        },
    }
}

/// Who wrote a chat's last message, by Graph's kind of identity: `app bot`, `person aadUser`,
/// `device`, `nobody` (a message from no one, such as a system event) or `no message`.
pub(crate) fn author(chat: &Value) -> String {
    let set = |v: &Value, key: &str| v.get(key).filter(|x| !x.is_null()).cloned();
    let Some(preview) = set(chat, "lastMessagePreview") else { return "no message".into() };
    let Some(from) = set(&preview, "from") else { return "nobody".into() };
    let kind = |v: &Value, key: &str| kind_name(v.get(key).and_then(Value::as_str));
    if let Some(app) = set(&from, "application") {
        format!("app {}", kind(&app, "applicationIdentityType"))
    } else if let Some(user) = set(&from, "user") {
        format!("person {}", kind(&user, "userIdentityType"))
    } else if set(&from, "device").is_some() {
        "device".into()
    } else {
        "nobody".into()
    }
}

/// Counts by name (kinds of chat, reasons): the first [`NAMES_KEPT`] names kept, the others
/// counted under "more" at once.
#[derive(Default)]
pub(crate) struct ByName {
    names: Vec<(String, usize)>,
    more: usize,
}

impl ByName {
    pub(crate) fn add(&mut self, name: String) {
        if let Some((_, n)) = self.names.iter_mut().find(|(k, _)| *k == name) {
            *n += 1;
        } else if self.names.len() < NAMES_KEPT {
            self.names.push((name, 1));
        } else {
            self.more += 1;
        }
    }

    /// `c 5, a 3, b 3`: the largest counts first (then by name), [`NAMES_SAID`] at most, the
    /// rest summed as `more N`.
    pub(crate) fn said(&self) -> String {
        let mut names: Vec<&(String, usize)> = self.names.iter().collect();
        names.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let more = self.more + names.iter().skip(NAMES_SAID).map(|(_, n)| n).sum::<usize>();
        let mut said: Vec<String> = names.iter().take(NAMES_SAID).map(|(k, n)| format!("{k} {n}")).collect();
        if more > 0 {
            said.push(format!("more {more}"));
        }
        said.join(", ")
    }
}
