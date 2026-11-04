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
    /// may open. The label loses its control characters and double quotes, is cut to
    /// [`CONFERENCE_LABEL_MAX`] characters, then trimmed.
    pub fn new(url: &str, label: &str) -> Option<Conference> {
        let url = url.trim();
        launchable_link(url).then(|| Conference { url: url.to_string(), label: clean_label(label), features: vec!["AUDIO".into(), "VIDEO".into()] })
    }
}

/// A label as kept: no control character or double quote, cut, then trimmed (so that a cut
/// leaves no space at the end for the reader to take off: written, it reads back the same).
fn clean_label(label: &str) -> String {
    label.chars().filter(|c| !c.is_control() && *c != '"').take(CONFERENCE_LABEL_MAX).collect::<String>().trim().to_string()
}

/// A feature as kept: 1 to 40 bytes of ASCII letters, digits and `-`.
fn is_feature(f: &str) -> bool {
    (1..=FEATURE_LEN_MAX).contains(&f.len()) && f.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// The features a `FEATURE` value names, trimmed, those that aren't one left out.
fn features_in(value: &str) -> impl Iterator<Item = &str> {
    value.split(',').map(str::trim).filter(|f| is_feature(f))
}

/// A feature that isn't a meeting to join: a phone, a chat, a feed.
fn not_a_meeting(f: &str) -> bool {
    ["PHONE", "CHAT", "FEED"].iter().any(|o| f.eq_ignore_ascii_case(o))
}

/// Read a `CONFERENCE` line into `e` when it has no conference yet and the line is one; `false`
/// leaves the line to be kept as it came.
pub(super) fn take(e: &mut CalEvent, l: &Line) -> bool {
    if e.conference.is_some() || l.param("VALUE").is_some_and(|v| !v.trim().eq_ignore_ascii_case("URI")) {
        return false;
    }
    let url = l.value.trim();
    let named = l.param("FEATURE").unwrap_or("");
    if !launchable_link(url) || (features_in(named).next().is_some() && features_in(named).all(not_a_meeting)) {
        return false;
    }
    e.conference = Some(Conference {
        url: url.to_string(),
        label: clean_label(l.param("LABEL").unwrap_or("")),
        features: features_in(named).take(FEATURES_MAX).map(str::to_ascii_uppercase).collect(),
    });
    true
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
