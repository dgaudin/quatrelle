// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The fake EWS server: which fields Exchange knows, and what the fields of a message's text give.

use super::*;
use crate::exchange::ews::{
    PID_LID_COMMON_END, PID_LID_COMMON_START, PID_LID_FLAG_REQUEST, PID_LID_REMINDER_SET, PID_LID_REMINDER_SIGNAL_TIME, PID_LID_REMINDER_TIME,
    PID_LID_TASK_DUE_DATE, PID_LID_TASK_START_DATE, PID_LID_TO_DO_TITLE,
};

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

/// A message's follow-up as Exchange keeps it ([MS-OXOFLAG] 2.2.1): its start and due dates
/// (`PidLidTaskStartDate`, `PidLidTaskDueDate`, with `PidLidCommonStart`/`PidLidCommonEnd` the
/// same midnights in the local zone), its reminder (`PidLidReminderSet`, `PidLidReminderTime`,
/// `PidLidReminderSignalTime`) and its title (`PidLidFlagRequest`), each as Exchange writes it
/// (`2026-10-30T09:12:00Z`) when it has one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FollowUp {
    pub start: Option<String>,
    pub due: Option<String>,
    pub common_start: Option<String>,
    pub common_end: Option<String>,
    pub reminder_set: bool,
    pub reminder_time: Option<String>,
    pub reminder_signal: Option<String>,
    pub request: Option<String>,
    /// The task's title (`PidLidToDoTitle`, 0x85A4), what the To-Do List shows as the row's
    /// subject when Outlook set one.
    pub title: Option<String>,
    /// The flag's colour as Outlook 2003 wrote it (`PidTagFollowupIcon`, 0x1095), kept as
    /// written: Outlook takes a coloured flag on an item for one of its colour categories.
    pub icon: Option<String>,
}

/// A message's follow-up as Exchange gives it, with the last thing done with it ([`Verb`]).
pub(super) fn follow_up_xml(f: &FollowUp) -> String {
    let mut x = String::new();
    let lid = |set: &str, id: u32, kind: &str, value: &str| {
        format!(
            "<t:ExtendedProperty><t:ExtendedFieldURI DistinguishedPropertySetId=\"{set}\" PropertyId=\"{id}\" PropertyType=\"{kind}\"/><t:Value>{}</t:Value></t:ExtendedProperty>",
            esc(value)
        )
    };
    if let Some(v) = &f.start {
        x.push_str(&lid("Task", PID_LID_TASK_START_DATE, "SystemTime", v));
    }
    if let Some(v) = &f.due {
        x.push_str(&lid("Task", PID_LID_TASK_DUE_DATE, "SystemTime", v));
    }
    if let Some(v) = &f.common_start {
        x.push_str(&lid("Common", PID_LID_COMMON_START, "SystemTime", v));
    }
    if let Some(v) = &f.common_end {
        x.push_str(&lid("Common", PID_LID_COMMON_END, "SystemTime", v));
    }
    if f.reminder_set {
        x.push_str(&lid("Common", PID_LID_REMINDER_SET, "Boolean", "true"));
    }
    if let Some(v) = &f.reminder_time {
        x.push_str(&lid("Common", PID_LID_REMINDER_TIME, "SystemTime", v));
    }
    if let Some(v) = &f.reminder_signal {
        x.push_str(&lid("Common", PID_LID_REMINDER_SIGNAL_TIME, "SystemTime", v));
    }
    if let Some(v) = &f.request {
        x.push_str(&lid("Common", PID_LID_FLAG_REQUEST, "String", v));
    }
    if let Some(v) = &f.title {
        x.push_str(&lid("Common", PID_LID_TO_DO_TITLE, "String", v));
    }
    x
}

/// A message's follow-up flag as Exchange gives it: `PidTagFlagStatus` (2 flagged, 1 complete)
/// and `PidTagFlagCompleteTime` when it has one; with the last thing done with it ([`Verb`]) and
/// when, and the follow-up's dates and reminder ([`FollowUp`]). Its icon comes only when asked
/// for ([`kind_xml`]).
pub(super) fn flag_xml(flagged: bool, complete: bool, time: Option<&str>, verb: &Verb, follow_up: &FollowUp) -> String {
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
    for (tag, kind, v) in [("0x1081", "Integer", &verb.last), ("0x1082", "SystemTime", &verb.time)] {
        if let Some(v) = v {
            x.push_str(&format!(
                "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"{tag}\" PropertyType=\"{kind}\"/><t:Value>{}</t:Value></t:ExtendedProperty>",
                esc(v)
            ));
        }
    }
    x.push_str(&follow_up_xml(follow_up));
    x
}

