// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! An appointment's Teams meeting, in its form: the block written at the end of its notes (the
//! link to join, the meeting ID and passcode, the phone, the options page), found again whatever
//! language the interface had when it was written (D10); the place, Join and the conference the
//! meeting brings; what removing it takes out, and what the window leaves to delete when it goes
//! without saving.
//!
//! What is hostile here: the notes, which anyone may have written (an invitation pasted in, a
//! block edited by hand). A block starts only at a line that is a join line's words followed by
//! nothing but a link of Teams' own ([`teams_meeting_url`]); it goes on for the lines that start
//! with the words of the block's other lines, [`BLOCK_LINES`] at most, and an options link is
//! taken from it only when it is Teams' own too. The meeting's values come from the engine, which
//! keeps only plain ones and Teams' own links.

use std::sync::OnceLock;

use quatrelle_engine::fill;
use quatrelle_engine::model::ical::Conference;
use quatrelle_engine::model::pim::teams::{TEAMS_CONFERENCE_LABEL, TeamsMeeting, teams_meeting_url};

use super::*;

/// The most lines a meeting's block takes: the link to join; the meeting ID and passcode; the
/// phone; the options page.
pub(crate) const BLOCK_LINES: usize = 4;

/// The join line, as the catalogs key it.
const JOIN: &str = "Join the Teams meeting: {link}";
/// The options line.
const OPTIONS: &str = "Meeting options: {link}";
/// Each of the lines that may follow the join line in a block (the options line among them).
const FOLLOWING: [&str; 5] =
    ["Meeting ID: {id}", "Passcode: {passcode}", "By phone: {number}, conference ID {conference}", "By phone: {number}", OPTIONS];

/// A meeting's block found in the notes.
pub(crate) struct Block {
    /// The line it starts at (the notes split at each `\n`).
    pub at: usize,
    /// How many lines it takes.
    pub lines: usize,
    /// The link to join the meeting, from its first line.
    pub join: String,
    /// The link to the meeting's options page, when its options line has one of Teams' own.
    pub options: Option<String>,
}

/// The block's lines for `m`, in the interface's language, each value as Teams gave it; a line
/// whose values are all missing is left out. Filled in as they are, not with `tf!`: its marks
/// around a value would go out in the invitation.
pub(crate) fn block_lines(m: &TeamsMeeting) -> Vec<String> {
    let mut lines = vec![fill(tl!("Join the Teams meeting: {link}"), &[("link", &m.join_url)])];
    let mut ids = Vec::new();
    if !m.meeting_id.is_empty() {
        ids.push(fill(tl!("Meeting ID: {id}"), &[("id", &m.meeting_id)]));
    }
    if !m.passcode.0.is_empty() {
        ids.push(fill(tl!("Passcode: {passcode}"), &[("passcode", &m.passcode.0)]));
    }
    if !ids.is_empty() {
        lines.push(ids.join("   "));
    }
    if !m.toll_number.is_empty() {
        lines.push(if m.conference_id.is_empty() {
            fill(tl!("By phone: {number}"), &[("number", &m.toll_number)])
        } else {
            fill(tl!("By phone: {number}, conference ID {conference}"), &[("number", &m.toll_number), ("conference", &m.conference_id)])
        });
    }
    if !m.options_url.is_empty() {
        lines.push(fill(tl!("Meeting options: {link}"), &[("link", &m.options_url)]));
    }
    lines
}

/// The first meeting's block in `notes`.
pub(crate) fn find_block(notes: &str) -> Option<Block> {
    let lines: Vec<&str> = notes.split('\n').collect();
    (0..lines.len()).find_map(|at| {
        let (n, join) = block_at(lines.get(at..).unwrap_or_default())?;
        let options = lines.get(at + 1..at + n).unwrap_or_default().iter().find_map(|l| link_after(l, &labels().options));
        Some(Block { at, lines: n, join: join.to_string(), options: options.map(str::to_string) })
    })
}

/// The notes without any meeting's block, the blank lines around it as for a Webex room's
/// ([`super::webex_rooms::without_blocks`]). Notes with no block come back as they were.
pub(crate) fn without_block(notes: &str) -> String {
    super::webex_rooms::without_blocks(notes, |lines| block_at(lines).map(|(n, _)| n))
}

/// The link to the options page of the meeting whose block is in `notes`.
pub(crate) fn options_link(notes: &str) -> Option<String> {
    find_block(notes).and_then(|b| b.options)
}

