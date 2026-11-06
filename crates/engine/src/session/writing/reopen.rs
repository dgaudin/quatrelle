// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A message saved before (a draft from Drafts, a message taken back out of the Outbox), opened
//! to carry on writing it as Outlook opens one: as it was saved. Its recipients (Bcc too, which
//! a draft's copy keeps), subject, importance and read receipt; its text with its formatting and
//! the line over a quote; a signature written in HTML as that HTML again; its pictures in place
//! (they came back as files) and its files; the original under a reply as it came, with what
//! was changed in it; and the message it answers or forwards, which gets its flag once it's
//! sent.

use super::*;

impl Session {
    /// Message `raw`, kept at `key`, as one being written from `account`.
    pub(in crate::session) fn compose_from_saved(&mut self, account: &str, key: &MessageKey, raw: &[u8]) -> Result<Compose, CmdError> {
        let p = mime::parse(raw).ok_or_else(|| CmdError::Failed("the message can't be read".into()))?;
        let html = mime::html_body(raw).filter(|h| p.had_html && !h.trim().is_empty());
        // The pictures its text shows stay in it; every other part is one of its files (a text
        // file's charset kept).
        let shows = |cid: &str| html.as_deref().is_some_and(|h| h.contains(&format!("cid:{cid}")));
        let mut attachments = Vec::new();
        let mut inline: Vec<mime::InlinePicture> = Vec::new();
        for a in &p.attachments {
            let Some(out) = mime::attachment_out(raw, a.index) else { continue };
            match a.content_id.as_deref().map(str::trim).filter(|c| !c.is_empty() && shows(c) && out.mime.starts_with("image/")) {
                Some(cid) if !inline.iter().any(|x| x.cid == cid) => {
                    inline.push(mime::InlinePicture { cid: cid.to_string(), name: a.name.clone(), mime: out.mime, data: out.data })
                }
                Some(_) => {}
                None => attachments.push(OutAttachment { name: a.name.clone(), ..out }),
            }
        }
        let (importance, read_receipt, named) = mime::sending_options(raw);
        // From another address than the account's own: still from it.
        let own = self.account(account).map(|a| a.config.email.clone()).unwrap_or_default();
        let from = if p.from.email.is_empty() || p.from.email.eq_ignore_ascii_case(&own) { String::new() } else { p.from.to_string() };
        let mut c = Compose {
            id: self.fresh_id(),
            account: account.to_string(),
            from,
            to: addr::format_field(&p.to),
            cc: addr::format_field(&p.cc),
            bcc: addr::format_field(&p.bcc),
            show_cc: !p.bcc.is_empty(),
            reply_to: addr::format_field(&p.reply_to),
            show_reply_to: !p.reply_to.is_empty(),
            subject: p.subject.clone(),
            in_reply_to: p.in_reply_to.clone(),
            references: p.references.clone(),
            attachments,
            inline,
            importance,
            read_receipt,
            saved_id: p.message_id.clone(),
            ..Default::default()
        };
        match html.as_deref() {
            Some(h) => {
                // The original under a reply, as it came, with what was changed in it.
                let own = match mime::split_marked(h, mime::QUOTE_CLASS).filter(|(_, q, _)| !q.trim().is_empty()) {
                    Some((own, q, _)) => {
                        c.quote = Some(Arc::new(Quote::saved(key.clone(), &p, q)));
                        own
                    }
                    None => h.to_string(),
                };
                own_text(&mut c, &own);
            }
            None => c.set_text(p.text.clone()),
        }
        // The message it answers or forwards, for its flag once it's sent: as its copy names it,
        // else (a draft saved elsewhere) the one its In-Reply-To names.
        let named = named.and_then(|(mode, k)| MessageKey::from_id(&k).filter(|k| self.account(&k.account).is_some()).map(|k| (k, mode)));
        let found = match named {
            Some(n) => Some(n),
            None => self.find_message(&key.account, &p.in_reply_to).map(|k| (k, ReplyMode::Reply)),
        };
        if let Some((src, mode)) = found {
            c.source = Some(src);
            c.mode = Some(mode);
        }
        Ok(c)
    }
}

/// The text of a saved message, from its HTML, into `c` with its formatting; a signature written
/// in HTML ([`mime::SIGNATURE_CLASS`]) stands where it was, as that HTML again.
fn own_text(c: &mut Compose, html: &str) {
    let rich = |h: &str| {
        let (mut text, mut styles) = mime::rich::from_rich(&mime::html_to_rich(h));
        tight_rules(&mut text, &mut styles);
        (text, styles)
    };
    let Some((before, code, after)) = mime::split_marked(html, mime::SIGNATURE_CLASS).filter(|(_, code, _)| !code.trim().is_empty()) else {
        (c.body, c.styles) = rich(html);
        return;
    };
    let (mut text, mut styles) = rich(&before);
    while text.ends_with('\n') {
        text.pop();
        styles.fit(&text);
    }
    let sig = mime::rich::SIGNATURE.to_string();
    if !text.is_empty() {
        mime::rich::append(&mut text, &mut styles, "\n", &mime::rich::Styles::plain("\n"));
    }
    mime::rich::append(&mut text, &mut styles, &sig, &mime::rich::Styles::plain(&sig));
    let (rest, rest_styles) = rich(&after);
    if !rest.trim().is_empty() {
        mime::rich::append(&mut text, &mut styles, "\n", &mime::rich::Styles::plain("\n"));
        mime::rich::append(&mut text, &mut styles, &rest, &rest_styles);
    }
    c.body = text;
    c.styles = styles;
    c.signature = sig;
    c.signature_html = code;
}

/// A line across the text (the one over the original under a reply), read back from HTML, comes
/// with an empty line after it that wasn't written (its paragraph's own room): that one goes,
/// or each save added one.
fn tight_rules(text: &mut String, styles: &mut mime::rich::Styles) {
    styles.fit(text);
    let lines: Vec<usize> = text.split('\n').map(|l| l.chars().count()).collect();
    let after_rule = |i: usize| i > 0 && styles.para(i - 1).rule && lines.get(i) == Some(&0) && !styles.para(i).rule;
    let gone: Vec<usize> = (1..lines.len()).filter(|&i| after_rule(i)).collect();
    for &i in gone.iter().rev() {
        // The line break before it: the empty line joins the rule's, which keeps its layout.
        let start: usize = lines.iter().take(i).map(|n| n + 1).sum();
        mime::rich::splice(text, styles, start.saturating_sub(1)..start, "", &mime::rich::Styles::plain(""));
    }
}
