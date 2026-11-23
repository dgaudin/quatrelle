// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! An item's marks as Exchange keeps them, and what Outlook writes when it replies or forwards;
//! what a message is, as Exchange says it.

use quatrelle_model::{MessageKind, Smime};

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
    // To all: the keyword that says so, with the reply.
    if verb == Some(VERB_REPLY_ALL) {
        keywords.push(quatrelle_model::REPLIED_ALL.to_string());
        keywords.sort_by_key(|k| k.to_ascii_lowercase());
    }
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

/// The verb and the icon Outlook writes on the original of a reply (`forward`: a forward; `all`:
/// a reply to all): `PidTagLastVerbExecuted` 102, 103 or 104 ([MS-OXOMSG] 2.2.1.14) and
/// `PidTagIconIndex` 0x105 or 0x106 ([MS-OXOMSG] 2.2.1.10, which has no icon of its own for a
/// reply to all: it's the reply's). A forward is one whatever `all` says.
pub(crate) fn replied_verb(forward: bool, all: bool) -> (i64, i64) {
    match (forward, all) {
        (true, _) => (VERB_FORWARD, 0x106),
        (false, true) => (VERB_REPLY_ALL, 0x105),
        (false, false) => (VERB_REPLY, 0x105),
    }
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
/// Exchange, or not told yet) the arrow kept here stays; told, the reply to all's keyword follows
/// the server's verb (a reply to the sender or a forward made in Outlook takes it off). Whether
/// anything changed.
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
        next.set_keyword(quatrelle_model::REPLIED_ALL, server.replied_all());
    }
    let changed = next != *m;
    *m = next;
    changed
}

/// What a message is, as Exchange says it ([`KIND_FIELDS`]): its kind
/// ([`MessageKind::from_exchange`], by its class then its icon) and what protects it
/// ([`Smime::from_class`]). A class or an icon said wrong, or not said, is ordinary mail.
pub(super) fn kind_of(it: &El) -> (MessageKind, Smime) {
    let class = it.text_of("ItemClass").unwrap_or_default();
    (MessageKind::from_exchange(class, extended(it, "0x1080")), Smime::from_class(class))
}

/// A message the pass reading what mail kept before is finds: its id, its marks, what it is and
/// what protects it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Found {
    pub id: String,
    pub flags: Flags,
    pub kind: MessageKind,
    pub smime: Smime,
}

/// The starts of the classes of messages that may be more than mail ([MS-OXCMSG] 2.2.1.3, compared
/// without case): meeting messages ([MS-OXOCAL] 2.2.6.1, 2.2.7.1, 2.2.8.1), reports ([MS-OXOMSG]
/// 2.2.2.1), S/MIME ([MS-OXOSMIME] 2.1.3.1.2.2), automatic replies, recalls and their reports
/// (Microsoft Learn, Outlook's "Item Types and Message Classes"). What each is, is
/// [`MessageKind::from_exchange`]'s to say.
const KIND_CLASSES: [&str; 6] =
    ["IPM.Schedule.Meeting", "REPORT.", "IPM.Note.SMIME", "IPM.Note.Rules.OofTemplate", "IPM.Outlook.Recall", "IPM.Recall.Report"];

/// What [`Client::kinds`] asks for: the classes above by their start, without case (`IPM.Note` is
/// mail, and so are the posts of public folders: neither is asked for), or replied to all
/// (`PidTagLastVerbExecuted` 103, which this version reads as a reply to all and the version
/// before as a reply).
pub(super) fn kinds_condition() -> String {
    let start = |class: &str| {
        format!(
            "<t:Contains ContainmentMode=\"Prefixed\" ContainmentComparison=\"IgnoreCase\"><t:FieldURI FieldURI=\"item:ItemClass\"/>\
             <t:Constant Value=\"{class}\"/></t:Contains>"
        )
    };
    let classes: String = KIND_CLASSES.iter().map(|c| start(c)).collect();
    format!(
        "<t:Or>{classes}<t:IsEqualTo><t:ExtendedFieldURI PropertyTag=\"0x1081\" PropertyType=\"Integer\"/>\
         <t:FieldURIOrConstant><t:Constant Value=\"{VERB_REPLY_ALL}\"/></t:FieldURIOrConstant></t:IsEqualTo></t:Or>"
    )
}

/// What the pass finds of an item: nothing without an id.
fn found_of(it: &El) -> Option<Found> {
    let (kind, smime) = kind_of(it);
    Some(Found { id: it.child("ItemId")?.attr("Id")?.to_string(), flags: flags(it), kind, smime })
}

impl Client {
    /// [`Client::newest`], those that may be more than mail (meeting messages, reports, S/MIME,
    /// automatic replies, recalls and their reports, by the start of their class) or were replied
    /// to all (`PidTagLastVerbExecuted` 103): their ids, marks and kinds only, a page of `page` at
    /// a time.
    pub(super) fn kinds(&self, folder: &str, limit: usize, since: Option<i64>, page: usize) -> Result<Vec<Found>> {
        self.newest_where(folder, limit, since, mailbox::Only::Kinds, page, found_of)
    }
}
