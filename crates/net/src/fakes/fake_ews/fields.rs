// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The fake EWS server: which fields Exchange knows, and what the fields of a message's text give.

use super::*;

/// The `item:` and `message:` names Exchange knows, from Microsoft's FieldURI page. A `GetItem`
/// naming any other is refused whole, whatever the other fields are.
pub(super) const MICROSOFT_FIELDS: [&str; 63] = [
    "item:ArchiveTag",
    "item:Attachments",
    "item:Body",
    "item:Categories",
    "item:ConversationId",
    "item:Culture",
    "item:DateTimeCreated",
    "item:DateTimeReceived",
    "item:DateTimeSent",
    "item:DisplayCc",
    "item:DisplayTo",
    "item:EffectiveRights",
    "item:Flag",
    "item:HasAttachments",
    "item:Importance",
    "item:InReplyTo",
    "item:InstanceKey",
    "item:InternetMessageHeaders",
    "item:IsAssociated",
    "item:IsDraft",
    "item:IsFromMe",
    "item:IsResend",
    "item:IsSubmitted",
    "item:IsUnmodified",
    "item:ItemClass",
    "item:ItemId",
    "item:LastModifiedName",
    "item:LastModifiedTime",
    "item:MimeContent",
    "item:NormalizedBody",
    "item:ParentFolderId",
    "item:PolicyTag",
    "item:Preview",
    "item:ReminderDueBy",
    "item:ReminderIsSet",
    "item:ReminderMinutesBeforeStart",
    "item:ResponseObjects",
    "item:RetentionDate",
    "item:Sensitivity",
    "item:Size",
    "item:StoreEntryId",
    "item:Subject",
    "item:TextBody",
    "item:UniqueBody",
    "message:ApprovalRequestData",
    "message:BccRecipients",
    "message:CcRecipients",
    "message:ConversationIndex",
    "message:ConversationTopic",
    "message:From",
    "message:InternetMessageId",
    "message:IsDeliveryReceiptRequested",
    "message:IsRead",
    "message:IsReadReceiptRequested",
    "message:IsResponseRequested",
    "message:ReceivedBy",
    "message:ReceivedRepresenting",
    "message:References",
    "message:ReminderMessageData",
    "message:ReplyTo",
    "message:Sender",
    "message:ToRecipients",
    "message:VotingInformation",
];

/// The fields a request names one by one.
pub(super) fn fields_named(op: &El) -> Vec<&str> {
    op.find("AdditionalProperties").map(|a| a.kids.iter().filter_map(|f| f.attr("FieldURI")).collect()).unwrap_or_default()
}

/// What `item:Body` and the fields asked with it add to an item: its text, the list of its files
/// (no bytes, as `GetItem` gives them), its class and importance.
pub(super) fn text_fields(it: &It, p: &quatrelle_mime::Parsed) -> String {
    let (kind, body) = if p.had_html { ("HTML", format!("<p>{}</p>", esc(&p.text))) } else { ("Text", p.text.clone()) };
    let files: String = p
        .attachments
        .iter()
        .map(|a| {
            let element = if a.mime.eq_ignore_ascii_case("message/rfc822") { "ItemAttachment" } else { "FileAttachment" };
            let cid = a.content_id.as_deref().map(|c| format!("<t:ContentId>{}</t:ContentId>", esc(c))).unwrap_or_default();
            format!(
                "<t:{element}><t:AttachmentId Id=\"{}:{}\"/><t:Name>{}</t:Name><t:ContentType>{}</t:ContentType>{cid}<t:Size>{}</t:Size><t:IsInline>{}</t:IsInline></t:{element}>",
                it.id,
                a.index,
                esc(&a.name),
                esc(&a.mime),
                a.size,
                a.inline
            )
        })
        .collect();
    let files = if files.is_empty() { files } else { format!("<t:Attachments>{files}</t:Attachments>") };
    let importance = match quatrelle_mime::summary_from_header(&it.raw, Default::default(), Default::default(), 0, 0).importance {
        1 => "High",
        -1 => "Low",
        _ => "Normal",
    };
    format!(
        "<t:ItemClass>{}</t:ItemClass><t:Body BodyType=\"{kind}\">{}</t:Body>{files}<t:Importance>{importance}</t:Importance>",
        esc(&it.class),
        esc(&body)
    )
}

