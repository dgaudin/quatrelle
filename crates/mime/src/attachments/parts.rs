// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What Quatrelle reads of mail-parser's parts: the files a message carries, a part's text and
//! bytes, and its header fields as written.

use std::borrow::Cow;

use mail_parser::{Charset, DecodeProblems, Encoding, MessagePart, MessageRef, PartKind, PartRole};

/// The parts of `m` (not those of the messages attached to it) the attachment bar lists, in the
/// order they come: mail-parser's attachments (RFC 8621), and also a picture, sound or video the
/// body shows in both its text and HTML versions, which RFC 8621 leaves out of the attachments
/// but which the reading pane finds its `cid:` pictures among, and a part in no list at all, so
/// nothing a message carries goes unseen.
pub(crate) fn files_of<'m>(m: MessageRef<'m>) -> impl Iterator<Item = MessagePart<'m>> {
    m.parts().filter(|p| p.is_attachment() || matches!(p.role(), PartRole::MediaBody | PartRole::Dropped))
}

/// A part's bytes as the reading pane takes them: a text part's text (read in its charset, as
/// UTF-8), any other's bytes with the transfer encoding undone (an attached message's source),
/// and nothing for a multipart.
pub(crate) fn contents<'m>(p: MessagePart<'m>) -> Cow<'m, [u8]> {
    match p.kind() {
        PartKind::Text | PartKind::Html => {
            let bytes = text_bytes(p);
            let charset = charset_of(&p, &bytes);
            match bytes {
                Cow::Borrowed(b) => match charset.decode(b) {
                    Cow::Borrowed(t) => Cow::Borrowed(t.as_bytes()),
                    Cow::Owned(t) => Cow::Owned(t.into_bytes()),
                },
                Cow::Owned(b) => Cow::Owned(charset.decode_owned(b).into_bytes()),
            }
        }
        PartKind::Multipart => Cow::Borrowed(&[]),
        PartKind::Binary | PartKind::InlineBinary | PartKind::Message(_) => p.decoded(),
    }
}

/// What a part says ASCII is called (`us-ascii` and its other names), which mail-parser reads as
/// Windows-1252, as browsers do.
const ASCII_NAMES: [&str; 12] = [
    "us-ascii",
    "us_ascii",
    "ascii",
    "ansi_x3.4-1968",
    "ansi_x3.4-1986",
    "iso-ir-6",
    "iso646-us",
    "iso_646.irv:1991",
    "us",
    "ibm367",
    "cp367",
    "csascii",
];

/// The charset a text part's `bytes` are read in: the one it names (UTF-8 when it names none, or
/// one unknown), except that a part said to be ASCII whose bytes aren't, but are UTF-8, is read as
/// UTF-8: a newsletter said `us-ascii` and wrote « répondre » in UTF-8, which came out
/// « rÃ©pondre ». Bytes that aren't UTF-8 either are read as they were.
fn charset_of(p: &MessagePart<'_>, bytes: &[u8]) -> Charset {
    let label = p.content_type().and_then(|c| c.attribute("charset")).map(str::trim);
    let named = label.and_then(|c| Charset::from_label(c.as_bytes())).unwrap_or_default();
    let ascii = label.is_some_and(|l| ASCII_NAMES.iter().any(|a| l.eq_ignore_ascii_case(a)));
    if ascii && !bytes.is_ascii() && std::str::from_utf8(bytes).is_ok() { Charset::Utf8 } else { named }
}

/// A text or HTML part's text, the transfer encoding undone and read in its charset
/// ([`charset_of`]), as mail-parser's `text()` otherwise reads it; `None` for other parts.
pub(crate) fn part_text<'m>(p: MessagePart<'m>) -> Option<Cow<'m, str>> {
    if !p.is_text() {
        return None;
    }
    let bytes = p.decoded();
    let charset = charset_of(&p, &bytes);
    Some(match bytes {
        Cow::Borrowed(b) => charset.decode(b),
        Cow::Owned(b) => Cow::Owned(charset.decode_owned(b)),
    })
}

/// The `index`-th part of a message's text body as text (an HTML part made text), as
/// mail-parser's `body_text`, each read by [`part_text`].
pub(crate) fn body_text<'m>(m: MessageRef<'m>, index: usize) -> Option<Cow<'m, str>> {
    let p = m.text_body().nth(index)?;
    match p.kind() {
        PartKind::Text => part_text(p),
        PartKind::Html => part_text(p).map(|h| Cow::Owned(mail_parser::html_to_text(&h))),
        _ => None,
    }
}

/// The `index`-th part of a message's HTML body as HTML (a text part made HTML), as
/// mail-parser's `body_html`, each read by [`part_text`].
pub(crate) fn body_html<'m>(m: MessageRef<'m>, index: usize) -> Option<Cow<'m, str>> {
    let p = m.html_body().nth(index)?;
    match p.kind() {
        PartKind::Html => part_text(p),
        PartKind::Text => part_text(p).map(|t| Cow::Owned(mail_parser::text_to_html(&t))),
        _ => None,
    }
}

