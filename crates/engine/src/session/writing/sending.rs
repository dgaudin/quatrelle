// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The session: whether a message can be sent from an account. Only an account with a server
//! to send through can: with none set up, or from an Outlook data file or a mailbox kept only on
//! this computer, nothing would go, so nothing says it went (a message written with only a data
//! file open was "sent" into the file's Sent Items). The sample mailbox, a demonstration, files
//! what it sends in its Sent folder.

use super::*;

/// Why mail can't be sent from an account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendBlock {
    /// There is no such account (none is set up, or it was removed).
    NoAccount,
    /// An Outlook data file opened here: it holds mail, it has no server to send through.
    DataFile,
    /// A mailbox kept on this computer only, with no server.
    NoServer,
    /// An account that sends over SMTP, without an outgoing server.
    NoOutgoingServer,
}

impl SendBlock {
    /// What the error says (the window says it in the user's language).
    pub fn reason(self) -> &'static str {
        match self {
            SendBlock::NoAccount => "add an account to send mail",
            SendBlock::DataFile => "mail can't be sent from an Outlook data file: add an account to send mail",
            SendBlock::NoServer => "mail can't be sent from a mailbox kept only on this computer: add an account to send mail",
            SendBlock::NoOutgoingServer => "the account has no outgoing (SMTP) server: add one in its settings to send mail",
        }
    }
}

impl Session {
    /// Why mail can't be sent from `account`, or `None` when it can.
    pub fn send_block(&self, account: &str) -> Option<SendBlock> {
        let Some(a) = self.account(account) else { return Some(SendBlock::NoAccount) };
        if a.is_data_file() {
            Some(SendBlock::DataFile)
        } else if a.is_local() {
            (a.config.id != crate::sample::SAMPLE_ACCOUNT).then_some(SendBlock::NoServer)
        } else if !a.config.is_api() && a.config.smtp.host.trim().is_empty() {
            Some(SendBlock::NoOutgoingServer)
        } else {
            None
        }
    }

    /// Why the message being written can't be sent from its account (none written: none).
    pub fn compose_send_block(&self) -> Option<SendBlock> {
        self.compose.as_ref().and_then(|c| self.send_block(&c.account))
    }

    /// `account` when mail can be sent from it; else the first account that can send, as Outlook
    /// writes from the default account in a data file's folders; else `account` still (its
    /// Send says why it can't).
    pub(crate) fn sending_account(&self, account: String) -> String {
        if self.send_block(&account).is_none() {
            return account;
        }
        self.accounts.iter().map(|a| &a.config.id).find(|id| self.send_block(id).is_none()).cloned().unwrap_or(account)
    }

    /// An account that can send was added: each message being written from one that can't (a
    /// data file's, or one gone) goes From it, with its signature, so the message the bar asked
    /// to add an account for can go.
    pub(crate) fn move_to_sending_account(&mut self) {
        let current = self.compose.take();
        let aside = std::mem::take(&mut self.composes);
        let mut moved = Vec::with_capacity(aside.len());
        // Each in turn is the current one while it changes (the signature works on that one).
        for c in aside {
            self.compose = Some(c);
            self.move_compose_to_sending_account();
            moved.extend(self.compose.take());
        }
        self.composes = moved;
        self.compose = current;
        self.move_compose_to_sending_account();
    }

    fn move_compose_to_sending_account(&mut self) {
        let Some((account, reply)) = self.compose.as_ref().filter(|c| !c.sending).map(|c| (c.account.clone(), c.mode.is_some())) else { return };
        let to = self.sending_account(account.clone());
        if to == account {
            return;
        }
        let sig = self.signature_in_message(&self.signature_when(&to, reply));
        if let Some(c) = self.compose.as_mut() {
            c.account = to;
            c.from.clear();
            put_signature(c, &sig);
        }
        self.style_signature();
    }

    /// An error unless mail can be sent from `account`.
    pub(crate) fn can_send(&self, account: &str) -> Result<(), CmdError> {
        match self.send_block(account) {
            Some(b) => Err(CmdError::Disabled(b.reason().into())),
            None => Ok(()),
        }
    }
}

/// `sig` (as it goes into a message) in place of `c`'s signature: where the old one stands, or,
/// when it had none, where a message starts with one (after the text of a new message, above
/// the quote of a reply or forward).
fn put_signature(c: &mut Compose, sig: &str) {
    let plain = |s: &str| mime::rich::Styles::plain(s);
    c.styles.fit(&c.body);
    if let Some(b) = (!c.signature.is_empty()).then(|| c.body.rfind(&c.signature)).flatten() {
        let start = c.body.get(..b).unwrap_or("").chars().count();
        let n = c.signature.chars().count();
        mime::rich::splice(&mut c.body, &mut c.styles, start..start + n, sig, &plain(sig));
    } else if !sig.is_empty() {
        let block = format!("\n\n{sig}");
        let at = if c.mode.is_some() { 0 } else { c.body.chars().count() };
        mime::rich::splice(&mut c.body, &mut c.styles, at..at, &block, &plain(&block));
    }
    c.signature = sig.to_string();
}
