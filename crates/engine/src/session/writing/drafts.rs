//! Drafts: the copies of a message being written kept in Drafts, found again by their
//! `Message-ID`; a reply written in the reading pane set aside when another message is read, as
//! Outlook sets it aside, and taken up again; a draft from Drafts carried on; the messages a
//! reply waits for, marked in the list.

use quatrelle_model::thread::norm_id;

use super::*;

/// Most replies set aside at once: past that, the oldest stays in Drafts only.
const MAX_SET_ASIDE: usize = 20;

impl Session {
    /// File `c` in Drafts in place of its earlier copies (`keep`: kept on this computer first,
    /// as Quatrelle closes). The new copy's key when it's known at once (a local account's), and
    /// the `Message-ID` the copies go by.
    pub(super) fn file_draft(&mut self, c: &Compose, keep: bool) -> Result<(Option<MessageKey>, String), CmdError> {
        let (raw, folder, mid) = self.draft_bytes(c)?;
        let mut earlier = self.saved_copies(c);
        if self.account(&c.account).is_none_or(AccountState::is_local) {
            if !earlier.is_empty() {
                self.apply_op(&c.account.clone(), Op::Expunge { keys: earlier });
            }
            return Ok((self.file_locally(&c.account, &folder, &raw, false), mid));
        }
        // The server numbers the new copy: the worker puts it in place of the earlier one, and
        // any other (saved again before the first was listed) goes.
        let replaces = (!earlier.is_empty()).then(|| earlier.remove(0));
        if !earlier.is_empty() {
            self.apply_op(&c.account.clone(), Op::Expunge { keys: earlier });
        }
        let job = self.draft_job(c, raw, folder, keep, replaces);
        self.jobs.push(job);
        Ok((None, mid))
    }

    /// The copies of `c` in Drafts: the one saved on this computer, and those its account's
    /// Drafts lists under its `Message-ID`.
    pub(super) fn saved_copies(&mut self, c: &Compose) -> Vec<MessageKey> {
        let mut keys: Vec<MessageKey> = c.draft.iter().cloned().collect();
        if c.saved_id.is_empty() {
            return keys;
        }
        let Some(drafts) = self.account(&c.account).and_then(|a| a.folder(FolderRole::Drafts)).map(|f| f.path.clone()) else { return keys };
        for m in &self.folder_state(&c.account, &drafts).messages {
            if m.message_id == c.saved_id && !keys.contains(&m.key) {
                keys.push(m.key.clone());
            }
        }
        keys
    }

    /// `c` is sent or discarded: its copies in Drafts go.
    pub(crate) fn remove_saved_copies(&mut self, c: &Compose) {
        let keys = self.saved_copies(c);
        if let Some(account) = keys.first().map(|k| k.account.clone()) {
            self.apply_op(&account, Op::Expunge { keys });
        }
        self.note_drafts();
    }

    /// The messages a reply waits for, from the replies set aside and every account's Drafts
    /// (their `In-Reply-To`, read with their source): the list marks them, as Outlook marks a
    /// message with a reply not sent yet.
    pub(crate) fn note_drafts(&mut self) {
        let mut ids: HashSet<String> = self.set_aside.iter().map(|c| norm_id(&c.in_reply_to)).filter(|m| !m.is_empty()).collect();
        let drafts: Vec<(String, String)> =
            self.accounts.iter().filter_map(|a| a.folder(FolderRole::Drafts).map(|f| (a.config.id.clone(), f.path.clone()))).collect();
        for (account, path) in drafts {
            ids.extend(self.folder_state(&account, &path).messages.iter().map(|m| norm_id(&m.in_reply_to)).filter(|m| !m.is_empty()));
        }
        self.drafted = ids;
    }

    /// Whether a reply to `m` waits in Drafts (or set aside).
    pub fn has_draft_reply(&self, m: &Summary) -> bool {
        !m.message_id.is_empty() && self.drafted.contains(&m.message_id)
    }

    /// The message going by `Message-ID` `mid` in the account: among its folders read so far,
    /// else in its Inbox.
    fn find_message(&mut self, account: &str, mid: &str) -> Option<MessageKey> {
        let wanted = norm_id(mid);
        if wanted.is_empty() {
            return None;
        }
        let read = self.cache.iter().filter(|((a, _), _)| a == account).find_map(|(_, st)| st.messages.iter().find(|m| m.message_id == wanted));
        if let Some(m) = read {
            return Some(m.key.clone());
        }
        let inbox = self.account(account)?.folder(FolderRole::Inbox)?.path.clone();
        self.folder_state(account, &inbox).messages.iter().find(|m| m.message_id == wanted).map(|m| m.key.clone())
    }

