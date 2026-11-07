// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Graph: a message filed in a folder as mail received and read (a `.msg` or `.eml` dropped on
//! a folder), not as a draft. Graph makes a draft of any message made from its MIME source, and
//! only at creation takes the message's flags (`PidTagMessageFlags`), so the message is made
//! from its fields instead, with the flags and the delivery and submit times as extended
//! properties; one too big for that (files over 3 MB together) goes the old way, as a draft.

use super::*;

/// The most attachment bytes sent inline with the message (Graph takes a request up to 4 MB).
const INLINE_MAX: usize = 3 * 1024 * 1024;

/// The JSON of a message to make (`None` when it has too much to send in one request).
pub(super) fn received_json(raw: &[u8]) -> Option<Value> {
    let p = mime::parse(raw)?;
    let addr = |a: &quatrelle_model::EmailAddress| json!({"emailAddress": {"name": a.name, "address": a.email}});
    let (kind, content) = match mime::html_body(raw) {
        Some(h) => ("html", h),
        None => ("text", p.text.clone()),
    };
    let mut v = json!({
        "subject": p.subject,
        "body": {"contentType": kind, "content": content},
        "toRecipients": p.to.iter().map(addr).collect::<Vec<_>>(),
        "ccRecipients": p.cc.iter().map(addr).collect::<Vec<_>>(),
    });
    if !p.from.email.is_empty() {
        v["from"] = addr(&p.from);
    }
    if !p.message_id.is_empty() {
        v["internetMessageId"] = json!(format!("<{}>", p.message_id));
    }
    // `PidTagMessageFlags` 1 is `MSGFLAG_READ`: read, and not unsent (a draft).
    let mut props = vec![json!({"id": "Integer 0x0E07", "value": "1"})];
    if p.date > 0 {
        let when = quatrelle_jmap::utc_date_string(p.date);
        props.push(json!({"id": "SystemTime 0x0E06", "value": when}));
        props.push(json!({"id": "SystemTime 0x0039", "value": when}));
    }
    v["singleValueExtendedProperties"] = json!(props);
    if !p.attachments.is_empty() {
        let wanted: Vec<usize> = p.attachments.iter().map(|a| a.index).collect();
        let data = mime::attachments_data(raw, &wanted, INLINE_MAX, INLINE_MAX);
        if data.len() < p.attachments.len() {
            return None;
        }
        let mut files = Vec::new();
        for a in &p.attachments {
            let bytes = &data.iter().find(|(i, _)| *i == a.index)?.1;
            files.push(json!({
                "@odata.type": "#microsoft.graph.fileAttachment", "name": a.name, "contentType": a.mime,
                "contentBytes": crate::imap::base64(bytes), "isInline": a.inline, "contentId": a.content_id,
            }));
        }
        v["attachments"] = json!(files);
    }
    Some(v)
}

impl Client {
    /// A message from its MIME source, made in `folder` as mail received and read; its id. One
    /// the fields can't carry (or a server that refuses them) is made as a draft the old way and
    /// marked read.
    pub fn import_received(&self, raw: &[u8], folder: &str, owner: Option<&str>) -> Result<String> {
        let me = who(owner);
        if let Some(body) = received_json(raw).and_then(|v| serde_json::to_vec(&v).ok())
            && let Ok(v) = self.post_raw(&format!("{me}/mailFolders/{}/messages", seg(folder)), body)
            && let Some(id) = v.get("id").and_then(Value::as_str)
        {
            return Ok(id.to_string());
        }
        let id = self.import(raw, Some(folder), owner)?;
        let read = serde_json::to_vec(&json!({"isRead": true})).map_err(|e| NetError::Protocol(e.to_string()))?;
        self.send_raw("PATCH", &format!("{me}/messages/{}", seg(&id)), "application/json", read)?;
        Ok(id)
    }
}
