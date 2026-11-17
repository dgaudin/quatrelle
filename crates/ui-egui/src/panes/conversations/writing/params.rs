// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What the box hands the engine and takes back from it: `teams.send`'s and `teams.edit`'s params
//! (the byte ranges of the text's bold, italic and mentions, [`pairs`], [`mention_pairs`]), read
//! back from those of a message refused ([`ranges_of`], [`mentions_of`], [`styled`]), and the
//! refusal put back in the box and said in the window's words ([`refused`], [`Refusal`]).
//!
//! What is hostile here: the params of a refusal (the box's own, or a script's): ranges and
//! mentions checked against the text's characters, at the command's caps.

use quatrelle_engine::{CmdError, TEAMS_LEFT, TEAMS_MAY_BE_IN_TEAMS, TEAMS_NOTHING_LEFT, TEAMS_RANGES_MAX, TEAMS_TOO_LONG, TEAMS_TOO_MANY};

use super::*;

/// Why something the box asked for wasn't taken, as the window says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    TooLong,
    TooMany,
    Left,
    MayBeInTeams,
    /// Anything else of a message (the chat gone from the list, say).
    NotSent,
    /// An edit leaving nothing of the message: deleting it is what removes it.
    NothingLeft,
    /// Anything else of an edit (the message no longer one the box may edit, say).
    NotEdited,
    /// ↑ on a last message the box can't hold.
    LastNotEditable,
}

impl Refusal {
    /// The engine's refusal of `id` as one of the window's (its English words are never drawn);
    /// none for a Retry or a Delete refused for a reason there's nothing to say of.
    fn of(id: &str, e: &CmdError) -> Option<Refusal> {
        let words = match e {
            CmdError::Params(m) | CmdError::Disabled(m) | CmdError::Failed(m) => m.as_str(),
            CmdError::Unknown(_) => "",
        };
        match words {
            TEAMS_TOO_LONG => Some(Refusal::TooLong),
            TEAMS_TOO_MANY => Some(Refusal::TooMany),
            TEAMS_MAY_BE_IN_TEAMS => Some(Refusal::MayBeInTeams),
            TEAMS_LEFT => Some(Refusal::Left),
            TEAMS_NOTHING_LEFT => Some(Refusal::NothingLeft),
            _ if id == "teams.send" => Some(Refusal::NotSent),
            _ if id == "teams.edit" => Some(Refusal::NotEdited),
            _ => None,
        }
    }

    pub(super) fn sentence(self) -> &'static str {
        match self {
            Refusal::TooLong => tl!("This message is too long for Teams"),
            Refusal::TooMany => tl!("Too many messages are waiting"),
            Refusal::Left => tl!("You're no longer in this conversation"),
            Refusal::MayBeInTeams => tl!("This message may already be in Teams"),
            Refusal::NotSent => tl!("This message couldn't be sent"),
            Refusal::NothingLeft => tl!("To remove a message, delete it"),
            Refusal::NotEdited => tl!("This message couldn't be edited"),
            Refusal::LastNotEditable => tl!("Your last message can't be edited here"),
        }
    }
}

/// Ranges as `teams.send` takes them: `[[from, to]]`.
pub(super) fn pairs(ranges: &[Range<usize>]) -> Value {
    Value::Array(ranges.iter().map(|r| json!([r.start, r.end])).collect())
}

/// Mentions as `teams.send` takes them: `[[from, to, "user id"]]`.
pub(super) fn mention_pairs(mentions: &[(Range<usize>, String)]) -> Value {
    Value::Array(mentions.iter().map(|(r, user)| json!([r.start, r.end, user])).collect())
}

/// `text`'s styles: bold and italic on the byte ranges given (one off the text's characters is
/// left out).
pub(super) fn styled(text: &str, bold: &[Range<usize>], italic: &[Range<usize>]) -> Styles {
    let mut st = Styles::plain(text);
    if bold.is_empty() && italic.is_empty() {
        return st;
    }
    let at = Chars::of(text);
    let looks: [(&[Range<usize>], fn(&mut Style)); 2] = [(bold, |s| s.bold = true), (italic, |s| s.italic = true)];
    for (ranges, set) in looks {
        for r in ranges.iter().filter_map(|r| at.range(r)).filter(|r| !r.is_empty()) {
            st.restyle(text, r, |s| {
                let mut s = s.clone();
                set(&mut s);
                s
            });
        }
    }
    st
}

/// The byte ranges of style `key` in `teams.send`'s or `teams.edit`'s params (at most
/// [`TEAMS_RANGES_MAX`]; one off `text`'s characters is left out).
pub(super) fn ranges_of(text: &str, params: &Value, key: &str) -> Vec<Range<usize>> {
    let given = params.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let number = |v: Option<&Value>| v.and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok());
    given
        .iter()
        .take(TEAMS_RANGES_MAX)
        .filter_map(|r| Some(number(r.get(0))?..number(r.get(1))?))
        .filter(|r| r.start < r.end && text.is_char_boundary(r.start) && text.is_char_boundary(r.end))
        .collect()
}

/// The mentions in `teams.send`'s or `teams.edit`'s params: byte ranges of `text` and user ids
/// (at most [`MAX_MENTIONS`]; one off its characters, or whose id is empty or longer than
/// [`MAX_ID`], is left out).
pub(super) fn mentions_of(text: &str, params: &Value) -> Vec<(Range<usize>, String)> {
    let given = params.get("mentions").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let number = |v: Option<&Value>| v.and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok());
    let user = |v: Option<&Value>| v.and_then(Value::as_str).filter(|u| !u.is_empty() && u.len() <= MAX_ID).map(str::to_string);
    given
        .iter()
        .take(MAX_MENTIONS)
        .filter_map(|m| Some((number(m.get(0))?..number(m.get(1))?, user(m.get(2))?)))
        .filter(|(r, _)| r.start < r.end && text.is_char_boundary(r.start) && text.is_char_boundary(r.end))
        .collect()
}

/// What the engine refused of the box's (`teams.send`, `.edit`, `.retry` or `.discard`, with
/// `params`; `open` is the chat on screen, which Retry and Delete are of): a message sent goes
/// back in its chat's box with its bold, italic and mentions, before what was typed since (into
/// the words set aside, while an edit is under way); an edit goes back as an edit of its message
/// again; why is said under the box.
pub(crate) fn refused(c: &mut ConversationsUi, open: Option<&str>, id: &str, params: &Value, e: &CmdError) {
    let chat = if matches!(id, "teams.send" | "teams.edit") { params.get("chat").and_then(Value::as_str) } else { open };
    let Some(chat) = chat.filter(|c| !c.is_empty() && c.len() <= MAX_ID) else { return };
    let said = Refusal::of(id, e);
    let d = draft(c, chat);
    if id == "teams.edit" {
        editing::refused_edit(d, params);
    } else if id == "teams.send"
        && let Some(text) = params.get("text").and_then(Value::as_str)
    {
        let styles = styled(text, &ranges_of(text, params, "bold"), &ranges_of(text, params, "italic"));
        let mentions = mentions::of_bytes(text, &mentions_of(text, params));
        match d.editing.as_mut() {
            Some(edit) => edit.set_aside.put_back(text, styles, mentions),
            None => {
                let mut words = Words::take(d);
                words.put_back(text, styles, mentions);
                words.into_box(d);
            }
        }
    }
    if said.is_some() {
        d.said = said;
    }
}
