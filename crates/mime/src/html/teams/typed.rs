// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What a chat's box holds (its text, the bold and italic ranges of it, the people @mentioned),
//! composed into a message and read back from one, for an edit.

use std::ops::Range;

use quatrelle_model::pim::teams::{BlockKind, ChatText, MAX_ID, MAX_MENTIONS, Mention, one_line};

use super::write::compose;

/// What the box holds: `text`, with byte ranges of it in `bold` and `italic`, and in `mentions`
/// the range of each person @mentioned and that member's user id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Typed {
    pub text: String,
    pub bold: Vec<Range<usize>>,
    pub italic: Vec<Range<usize>>,
    pub mentions: Vec<(Range<usize>, String)>,
}

/// Whether `text` and `user` can be a mention: the text one line as it stands (so the member's
/// name as it was picked), a user id there is, not longer than [`MAX_ID`], no control character.
fn usable(text: &str, user: &str) -> bool {
    !text.is_empty() && one_line(text) == text && !user.is_empty() && user.chars().count() <= MAX_ID && !user.chars().any(char::is_control)
}

/// [`teams_compose`](super::teams_compose) of what the box holds, and with it the mentions: each
/// range of `t.mentions` one run (`mention: Some(n)`, `n` from 0 in the text's order) in the bold
/// and italic of its first character, with the [`Mention`] to send. A range is left as plain text
/// when it is off the text or a character's boundary, empty, holds a line break or a control
/// character, overlaps one before it, isn't one line as it stands, its user id is empty, longer
/// than [`MAX_ID`] or holds a control character; and past [`MAX_MENTIONS`].
pub fn teams_compose_typed(t: &Typed) -> (ChatText, Vec<Mention>) {
    let mut kept: Vec<(&Range<usize>, &String, &str)> =
        t.mentions.iter().filter_map(|(r, user)| t.text.get(r.clone()).filter(|text| usable(text, user)).map(|text| (r, user, text))).collect();
    kept.sort_by_key(|(r, ..)| r.start);
    let mut marks: Vec<(Range<usize>, u32)> = Vec::new();
    let mut mentions = Vec::new();
    for (range, user, text) in kept {
        if marks.last().is_some_and(|(last, _)| range.start < last.end) || mentions.len() >= MAX_MENTIONS {
            continue;
        }
        let id = mentions.len() as u32;
        marks.push((range.clone(), id));
        mentions.push(Mention { id, text: text.to_string(), user_id: Some(user.clone()) });
    }
    let text = compose(&t.text, &t.bold, &t.italic, &marks);
    // What was cut off for being too long to send leaves its mentions without a run.
    mentions.retain(|m| text.blocks.iter().flat_map(|b| &b.runs).any(|r| r.mention == Some(m.id)));
    (text, mentions)
}

/// A message read back into what the box holds, to be edited: its blocks joined by `\n` (an empty
/// paragraph an empty line), its runs' line breaks kept, its bold, italic and mention runs as
/// byte ranges. `None` when the box can't hold it whole: a block that isn't a paragraph, a run
/// underlined, struck, code, a picture, an attachment's place, a link whose text isn't its
/// address, or a mention that isn't one of `mentions` having a user id.
pub fn teams_typed(text: &ChatText, mentions: &[Mention]) -> Option<Typed> {
    let mut typed = Typed::default();
    for (n, block) in text.blocks.iter().enumerate() {
        if block.kind != BlockKind::Paragraph {
            return None;
        }
        if n > 0 {
            typed.text.push('\n');
        }
        for run in &block.runs {
            if run.underline || run.strike || run.code || run.hosted.is_some() || run.outside.is_some() || run.attachment.is_some() {
                return None;
            }
            if run.link.as_ref().is_some_and(|link| *link != run.text) {
                return None;
            }
            let user = match run.mention {
                Some(id) => Some(mentions.iter().find(|m| m.id == id)?.user_id.as_deref().filter(|user| usable(&run.text, user))?),
                None => None,
            };
            let start = typed.text.len();
            typed.text.push_str(&run.text);
            let range = start..typed.text.len();
            if range.is_empty() {
                continue;
            }
            for (on, ranges) in [(run.bold, &mut typed.bold), (run.italic, &mut typed.italic)] {
                if !on {
                    continue;
                }
                match ranges.last_mut() {
                    Some(last) if last.end == range.start => last.end = range.end,
                    _ => ranges.push(range.clone()),
                }
            }
            if let Some(user) = user {
                typed.mentions.push((range, user.to_string()));
            }
        }
    }
    Some(typed)
}
