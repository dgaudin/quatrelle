// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What a message is, as the list shows it: its kind (a report, a meeting message, an automatic
//! reply, a recall…), what was done with it (replied, replied to all, forwarded) and what protects
//! it (S/MIME or OpenPGP, as its header says), with the one icon the list's column shows for them.
//!
//! The kind is said by the server where it keeps one (Exchange's message class and icon), else by
//! the message's source once it is read; until then it is [`MessageKind::Unknown`], which isn't
//! written, so a cache from before reads the same and an older Quatrelle reads this one's.

use serde::{Deserialize, Serialize};

use crate::{Flag, Flags, Summary};

/// The keyword a reply to all sets beside `\Answered`: IMAP and JMAP have no flag of their own for
/// it.
pub const REPLIED_ALL: &str = "$RepliedAll";

/// The longest message class read: [MS-OXCMSG] 2.2.1.3 keeps it under 256 characters, ASCII.
const MAX_CLASS: usize = 255;

/// What a message is beyond mail, as its server or its source says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageKind {
    /// Not said yet: the server keeps no kind, and the source hasn't been read.
    #[default]
    Unknown,
    /// Ordinary mail.
    Plain,
    /// An automatic reply (out of the office).
    AutoReply,
    /// A report that a message couldn't be delivered.
    NotDelivered,
    /// A report that a message's delivery is delayed.
    Delayed,
    /// A report that a message was delivered.
    Delivered,
    /// A receipt saying a message was read.
    ReadReceipt,
    /// A receipt saying a message was deleted without being read.
    NotRead,
    /// A request to recall a message sent earlier.
    Recall,
    /// A report that a recall succeeded.
    RecallDone,
    /// A report that a recall failed.
    RecallFailed,
    /// A meeting request (or its update).
    MeetingRequest,
    /// A meeting's cancellation.
    MeetingCanceled,
    /// An answer to a meeting request: accepted.
    MeetingAccepted,
    /// An answer to a meeting request: tentative.
    MeetingTentative,
    /// An answer to a meeting request: declined.
    MeetingDeclined,
}

impl MessageKind {
    pub fn is_unknown(&self) -> bool {
        *self == MessageKind::Unknown
    }

    /// From Exchange's message class and icon: the class first (its first 255 characters, ASCII
    /// without case), the icon for a class that is empty, `IPM.Note` or under it. Never
    /// [`MessageKind::Unknown`]: what Exchange says nothing special about is [`MessageKind::Plain`].
    pub fn from_exchange(class: &str, icon: Option<i64>) -> MessageKind {
        use MessageKind as K;
        let class = class_key(class);
        let under = |base: &str| is_class(&class, base);
        if under("report") {
            return report(&class);
        }
        // Meeting messages: [MS-OXOCAL] 2.2.6.1 (a request), 2.2.7.1 (an answer), 2.2.8.1 (a
        // cancellation).
        if under("ipm.schedule.meeting.request") {
            return K::MeetingRequest;
        }
        if under("ipm.schedule.meeting.canceled") {
            return K::MeetingCanceled;
        }
        if under("ipm.schedule.meeting.resp.pos") {
            return K::MeetingAccepted;
        }
        if under("ipm.schedule.meeting.resp.tent") {
            return K::MeetingTentative;
        }
        if under("ipm.schedule.meeting.resp.neg") {
            return K::MeetingDeclined;
        }
        // A recall and its reports, and an automatic reply: the classes Microsoft Learn lists
        // for Outlook's items (Outlook VBA, "Item Types and Message Classes"). No document names
        // what follows `IPM.Recall.Report`: how a recall went is read from the icon alone.
        if under("ipm.outlook.recall") {
            return K::Recall;
        }
        if under("ipm.recall.report") {
            return match icon {
                Some(0x10C) => K::RecallDone,
                Some(0x10D) => K::RecallFailed,
                _ => K::Plain,
            };
        }
        if under("ipm.note.rules.ooftemplate") {
            return K::AutoReply;
        }
        if class.is_empty() || under("ipm.note") { by_icon(icon) } else { K::Plain }
    }

    /// The kind kept (`self`) once the server says `server`: what the server says replaces it,
    /// unless that is nothing special ([`MessageKind::Plain`] or [`MessageKind::Unknown`]), which
    /// never undoes a kind kept.
    pub fn told_by_server(self, server: MessageKind) -> MessageKind {
        match server {
            MessageKind::Unknown | MessageKind::Plain => self,
            said => said,
        }
    }

    /// The kind kept (`self`) once the message's source says `source`: the source settles a kind
    /// not said yet or ordinary, and an automatic reply when it says more (a report that also
    /// says it was sent automatically); a kind the server said stays.
    pub fn settled_by_source(self, source: MessageKind) -> MessageKind {
        use MessageKind as K;
        match (self, source) {
            (_, K::Unknown) | (K::AutoReply, K::Plain) => self,
            (K::Unknown | K::Plain | K::AutoReply, said) => said,
            _ => self,
        }
    }
}

