// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The session: the Outbox.

use super::*;

/// Said, before what went wrong, when a message deleted from the Outbox couldn't be taken out of
/// it: it's still there, and may still go (unless it was refused, or may have gone already).
const NOT_DELETED: &str = "a message couldn't be taken out of the Outbox; it's still there, and may still be sent";
/// Said, before what went wrong, when a message to change couldn't be taken out of the Outbox:
/// opened all the same, it would go twice (as it was, and as changed).
const NOT_TAKEN_BACK: &str = "the message couldn't be taken out of the Outbox to be changed";
/// What the worker puts before why an Outbox message wasn't sent when the server never said
/// whether it took it (it may have gone), and when it refused it: neither goes again by itself.
const MAYBE_GONE: &str = "unconfirmed: ";
const REFUSED: &str = "refused: ";

impl Session {
    /// Re-read the Outbox from disk.
    pub fn refresh_outbox(&mut self) {
        let Some(store) = &self.store else { return };
        self.outbox = self.accounts.iter().flat_map(|a| store.outbox(&a.config.id).into_iter().map(|i| (a.config.id.clone(), i))).collect();
    }

    pub(super) fn after_outbox_change(&mut self) {
        let before: Vec<(String, u64)> = self.outbox.iter().map(|(a, i)| (a.clone(), i.id)).collect();
        self.refresh_outbox();
        if self.view == View::Outbox {
            self.follow_outbox_rows(&before);
            self.refresh_list();
        }
    }

    /// An Outbox key counts rows, so a message's changes when one above it is sent or deleted.
    /// What's selected and what's open follow their messages to their new rows: left on the row
    /// number, they were another message, and Delete then took that one, never sent.
    fn follow_outbox_rows(&mut self, before: &[(String, u64)]) {
        let now = &self.outbox;
        let moved = |k: &MessageKey| -> Option<MessageKey> {
            if k.folder != OUTBOX {
                return Some(k.clone());
            }
            let (account, id) = before.get(usize::try_from(k.uid).ok()?.checked_sub(1)?)?;
            let at = now.iter().position(|(a, i)| a == account && i.id == *id)?;
            Some(MessageKey { uid: u32::try_from(at + 1).ok()?, ..k.clone() })
        };
        let selection: Vec<MessageKey> = self.selection.iter().filter_map(&moved).collect();
        let focus = self.focus.as_ref().and_then(moved);
        let anchor = self.anchor.as_ref().and_then(moved);
        let reading = self.reading.as_ref().map(|r| moved(&r.key));
        (self.selection, self.focus, self.anchor) = (selection, focus, anchor);
        match (reading, &mut self.reading) {
            (Some(Some(key)), Some(r)) => r.key = key,
            (Some(None), _) => self.reading = None,
            _ => {}
        }
    }

    /// Queued messages, as list rows. Their key is `(account, OUTBOX, 0, n)`, `n` counting from 1
    /// in Outbox order.
    pub(super) fn outbox_summaries(&self) -> Vec<Summary> {
        let Some(store) = &self.store else { return Vec::new() };
        let mut out = Vec::new();
        for (n, (account, item)) in self.outbox.iter().enumerate() {
            let key = MessageKey { account: account.clone(), folder: OUTBOX.to_string(), validity: 0, uid: u32::try_from(n + 1).unwrap_or(u32::MAX) };
            let raw = store.outbox_raw(account, item.id).unwrap_or_default();
            let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
            let flags = quatrelle_model::Flags { seen: true, ..Default::default() };
            let mut m = mime::summary_from_full(&raw, key, flags, size, item.queued);
            m.preview = match Self::outbox_trouble(item) {
                // The server never said whether it took it: it may have gone, never "Not sent".
                Some((true, e)) => format!("May have been sent: {e}"),
                Some((false, e)) => format!("Not sent yet: {e}"),
                None if item.not_before > self.clock.now => {
                    format!("Will be sent {}", quatrelle_model::dates::full_time(item.not_before, &self.clock))
                }
                None => "Waiting to be sent".to_string(),
            };
            out.push(m);
        }
        out
    }

    /// What went wrong the last time an Outbox message was tried, as the user is told it: `None`
    /// when nothing did (or it was only offline); else whether it may have gone already (the
    /// server never said whether it took it), and why, without the worker's mark.
    pub fn outbox_trouble(item: &OutboxItem) -> Option<(bool, &str)> {
        let e = item.error.as_deref().filter(|e| *e != "offline")?;
        Some(match e.strip_prefix(MAYBE_GONE) {
            Some(why) => (true, why),
            None => (false, e.strip_prefix(REFUSED).unwrap_or(e)),
        })
    }

    /// The Outbox entry behind an Outbox key.
    pub fn outbox_entry(&self, key: &MessageKey) -> Option<(String, OutboxItem)> {
        if key.folder != OUTBOX {
            return None;
        }
        let n = usize::try_from(key.uid).ok()?.checked_sub(1)?;
        self.outbox.get(n).filter(|(a, _)| *a == key.account).cloned()
    }

    /// Send one held-back Outbox message now (Delay Delivery off), as its Send now button. One
    /// that may have gone already waits for the user's word, and this is it (its row and the
    /// reading pane over its button say it may have gone, and how to find out first): it's sent
    /// again.
    pub fn send_outbox_now(&mut self, key: &MessageKey) -> CmdResult {
        let (account, mut item) = self.outbox_entry(key).ok_or_else(|| CmdError::Params("not an Outbox message".into()))?;
        self.can_send(&account)?;
        let again = Self::outbox_trouble(&item).is_some_and(|(maybe_gone, _)| maybe_gone);
        if item.not_before != 0 || again {
            item.not_before = 0;
            if again {
                item.error = None;
            }
            if let Some(store) = &self.store {
                store.outbox_update(&account, &item).map_err(|e| CmdError::Failed(e.to_string()))?;
            }
            self.after_outbox_change();
        }
        self.jobs.push(Job::FlushOutbox { account });
        Ok(serde_json::json!({ "sending": item.id }))
    }