/// The last thing done with a message, as Exchange keeps it: `PidTagLastVerbExecuted` (102
/// replied to the sender, 103 to all, 104 forwarded…), `PidTagLastVerbExecutionTime` when, and
/// `PidTagIconIndex` the icon Outlook's list draws.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Verb {
    pub last: Option<String>,
    pub time: Option<String>,
    pub icon: Option<String>,
}

/// A message's follow-up flag as Exchange gives it: `PidTagFlagStatus` (2 flagged, 1 complete)
/// and `PidTagFlagCompleteTime` when it has one; with the last thing done with it ([`Verb`]).
pub(super) fn flag_xml(flagged: bool, complete: bool, time: Option<&str>, verb: &Verb) -> String {
    let status = if flagged { Some(2) } else { complete.then_some(1) };
    let mut x = status
        .map(|v| {
            format!(
                "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/><t:Value>{v}</t:Value></t:ExtendedProperty>"
            )
        })
        .unwrap_or_default();
    if let Some(t) = time {
        x.push_str(&format!(
            "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x1091\" PropertyType=\"SystemTime\"/><t:Value>{}</t:Value></t:ExtendedProperty>",
            esc(t)
        ));
    }
    for (tag, kind, v) in [("0x1081", "Integer", &verb.last), ("0x1082", "SystemTime", &verb.time), ("0x1080", "Integer", &verb.icon)] {
        if let Some(v) = v {
            x.push_str(&format!(
                "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"{tag}\" PropertyType=\"{kind}\"/><t:Value>{}</t:Value></t:ExtendedProperty>",
                esc(v)
            ));
        }
    }
    x
}

/// An `UpdateItem` change to the follow-up flag taken (`SetItemField` or `DeleteItemField` of
/// `0x1090` or `0x1091`), or to the last thing done with it (`0x1081`, `0x1082`, `0x1080`).
pub(super) fn take_flag_update(it: &mut It, u: &El) {
    let set = u.name == "SetItemField";
    match u.find("ExtendedFieldURI").and_then(|e| e.attr("PropertyTag")) {
        Some("0x1090") => {
            let v = u.find("Value").map(|v| v.text.trim().to_string());
            it.flagged = set && v.as_deref() == Some("2");
            it.complete = set && v.as_deref() == Some("1");
        }
        Some("0x1091") => it.complete_time = u.find("Value").map(|v| v.text.trim().to_string()).filter(|_| set),
        // The last thing done with it, when, and its icon (as written).
        Some(tag @ ("0x1081" | "0x1082" | "0x1080")) => {
            let v = u.find("Value").map(|v| v.text.trim().to_string()).filter(|_| set);
            match tag {
                "0x1081" => it.verb.last = v,
                "0x1082" => it.verb.time = v,
                _ => it.verb.icon = v,
            }
        }
        _ => {}
    }
}

/// A restriction's range on `PidTagLastVerbExecuted` (`0x1081` at least and at most so much).
pub(super) fn verb_range(restriction: &El) -> Option<(i64, i64)> {
    let bound = |name: &str| {
        let mut all = Vec::new();
        restriction.all(name, &mut all);
        all.into_iter()
            .find(|c| c.find("ExtendedFieldURI").and_then(|e| e.attr("PropertyTag")) == Some("0x1081"))
            .and_then(|c| c.find("Constant").and_then(|k| k.attr("Value")).and_then(|v| v.parse().ok()))
    };
    Some((bound("IsGreaterThanOrEqualTo")?, bound("IsLessThanOrEqualTo")?))
}
