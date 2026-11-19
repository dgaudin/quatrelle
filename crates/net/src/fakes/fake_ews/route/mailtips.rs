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

/// One `<m:MailTips>` element for `t`, as Exchange writes it: its address with its routing
/// type, the tips not ready yet, then the others (`pending`: the automatic reply isn't).
fn tip_xml(t: &MailTip, pending: bool) -> String {
    let mut inner = format!(
        "<t:RecipientAddress><t:Name/><t:EmailAddress>{}</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType></t:RecipientAddress>",
        esc(&t.email)
    );
    let away = !t.automatic_replies.is_empty();
    inner.push_str(if pending && away { "<t:PendingMailTips>OutOfOfficeMessage</t:PendingMailTips>" } else { "<t:PendingMailTips/>" });
    if away && !pending {
        inner.push_str(&format!("<t:OutOfOffice><t:ReplyBody><t:Message>{}</t:Message></t:ReplyBody></t:OutOfOffice>", esc(&t.automatic_replies)));
    } else {
        inner.push_str("<t:OutOfOffice><t:ReplyBody><t:Message/></t:ReplyBody></t:OutOfOffice>");
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
    /// address gets an element, in the order asked, as Exchange answers (an ordinary recipient's
    /// tells nothing), under the address the server knows it by.
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
        let pending = self.mail_tips_pending > 0;
        self.mail_tips_pending = self.mail_tips_pending.saturating_sub(1);
        let mut inner = String::new();
        for email in &asked {
            let known = self.mail_tips_known_as.iter().find(|(a, _)| a.eq_ignore_ascii_case(email)).map_or(email.as_str(), |(_, k)| k.as_str());
            match self.mail_tips.iter().find(|t| t.email.eq_ignore_ascii_case(known)) {
                Some(t) => inner.push_str(&tip_xml(t, pending)),
                None => inner.push_str(&tip_xml(&MailTip { email: known.to_string(), ..Default::default() }, false)),
            }
        }
        format!("<m:R ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>{inner}</m:R>")
    }
}