    /// Ask the accounts holding Outbox mail whose time has come to send it. Called with
    /// [`Session::tick`]; each message is asked for once.
    pub(crate) fn flush_due_outbox(&mut self) {
        let now = self.clock.now;
        let due: Vec<(String, u64)> = self
            .outbox
            .iter()
            .filter(|(a, i)| i.not_before != 0 && i.not_before <= now && !self.flushed.contains(&i.id) && self.send_block(a).is_none())
            .map(|(a, i)| (a.clone(), i.id))
            .collect();
        let mut accounts: Vec<String> = Vec::new();
        for (a, id) in due {
            self.flushed.insert(id);
            if !accounts.contains(&a) {
                accounts.push(a);
            }
        }
        for account in accounts {
            self.jobs.push(Job::FlushOutbox { account });
        }
    }

    /// Try to send everything in the Outbox now (what's held for an account that can't send
    /// stays: [`SendBlock`](crate::SendBlock)).
    pub fn send_outbox(&mut self) -> CmdResult {
        let mut accounts: Vec<String> = self.outbox.iter().map(|(a, _)| a.clone()).filter(|a| self.send_block(a).is_none()).collect();
        accounts.dedup();
        if accounts.is_empty()
            && let Some(b) = self.outbox.first().and_then(|(a, _)| self.send_block(a))
        {
            return Err(CmdError::Disabled(b.reason().into()));
        }
        for a in &accounts {
            self.jobs.push(Job::FlushOutbox { account: a.clone() });
        }
        Ok(serde_json::json!({ "accounts": accounts.len(), "messages": self.outbox.len() }))
    }

    /// Delete the selected Outbox messages (they are never sent). One that can't be taken out is
    /// said: it's still there, to be sent, with the flag it owes.
    pub fn delete_outbox_selection(&mut self) -> CmdResult {
        let mut gone: Vec<(String, OutboxItem)> = self.selection.iter().filter_map(|k| self.outbox_entry(k)).collect();
        let mut kept = None;
        if let Some(store) = &self.store {
            gone.retain(|(a, i)| match store.outbox_remove(a, i.id) {
                Ok(()) => true,
                Err(e) => {
                    kept = Some(e);
                    false
                }
            });
        }
        // Removed from the Outbox undelivered: whatever flags they owed are dropped.
        let before = self.pending_marks.len();
        self.pending_marks.retain(|id, _| !gone.iter().any(|(_, i)| i.id == *id));
        if self.pending_marks.len() != before {
            self.save_pending_marks();
        }
        self.reading = None;
        self.selection.clear();
        self.focus = None;
        self.after_outbox_change();
        if let Some(e) = kept {
            return Err(CmdError::Failed(format!("{NOT_DELETED}: {e}")));
        }
        Ok(serde_json::json!({ "deleted": gone.len() }))
    }

    /// Take a message back out of the Outbox to change it: it opens for editing.
    pub fn edit_outbox(&mut self, key: &MessageKey) -> CmdResult {
        let (account, item) = self.outbox_entry(key).ok_or_else(|| CmdError::Params("not an Outbox message".into()))?;
        let store = self.store.clone().ok_or_else(|| CmdError::Failed("no mail cache".into()))?;
        let raw = store.outbox_raw(&account, item.id).ok_or_else(|| CmdError::Failed("the message is gone".into()))?;
        let p = mime::parse(&raw).ok_or_else(|| CmdError::Failed("the message can't be read".into()))?;
        // As it went in: formatted, its pictures in place (it came back as plain text, its
        // pictures as files), a reply's original as it was.
        let mut c = self.compose_from_saved(&account, key, &raw)?;
        // Bcc isn't written into a message: it's the envelope recipients not in To or Cc.
        let shown: Vec<String> = p.to.iter().chain(&p.cc).map(|a| a.email.to_ascii_lowercase()).collect();
        let bcc: Vec<String> = item.recipients.iter().filter(|r| !shown.contains(&r.to_ascii_lowercase())).cloned().collect();
        c.bcc = if bcc.is_empty() { String::new() } else { format!("{}; ", bcc.join("; ")) };
        c.show_cc = !bcc.is_empty();
        // What it answers comes with what the Outbox owed it, below; it's no copy in Drafts.
        (c.source, c.mode) = (None, None);
        c.saved_id.clear();
        // Out of the Outbox before it opens: left there, it would go as well as the changed one.
        store.outbox_remove(&account, item.id).map_err(|e| CmdError::Failed(format!("{NOT_TAKEN_BACK}: {e}")))?;
        let id = c.id;
        self.open_compose(c);
        // Pulled back for editing: the owed flag moves onto the compose, for its delivery.
        if let Some((src, flag)) = self.pending_marks.remove(&item.id) {
            self.save_pending_marks();
            if let Some(c) = self.compose.as_mut() {
                c.source = Some(src);
                c.mode = Some(if flag == Flag::Forwarded { ReplyMode::Forward } else { ReplyMode::Reply });
            }
        }
        self.reading = None;
        self.selection.clear();
        self.after_outbox_change();
        Ok(serde_json::json!({ "compose": id }))
    }
}
