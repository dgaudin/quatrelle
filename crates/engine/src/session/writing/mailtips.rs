// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! MailTips for a message being written (Outlook's: an automatic reply, a full mailbox, a
//! restricted or moderated recipient, a list reaching outside the organisation). An Exchange or
//! Microsoft 365 account asks its server while the form stays responsive; what was asked for
//! each message is kept here, so the form draws it frame after frame and asks once per set of
//! recipients. Informational only: nothing here holds a message back.

use quatrelle_model::scheduling::{MAX_MAIL_TIPS, MailTip};

use super::*;
use crate::session::scheduling::Fetched;

/// The recipients a message is about, as a server is asked: the addresses in To, Cc and Bcc,
/// lower case, once each and in that order. Entries still being typed are left out; a field can
/// name several, separated as the contacts fields accept.
pub fn mail_tips_recipients(c: &Compose) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for field in [&c.to, &c.cc, &c.bcc] {
        for entry in addr::split_entries(field) {
            let Some(a) = addr::parse_one(entry) else { continue };
            let email = a.email.trim().to_ascii_lowercase();
            if !email.contains('@') || email.len() > 320 || out.contains(&email) {
                continue;
            }
            out.push(email);
            if out.len() >= MAX_MAIL_TIPS {
                return out;
            }
        }
    }
    out
}

/// What a message being written asked its server about its recipients, and what came back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailTipsState {
    /// The account asked (an answer for another one is let go).
    pub account: String,
    /// The recipients asked about, as [`mail_tips_recipients`] has them.
    pub recipients: Vec<String>,
    /// The question's number: an answer to an older one is let go.
    pub ask: u64,
    /// The server's answer, while it's on its way.
    pub found: Fetched<Vec<MailTip>>,
}

impl Session {
    /// Whether the message being written should ask about its recipients now: it has some, its
    /// account is one whose server answers MailTips, and what's known is for another set (a
    /// recipient was added or taken away). A request on its way counts as asked, so a form
    /// drawn frame after frame asks once.
    pub fn mail_tips_wanted(&self) -> bool {
        let Some(c) = self.compose.as_ref() else { return false };
        if c.sending || !self.server_schedules(&c.account) {
            return false;
        }
        let recipients = mail_tips_recipients(c);
        if recipients.is_empty() {
            return false;
        }
        !self.mail_tips.get(&c.id).is_some_and(|s| s.account == c.account && s.recipients == recipients)
    }

    /// Ask the server what it says about the current message's recipients (`refresh`: again,
    /// even if this set was asked about already). An answer comes as [`Event::MailTips`].
    pub fn request_mail_tips(&mut self, refresh: bool) -> CmdResult {
        let c = self.compose.as_ref().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
        if c.sending {
            return Err(CmdError::Disabled("the message is being sent".into()));
        }
        if self.account(&c.account).is_none() {
            return Err(CmdError::Params(format!("no account `{}`", c.account)));
        }
        if !self.server_schedules(&c.account) {
            return Err(CmdError::Disabled("MailTips come from Exchange and Microsoft 365 accounts".into()));
        }
        let recipients = mail_tips_recipients(c);
        if recipients.is_empty() {
            return Err(CmdError::Params("add a recipient first: MailTips are about them".into()));
        }
        let id = c.id;
        if !refresh && self.mail_tips.get(&id).is_some_and(|s| s.account == c.account && s.recipients == recipients) {
            return Ok(self.mail_tips_json(id));
        }
        let account = c.account.clone();
        let from = self.compose_from(c).email;
        self.mail_tips_ask = self.mail_tips_ask.saturating_add(1);
        let ask = self.mail_tips_ask;
        self.mail_tips.insert(id, MailTipsState { account: account.clone(), recipients: recipients.clone(), ask, found: Fetched::Loading });
        self.jobs.push(Job::GetMailTips { account, from, recipients, ask });
        Ok(self.mail_tips_json(id))
    }

    /// What's known of message `id`'s MailTips; `None` until it was asked for.
    pub fn mail_tips_now(&self, id: u64) -> Option<&MailTipsState> {
        self.mail_tips.get(&id)
    }

    /// The server's answer to question `ask`: what's kept for the message that asked it. An
    /// answer to a question since replaced (the recipients changed) is let go.
    pub(crate) fn mail_tips_answered(&mut self, account: &str, ask: u64, tips: Vec<MailTip>, error: Option<String>) {
        let Some(state) = self.mail_tips.values_mut().find(|s| s.ask == ask && s.account == account) else { return };
        state.found = match error {
            Some(e) => Fetched::Failed(e),
            None => Fetched::Ready(
                tips.into_iter().take(MAX_MAIL_TIPS).filter(|t| t.email.contains('@') && t.email.len() <= 320 && t.worth_telling()).collect(),
            ),
        };
    }

    /// One message's MailTips as commands answer them.
    pub(crate) fn mail_tips_json(&self, id: u64) -> Value {
        let Some(s) = self.mail_tips.get(&id) else {
            return serde_json::json!({"state": "none", "tips": []});
        };
        let mut v = serde_json::json!({
            "state": s.found.state(),
            "ask": s.ask,
            "recipients": s.recipients,
        });
        match &s.found {
            Fetched::Ready(tips) => v["tips"] = serde_json::json!(tips),
            Fetched::Failed(e) => v["error"] = serde_json::json!(e),
            Fetched::Loading => {}
        }
        v
    }
}
