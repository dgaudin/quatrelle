// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! EWS MailTips: what the server says about a message's recipients before it goes (an automatic
//! reply, a full mailbox, a restricted or moderated recipient, a list reaching outside the
//! organisation), as Outlook shows it above the message.

use quatrelle_model::scheduling::{MAX_MAIL_TIPS, MailTip};

use super::*;

/// A tip's text kept at most (an automatic reply can be a whole letter).
const MAX_TIP_TEXT: usize = 4000;

/// Times a question is asked again for the recipients whose tips the server hasn't ready
/// (`PendingMailTips`: an automatic reply read from the recipient's mailbox), and the wait
/// before each time.
const PENDING_TRIES: usize = 2;
const PENDING_WAIT: std::time::Duration = std::time::Duration::from_millis(1500);

/// The `GetMailTips` request for `emails` sent as `from`: each address as SMTP, as Microsoft's
/// example names them, in the schema's order.
pub(crate) fn mail_tips_request(from: &str, emails: &[String]) -> String {
    let address = |e: &str| format!("<t:EmailAddress>{}</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType>", esc(e.trim()));
    let recipients: String = emails.iter().take(MAX_MAIL_TIPS).map(|e| format!("<t:Mailbox>{}</t:Mailbox>", address(e))).collect();
    format!(
        "<m:GetMailTips><m:SendingAs>{}</m:SendingAs>\
         <m:Recipients>{recipients}</m:Recipients><m:MailTipsRequested>All</m:MailTipsRequested></m:GetMailTips>",
        address(from)
    )
}

/// A tip's text as the interface shows it: HTML turned into words, and bounded.
fn plain(text: &str) -> String {
    let text = text.trim();
    let text = if text.contains('<') { quatrelle_mime::html_to_plain(text) } else { text.to_string() };
    text.trim().chars().take(MAX_TIP_TEXT).collect()
}

/// `Internal`, `External`, `ExternalPartner`… as Graph spells them (`internal`, `external`…).
fn scope_of(scope: &str) -> String {
    match scope.trim().to_ascii_lowercase().as_str() {
        "internal" => "internal".into(),
        "external" => "external".into(),
        "externalpartner" => "externalPartner".into(),
        "externalnonpartner" => "externalNonPartner".into(),
        _ => String::new(),
    }
}

/// One `MailTips` element as a [`MailTip`]; `None` when it names no address (the wrapper element
/// carries no tip of its own).
fn tip_of(e: &El) -> Option<MailTip> {
    let email = e.child("RecipientAddress").and_then(|a| a.text_of("EmailAddress"))?.trim().to_ascii_lowercase();
    tip_for(e, email)
}

/// One `MailTips` element as the tip of `email`.
fn tip_for(e: &El, email: String) -> Option<MailTip> {
    if !email.contains('@') || email.len() > 320 {
        return None;
    }
    let flag = |name: &str| e.text_of(name).is_some_and(|v| v.trim().eq_ignore_ascii_case("true"));
    let automatic_replies =
        e.child("OutOfOffice").and_then(|o| o.child("ReplyBody")).and_then(|b| b.text_of("Message")).map(plain).unwrap_or_default();
    Some(MailTip {
        email,
        automatic_replies,
        mailbox_full: flag("MailboxFull"),
        custom: e.text_of("CustomMailTip").map(plain).unwrap_or_default(),
        restricted: flag("DeliveryRestricted"),
        external_members: e.text_of("ExternalMemberCount").and_then(|v| v.trim().parse().ok()).unwrap_or(0),
        moderated: flag("IsModerated"),
        invalid: flag("InvalidRecipient"),
        max_message_size: e.text_of("MaxMessageSize").and_then(|v| v.trim().parse().ok()).unwrap_or(0),
        scope: e.text_of("Scope").map(scope_of).unwrap_or_default(),
    })
}

/// The tips a `GetMailTips` answer gives: those with something to tell, at most
/// [`MAX_MAIL_TIPS`].
pub fn mail_tips_read(doc: &El) -> Vec<MailTip> {
    let mut found = Vec::new();
    doc.all("MailTips", &mut found);
    found.iter().filter_map(|e| tip_of(e)).filter(MailTip::worth_telling).take(MAX_MAIL_TIPS).collect()
}

/// What one answer says about the recipients `asked`: the tips with something to tell, each
/// under the address asked; the addresses whose tips aren't ready; and how many came under an
/// address not asked about.
struct Answer {
    tips: Vec<MailTip>,
    pending: Vec<String>,
    elsewhere: usize,
}

/// Exchange answers a tip for each recipient, in the order asked, under the address it knows the
/// recipient by (the main one rather than the one typed): one under an address not asked about
/// is the one asked in its place, when there's one for each.
fn answer_of(doc: &El, asked: &[String]) -> Answer {
    let mut found = Vec::new();
    doc.all("MailTips", &mut found);
    let in_order = found.len() == asked.len();
    let mut out = Answer { tips: Vec::new(), pending: Vec::new(), elsewhere: 0 };
    for (i, e) in found.iter().enumerate().take(MAX_MAIL_TIPS) {
        let said = e.child("RecipientAddress").and_then(|a| a.text_of("EmailAddress")).map(|a| a.trim().to_ascii_lowercase()).unwrap_or_default();
        let email = match asked.get(i) {
            Some(a) if in_order && !asked.contains(&said) => {
                out.elsewhere += 1;
                a.clone()
            }
            _ => said,
        };
        if e.text_of("PendingMailTips").is_some_and(|p| !p.trim().is_empty()) && asked.contains(&email) {
            out.pending.push(email.clone());
        }
        if let Some(t) = tip_for(e, email).filter(MailTip::worth_telling) {
            out.tips.push(t);
        }
    }
    out
}

