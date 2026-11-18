// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The fake EWS server's MailTips (`GetMailTips`).

use quatrelle_model::scheduling::{MAX_MAIL_TIPS, MailTip};

use super::*;

/// A tip's `Scope` as Exchange spells it.
fn scope_xml(scope: &str) -> &'static str {
    match scope {
        "internal" => "Internal",
        "external" => "External",
        "externalPartner" => "ExternalPartner",
        "externalNonPartner" => "ExternalNonPartner",
        _ => "",
    }
}

/// One `<m:MailTips>` element for `t`.
fn tip_xml(t: &MailTip) -> String {
    let mut inner = format!("<t:RecipientAddress><t:EmailAddress>{}</t:EmailAddress></t:RecipientAddress>", esc(&t.email));
    if !t.automatic_replies.is_empty() {
        inner.push_str(&format!("<t:OutOfOffice><t:ReplyBody><t:Message>{}</t:Message></t:ReplyBody></t:OutOfOffice>", esc(&t.automatic_replies)));
    }
    inner.push_str(&format!(
        "<t:MailboxFull>{}</t:MailboxFull><t:ExternalMemberCount>{}</t:ExternalMemberCount>\
         <t:MaxMessageSize>{}</t:MaxMessageSize><t:DeliveryRestricted>{}</t:DeliveryRestricted>\
         <t:IsModerated>{}</t:IsModerated><t:InvalidRecipient>{}</t:InvalidRecipient>",
        t.mailbox_full, t.external_members, t.max_message_size, t.restricted, t.moderated, t.invalid
    ));
    if !t.custom.is_empty() {
        inner.push_str(&format!("<t:CustomMailTip>{}</t:CustomMailTip>", esc(&t.custom)));
    }
    let scope = scope_xml(&t.scope);
    if !scope.is_empty() {
        inner.push_str(&format!("<t:Scope>{scope}</t:Scope>"));
    }
    format!("<m:MailTips>{inner}</m:MailTips>")
}

impl Data {
    /// A `GetMailTips` answer: the sender and the addresses asked about are kept, and each
    /// address with a tip configured gets one (an ordinary recipient gets none).
    pub(super) fn mail_tips(&mut self, op: &El) -> String {
        let sender = op.find("SendingAs").and_then(|s| s.text_of("EmailAddress")).unwrap_or("").trim().to_ascii_lowercase();
        let asked: Vec<String> = op
            .find("Recipients")
            .map(|r| r.children("Mailbox").filter_map(|m| m.text_of("EmailAddress")).map(|e| e.trim().to_ascii_lowercase()).collect())
            .unwrap_or_default();
        if asked.len() > MAX_MAIL_TIPS {
            return "<m:R ResponseClass=\"Error\"><m:MessageText>ErrorInvalidRequest</m:MessageText>\
                    <m:ResponseCode>ErrorInvalidRequest</m:ResponseCode></m:R>"
                .into();
        }
        self.mail_tips_asked.push((sender, asked.clone()));
        let mut inner = String::new();
        for email in &asked {
            if let Some(t) = self.mail_tips.iter().find(|t| t.email.eq_ignore_ascii_case(email)) {
                inner.push_str(&tip_xml(t));
            }
        }
        format!("<m:R ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>{inner}</m:R>")
    }
}