/// A message class as it's compared ([MS-OXCMSG] 2.2.1.3: without case): trimmed, its first
/// [`MAX_CLASS`] characters, ASCII letters in lower case. Only for comparing: never kept.
fn class_key(class: &str) -> String {
    class.trim().chars().take(MAX_CLASS).collect::<String>().to_ascii_lowercase()
}

/// Whether `class` is `base` or a class under it: [MS-OXCMSG] 2.2.1.3 reads a class in groups
/// between periods (`IPM.Note.SMIME` is under `IPM.Note`, `IPM.Notes` isn't).
fn is_class(class: &str, base: &str) -> bool {
    class.strip_prefix(base).is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

/// A report's kind by the last part of its class (`class` as [`class_key`] makes it):
/// `REPORT.<class>.IPNRN`, `.IPNNRN`, `.DR` or `.NDR` ([MS-OXOMSG] 2.2.2.1); a delivery status
/// notification's `delayed`, `relayed` and `expanded` make `.Delayed.DR`, `.Relayed.DR` and
/// `.Expanded.DR` ([MS-OXCMAIL] 2.2.3.7.1.1), and Quatrelle's own `.pst` export writes a delay
/// as `.DelayedDR` (`net/src/data_files/pst_export/kinds.rs`), read back the same.
fn report(class: &str) -> MessageKind {
    use MessageKind as K;
    let ends = |suffix: &str| class.ends_with(suffix);
    if ends(".ndr") {
        K::NotDelivered
    } else if ends(".delayed.dr") || ends(".delayeddr") {
        K::Delayed
    } else if ends(".dr") {
        K::Delivered
    } else if ends(".ipnrn") {
        K::ReadReceipt
    } else if ends(".ipnnrn") {
        K::NotRead
    } else {
        K::Plain
    }
}

/// A note's kind by its icon: [MS-OXOMSG] 2.2.1.10 for mail, [MS-OXOCAL] 2.2.1.49 for meeting
/// messages. Any other value (read, unread, replied, forwarded, -1, a value of no meaning) is
/// ordinary mail.
fn by_icon(icon: Option<i64>) -> MessageKind {
    use MessageKind as K;
    match icon {
        Some(0x108) => K::Delivered,
        Some(0x109) => K::ReadReceipt,
        Some(0x10A) => K::NotDelivered,
        Some(0x10B) => K::NotRead,
        Some(0x10C) => K::RecallDone,
        Some(0x10D) => K::RecallFailed,
        Some(0x11B) => K::AutoReply,
        Some(0x11C) => K::Recall,
        Some(0x404 | 0x409) => K::MeetingRequest,
        Some(0x405) => K::MeetingAccepted,
        Some(0x406) => K::MeetingDeclined,
        Some(0x407) => K::MeetingTentative,
        Some(0x408) => K::MeetingCanceled,
        _ => K::Plain,
    }
}

/// The kind as written, or [`MessageKind::Unknown`] for any value this version doesn't know (a
/// newer version's, a damaged disk's: a string, a number, `null`, an object): the page it's in
/// still reads, rather than being set aside and its mail read again from the server.
pub(crate) fn lenient<'de, D: serde::Deserializer<'de>>(d: D) -> Result<MessageKind, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(MessageKind::deserialize(v).unwrap_or_default())
}

/// What a message's header says protects it: S/MIME (RFC 8551 §3.2) or OpenPGP (RFC 3156).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Smime {
    #[default]
    None,
    Signed,
    Encrypted,
}

impl Smime {
    pub fn is_none(&self) -> bool {
        *self == Smime::None
    }

    /// From a message's top-level `Content-Type`: its type and subtype, and its `protocol`,
    /// `smime-type` and `name` parameters.
    pub fn from_content_type(kind: &str, subtype: &str, protocol: Option<&str>, smime_type: Option<&str>, name: Option<&str>) -> Smime {
        let (kind, subtype) = (kind.to_ascii_lowercase(), subtype.to_ascii_lowercase());
        let lower = |v: Option<&str>| v.map(|s| s.trim().to_ascii_lowercase()).unwrap_or_default();
        if kind == "multipart" && subtype == "signed" {
            return match lower(protocol).as_str() {
                "application/pkcs7-signature" | "application/x-pkcs7-signature" | "application/pgp-signature" => Smime::Signed,
                _ => Smime::None,
            };
        }
        // PGP/MIME (RFC 3156) shows the same way.
        if kind == "multipart" && subtype == "encrypted" && lower(protocol) == "application/pgp-encrypted" {
            return Smime::Encrypted;
        }
        let p7m = lower(name).ends_with(".p7m");
        if kind == "application" && (subtype == "pkcs7-mime" || subtype == "x-pkcs7-mime" || (subtype == "octet-stream" && p7m)) {
            return if lower(smime_type) == "signed-data" { Smime::Signed } else { Smime::Encrypted };
        }
        Smime::None
    }

