// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Where to join an event online, as its calendar says (RFC 7986 §5.11 `CONFERENCE`).
//!
//! Read: the first `CONFERENCE` line whose value is a web link Quatrelle may open
//! ([`launchable_link`]), `VALUE=URI` or none, and whose features, when it names any, aren't only
//! `PHONE`, `CHAT` or `FEED`; every other one (a `tel:` link, a chat, a second one) stays among the
//! event's kept lines, written back as it came. Written with its label in double quotes when it
//! holds a `,`, `;` or `:` (a parameter's value has no backslash escapes, RFC 5545 §3.2).

use super::*;

/// The longest label kept, in characters.
pub const CONFERENCE_LABEL_MAX: usize = 80;
/// The most features kept.
const FEATURES_MAX: usize = 10;
/// The longest feature kept, in bytes.
const FEATURE_LEN_MAX: usize = 40;

/// Where to join an event online (RFC 7986 `CONFERENCE`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conference {
    /// The web link that joins it.
    pub url: String,
    /// Its words (`LABEL`): at most [`CONFERENCE_LABEL_MAX`] characters, no control character or
    /// double quote.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// What it offers (`FEATURE`: `AUDIO`, `VIDEO`, `SCREEN`…), as the calendar named them; none
    /// when it didn't say.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
}

impl Conference {
    /// One made here, with audio and video: `None` unless `url` (trimmed) is a web link Quatrelle
    /// may open. The label loses its control characters, double quotes and the characters that
    /// turn text round or hide in it, is cut to [`CONFERENCE_LABEL_MAX`] characters, then trimmed.
    pub fn new(url: &str, label: &str) -> Option<Conference> {
        let url = url.trim();
        launchable_link(url).then(|| Conference { url: url.to_string(), label: clean_label(label), features: vec!["AUDIO".into(), "VIDEO".into()] })
    }
}

impl CalEvent {
    /// Take its conference from its kept lines, where a Quatrelle that didn't read `CONFERENCE`
    /// left the line (an event cached before): the first one there that reading the file would
    /// have taken, outside the components kept whole, comes out of them. Nothing changes when it
    /// has a conference already. Called on events loaded from the cache.
    pub fn adopt_kept_conference(&mut self) {
        if self.conference.is_some() {
            return;
        }
        let Some((i, c)) = kept(&self.extra).find_map(|(i, l)| read(&l).map(|c| (i, c))) else { return };
        if i < self.extra.len() {
            self.extra.remove(i);
            self.conference = Some(c);
        }
    }
}

/// A label as kept: no control character, double quote, or character that turns text round or
/// hides in it (a stranger's label could make the window show other words); cut, then trimmed
/// (so that a cut leaves no space at the end for the reader to take off: written, it reads back
/// the same).
fn clean_label(label: &str) -> String {
    label.chars().filter(|c| !c.is_control() && *c != '"' && !is_hidden(*c)).take(CONFERENCE_LABEL_MAX).collect::<String>().trim().to_string()
}

/// Bidirectional embeddings, overrides and isolates, zero-width characters and marks, line and
/// paragraph separators, the byte order mark, the Arabic letter mark.
fn is_hidden(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}' | '\u{061c}')
}

/// A feature as kept: 1 to 40 bytes of ASCII letters, digits and `-`.
fn is_feature(f: &str) -> bool {
    (1..=FEATURE_LEN_MAX).contains(&f.len()) && f.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A feature that isn't a meeting to join: a phone, a chat, a feed.
fn not_a_meeting(f: &str) -> bool {
    ["PHONE", "CHAT", "FEED"].iter().any(|o| f.eq_ignore_ascii_case(o))
}

/// The conference a `CONFERENCE` line gives, if it is one. Its features are the first ten that
/// are features, upper-cased; when those are only a phone, a chat or a feed, it isn't one (so a
/// conference read, written and read again, is still one).
fn read(l: &Line) -> Option<Conference> {
    if l.param("VALUE").is_some_and(|v| !v.trim().eq_ignore_ascii_case("URI")) {
        return None;
    }
    let url = l.value.trim();
    if !launchable_link(url) {
        return None;
    }
    let named = l.param("FEATURE").unwrap_or("");
    let features: Vec<String> = named.split(',').map(str::trim).filter(|f| is_feature(f)).take(FEATURES_MAX).map(str::to_ascii_uppercase).collect();
    if !features.is_empty() && features.iter().all(|f| not_a_meeting(f)) {
        return None;
    }
    Some(Conference { url: url.to_string(), label: clean_label(l.param("LABEL").unwrap_or("")), features })
}

/// Read a `CONFERENCE` line into `e` when it has no conference yet and the line is one; `false`
/// leaves the line to be kept as it came.
pub(super) fn take(e: &mut CalEvent, l: &Line) -> bool {
    if e.conference.is_some() {
        return false;
    }
    e.conference = read(l);
    e.conference.is_some()
}

/// The `CONFERENCE` lines among kept ones, with their places, outside the components kept whole
/// with them (an alarm Quatrelle can't read). Only the lines that may be one of those, or a
/// component's start or end, are read (every cached event goes through here).
fn kept(extra: &[String]) -> impl Iterator<Item = (usize, Line)> + '_ {
    let mut depth = 0usize;
    extra.iter().enumerate().filter_map(move |(i, raw)| {
        let starts = |n: &str| raw.get(..n.len()).is_some_and(|p| p.eq_ignore_ascii_case(n));
        if !(starts("CONFERENCE") || starts("BEGIN") || starts("END")) {
            return None;
        }
        let l = parse_line(raw)?;
        match l.name.as_str() {
            "BEGIN" => depth = depth.saturating_add(1),
            "END" => depth = depth.saturating_sub(1),
            "CONFERENCE" if depth == 0 => return Some((i, l)),
            _ => {}
        }
        None
    })
}

/// The places of the kept lines that give the link of the conference written with them (any
/// case), left out so that a link is never written twice.
pub(super) fn repeats(e: &CalEvent) -> Vec<usize> {
    let Some(c) = e.conference.as_ref().filter(|c| launchable_link(&c.url)) else { return Vec::new() };
    kept(&e.extra).filter(|(_, l)| l.value.trim().eq_ignore_ascii_case(&c.url)).map(|(i, _)| i).collect()
}

/// Its content line, unfolded; `None` when its link may not be opened (a damaged cache). The
/// label and the features are cleaned as when read, so nothing in them can cut the line.
pub(super) fn line(c: &Conference) -> Option<String> {
    if !launchable_link(&c.url) {
        return None;
    }
    let mut s = String::from("CONFERENCE;VALUE=URI");
    let features: Vec<&str> = c.features.iter().map(String::as_str).filter(|f| is_feature(f)).take(FEATURES_MAX).collect();
    if !features.is_empty() {
        s.push_str(";FEATURE=");
        s.push_str(&features.join(","));
    }
    let label = clean_label(&c.label);
    if label.contains([',', ';', ':']) {
        s.push_str(&format!(";LABEL=\"{label}\""));
    } else if !label.is_empty() {
        s.push_str(";LABEL=");
        s.push_str(&label);
    }
    s.push(':');
    s.push_str(&c.url);
    Some(s)
}