    /// The reading pane moves on to `next` (another message, or another folder: `None`) while a
    /// reply is written there: it's set aside, as Outlook sets it aside — kept in Drafts, and
    /// taken up again when its message is read again ([`Session::take_up`]). Left as it came
    /// (nothing typed) it simply goes; one being sent, or read again, stays.
    pub(crate) fn set_compose_aside(&mut self, next: Option<&MessageKey>) {
        let Some(c) = self.compose.as_ref() else { return };
        if c.window || c.sending || next.is_some_and(|k| c.source.as_ref() == Some(k) || c.draft.as_ref() == Some(k)) {
            return;
        }
        let keep = self.written(c) && !c.untouched();
        let Some(c) = self.compose.take() else { return };
        if !keep {
            return;
        }
        match self.file_draft(&c, false) {
            Ok((draft, saved_id)) => {
                self.set_aside.push(Compose { draft, saved_id, ..c });
                if self.set_aside.len() > MAX_SET_ASIDE {
                    self.set_aside.remove(0);
                }
                self.notice("Draft saved", false);
            }
            // It can't go in Drafts (the account has none): in a window of its own, not lost.
            Err(_) => self.composes.push(Compose { window: true, ..c }),
        }
        self.note_drafts();
    }

    /// A reply set aside for the message `key` (or the draft `key` itself), taken up again in
    /// the reading pane. Whether there was one.
    pub(crate) fn take_up(&mut self, key: &MessageKey) -> bool {
        let at = self.set_aside.iter().position(|c| c.source.as_ref() == Some(key) || c.draft.as_ref() == Some(key)).or_else(|| {
            let id = self.summary(key).map(|m| m.message_id.clone()).filter(|m| !m.is_empty())?;
            self.set_aside.iter().position(|c| c.saved_id == id)
        });
        let Some(at) = at else { return false };
        let c = self.set_aside.remove(at);
        self.bring_back(c);
        self.note_drafts();
        true
    }

    /// Act on `c` again as it was left (its signing and encryption as chosen): the message being
    /// written, if anything's in it, is set aside in a window of its own.
    fn bring_back(&mut self, c: Compose) {
        if let Some(mut old) = self.compose.take()
            && (self.written(&old) || old.sending)
        {
            old.window = true;
            self.composes.push(old);
        }
        self.compose = Some(c);
    }

    /// Carry on writing the draft `key` from Drafts (a reply set aside comes back as it was
    /// left): its copy is replaced at the next save, and goes once the message is sent or
    /// discarded. Where it's written is where new mail is (`Prefs::new_mail_window`).
    pub fn edit_draft(&mut self, key: &MessageKey) -> CmdResult {
        if self.take_up(key) {
            return Ok(serde_json::json!({ "compose": self.compose.as_ref().map(|c| c.id) }));
        }
        let raw = self.readable_raw(key).ok_or_else(|| CmdError::Disabled("the draft is still downloading".into()))?;
        let p = mime::parse(&raw).ok_or_else(|| CmdError::Failed("the draft can't be read".into()))?;
        // Every part comes along, the pictures in the text too (as attachments): nothing is lost.
        let attachments = p
            .attachments
            .iter()
            .filter_map(|a| mime::attachment_data(&raw, a.index).map(|data| OutAttachment { name: a.name.clone(), mime: a.mime.clone(), data }))
            .collect();
        let (body, styles) =
            if p.had_html && !p.rich.is_empty() { mime::rich::from_rich(&p.rich) } else { (p.text.clone(), mime::rich::Styles::plain(&p.text)) };
        let id = self.fresh_id();
        let window = self.prefs.new_mail_window;
        // A reply's message gets its Replied mark once it's sent, as when it was started here.
        let source = self.find_message(&key.account, &p.in_reply_to);
        let mode = source.as_ref().map(|_| ReplyMode::Reply);
        // From another address than the account's own: still from it.
        let own = self.account(&key.account).map(|a| a.config.email.clone()).unwrap_or_default();
        let from = if p.from.email.is_empty() || p.from.email.eq_ignore_ascii_case(&own) { String::new() } else { p.from.to_string() };
        self.bring_back(Compose {
            id,
            account: key.account.clone(),
            from,
            to: addr::format_field(&p.to),
            cc: addr::format_field(&p.cc),
            subject: p.subject.clone(),
            body,
            styles,
            in_reply_to: p.in_reply_to.clone(),
            references: p.references.clone(),
            attachments,
            draft: Some(key.clone()),
            saved_id: p.message_id.clone(),
            window,
            source,
            mode,
            ..Default::default()
        });
        Ok(serde_json::json!({ "compose": id }))
    }
}
