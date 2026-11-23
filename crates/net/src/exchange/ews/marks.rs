// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! An item's marks as Exchange keeps them, and what Outlook writes when it replies or forwards.

use super::*;

pub(super) fn flags(it: &El) -> Flags {
    let mut keywords: Vec<String> = it
        .child("Categories")
        .map(|c| c.children("String").map(|s| quatrelle_model::keyword_for(&s.text)).filter(|k| quatrelle_model::is_keyword(k)).collect())
        .unwrap_or_default();
    keywords.sort_by_key(|k| k.to_ascii_lowercase());
    keywords.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    keywords.truncate(ITEM_CATEGORIES);
    // Outlook's Mark Complete: status 1 (2 is flagged), and when it was done.
    let status = extended(it, "0x1090");
    if status == Some(1) {
        keywords.push(quatrelle_model::FOLLOW_UP_COMPLETE.to_string());
        keywords.sort_by_key(|k| k.to_ascii_lowercase());
    }
    let completed = extended_text(it, "0x1091").and_then(jmap::utc_date).filter(|t| *t > 0 && status == Some(1)).unwrap_or(0);
    // The last thing done with it, as Outlook writes it ([MS-OXOMSG] `PidTagLastVerbExecuted`):
    // 102 replied to the sender, 103 to all, 104 forwarded; another (a vote) is neither. Only
    // the last is kept, so a reply followed by a forward shows the forward, as Outlook does.
    let verb = extended(it, "0x1081");
    let answered = matches!(verb, Some(VERB_REPLY | VERB_REPLY_ALL));
    let forwarded = verb == Some(VERB_FORWARD);
    let acted = extended_text(it, "0x1082").filter(|_| answered || forwarded).map_or(0, verb_time);
    Flags {
        seen: it.text_of("IsRead").is_some_and(|v| v == "true"),
        flagged: status == Some(2),
        answered,
        forwarded,
        draft: extended(it, "0x0E07").is_some_and(|f| f & 8 != 0),
        keywords,
        completed,
        acted,
        ..Default::default()
    }
}

/// When it was done (`PidTagLastVerbExecutionTime`), in Unix seconds: a time that isn't one,
/// before 1970, or MAPI's "none" (4501) is none (0).
pub(crate) fn verb_time(text: &str) -> i64 {
    let none = quatrelle_model::dates::days_from_civil(4501, 1, 1).saturating_mul(86_400);
    jmap::utc_date(text).filter(|t| *t > 0 && *t < none).unwrap_or(0)
}

/// The verb and the icon Outlook writes on the original of a reply (`forward`: a forward):
/// `PidTagLastVerbExecuted` 102 or 104 and `PidTagIconIndex` 0x105 or 0x106 ([MS-OXOMSG]). The
/// engine doesn't say a reply was to all (103): it's written as a reply to the sender, which
/// Outlook shows alike.
pub(crate) fn replied_verb(forward: bool) -> (i64, i64) {
    if forward { (VERB_FORWARD, 0x106) } else { (VERB_REPLY, 0x105) }
}

/// `PidTagLastVerbExecuted` for a reply to the sender ([MS-OXOMSG]; `NOTEIVERB_REPLYTOSENDER`).
pub(crate) const VERB_REPLY: i64 = 102;
/// … for a reply to all (`NOTEIVERB_REPLYTOALL`).
pub(crate) const VERB_REPLY_ALL: i64 = 103;
/// … for a forward (`NOTEIVERB_FORWARD`).
pub(crate) const VERB_FORWARD: i64 = 104;

/// The marks of every message in an answer, as [`flags`] reads them (the fuzz target reads
/// them so).
pub fn marks_listed(doc: &El) -> Vec<Flags> {
    let mut items = Vec::new();
    doc.all("Message", &mut items);
    items.into_iter().map(flags).collect()
}

/// A message's marks as the server has them now: read, flagged or complete (and when), a draft
/// and its categories (Exchange keeps them by name, as [`flags`] reads them); the keywords that
/// are Quatrelle's own (`$Pinned`, `$Junk`…) stay. Replied to or forwarded, the last of them and
/// when, once the server says one; until then (a reply made here by a version that didn't tell
/// Exchange, or not told yet) the arrow kept here stays. Whether anything changed.
pub(super) fn take_marks(m: &mut Flags, server: &Flags) -> bool {
    use quatrelle_model::FOLLOW_UP_COMPLETE;
    let mut next = m.clone();
    next.seen = server.seen;
    next.flagged = server.flagged;
    next.draft = server.draft;
    next.keywords.retain(|k| k.starts_with('$') && !k.eq_ignore_ascii_case(FOLLOW_UP_COMPLETE));
    for k in &server.keywords {
        next.set_keyword(k, true);
    }
    next.completed = server.completed;
    if server.answered || server.forwarded {
        next.answered = server.answered;
        next.forwarded = server.forwarded;
        next.acted = server.acted;
    }
    let changed = next != *m;
    *m = next;
    changed
}