/// A text part's bytes, the transfer encoding undone. One whose base64 can't be read is its
/// source as it came: text sent as base64 by mistake stays readable (mail-parser decodes what
/// it can of it, passing over what isn't base64, which turns such text into noise).
fn text_bytes(p: MessagePart<'_>) -> Cow<'_, [u8]> {
    if p.encoding() != Encoding::Base64 {
        return p.decoded();
    }
    match p.decoded_checked() {
        (_, problems) if problems.contains(DecodeProblems::MALFORMED_TRANSFER_ENCODING) => Cow::Borrowed(p.raw_body()),
        (bytes, _) => bytes,
    }
}

/// A text file's bytes as they were sent, only the transfer encoding undone: [`contents`] gives
/// a `text/*` part as text, read in its charset or, with none, as UTF-8 with any other byte
/// replaced, so a file in Latin-1 or Windows-1252 would be saved changed. `None` for other
/// parts, whose bytes are already these.
pub(crate) fn sent_bytes(p: MessagePart<'_>) -> Option<Vec<u8>> {
    p.is_text().then(|| text_bytes(p).into_owned())
}

/// The value of a message's last `name` header field as written (folding and all), when it is
/// UTF-8.
pub(crate) fn header_raw<'m>(m: MessageRef<'m>, name: &str) -> Option<&'m str> {
    m.headers().get(name).and_then(|h| std::str::from_utf8(h.raw_value()).ok())
}

/// How much of a message's header is read for what the receiving server wrote in it.
const SERVER_HEADER_MAX: usize = 256 << 10;

/// Whether an `Authentication-Results` field of `raw`'s header (RFC 8601) says the message
/// failed its sender's domain's check (`dmarc=fail`). Only a failure is read: anyone on the way
/// can write a pass, and a failure added by the sender only works against them.
pub(crate) fn dmarc_failed(raw: &[u8]) -> bool {
    let head = String::from_utf8_lossy(raw.get(..raw.len().min(SERVER_HEADER_MAX)).unwrap_or_default());
    let says_so = |field: &str| {
        let f: String = field.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
        f.starts_with("authentication-results:") && f.contains("dmarc=fail")
    };
    let mut field = String::new();
    for line in head.lines() {
        if line.is_empty() {
            break;
        }
        if !line.starts_with([' ', '\t']) {
            if says_so(&field) {
                return true;
            }
            field.clear();
        }
        field.push_str(line);
    }
    says_so(&field)
}

#[cfg(test)]
mod tests {
    /// A message of one part of `content_type`, its body as it came.
    fn message(content_type: &str, body: &[u8]) -> Vec<u8> {
        let mut raw = format!(
            "From: Studio <news@contoso.example>\r\nTo: riley@contoso.example\r\nSubject: Survey\r\nMessage-ID: <survey@contoso.example>\r\n\
             MIME-Version: 1.0\r\nContent-Type: {content_type}\r\nContent-Transfer-Encoding: 8bit\r\n\r\n"
        )
        .into_bytes();
        raw.extend_from_slice(body);
        raw
    }

    /// A part said to be ASCII (`us-ascii`, as a newsletter's HTML said) that writes « répondre »
    /// in UTF-8, its hidden preview line too: read as UTF-8, in the message, in its list row's
    /// preview and in what search finds, the text part's and the HTML part's alike. It read
    /// « rÃ©pondre » (ASCII is read as Windows-1252).
    #[test]
    fn a_part_said_to_be_ascii_that_is_utf8_reads_as_utf8() {
        let words = "Une minute pour répondre à notre enquête";
        for (ct, body) in [
            ("text/html; charset=\"us-ascii\"", format!("<html><body><div style=\"display:none\">{words}</div><p>{words}</p></body></html>")),
            ("text/plain; charset=US-ASCII", format!("{words}\r\n")),
            ("text/plain; charset=ascii", format!("{words}\r\n")),
        ] {
            let raw = message(ct, body.as_bytes());
            let p = crate::parse(&raw).unwrap_or_else(|| panic!("{ct}: a message"));
            assert!(p.text.contains(words), "{ct}: the message's text: {}", p.text);
            let rich: String = p.rich.iter().flatten().map(|s| s.text.as_str()).collect();
            assert!(!rich.contains('Ã') && (rich.is_empty() || rich.contains(words)), "{ct}: the message as shown: {rich}");
            let preview = crate::preview_from_source(&raw);
            assert!(preview.contains("répondre") && !preview.contains('Ã'), "{ct}: the list's preview: {preview}");
            assert!(crate::search_text(&raw).contains("répondre"), "{ct}: what search finds");
        }
    }

    /// A part said to be ASCII that is: as before. One that isn't UTF-8 either (Latin-1 sent as
    /// `us-ascii`): as before, read as Windows-1252. A part in another charset: in its own, even
    /// when it's UTF-8 sent as Latin-1.
    #[test]
    fn other_parts_read_as_before() {
        let text = |ct: &str, body: &[u8]| crate::parse(&message(ct, body)).map(|p| p.text.trim().to_string()).unwrap_or_default();
        assert_eq!(text("text/plain; charset=us-ascii", b"Plain words, nothing else.\r\n"), "Plain words, nothing else.");
        assert_eq!(
            crate::preview_from_source(&message("text/plain; charset=us-ascii", b"Plain words, nothing else.\r\n")),
            "Plain words, nothing else."
        );
        assert_eq!(text("text/plain; charset=us-ascii", b"Caf\xe9 cr\xe8me\r\n"), "Café crème");
        assert_eq!(text("text/plain; charset=iso-8859-1", "répondre".as_bytes()), "rÃ©pondre");
        assert_eq!(text("text/plain", "répondre".as_bytes()), "répondre");
    }
}
