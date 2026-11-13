// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The session: messages being written, drafts and sending.

use quatrelle_model::lookalike;

use super::*;

mod assistant;
mod compose;
mod drafts;
mod items;
mod names;
mod quote;
mod reopen;
mod senders;
mod sending;
mod signature_web;
mod vote;
pub use assistant::{AssistKind, Assistant, Filing, LlmModels, Rewrite};
use compose::with_html_signature;
pub use compose::{Compose, add_pictures, signature_block, style_signature};
pub use items::{ItemChoice, ItemKind};
pub(crate) use names::NameCheck;
pub use names::{NAMES_NOT_FOUND, NameChoice, Outcome, Resolved, group_entry};
pub use quote::{Quote, QuotePoint, Written};
pub use senders::Sender;
pub use sending::SendBlock;

/// From this many people (counted as [`SendCheck::Exposed`] says), sending to all of them in To
/// and Cc asks first.
const EXPOSED_FROM: usize = 8;

/// Something to look at before a message goes (`compose.send` holds it and says what).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SendCheck {
    /// A recipient's domain looks like `like`, one the user deals with, but isn't it: a typo,
    /// or a reply going to someone passing for a colleague.
    Lookalike { address: String, like: String },
    /// Everyone in To and Cc sees everyone else's address: `count` people who may not know each
    /// other (each organisation outside the sender's own counted once, each address at a big mail
    /// service on its own).
    Exposed { count: usize },
}

impl Session {
    pub(super) fn me(&self, account: &str) -> Vec<String> {
        let mut v: Vec<String> = self.accounts.iter().map(|a| a.config.email.clone()).collect();
        if let Some(a) = self.account(account) {
            v.push(a.config.username.clone());
        }
        v
    }

    /// An account's signature as it goes into a message: its text, or where one written in HTML
    /// stands ([`mime::rich::SIGNATURE`], drawn and sent as it was written).
    pub(super) fn signature(&self, account: &str) -> String {
        if self.html_signature(account).is_some() { mime::rich::SIGNATURE.to_string() } else { self.signature_rich(account).0 }
    }

    /// An account's signature written in HTML (Signatures… › HTML code), when it has one with
    /// words in it.
    pub(crate) fn html_signature(&self, account: &str) -> Option<String> {
        let c = &self.account(account)?.config;
        (!c.signature_raw_html.trim().is_empty() && !c.signature.trim().is_empty()).then(|| c.signature_raw_html.clone())
    }

    /// The signature added when a message starts: a new one (`reply` false) or a reply or
    /// forward, as the account says (Outlook's New messages and Replies/forwards); empty when
    /// it adds none there.
    pub(super) fn signature_when(&self, account: &str, reply: bool) -> String {
        let off = self.account(account).is_some_and(|a| if reply { a.config.signature_not_replies } else { a.config.signature_not_new });
        if off { String::new() } else { self.signature(account) }
    }

    /// An account's signature with its look: its text and styles (plain when it has none). The
    /// styles its editor kept come back as they were; a signature from before that was kept
    /// (or pasted as HTML from the command line) is read from its HTML.
    pub fn signature_rich(&self, account: &str) -> (String, mime::rich::Styles) {
        let Some(a) = self.account(account) else { return (String::new(), mime::rich::Styles::default()) };
        let c = &a.config;
        if !c.signature_styles.is_empty()
            && !c.signature.trim().is_empty()
            && let Ok(mut styles) = serde_json::from_str::<mime::rich::Styles>(&c.signature_styles)
        {
            styles.fit(&c.signature);
            return (c.signature.clone(), styles);
        }
        if !c.signature_html.trim().is_empty() {
            let (text, styles) = mime::signature_html(&c.signature_html);
            if !text.trim().is_empty() {
                return (text, styles);
            }
        }
        (c.signature.clone(), mime::rich::Styles::plain(&c.signature))
    }

