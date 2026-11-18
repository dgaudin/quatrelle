// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Microsoft 365 MailTips through Graph: what the server says about a message's recipients
//! before it goes (an automatic reply, a full mailbox, a restricted or moderated recipient, a
//! list reaching outside the organisation), as Outlook shows it above the message.

use quatrelle_model::scheduling::{MAX_MAIL_TIPS, MailTip};

use super::*;

/// A tip's text kept at most (an automatic reply can be a whole letter).
const MAX_TIP_TEXT: usize = 4000;

/// What Graph is asked for: every kind of tip Quatrelle shows.
const OPTIONS: &str = "automaticReplies, customMailTip, deliveryRestriction, externalMemberCount, mailboxFullStatus, \
                       maxMessageSize, moderationStatus, recipientScope, totalMemberCount";

/// The `getMailTips` body for these addresses.
pub(crate) fn mail_tips_request(emails: &[String]) -> Value {
    json!({
        "EmailAddresses": emails.iter().take(MAX_MAIL_TIPS).map(|e| e.trim()).collect::<Vec<_>>(),
        "MailTipsOptions": OPTIONS,
    })
}

/// A tip's text as the interface shows it: HTML turned into words, and bounded.
fn plain(text: &str) -> String {
    let text = text.trim();
    let text = if text.contains('<') { quatrelle_mime::html_to_plain(text) } else { text.to_string() };
    text.trim().chars().take(MAX_TIP_TEXT).collect()
}

/// A `mailTips` resource as a [`MailTip`]; `None` when it names no address.
fn tip_of(v: &Value) -> Option<MailTip> {
    let email = s(v.get("emailAddress")?, "address").trim().to_ascii_lowercase();
    if !email.contains('@') || email.len() > 320 {
        return None;
    }
    let flag = |name: &str| v.get(name).and_then(Value::as_bool).unwrap_or(false);
    let automatic_replies = v.get("automaticReplies").map(|a| plain(&s(a, "message"))).unwrap_or_default();
    let scope = s(v, "recipientScope");
    Some(MailTip {
        email,
        automatic_replies,
        mailbox_full: flag("mailboxFull"),
        custom: plain(&s(v, "customMailTip")),
        restricted: flag("deliveryRestricted"),
        external_members: v.get("externalMemberCount").and_then(Value::as_u64).unwrap_or(0).min(u64::from(u32::MAX)) as u32,
        moderated: flag("isModerated"),
        invalid: flag("invalidRecipient"),
        max_message_size: v.get("maxMessageSize").and_then(Value::as_u64).unwrap_or(0),
        scope: match scope.as_str() {
            "internal" => "internal".into(),
            "external" => "external".into(),
            "externalPartner" => "externalPartner".into(),
            "externalNonOrganization" => "externalNonOrganization".into(),
            _ => String::new(),
        },
    })
}

/// What Graph says about these recipients; only those with something to tell are kept.
pub fn mail_tips(c: &Client, emails: &[String]) -> Result<Vec<MailTip>> {
    if emails.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for chunk in emails.chunks(MAX_MAIL_TIPS) {
        let v = c.post("/me/getMailTips", &mail_tips_request(chunk))?;
        let answers = v.get("value").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        out.extend(answers.iter().take(MAX_MAIL_TIPS).filter_map(tip_of).filter(MailTip::worth_telling));
        if out.len() >= MAX_MAIL_TIPS {
            break;
        }
    }
    out.truncate(MAX_MAIL_TIPS);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_names_every_recipient_and_the_tips_wanted() {
        let v = mail_tips_request(&["ana@contoso.example".into(), " Bo <bo@contoso.example> ".into()]);
        assert_eq!(v["EmailAddresses"], json!(["ana@contoso.example", "Bo <bo@contoso.example>"]));
        for wanted in ["automaticReplies", "customMailTip", "deliveryRestriction", "externalMemberCount", "mailboxFullStatus", "maxMessageSize"] {
            assert!(v["MailTipsOptions"].as_str().unwrap_or_default().contains(wanted), "{wanted}");
        }
    }

    #[test]
    fn a_tip_is_read_from_its_resource() {
        let v = json!({"value": [
            {"emailAddress": {"address": "Ana@Contoso.Example"},
             "automaticReplies": {"message": "<p>Back on <b>Monday</b></p>"},
             "mailboxFull": true, "customMailTip": "Ring first", "deliveryRestricted": true,
             "externalMemberCount": 12, "isModerated": true, "maxMessageSize": 10485760,
             "recipientScope": "external"},
            {"emailAddress": {"address": "bo@contoso.example"}, "mailboxFull": false, "maxMessageSize": 2048},
            {"automaticReplies": {"message": "no address"}},
        ]});
        let tips: Vec<MailTip> = v["value"].as_array().unwrap().iter().filter_map(tip_of).collect();
        assert_eq!(tips.len(), 2);
        assert_eq!(tips[0].email, "ana@contoso.example");
        assert_eq!(tips[0].automatic_replies, "Back on Monday");
        assert!(tips[0].mailbox_full && tips[0].restricted && tips[0].moderated && !tips[0].invalid);
        assert_eq!(tips[0].custom, "Ring first");
        assert_eq!((tips[0].external_members, tips[0].max_message_size, tips[0].scope.as_str()), (12, 10_485_760, "external"));
        assert!(tips[1].worth_telling(), "a size limit alone is something to tell");
        assert_eq!(tips[1].max_message_size, 2048);
    }
}
