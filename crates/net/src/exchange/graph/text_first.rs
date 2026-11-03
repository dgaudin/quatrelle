// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
//! A message's text first, from Microsoft Graph: its body, headers and the list of its files
//! without their bytes, in one request, to read while its whole source comes. Graph gives no
//! message sizes, so every message with files can come this way.

use quatrelle_model::EmailAddress;

use super::*;

/// What's asked for besides the files: the headers to show and the body. `contentId` is left out:
/// a derived type's field needs its own `$select`, and a picture that can't be found yet shows
/// the spinner.
const FIELDS: &str = "subject,body,from,sender,toRecipients,ccRecipients,replyTo,sentDateTime,receivedDateTime,internetMessageId,importance";
/// The files' names, types and sizes. Their `@odata.type`, which Graph writes unasked on a
/// derived type, tells an attached message from a file.
const FILE_FIELDS: &str = "$expand=attachments($select=name,contentType,size,isInline)";
const ANY_FILE: &str = "application/octet-stream";
/// Most recipients of one kind read, as in a message's headers.
const MOST_RECIPIENTS: usize = 500;

/// An address as the list reads it, a name alone kept: Exchange's own addresses (a colleague's on
/// premises) aren't addresses, and the engine reads an empty one as "not known yet".
fn address(v: &Value) -> Option<EmailAddress> {
    let a = v.get("emailAddress")?;
    let name = a.get("name").and_then(Value::as_str).unwrap_or_default().trim();
    let email = a.get("address").and_then(Value::as_str).map(str::trim).filter(|e| e.contains('@')).unwrap_or_default();
    (!name.is_empty() || !email.is_empty()).then(|| EmailAddress::new(name, email))
}

fn addresses(v: &Value, field: &str) -> Vec<EmailAddress> {
    v.get(field).and_then(Value::as_array).map(|l| l.iter().take(MOST_RECIPIENTS).filter_map(address).collect()).unwrap_or_default()
}

/// What Graph writes of a derived type, lower case: `#microsoft.graph.eventMessage`.
fn kind_of(v: &Value) -> String {
    v.get("@odata.type").and_then(Value::as_str).unwrap_or_default().to_ascii_lowercase()
}

/// A file in the list. An attached message has no `contentType` to go by, and a link to a file
/// is no file of the type it says.
fn pending_file(a: &Value) -> mime::PendingFile {
    let kind = kind_of(a);
    let media = if kind.ends_with("itemattachment") {
        "message/rfc822"
    } else if kind.ends_with("referenceattachment") {
        ANY_FILE
    } else {
        a.get("contentType").and_then(Value::as_str).filter(|t| !t.trim().is_empty()).unwrap_or(ANY_FILE)
    };
    mime::PendingFile {
        name: a.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
        mime: media.to_string(),
        size: a.get("size").and_then(Value::as_u64).unwrap_or(0),
        content_id: None,
        inline: a.get("isInline").and_then(Value::as_bool).unwrap_or(false),
    }
}

/// The message in a `$expand=attachments` answer as MIME, its files listed empty. `None` when it
/// is left to come whole: no text or no file to show, or its real content isn't in its text (an
/// invitation, a signed or encrypted message).
pub fn text_first(v: &Value) -> Option<Vec<u8>> {
    if kind_of(v).contains("eventmessage") {
        return None;
    }
    let body = v.get("body")?;
    let content = body.get("content").and_then(Value::as_str).filter(|c| !c.trim().is_empty())?;
    let list = v.get("attachments").and_then(Value::as_array).filter(|l| !l.is_empty())?;
    let protected = |a: &Value| {
        mime::is_protected_file(
            a.get("contentType").and_then(Value::as_str).unwrap_or_default(),
            a.get("name").and_then(Value::as_str).unwrap_or_default(),
        )
    };
    if list.iter().any(protected) {
        return None;
    }
    let files: Vec<mime::PendingFile> = list.iter().take(mime::MOST_PENDING_FILES).map(pending_file).collect();
    let html = !body.get("contentType").and_then(Value::as_str).is_some_and(|t| t.trim().eq_ignore_ascii_case("text"));
    let draft = mime::Draft {
        to: addresses(v, "toRecipients"),
        cc: addresses(v, "ccRecipients"),
        reply_to: addresses(v, "replyTo"),
        subject: v.get("subject").and_then(Value::as_str).unwrap_or_default().to_string(),
        body: if html { String::new() } else { content.to_string() },
        html: html.then(|| content.to_string()),
        importance: match v.get("importance").and_then(Value::as_str).map(str::trim) {
            Some(i) if i.eq_ignore_ascii_case("high") => 1,
            Some(i) if i.eq_ignore_ascii_case("low") => -1,
            _ => 0,
        },
        ..Default::default()
    };
    let from = v.get("from").and_then(address).or_else(|| v.get("sender").and_then(address)).unwrap_or_default();
    let date = ["sentDateTime", "receivedDateTime"].iter().find_map(|f| v.get(*f).and_then(Value::as_str).and_then(jmap::utc_date)).unwrap_or(0);
    let id = v.get("internetMessageId").and_then(Value::as_str).unwrap_or_default();
    mime::text_first(&draft, &from, date, id, &files).ok()
}