    /// `sig` as it goes into a message: after a `-- ` line when the preferences ask for one.
    pub(crate) fn signature_in_message(&self, sig: &str) -> String {
        if sig.trim().is_empty() { String::new() } else { signature_block(sig, self.prefs.signature_dashes) }
    }

    /// The signature in the message being made, given its look (the text went in plain), or the
    /// HTML it stands for.
    pub(super) fn style_signature(&mut self) {
        let Some(account) = self.compose.as_ref().map(|c| c.account.clone()) else { return };
        let sig = self.signature_rich(&account);
        let pictures = self.signature_pictures(&account);
        let html = self.html_signature(&account);
        let dashes = self.prefs.signature_dashes;
        if let Some(c) = self.compose.as_mut() {
            if c.body.contains(mime::rich::SIGNATURE)
                && let Some(html) = html
            {
                c.signature_html = html;
            } else {
                style_signature(c, &sig, dashes);
            }
            add_pictures(c, pictures);
        }
    }

    /// An account's signature pictures, as pictures of a message.
    pub fn signature_pictures(&self, account: &str) -> Vec<mime::InlinePicture> {
        self.account(account)
            .map(|a| {
                a.config
                    .signature_pictures
                    .iter()
                    .map(|p| mime::InlinePicture { cid: p.cid.clone(), name: p.name.clone(), mime: p.mime.clone(), data: p.data.clone() })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A new message from a `mailto:` link (Quatrelle as the system's mail app): its addresses,
    /// subject and first words, above the signature. Nothing is attached and nothing is sent.
    pub fn mailto_compose(&mut self, m: &quatrelle_model::mailto::Mailto) -> CmdResult {
        let c = self.new_compose(None)?;
        let cap = |x: &str| x.chars().take(1 << 20).collect::<String>();
        c.to = cap(&m.to);
        c.cc = cap(&m.cc);
        c.bcc = cap(&m.bcc);
        c.show_cc |= !c.bcc.is_empty();
        c.subject = cap(&m.subject);
        if !m.body.is_empty() {
            let text = format!("{}{}", cap(&m.body), c.body);
            c.set_text(text);
        }
        let id = c.id;
        self.style_signature();
        Ok(serde_json::json!({"compose": id}))
    }

    /// Start writing `c`: the message being written, if any, is set aside in a window of its
    /// own ([`Session::composes`]), never lost; one that's empty (nothing typed) goes.
    pub fn open_compose(&mut self, mut c: Compose) {
        self.smime_defaults(&mut c);
        if let Some(mut old) = self.compose.take()
            && (self.written(&old) || old.sending)
        {
            old.window = true;
            self.composes.push(old);
        }
        self.compose = Some(c);
    }

    /// Act on message `id` (one set aside comes back; the current one is set aside in turn).
    /// False when there's no such message.
    pub fn activate_compose(&mut self, id: u64) -> bool {
        if self.compose.as_ref().is_some_and(|c| c.id == id) {
            return true;
        }
        let Some(at) = self.composes.iter().position(|c| c.id == id) else { return false };
        let c = self.composes.remove(at);
        if let Some(old) = self.compose.replace(c) {
            self.composes.insert(at, old);
        }
        true
    }

    /// Every message being written: the current one, then those set aside.
    pub fn all_composes(&self) -> impl Iterator<Item = &Compose> {
        self.compose.iter().chain(self.composes.iter())
    }

    pub fn new_compose(&mut self, account: Option<&str>) -> Result<&mut Compose, CmdError> {
        let account = match account {
            Some(a) => self.account(a).map(|a| a.config.id.clone()),
            None => self.current_account().map(|a| self.sending_account(a.config.id.clone())),
        }
        .ok_or_else(|| CmdError::Disabled("add an account first".into()))?;
        let signature = self.signature_in_message(&self.signature_when(&account, false));
        let id = self.fresh_id();
        let body = if signature.is_empty() { String::new() } else { format!("\n\n{signature}") };
        let window = self.prefs.new_mail_window;
        let styles = mime::rich::Styles::plain(&body);
        self.open_compose(Compose { id, account, body, styles, window, signature, ..Default::default() });
        self.style_signature();
        self.compose.as_mut().ok_or_else(|| CmdError::Failed("compose".into()))
    }

    pub fn reply_compose(&mut self, mode: ReplyMode) -> CmdResult {
        let r = self.reading.clone().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        // Answered the way it came (S/MIME or OpenPGP): encrypted too when it was and the account
        // can.
        let came_as = r.security.as_ref().filter(|s| s.lock.is_some() || s.signed.is_some()).map(|s| if s.pgp { Scheme::Pgp } else { Scheme::Smime });
        let can = match came_as {
            Some(Scheme::Pgp) => self.pgp_active(&r.key.account),
            _ => self.identity(&r.key.account).is_some(),
        };
        let encrypted = r.security.as_ref().is_some_and(|s| matches!(s.lock, Some(Lock::Open { .. }))) && can;
        // The reply's own copy (the quote's pictures are put in it), the one shown left as it is.
        let shown = r.parsed.clone();
        let mut parsed = Arc::unwrap_or_clone(r.parsed.ok_or_else(|| CmdError::Disabled(crate::STILL_DOWNLOADING.into()))?);
        if r.text_first && !answerable_from_text(&parsed, mode) {
            self.whole_here()?;
        }
        // Answered from an account that can send (not the data file it's in).
        let account = self.sending_account(r.key.account.clone());
        let sig = self.signature_in_message(&self.signature_when(&account, true));
        // In another mailbox's folder (a shared one): from that mailbox, which isn't written to.
        let from = self.reply_from(&r.key, &parsed);
        let mut me = self.me(&account);
        me.extend(addr::parse_list(&from).unwrap_or_default().into_iter().map(|a| a.email));
        let raw = self.readable_raw(&r.key);
        // The pictures in the quoted message go with it (they showed as "[picture]", issue #10).
        let (inline, quoted, given) = raw.as_deref().map(|raw| mime::quoted_pictures_given(&mut parsed, raw)).unwrap_or_default();
        // Written in HTML, the original goes as it came, its columns and all (the editor's text
        // has no tables), after who sent it to whom and when, which stay the text's: as Outlook
        // quotes it, changed where it stands.
        let html = raw.as_deref().filter(|_| parsed.had_html).and_then(mime::html_body).unwrap_or_default();
        let quote = shown.and_then(|shown| Quote::of(r.key.clone(), shown, &parsed, &given, &html)).map(Arc::new);
        let (d, styles) = match quote {
            Some(_) => mime::reply_head_doc(&parsed, mode, &me, &self.clock, &sig, &self.quote_words),
            None => mime::reply_doc(&parsed, mode, &me, &self.clock, &sig, &self.quote_words),
        };
        // Whoever it goes to, as worked out above (one's own message goes to its recipients):
        // a name alone can't be written to.
        if r.text_first && d.to.iter().chain(&d.cc).any(|a| a.email.trim().is_empty()) {
            self.whole_here()?;
        }
        let id = self.fresh_id();
        let mut attachments = Vec::new();
        if mode == ReplyMode::Forward
            && let Some(raw) = &raw
        {
            // Pictures the quote shows go in it; any other, inline or not, as a file (a picture
            // sent inline beside plain text was lost).
            for a in parsed.attachments.iter().filter(|a| !quoted.contains(&a.index)) {
                if let Some(out) = mime::attachment_out(raw, a.index) {
                    attachments.push(OutAttachment { name: a.name.clone(), ..out });
                }
            }
        }
        self.open_compose(Compose {
            id,
            account,
            from,
            signature: sig,
            to: addr::format_field(&d.to),
            cc: addr::format_field(&d.cc),
            subject: d.subject,
            body: d.body,
            styles,
            in_reply_to: d.in_reply_to,
            references: d.references,
            attachments,
            inline,
            mode: Some(mode),
            source: Some(r.key),
            // An answer to encrypted mail quotes it: encrypted too, and signed, when it can be.
            sign: encrypted,
            encrypt: encrypted,
            came_as,
            window: self.prefs.reply_window,
            quote,
            ..Default::default()
        });
        self.style_signature();
        if let Some(c) = self.compose.as_mut() {
            c.started = Some(c.fingerprint());
        }
        Ok(serde_json::json!({ "compose": id }))
    }

    /// Forward the open message as an attachment, as Outlook's More › Forward as Attachment: the
    /// original goes along whole (an `.eml`), the new message holds only the signature.
    pub fn forward_as_attachment(&mut self) -> CmdResult {
        self.whole_here()?;
        let r = self.reading.clone().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        // Shown (the command's `when`), but neither kept nor held (past the cap): it won't come.
        let raw = self.kept_or_held(&r.key).ok_or_else(|| CmdError::Failed(crate::NOT_DOWNLOADED.into()))?;
        let out = self.reply_compose(ReplyMode::Forward)?;
        // The signature of the account it goes from (not the data file it's in).
        let from = self.compose.as_ref().map_or(r.key.account.clone(), |c| c.account.clone());
        let sig = self.signature_in_message(&self.signature_when(&from, true));
        let subject = r.parsed.as_ref().map(|p| p.subject.trim().to_string()).unwrap_or_default();
        let name = format!("{}.eml", if subject.is_empty() { "message" } else { subject.as_str() });
        if let Some(c) = self.compose.as_mut() {
            c.set_text(if sig.is_empty() { String::new() } else { format!("\n\n{sig}") });
            c.signature = sig;
            c.quote = None;
            c.attachments = vec![OutAttachment { name, mime: "message/rfc822".into(), data: raw, ..Default::default() }];
        }
        self.style_signature();
        Ok(out)
    }

    pub(super) fn compose_draft(&self, c: &Compose) -> Result<mime::Draft, CmdError> {
        let parse = |field: &str, s: &str| addr::parse_list(s).map_err(|bad| CmdError::Params(format!("{field}: not an email address: {bad}")));
        let (body, html, inline) = c.own_bodies();
        // A signature written in HTML goes as it was written, where its text stands (one that
        // stands as itself already went in); the original kept as it came after it all.
        let raw = self.account(&c.account).map(|a| a.config.signature_raw_html.clone()).unwrap_or_default();
        let html =
            if raw.trim().is_empty() || c.plain || c.body.contains(mime::rich::SIGNATURE) { html } else { with_html_signature(c, &raw).or(html) };
        let (body, html, inline) = c.with_quote((body, html, inline));
        Ok(mime::Draft {
            reply_to: parse("Replies to", &c.reply_to)?,
            to: parse("To", &c.to)?,
            cc: parse("Cc", &c.cc)?,
            bcc: parse("Bcc", &c.bcc)?,
            subject: c.subject.trim().to_string(),
            body,
            in_reply_to: c.in_reply_to.clone(),
            references: c.references.clone(),
            attachments: c.attachments.clone(),
            html,
            inline,
            importance: c.importance,
            read_receipt: c.read_receipt,
            // Exchange alone carries voting buttons.
            voting: if self.account(&c.account).is_some_and(|a| a.config.ews.is_some()) { c.voting.clone() } else { String::new() },
            seven_bit: false,
        })
    }

    pub(super) fn compose_bytes(&mut self, c: &Compose, d: &mime::Draft) -> Result<(Vec<u8>, String), CmdError> {
        self.compose_bytes_as(c, d, None).map(|(raw, email, _)| (raw, email))
    }

    /// [`Session::compose_bytes`] under the `Message-ID` given (a new one without), and that id.
    fn compose_bytes_as(&mut self, c: &Compose, d: &mime::Draft, mid: Option<&str>) -> Result<(Vec<u8>, String, String), CmdError> {
        let acc = self.account(&c.account).ok_or_else(|| CmdError::Failed("the sending account is gone".into()))?;
        // The account signs in and hands it over (the envelope's sender), whoever it's from.
        let email = acc.config.email.clone();
        let from = self.compose_from(c);
        let now = self.clock.now;
        let mid = match mid {
            Some(m) => m.to_string(),
            None => mime::new_message_id(&email, now, self.random()),
        };
        let raw = mime::build(d, &from, now, &mid).map_err(CmdError::Failed)?;
        Ok((raw, email, mid))
    }

    /// What to look at before `c` goes ([`SendCheck`]).
    pub fn send_checks(&mut self, c: &Compose, d: &mime::Draft) -> Vec<SendCheck> {
        let mut found = Vec::new();
        let known = self.known_domains();
        for a in d.to.iter().chain(&d.cc).chain(&d.bcc) {
            if let Some(like) = lookalike::imitates(&lookalike::domain_of(&a.email), &known)
                && found.len() < 20
            {
                found.push(SendCheck::Lookalike { address: a.email.trim().to_string(), like: like.to_string() });
            }
        }
        let mine = self.account(&c.account).map(|a| lookalike::domain_of(&a.config.email)).unwrap_or_default();
        let mut people = BTreeSet::new();
        for a in d.to.iter().chain(&d.cc) {
            let domain = lookalike::domain_of(&a.email);
            if domain.is_empty() || domain == mine {
                continue;
            }
            people.insert(if lookalike::is_mail_service(&domain) { a.email.trim().to_lowercase() } else { domain });
        }
        if people.len() >= EXPOSED_FROM {
            found.push(SendCheck::Exposed { count: people.len() });
        }
        found
    }

    /// Everyone in To and Cc moved to Bcc, with the sender in To: each gets the message without
    /// seeing who else did.
    pub fn hide_recipients(&mut self) -> CmdResult {
        let c = self.compose.as_ref().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
        let me = self.compose_from(c).to_string();
        let c = self.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
        let moved: Vec<&str> = [c.to.as_str(), c.cc.as_str(), c.bcc.as_str()]
            .into_iter()
            .map(|f| f.trim().trim_end_matches(';').trim())
            .filter(|f| !f.is_empty())
            .collect();
        c.bcc = moved.join("; ");
        c.to = if me.is_empty() { String::new() } else { format!("{me}; ") };
        c.cc.clear();
        c.show_cc = true;
        Ok(serde_json::json!({ "bcc": c.bcc }))
    }

    /// Send the message being written. Unless `checked`, it's held when [`Session::send_checks`]
    /// finds anything, and the answer says what (`{"check": [...]}`).
    pub fn send_compose(&mut self, allow_empty_subject: bool, checked: bool) -> CmdResult {
        // Encryption following the recipients: as they are now.
        if self.compose.as_ref().is_some_and(|c| c.encrypt_auto) {
            self.encryption_reach();
        }
        let c = self.compose.clone().ok_or_else(|| CmdError::Disabled("nothing to send".into()))?;
        if c.sending {
            return Err(CmdError::Disabled("already sending".into()));
        }
        self.can_send(&c.account)?;
        let mut d = self.compose_draft(&c)?;
        if d.recipients().is_empty() {
            return Err(CmdError::Params("add at least one recipient".into()));
        }
        if d.subject.is_empty() && !allow_empty_subject {
            return Err(CmdError::Params("the subject is empty (send with {\"allowEmptySubject\": true} to send anyway)".into()));
        }
        // Where the photos were taken stays with the sender (the draft keeps them as attached).
        if !self.prefs.keep_photo_location {
            for a in &mut d.attachments {
                if let Some(clean) = mime::without_location(&a.data) {
                    a.data = clean;
                }
            }
        }
        if !checked {
            let found = self.send_checks(&c, &d);
            if !found.is_empty() {
                return Ok(serde_json::json!({ "check": found }));
            }
        }
        // Signed or encrypted (S/MIME or OpenPGP): its parts must reach the reader byte for byte.
        if c.sign || c.encrypt {
            d.seven_bit = true;
        }
        let (raw, from) = self.compose_bytes(&c, &d)?;
        // The account's OpenPGP key goes along, for answers to be encrypted (Autocrypt).
        let raw = self.with_autocrypt(&c, raw);
        // Written to: suggested first from now on, as Outlook's AutoComplete list does.
        let written: Vec<EmailAddress> = d.to.iter().chain(&d.cc).chain(&d.bcc).cloned().collect();
        self.contacts_sent(&written);
        self.remember_sender(&c);
        let recipients = d.recipients();
        if let Some(job) = self.protect_job(&c, &raw, &recipients, &from)? {
            self.jobs.push(job);
            self.mark_sending(c.id);
            return Ok(serde_json::json!({ "protecting": c.id }));
        }
        self.deliver(&c, raw, from, recipients)
    }

    fn mark_sending(&mut self, id: u64) {
        for cm in self.compose.iter_mut().chain(self.composes.iter_mut()).filter(|x| x.id == id) {
            cm.sending = true;
            cm.error = None;
        }
    }

    /// `c` as `raw` (signed or encrypted already, if it's to be), on its way: into the Outbox for
    /// later, filed in Sent in the sample mailbox, or handed to the network layer. From an
    /// account that can't send ([`SendBlock`]), refused: nothing goes, nothing says it went.
    pub(super) fn deliver(&mut self, c: &Compose, raw: Vec<u8>, from: String, recipients: Vec<String>) -> CmdResult {
        self.can_send(&c.account)?;
        let local = self.account(&c.account).is_some_and(AccountState::is_local);
        let sent_folder = self.account(&c.account).and_then(|a| a.folder(FolderRole::Sent)).map(|f| f.path.clone());
        // Delay Delivery: into the Outbox, sent once it's time (the app has to be running then).
        if let Some(at) = c.send_after.filter(|at| *at > self.clock.now && !local) {
            let item = OutboxItem { id: c.id, from, recipients, sent_folder, queued: self.clock.now, attempts: 0, error: None, not_before: at };
            let store = self.store.as_ref().ok_or_else(|| CmdError::Failed("there is nowhere to keep it".into()))?;
            store.outbox_add(&c.account, &item, &raw).map_err(|e| CmdError::Failed(format!("the Outbox couldn't keep it: {e}")))?;
            self.after_outbox_change();
            self.finish_sent(c, false);
            // finish_sent says "sent"; it isn't yet.
            self.notices.pop();
            self.notice_with("It will be sent {when}", &[("when", &quatrelle_model::dates::full_time(at, &self.clock))], false);
            return Ok(serde_json::json!({ "queued": c.id, "sendAfter": at }));
        }
        if local {
            // Not kept in Sent: the message stays open, its draft too.
            if let Some(folder) = &sent_folder {
                self.file_locally(&c.account, folder, &raw, true)?;
            }
            self.finish_sent(c, true);
            return Ok(serde_json::json!({ "sent": c.id, "local": true }));
        }
        self.jobs.push(Job::Send { account: c.account.clone(), id: c.id, raw, from, recipients, sent_folder });
        self.mark_sending(c.id);
        Ok(serde_json::json!({ "sending": c.id }))
    }

    pub(super) fn save_pending_marks(&self) {
        let list: Vec<(u64, &MessageKey, &Flag)> = self.pending_marks.iter().map(|(id, (k, f))| (*id, k, f)).collect();
        if let Some(store) = &self.store
            && let Err(e) = store.save_doc(PENDING_MARKS, &list)
        {
            log::warn!("{PENDING_MARKS}: {e}");
        }
    }

    /// The compose is finished. `delivered`: the server accepted it, so the original gets its
    /// Replied or Forwarded flag now; otherwise it sits in the Outbox and the flag is owed until
    /// delivery.
    pub(super) fn finish_sent(&mut self, c: &Compose, delivered: bool) {
        if let (Some(src), Some(mode)) = (&c.source, c.mode) {
            let flag = if mode == ReplyMode::Forward { Flag::Forwarded } else { Flag::Answered };
            if delivered {
                self.mark_original(src.clone(), flag);
            } else if self.pending_marks.len() < 1000 {
                self.pending_marks.insert(c.id, (src.clone(), flag));
                self.save_pending_marks();
            }
        }
        self.remove_saved_copies(c);
        if self.compose.as_ref().is_some_and(|x| x.id == c.id) {
            self.compose = None;
        }
        self.composes.retain(|x| x.id != c.id);
        self.set_aside.retain(|x| x.id != c.id);
        self.notice("Message sent", false);
    }

    /// A reply or forward delivered: its original marked replied to or forwarded. Exchange keeps
    /// the last thing done with a message, and Outlook shows that one: there the other arrow
    /// goes first (the server, told the new one, has no use for that change). IMAP and JMAP keep
    /// both marks, each a fact of its own.
    pub(super) fn mark_original(&mut self, key: MessageKey, flag: Flag) {
        let exchange = self.account(&key.account).is_some_and(|a| a.config.ews.is_some() || a.config.graph.is_some());
        let other = if flag == Flag::Forwarded { Flag::Answered } else { Flag::Forwarded };
        if exchange && self.summary(&key).is_some_and(|m| other.get(&m.flags)) {
            self.apply_op(&key.account.clone(), Op::SetFlag { keys: vec![key.clone()], flag: other, on: false });
        }
        self.apply_op(&key.account.clone(), Op::SetFlag { keys: vec![key], flag, on: true });
    }

    /// Save the message being written to Drafts.
    pub fn save_draft(&mut self) -> CmdResult {
        let c = self.compose.clone().ok_or_else(|| CmdError::Disabled("nothing to save".into()))?;
        let (draft, saved_id) = self.file_draft(&c, false)?;
        if let Some(cm) = self.compose.as_mut() {
            cm.draft = draft;
            cm.saved_id = saved_id;
        }
        self.notice("Draft saved", false);
        Ok(serde_json::json!({ "saved": c.id }))
    }

    /// Every message being written with something in it, kept as a draft as Quatrelle closes:
    /// on this computer at once, and in Drafts on the server at the next connection. How many.
    pub fn keep_drafts(&mut self) -> usize {
        let written: Vec<Compose> = self.all_composes().filter(|c| self.written(c) && !c.sending).cloned().collect();
        let mut kept = 0;
        for c in written {
            if self.file_draft(&c, true).is_ok() {
                kept += 1;
            }
        }
        kept
    }

    /// The job filing `c`'s draft `raw` in `folder` on the server in place of `replaces`
    /// (`keep`: kept on this computer first, as Quatrelle closes): encrypted to the account's
    /// own certificate (or OpenPGP key) first when the message is to be encrypted.
    fn draft_job(&self, c: &Compose, raw: Vec<u8>, folder: String, keep: bool, replaces: Option<MessageKey>) -> Job {
        let config = self.account(&c.account).map(|a| a.config.clone());
        match config {
            Some(account) if self.seals_with_pgp(c) => {
                Job::Pgp { account, work: quatrelle_model::job::PgpWork::SealDraft { raw: quatrelle_model::Private(raw), folder, replaces, keep } }
            }
            Some(account) if c.encrypt && self.identity(&c.account).is_some() => {
                Job::SealDraft { account, raw: quatrelle_model::Private(raw), folder, replaces, keep }
            }
            _ if keep => Job::KeepDraft { account: c.account.clone(), raw, folder, replaces },
            _ => Job::SaveDraft { account: c.account.clone(), raw, folder, replaces },
        }
    }

    /// A message as a draft's source, the Drafts folder it goes in, and the `Message-ID` it goes
    /// by (the one its earlier copies went by, so that they're found).
    pub(super) fn draft_bytes(&mut self, c: &Compose) -> Result<(Vec<u8>, String, String), CmdError> {
        // Drafts may hold unfinished addresses: keep only the valid ones. A contact group not yet
        // expanded (`Team <group:ID>`, which has no address) is one of them: the copy filed here
        // or on the server carries the addresses alone, with no error, and the group stays only
        // in the message being written. Expanding it at every save would make saving wait on the
        // server; a draft reopened from Drafts has its addresses, and the group is typed again.
        let lenient = |s: &str| addr::split_entries(s).into_iter().filter_map(addr::parse_one).collect::<Vec<_>>();
        let (body, html, inline) = c.bodies();
        let d = mime::Draft {
            reply_to: lenient(&c.reply_to),
            to: lenient(&c.to),
            cc: lenient(&c.cc),
            bcc: lenient(&c.bcc),
            subject: c.subject.clone(),
            body,
            in_reply_to: c.in_reply_to.clone(),
            references: c.references.clone(),
            attachments: c.attachments.clone(),
            html,
            inline,
            importance: c.importance,
            read_receipt: c.read_receipt,
            voting: String::new(),
            seven_bit: false,
        };
        // A server account without one gets Drafts made where it's filed, as Outlook makes it
        // (refused, the message was lost as Quatrelle closed).
        let account = self.account(&c.account);
        let folder = match account.and_then(|a| a.folder(FolderRole::Drafts)) {
            Some(f) => f.path.clone(),
            None if account.is_some_and(|a| !a.is_local()) => "Drafts".to_string(),
            None => return Err(CmdError::Failed("this account has no Drafts folder".into())),
        };
        let earlier = Some(c.saved_id.as_str()).filter(|m| !m.is_empty());
        let (raw, _, mid) = self.compose_bytes_as(c, &d, earlier)?;
        // Its copy keeps its Bcc and the message it answers, as Outlook's drafts do (opened
        // again, it had neither).
        let source = c.source.as_ref().zip(c.mode).map(|(k, m)| (m, k.to_id()));
        let raw = mime::draft_copy(raw, &d.bcc, source.as_ref().map(|(m, k)| (*m, k.as_str())));
        Ok((raw, folder, mid))
    }

    /// File a message in a local account's folder; returns its key. A folder that can't be read,
    /// or a source that can't be kept, changes nothing: the folder was saved as if empty (its
    /// mail gone), or listed a message that had no source.
    pub(super) fn file_locally(&mut self, account: &str, folder: &str, raw: &[u8], seen: bool) -> Result<MessageKey, CmdError> {
        let store = self.store.clone().ok_or_else(|| CmdError::Failed("there is nowhere to keep it".into()))?;
        // The folder read after the marks made before.
        self.finish_local_marks();
        let failed = |e: quatrelle_store::StoreError| CmdError::Failed(format!("The message couldn't be kept in {folder}: {e}"));
        let mut st = store.load_folder(account, folder).map_err(failed)?;
        if st.validity == 0 {
            st.validity = 1;
        }
        let mut next = st.free_uid(account, folder).map_err(failed)?;
        let uid = quatrelle_store::FolderState::take_uid(&mut next, account, folder).map_err(failed)?;
        st.uid_next = next;
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: st.validity, uid };
        let flags = quatrelle_model::Flags { seen, draft: !seen, ..Default::default() };
        let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
        let s = mime::summary_from_full(raw, key.clone(), flags, size, self.clock.now);
        store.write_body(&key, raw).map_err(failed)?;
        st.messages.push(s);
        sort_newest_first(&mut st.messages);
        store.save_folder(account, folder, &st).map_err(failed)?;
        self.reload_folder(account, folder);
        self.recount_local(account, Some(&[folder.to_string()]));
        self.refresh_list();
        Ok(key)
    }
}

/// Whether an answer can be written from a message's text alone, as far as its content goes. It
/// can't quote pictures not here yet (a `cid:` the text names counts: Microsoft Graph's inline
/// files read as plain ones until the whole message comes), nor forward what's still coming.
fn answerable_from_text(p: &Parsed, mode: ReplyMode) -> bool {
    let pictures = p.attachments.iter().any(|a| a.inline) || p.images.iter().any(|i| i.get(..4).is_some_and(|s| s.eq_ignore_ascii_case("cid:")));
    mode != ReplyMode::Forward && !pictures
}