/// Teams' answer in the appointment: its block at the end of the notes, in place of any meeting's
/// block there; its link as the place when that's empty or the replaced block's link (one typed
/// stays); Join; its conference; its id, made in this window and not saved yet.
pub(crate) fn take_meeting(f: &mut EventForm, m: &TeamsMeeting) {
    let replaced = find_block(&f.description).map(|b| b.join);
    let notes = without_block(&f.description);
    let notes = notes.trim_end();
    let gap = if notes.is_empty() { "" } else { "\n\n" };
    f.description = format!("{notes}{gap}{}\n", block_lines(m).join("\n"));
    let place = f.location.trim();
    if place.is_empty() || replaced.as_deref() == Some(place) {
        f.location = m.join_url.clone();
    }
    f.join = Some(m.join_url.clone());
    if let Some(c) = Conference::new(&m.join_url, TEAMS_CONFERENCE_LABEL) {
        f.conference = Some(c);
        f.conference_shown = true;
    }
    f.teams_meeting_id = Some(m.id.clone());
    f.teams_fresh = true;
}

/// Takes the meeting out of the form: its block, the place when it's the meeting's link (one typed
/// stays), Join, its conference (the one the window opened with comes back, unless it was this
/// meeting's), its id; the user removed it. A meeting made in this window and never saved is
/// deleted at once: `event.teamsMeeting`'s params for it. The saved appointment's is deleted by
/// the engine when the appointment is saved without it.
pub(crate) fn remove(f: &mut EventForm) -> Option<Value> {
    let link = meeting_link(f);
    f.description = without_block(&f.description);
    if let Some(link) = &link {
        if f.location.trim() == link {
            f.location.clear();
        }
        if f.join.as_ref() == Some(link) {
            f.join = None;
        }
        if f.conference.as_ref().is_some_and(|c| c.url == *link) {
            f.conference = f.conference_opened.clone().filter(|c| c.url != *link);
            f.conference_shown = f.conference.as_ref().is_some_and(|c| super::event::link_shown(&c.url, &f.location, &f.description));
        }
    }
    f.teams_removed = true;
    let fresh = std::mem::take(&mut f.teams_fresh);
    f.teams_meeting_id.take().filter(|_| fresh).map(|id| json!({"remove": {"id": id}}))
}

/// `event.teamsMeeting`'s params for what the window leaves unsaved: the meeting made in it, or
/// the one on its way. Never the saved appointment's.
pub(crate) fn leaving(f: &EventForm) -> Option<Value> {
    match (&f.teams_meeting_id, f.teams_wait) {
        (Some(id), _) if f.teams_fresh => Some(json!({"remove": {"id": id}})),
        (_, Some(ask)) => Some(json!({"remove": {"ask": ask}})),
        _ => None,
    }
}

/// The link of the form's meeting: its block's, else its conference's when Quatrelle labelled it
/// Teams', else Join's; only one of Teams' own.
fn meeting_link(f: &EventForm) -> Option<String> {
    let conference = f.conference.as_ref().filter(|c| c.label == TEAMS_CONFERENCE_LABEL).map(|c| c.url.clone());
    find_block(&f.description).map(|b| b.join).or(conference).or_else(|| f.join.clone()).filter(|l| teams_meeting_url(l))
}

/// The words before each line's value, in every catalog's language.
struct Labels {
    join: Vec<String>,
    following: Vec<String>,
    options: Vec<String>,
}

/// The block's labels, read once: the catalogs don't change while Quatrelle runs.
fn labels() -> &'static Labels {
    static LABELS: OnceLock<Labels> = OnceLock::new();
    LABELS.get_or_init(|| Labels { join: words(JOIN), following: FOLLOWING.iter().flat_map(|k| words(k)).collect(), options: words(OPTIONS) })
}

/// What each translation of `key` says before its first value, the blanks after it taken off;
/// never empty (it would start every line).
fn words(key: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in crate::i18n::in_every_language(key) {
        let label = t.split('{').next().unwrap_or_default().trim_end();
        if !label.trim().is_empty() && !out.iter().any(|l| l == label) {
            out.push(label.to_string());
        }
    }
    out
}

/// How many lines a meeting's block takes at the start of `lines`, and its link to join, when one
/// starts there: a join line, then the lines that start as the block's others do, [`BLOCK_LINES`]
/// in all at most.
fn block_at<'a>(lines: &[&'a str]) -> Option<(usize, &'a str)> {
    let l = labels();
    let join = link_after(lines.first().copied()?, &l.join)?;
    let follows = lines.iter().skip(1).take(BLOCK_LINES - 1).take_while(|line| l.following.iter().any(|w| line.trim().starts_with(w.as_str())));
    Some((1 + follows.count(), join))
}

/// The link after one of `labels` on `line` (both trimmed), when it's all that follows and
/// Teams' own.
fn link_after<'a>(line: &'a str, labels: &[String]) -> Option<&'a str> {
    let line = line.trim();
    labels.iter().filter_map(|w| line.strip_prefix(w.as_str())).map(str::trim).find(|rest| teams_meeting_url(rest))
}