/// An `UpdateItem` change to the follow-up flag taken (`SetItemField` or `DeleteItemField` of
/// `0x1090` or `0x1091`), or to the last thing done with it (`0x1081`, `0x1082`, `0x1080`).
pub(super) fn take_flag_update(it: &mut It, u: &El) {
    let set = u.name == "SetItemField";
    // The follow-up's dates, reminder and title, by the property set and id Exchange names
    // them ([MS-OXPROPS] 1.3.2).
    if let Some(uri) = u.find("ExtendedFieldURI")
        && let (Some(id), Some(set_name)) = (uri.attr("PropertyId"), uri.attr("DistinguishedPropertySetId"))
    {
        let v = u.find("Value").map(|v| v.text.trim().to_string()).filter(|_| set);
        let Ok(id) = id.parse::<u32>() else { return };
        match (set_name, id) {
            ("Task", PID_LID_TASK_START_DATE) => it.follow_up.start = v,
            ("Task", PID_LID_TASK_DUE_DATE) => it.follow_up.due = v,
            ("Common", PID_LID_COMMON_START) => it.follow_up.common_start = v,
            ("Common", PID_LID_COMMON_END) => it.follow_up.common_end = v,
            ("Common", PID_LID_REMINDER_SET) => it.follow_up.reminder_set = set && v.as_deref() == Some("true"),
            ("Common", PID_LID_REMINDER_TIME) => it.follow_up.reminder_time = v,
            ("Common", PID_LID_REMINDER_SIGNAL_TIME) => it.follow_up.reminder_signal = v,
            ("Common", PID_LID_FLAG_REQUEST) => it.follow_up.request = v,
            ("Common", PID_LID_TO_DO_TITLE) => it.follow_up.title = v,
            _ => {}
        }
        return;
    }
    match u.find("ExtendedFieldURI").and_then(|e| e.attr("PropertyTag")) {
        Some("0x1090") => {
            let v = u.find("Value").map(|v| v.text.trim().to_string());
            it.flagged = set && v.as_deref() == Some("2");
            it.complete = set && v.as_deref() == Some("1");
        }
        Some("0x1091") => it.complete_time = u.find("Value").map(|v| v.text.trim().to_string()).filter(|_| set),
        Some("0x1095") => it.follow_up.icon = u.find("Value").map(|v| v.text.trim().to_string()).filter(|_| set),
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

/// Whether a request's shape asks for what a message is: its class (`item:ItemClass`) and its
/// icon (`PidTagIconIndex`, `0x1080`).
pub(super) fn kind_asked(op: &El) -> (bool, bool) {
    let Some(shape) = op.find("AdditionalProperties") else { return (false, false) };
    let class = shape.kids.iter().any(|f| f.attr("FieldURI") == Some("item:ItemClass"));
    let icon = shape.kids.iter().any(|f| f.name == "ExtendedFieldURI" && f.attr("PropertyTag") == Some("0x1080"));
    (class, icon)
}

/// A message's class and icon, as far as they're asked for (its icon only when it has one).
pub(super) fn kind_xml(it: &It, class: bool, icon: bool) -> String {
    let mut x = if class { format!("<t:ItemClass>{}</t:ItemClass>", esc(&it.class)) } else { String::new() };
    if let Some(v) = it.verb.icon.as_deref().filter(|_| icon) {
        x.push_str(&format!(
            "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x1080\" PropertyType=\"Integer\"/><t:Value>{}</t:Value></t:ExtendedProperty>",
            esc(v)
        ));
    }
    x
}

/// An item's answer (`<t:Message>…</t:Message>`) with its class and icon, as far as `op` asks for
/// them; the class not again when the text's fields carry it (`text`).
pub(super) fn with_kind(x: String, it: &It, op: &El, text: bool) -> String {
    let (class, icon) = kind_asked(op);
    let more = kind_xml(it, class && !text, icon);
    match x.rfind("</t:Message>") {
        Some(at) if !more.is_empty() => format!("{}{more}{}", x.get(..at).unwrap_or_default(), x.get(at..).unwrap_or_default()),
        _ => x,
    }
}

/// The search for what messages are (`FindItem` restricted to an `Or` of classes by their start,
/// `Contains` `Prefixed` on `item:ItemClass`, and of last verbs, `IsEqualTo` on `0x1081`).
pub(super) struct KindsSearch {
    starts: Vec<String>,
    verbs: Vec<String>,
}

impl KindsSearch {
    /// Whether a message is one the search finds: its class starts with one of them (without
    /// case), or its last verb is one of them.
    pub(super) fn admits(&self, it: &It) -> bool {
        let class = it.class.to_ascii_lowercase();
        self.starts.iter().any(|s| class.starts_with(s.as_str())) || it.verb.last.as_ref().is_some_and(|v| self.verbs.contains(v))
    }
}

/// The search for what messages are, if a restriction is one.
pub(super) fn kinds_search(restriction: &El) -> Option<KindsSearch> {
    let or = restriction.find("Or")?;
    let mut contains = Vec::new();
    or.all("Contains", &mut contains);
    let starts: Vec<String> = contains
        .iter()
        .filter(|c| c.find("FieldURI").and_then(|f| f.attr("FieldURI")) == Some("item:ItemClass"))
        .filter_map(|c| c.find("Constant").and_then(|k| k.attr("Value")).map(str::to_ascii_lowercase))
        .collect();
    if starts.is_empty() {
        return None;
    }
    let mut equal = Vec::new();
    or.all("IsEqualTo", &mut equal);
    let verbs = equal
        .iter()
        .filter(|c| c.find("ExtendedFieldURI").and_then(|e| e.attr("PropertyTag")) == Some("0x1081"))
        .filter_map(|c| c.find("Constant").and_then(|k| k.attr("Value")).map(str::to_string))
        .collect();
    Some(KindsSearch { starts, verbs })
}