impl Client {
    /// Message `id`'s text first, as MIME (`owner`: in another mailbox opened in the account):
    /// `None` when [`text_first()`] leaves it whole.
    pub fn download_text(&self, id: &str, owner: Option<&str>) -> Result<Option<Vec<u8>>> {
        let v = self.get(&format!("{}/messages/{}?$select={FIELDS}&{FILE_FIELDS}", who(owner), seg(id)))?;
        Ok(text_first(&v))
    }
}

/// A cached message's text and its id on the server, for a message without its source here and
/// with an id there: `None` otherwise (the whole path, [`fetch_body`], tells a message is gone),
/// or when it's left whole. Nothing is kept: the text isn't the message.
pub fn fetch_text(c: &Client, st: &State, store: &Store, key: &MessageKey) -> Result<Option<(Vec<u8>, String)>> {
    if key.validity != VALIDITY || store.has_body(key) {
        return Ok(None);
    }
    let Some(id) = st.id(key.uid) else { return Ok(None) };
    Ok(c.download_text(&id, st.owner(&key.folder))?.map(|raw| (raw, id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A message as Graph answers `$expand=attachments`, its fields and files as given.
    fn message(fields: Value, files: Value) -> Value {
        let mut m = json!({
            "subject": "Rushes reel 2",
            "body": {"contentType": "html", "content": "<p>Here's reel 2.</p>"},
            "from": {"emailAddress": {"name": "Sam Lee", "address": "sam@contoso.example"}},
            "attachments": files,
        });
        if let (Some(m), Some(f)) = (m.as_object_mut(), fields.as_object()) {
            m.extend(f.clone());
        }
        m
    }

    fn pdf() -> Value {
        json!({"@odata.type": "#microsoft.graph.fileAttachment", "name": "plan.pdf", "contentType": "application/pdf", "size": 9, "isInline": false})
    }

    fn built(fields: Value, files: Value) -> Option<mime::Parsed> {
        mime::parse_text_first(&text_first(&message(fields, files))?)
    }

    /// A colleague's mail from Exchange on premises: the addresses are Exchange's own, not
    /// addresses; their names stay, and nothing in a name adds a header.
    #[test]
    fn a_name_alone_stays() {
        let raw = text_first(&message(
            json!({
                "from": {"emailAddress": {"name": "Sam Lee", "address": "/O=CONTOSO/OU=EXCHANGE/CN=SAM"}},
                "toRecipients": [
                    {"emailAddress": {"name": "Riley Chen", "address": "riley@contoso.example"}},
                    {"emailAddress": {"name": "Alex Martin", "address": "/O=CONTOSO/CN=ALEX"}},
                    {"emailAddress": {"name": "Lee\r\nBcc: x@y.example", "address": ""}},
                    {"emailAddress": {"name": "", "address": ""}},
                    {"emailAddress": {}},
                    {},
                ],
                "ccRecipients": [{"emailAddress": {"name": "Zoe Park"}}],
                "replyTo": [{"emailAddress": {"name": "Desk", "address": "desk@contoso.example"}}],
            }),
            json!([pdf()]),
        ))
        .unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("\nBcc:"));
        let p = mime::parse_text_first(&raw).unwrap();
        assert_eq!((p.from.name.as_str(), p.from.email.as_str()), ("Sam Lee", ""));
        let to: Vec<(&str, &str)> = p.to.iter().map(|a| (a.name.as_str(), a.email.as_str())).collect();
        assert_eq!(to, [("Riley Chen", "riley@contoso.example"), ("Alex Martin", ""), ("Lee  Bcc: x@y.example", "")]);
        assert_eq!(p.cc.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Zoe Park"]);
        assert_eq!(p.reply_to[0].email, "desk@contoso.example");
    }

    /// A file, an embedded picture, an attached message, a link to a file, and files that say
    /// less than they could.
    #[test]
    fn the_files_listed_as_graph_gives_them() {
        let raw = text_first(&message(
            json!({}),
            json!([
                {"@odata.type": "#microsoft.graph.fileAttachment", "name": "reel2.mov", "contentType": "video/quicktime", "size": 20971586, "isInline": false},
                {"@odata.type": "#microsoft.graph.fileAttachment", "name": "still.png", "contentType": "image/png", "size": 2048, "isInline": true},
                {"@odata.type": "#microsoft.graph.itemAttachment", "name": "Earlier note", "size": 4096, "isInline": false},
                {"@odata.type": "#microsoft.graph.referenceAttachment", "name": "Plan on the share", "contentType": "text/plain", "size": 150},
                {"name": "typeless.bin", "size": -3},
                {"@odata.type": "#microsoft.graph.fileAttachment", "contentType": " ", "size": "big"},
            ]),
        ))
        .unwrap();
        let p = mime::parse_text_first(&raw).unwrap();
        let files: Vec<(&str, usize)> = p.attachments.iter().map(|a| (a.mime.as_str(), a.size)).collect();
        assert_eq!(
            files,
            [
                ("video/quicktime", 20_971_586),
                ("image/png", 2048),
                ("message/rfc822", 4096),
                ("application/octet-stream", 150),
                ("application/octet-stream", 0),
                ("application/octet-stream", 0),
            ]
        );
        assert_eq!(p.attachments[0].name, "reel2.mov");
        assert!(p.attachments.iter().all(|a| a.content_id.is_none()), "contentId isn't asked for");
        // `isInline` is written as the file's disposition; it is the whole message's parts that
        // tell a picture from a file, once they come.
        let written = String::from_utf8_lossy(&raw).to_ascii_lowercase();
        assert_eq!(written.matches("content-disposition: inline").count(), 1, "{written}");
        assert_eq!(written.matches("content-disposition: attachment").count(), 5, "{written}");
    }

    /// What the text alone wouldn't show, and a message with nothing to show first.
    #[test]
    fn what_is_left_whole() {
        let ok = || built(json!({}), json!([pdf()]));
        assert!(ok().is_some());
        for kind in ["#microsoft.graph.eventMessage", "#Microsoft.Graph.EventMessageRequest", "#microsoft.graph.eventMessageResponse"] {
            assert!(built(json!({"@odata.type": kind}), json!([pdf()])).is_none(), "{kind}");
        }
        assert!(built(json!({"@odata.type": "#microsoft.graph.message"}), json!([pdf()])).is_some(), "an ordinary message says its type");
        for (name, kind) in [
            ("smime.p7m", "application/octet-stream"),
            ("SMIME.P7M", ""),
            ("a.bin", "application/pkcs7-mime"),
            ("a.bin", "application/x-pkcs7-mime"),
            ("a.bin", "multipart/signed"),
        ] {
            let protected = json!({"name": name, "contentType": kind, "size": 9});
            assert!(built(json!({}), json!([pdf(), protected])).is_none(), "{name} {kind}");
        }
        let many: Vec<Value> = (0..600).map(|_| pdf()).chain([json!({"name": "smime.p7m", "size": 9})]).collect();
        assert!(built(json!({}), json!(many)).is_none(), "a signed message is known by a file past the ones listed");
        for body in [
            json!({"contentType": "html", "content": ""}),
            json!({"contentType": "text", "content": " \r\n "}),
            json!({"contentType": "html"}),
            json!({}),
        ] {
            assert!(built(json!({"body": body}), json!([pdf()])).is_none());
        }
        assert!(text_first(&json!({"subject": "no body", "attachments": [pdf()]})).is_none());
        assert!(built(json!({}), json!([])).is_none(), "no file");
        assert!(text_first(&json!({"body": {"content": "x"}})).is_none(), "no list of files");
        assert!(text_first(&json!({"body": {"content": "x"}, "attachments": "none"})).is_none());
        assert!(text_first(&Value::Null).is_none());
    }

    #[test]
    fn text_is_text_in_any_case_and_html_otherwise() {
        let text = built(json!({"body": {"contentType": "TEXT", "content": "Line <one>"}}), json!([pdf()])).unwrap();
        assert!(!text.had_html && text.text.contains("Line <one>"), "{}", text.text);
        assert!(built(json!({"body": {"content": "<p>Hi</p>"}}), json!([pdf()])).unwrap().had_html, "unsaid: html");
        assert!(built(json!({"body": {"contentType": "html", "content": "<p>Hi</p>"}}), json!([pdf()])).unwrap().had_html);
    }

    #[test]
    fn a_date_and_importance_read_as_the_server_says() {
        let sent = built(json!({"sentDateTime": "2026-10-30T08:02:19Z", "receivedDateTime": "2026-10-30T08:03:00Z"}), json!([pdf()])).unwrap();
        assert_eq!(sent.date, jmap::utc_date("2026-10-30T08:02:19Z").unwrap());
        let received = built(json!({"sentDateTime": "not a date", "receivedDateTime": "2026-10-30T08:03:00Z"}), json!([pdf()])).unwrap();
        assert_eq!(received.date, jmap::utc_date("2026-10-30T08:03:00Z").unwrap(), "an odd date sent: received");
        for (level, header) in [("high", Some("Importance: high")), ("LOW", Some("Importance: low")), ("normal", None)] {
            let raw = text_first(&message(json!({"importance": level}), json!([pdf()]))).unwrap();
            let raw = String::from_utf8_lossy(&raw).into_owned();
            assert_eq!(raw.contains("Importance:"), header.is_some(), "{level}");
            assert!(header.is_none_or(|h| raw.contains(h)), "{level}");
        }
    }

    /// More files than a message could show are listed up to the limit.
    #[test]
    fn the_files_listed_have_a_limit() {
        let many: Vec<Value> = (0..600).map(|_| pdf()).collect();
        assert_eq!(built(json!({}), json!(many)).unwrap().attachments.len(), mime::MOST_PENDING_FILES);
    }
}