    /// From Exchange's message class ([MS-OXOSMIME] 2.1.3.1.2.2, 2.1.3.2.2.2): clear-signed for
    /// `IPM.Note.SMIME.MultipartSigned`, encrypted for `IPM.Note.SMIME` (which may also be an
    /// opaque signature: the source settles it when it comes).
    pub fn from_class(class: &str) -> Smime {
        let class = class_key(class);
        if is_class(&class, "ipm.note.smime.multipartsigned") {
            Smime::Signed
        } else if is_class(&class, "ipm.note.smime") {
            Smime::Encrypted
        } else {
            Smime::None
        }
    }
}

impl Flags {
    /// Replied to all: `\Answered` with [`REPLIED_ALL`] (the keyword alone is no reply).
    pub fn replied_all(&self) -> bool {
        self.answered && self.has(REPLIED_ALL)
    }
}

impl Flag {
    /// [`Flag::set`], a reply to all with it: setting `\Answered` with `all` sets [`REPLIED_ALL`]
    /// too, taking it off takes both off, and setting it without `all` leaves the keyword as it
    /// is (IMAP keeps each mark as a fact of its own).
    pub fn apply(self, flags: &mut Flags, on: bool, all: bool) {
        self.set(flags, on);
        if self == Flag::Answered && (!on || all) {
            flags.set_keyword(REPLIED_ALL, on);
        }
    }
}

/// The icon the list shows for a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListIcon {
    RepliedAll,
    Replied,
    Forwarded,
    /// What the message is ([`MessageKind::Unknown`] and [`MessageKind::Plain`] are never shown).
    Kind(MessageKind),
    Encrypted,
    Signed,
}

impl ListIcon {
    /// Shown in the colour that asks for attention: a message not delivered, a meeting canceled.
    pub fn urgent(self) -> bool {
        matches!(self, ListIcon::Kind(MessageKind::NotDelivered | MessageKind::MeetingCanceled))
    }
}

/// Where a kind stands among what the list can say of a message.
enum Rank {
    /// Before what was done with it: something to act on (a failure, a meeting).
    First,
    /// After what was done with it: it only informs.
    After,
    /// Nothing to show.
    Not,
}

fn rank(kind: MessageKind) -> Rank {
    use MessageKind as K;
    match kind {
        K::NotDelivered | K::Delayed | K::MeetingRequest | K::MeetingCanceled | K::MeetingAccepted | K::MeetingTentative | K::MeetingDeclined => {
            Rank::First
        }
        K::AutoReply | K::ReadReceipt | K::NotRead | K::Delivered | K::Recall | K::RecallDone | K::RecallFailed => Rank::After,
        K::Unknown | K::Plain => Rank::Not,
    }
}

/// Every mark of a message, by priority, a place for each (`None` when it isn't true).
fn marks(flags: &Flags, kind: MessageKind, smime: Smime) -> [Option<ListIcon>; 5] {
    let reply = if flags.replied_all() { Some(ListIcon::RepliedAll) } else { flags.answered.then_some(ListIcon::Replied) };
    let (first, after) = match rank(kind) {
        Rank::First => (Some(ListIcon::Kind(kind)), None),
        Rank::After => (None, Some(ListIcon::Kind(kind))),
        Rank::Not => (None, None),
    };
    let smime = match smime {
        Smime::Encrypted => Some(ListIcon::Encrypted),
        Smime::Signed => Some(ListIcon::Signed),
        Smime::None => None,
    };
    [first, reply, flags.forwarded.then_some(ListIcon::Forwarded), after, smime]
}

/// The one icon the list shows for a message, the first by priority of [`list_said`]; S/MIME
/// only `with_smime` (the two-line view has its own mark for it).
pub fn list_icon(flags: &Flags, kind: MessageKind, smime: Smime, with_smime: bool) -> Option<ListIcon> {
    marks(flags, kind, smime).into_iter().flatten().find(|i| with_smime || !matches!(i, ListIcon::Encrypted | ListIcon::Signed))
}

/// Everything true of a message, in the order of priority (what its icon's tooltip says), S/MIME
/// always included: a report not delivered or delayed, a meeting message; then what was done
/// with it (replied to all or replied, then forwarded); then an automatic reply, a receipt, a
/// report of delivery, a recall; then encrypted or signed.
pub fn list_said(flags: &Flags, kind: MessageKind, smime: Smime) -> Vec<ListIcon> {
    marks(flags, kind, smime).into_iter().flatten().collect()
}

impl Summary {
    /// The one icon the list shows for it ([`list_icon`]).
    pub fn list_icon(&self, with_smime: bool) -> Option<ListIcon> {
        list_icon(&self.flags, self.kind, self.smime, with_smime)
    }
}

#[cfg(test)]
mod tests;
