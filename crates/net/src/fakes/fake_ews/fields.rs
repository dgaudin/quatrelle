// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
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