/// What the server says about these recipients, sent as `from`; only those with something to
/// tell are kept, in the order asked. Those whose tips aren't ready are asked about again, a
/// little later. A request the server refused says why.
pub fn mail_tips(c: &Client, from: &str, emails: &[String]) -> Result<Vec<MailTip>> {
    if emails.is_empty() {
        return Ok(Vec::new());
    }
    let from = if from.trim().is_empty() { c.email() } else { from.trim() };
    let asked: Vec<String> = emails.iter().take(MAX_MAIL_TIPS).map(|e| e.trim().to_ascii_lowercase()).collect();
    let mut ask = asked.clone();
    let mut out: Vec<MailTip> = Vec::new();
    for round in 0..=PENDING_TRIES {
        let doc = c.call(&mail_tips_request(from, &ask))?;
        let a = answer_of(&doc, &ask);
        // Nothing at all: the response messages carry the refusal (an operation a server
        // doesn't have, an address it won't tell about).
        if round == 0 && a.tips.is_empty() && a.pending.is_empty() {
            for m in crate::ews::messages(&doc) {
                crate::ews::check_message(m)?;
            }
        }
        if a.elsewhere > 0 || !a.pending.is_empty() {
            log::info!("MailTips: {} of {} under another address, {} not ready yet", a.elsewhere, ask.len(), a.pending.len());
        }
        for t in a.tips {
            out.retain(|o| o.email != t.email);
            out.push(t);
        }
        if a.pending.is_empty() || round == PENDING_TRIES {
            break;
        }
        ask = a.pending;
        std::thread::sleep(PENDING_WAIT);
    }
    out.sort_by_key(|t| asked.iter().position(|a| *a == t.email).unwrap_or(usize::MAX));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_names_the_sender_and_every_recipient() {
        let xml = mail_tips_request("riley@contoso.example", &["ana@contoso.example".into(), " Bo <bo@contoso.example> ".into()]);
        assert!(xml.contains("<m:GetMailTips>"));
        assert!(xml.contains("<t:EmailAddress>riley@contoso.example</t:EmailAddress>"));
        assert_eq!(xml.matches("<t:Mailbox>").count(), 2);
        assert!(xml.contains("<t:EmailAddress>Bo &lt;bo@contoso.example&gt;</t:EmailAddress>"), "what isn't an address is escaped as it came");
        assert!(xml.contains("<m:MailTipsRequested>All</m:MailTipsRequested>"));
        assert_eq!(xml.matches("<t:RoutingType>SMTP</t:RoutingType>").count(), 3, "each address as SMTP, as Microsoft's example: {xml}");
    }

    #[test]
    fn a_tip_is_read_from_its_element() {
        let doc = crate::ews::parse(
            "<s:Envelope xmlns:s=\"s\"><s:Body><m:GetMailTipsResponse xmlns:m=\"m\"><m:ResponseMessages>\
             <m:MailTipsResponseMessageType ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>\
             <m:MailTips><t:RecipientAddress><t:EmailAddress>Ana@Contoso.Example</t:EmailAddress></t:RecipientAddress>\
             <t:OutOfOffice><t:ReplyBody><t:Message>&lt;p&gt;Away &lt;b&gt;until Monday&lt;/b&gt;&lt;/p&gt;</t:Message></t:ReplyBody></t:OutOfOffice>\
             <t:MailboxFull>true</t:MailboxFull><t:CustomMailTip>Ring first</t:CustomMailTip>\
             <t:ExternalMemberCount>12</t:ExternalMemberCount><t:MaxMessageSize>10485760</t:MaxMessageSize>\
             <t:DeliveryRestricted>true</t:DeliveryRestricted><t:IsModerated>true</t:IsModerated>\
             <t:InvalidRecipient>false</t:InvalidRecipient><t:Scope>External</t:Scope></m:MailTips>\
             <m:MailTips><t:RecipientAddress><t:EmailAddress>bo@contoso.example</t:EmailAddress></t:RecipientAddress>\
             <t:MailboxFull>false</t:MailboxFull><t:MaxMessageSize>2048</t:MaxMessageSize></m:MailTips>\
             </m:MailTipsResponseMessageType></m:ResponseMessages></m:GetMailTipsResponse></s:Body></s:Envelope>",
        );
        let mut elements = Vec::new();
        doc.all("MailTips", &mut elements);
        let tips: Vec<MailTip> = elements.iter().filter_map(|e| tip_of(e)).collect();
        assert_eq!(tips.len(), 2);
        assert_eq!(tips[0].email, "ana@contoso.example");
        assert_eq!(tips[0].automatic_replies, "Away until Monday");
        assert!(tips[0].mailbox_full && tips[0].restricted && tips[0].moderated && !tips[0].invalid);
        assert_eq!(tips[0].custom, "Ring first");
        assert_eq!((tips[0].external_members, tips[0].max_message_size, tips[0].scope.as_str()), (12, 10_485_760, "external"));
        assert!(tips[1].worth_telling(), "a size limit alone is something to tell");
        assert_eq!(tips[1].max_message_size, 2048);
    }

    /// Exchange's scopes, spelled as Graph spells them: one outside the organisation and its
    /// partners is `externalNonPartner`.
    #[test]
    fn a_scope_is_spelled_as_graph_spells_it() {
        assert_eq!(scope_of("ExternalNonPartner"), "externalNonPartner");
        assert_eq!(scope_of("ExternalPartner"), "externalPartner");
        assert_eq!(scope_of("None"), "");
    }
}
