// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What a message's list icon is, its colour, and the words that say it (the tip under the
//! pointer, and what a screen reader hears).

use egui::Color32;
use quatrelle_engine::Session;
use quatrelle_engine::model::{ListIcon, MessageKind as K, Summary};

use crate::icons::Icon;
use crate::theme;

/// The picture for what a list icon says.
pub(crate) fn icon_of(i: ListIcon) -> Icon {
    match i {
        ListIcon::RepliedAll => Icon::ReplyAll,
        ListIcon::Replied => Icon::Reply,
        ListIcon::Forwarded => Icon::Forward,
        ListIcon::Encrypted => Icon::Lock,
        ListIcon::Signed => Icon::Seal,
        ListIcon::Kind(k) => match k {
            K::ReadReceipt => Icon::Read,
            K::NotRead => Icon::Ignore,
            K::MeetingRequest => Icon::Calendar,
            K::Recall | K::RecallDone | K::RecallFailed => Icon::Recall,
            K::AutoReply => Icon::AutoReply,
            K::NotDelivered => Icon::NotDelivered,
            K::Delayed => Icon::Delayed,
            K::Delivered => Icon::Delivered,
            K::MeetingCanceled => Icon::MeetingCanceled,
            K::MeetingAccepted => Icon::MeetingAccepted,
            K::MeetingTentative => Icon::MeetingTentative,
            K::MeetingDeclined => Icon::MeetingDeclined,
            // Never shown: the engine gives no icon for them.
            K::Unknown | K::Plain => Icon::Read,
        },
    }
}

/// Dim, but for what asks for attention: a message not delivered, a meeting canceled.
pub(crate) fn color_of(i: ListIcon, t: &theme::Tokens) -> Color32 {
    if i.urgent() { t.danger } else { t.dim }
}

/// The words for an icon: its tip, and what a screen reader hears.
pub(crate) fn label(i: ListIcon) -> &'static str {
    match i {
        ListIcon::RepliedAll => tl!("Replied to all"),
        ListIcon::Replied => tl!("Replied"),
        ListIcon::Forwarded => tl!("Forwarded"),
        ListIcon::Encrypted => tl!("Encrypted"),
        ListIcon::Signed => tl!("Signed"),
        ListIcon::Kind(k) => match k {
            K::AutoReply => tl!("Automatic reply"),
            K::NotDelivered => tl!("Not delivered"),
            K::Delayed => tl!("Delivery delayed"),
            K::Delivered => tl!("Delivered"),
            K::ReadReceipt => tl!("Read receipt"),
            K::NotRead => tl!("Deleted without being read"),
            K::Recall => tl!("Recall request"),
            K::RecallDone => tl!("Recall succeeded"),
            K::RecallFailed => tl!("Recall failed"),
            K::MeetingRequest => tl!("Meeting request"),
            K::MeetingCanceled => tl!("Meeting canceled"),
            K::MeetingAccepted => tl!("Answer: accepted"),
            K::MeetingTentative => tl!("Answer: tentative"),
            K::MeetingDeclined => tl!("Answer: declined"),
            K::Unknown | K::Plain => "",
        },
    }
}

/// Everything true of a message, as its icon's tip says it.
pub(crate) fn said(s: &Session, m: &Summary) -> String {
    s.list_said(m).into_iter().map(label).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" · ")
}
