// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A message saved before (a draft, a message held in the Outbox), read back to carry on writing
//! it as it was saved: what a draft's copy keeps that mail sent doesn't (its Bcc, the message it
//! answers or forwards), what it asks of its sending (importance, a read receipt), and the parts
//! of its HTML Quatrelle marked (a signature written in HTML, the original under a reply).

use mail_builder::headers::Header;
use mail_builder::headers::address::Address as BAddress;

use super::*;

/// The header a draft's copy names the message it answers or forwards in: `reply` or `forward`,
/// then the message's key in hexadecimal (a key holds a folder's name, which could hold
/// anything). Never in mail sent.
pub const SOURCE_HEADER: &str = "X-Quatrelle-Source";

/// The class of the `<div>` a signature written in HTML goes out in, where it stands in the text
/// ([`split_marked`]).
pub const SIGNATURE_CLASS: &str = "quatrelle-signature";

/// The class of the `<div>` the original under a reply or forward goes out in, after the
/// reply's own text ([`split_marked`]).
pub const QUOTE_CLASS: &str = "quatrelle-quote";

/// Most of a source key kept, in characters.
const SOURCE_MAX: usize = 900;

/// `raw` (a message as [`build`] wrote it) as a draft's copy: with its Bcc, which mail sent never
/// carries (it goes in the envelope), and the message it answers or forwards (`source`, as
/// [`source_of`] reads it back). Outlook's drafts keep both.
pub fn draft_copy(raw: Vec<u8>, bcc: &[EmailAddress], source: Option<(ReplyMode, &str)>) -> Vec<u8> {
    let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 2) else { return raw };
    let mut head: Vec<u8> = Vec::new();
    let bcc: Vec<BAddress<'_>> = bcc
        .iter()
        .filter(|a| !a.email.trim().is_empty() && !a.email.chars().any(char::is_control))
        .map(|a| BAddress::new_address(Some(a.name.trim().to_string()).filter(|n| !n.is_empty()), a.email.trim().to_string()))
        .collect();
    if !bcc.is_empty() {
        head.extend_from_slice(b"Bcc: ");
        BAddress::new_list(bcc).write_header(&mut head, 5);
    }
    if let Some((mode, key)) = source {
        let word = if mode == ReplyMode::Forward { "forward" } else { "reply" };
        let hex: String = key.chars().take(SOURCE_MAX).collect::<String>().bytes().map(|b| format!("{b:02x}")).collect();
        if !hex.is_empty() {
            head.extend_from_slice(format!("{SOURCE_HEADER}: {word} {hex}\r\n").as_bytes());
        }
    }
    let mut out = Vec::with_capacity(raw.len() + head.len());
    out.extend_from_slice(raw.get(..end).unwrap_or_default());
    out.extend_from_slice(&head);
    out.extend_from_slice(raw.get(end..).unwrap_or_default());
    out
}

/// What a saved message asks of its sending: its importance (high 1, low -1), whether it asks
/// for a read receipt, and the message it answers or forwards (its mode and key, as
/// [`draft_copy`] wrote them).
pub fn sending_options(raw: &[u8]) -> (i8, bool, Option<(ReplyMode, String)>) {
    let Some(m) = MessageParser::default().parse(raw) else { return (0, false, None) };
    let receipt = header_raw(m.root(), "Disposition-Notification-To").is_some_and(|v| !v.trim().is_empty());
    (importance(&m), receipt, header_raw(m.root(), SOURCE_HEADER).and_then(source_of))
}

/// The message a draft of a forward forwards, as its [`SOURCE_HEADER`] names it (a key's id,
/// [`MessageKey::to_id`]); empty for any other message, a reply's draft included (it is known by
/// its In-Reply-To).
pub(crate) fn forward_of(m: &Message<'_>) -> String {
    let named = header_raw(m.root(), SOURCE_HEADER).and_then(source_of);
    match named {
        Some((ReplyMode::Forward, key)) => MessageKey::from_id(&key).map(|k| k.to_id()).unwrap_or_default(),
        _ => String::new(),
    }
}

/// A [`SOURCE_HEADER`] read back: its mode and the message's key.
fn source_of(v: &str) -> Option<(ReplyMode, String)> {
    let (word, hex) = v.trim().split_once(' ')?;
    let mode = match word {
        "forward" => ReplyMode::Forward,
        "reply" => ReplyMode::Reply,
        _ => return None,
    };
    let hex = hex.trim();
    if hex.len() % 2 != 0 || hex.len() > SOURCE_MAX * 8 {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..hex.len()).step_by(2).map(|i| hex.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok())).collect();
    let key = String::from_utf8(bytes?).ok()?;
    (!key.is_empty() && !key.chars().any(char::is_control)).then_some((mode, key))
}

/// HTML Quatrelle wrote with a part marked `class` in it (the original under a reply, a
/// signature written in HTML): the HTML before it, the part's own and what follows it; `None`
/// when there's none. The part ends at the `</div>` that closes its `<div>`, the ones in it
/// counted (one never closed runs to the end).
pub fn split_marked(html: &str, class: &str) -> Option<(String, String, String)> {
    let mark = format!("<div class=\"{class}\">");
    let start = html.find(&mark)?;
    let inner = start + mark.len();
    let lower = html.to_ascii_lowercase();
    let (mut depth, mut at) = (1usize, inner);
    let end = loop {
        let Some(rest) = lower.get(at..) else { break html.len() };
        let open = rest.find("<div").map(|i| at + i);
        let close = rest.find("</div").map(|i| at + i);
        match (open, close) {
            (Some(o), Some(c)) if o < c => {
                depth += 1;
                at = o + 4;
            }
            (_, Some(c)) => {
                depth -= 1;
                if depth == 0 {
                    break c;
                }
                at = c + 5;
            }
            (_, None) => break html.len(),
        }
    };
    let after = html.get(end..).and_then(|r| r.find('>').and_then(|e| r.get(e + 1..))).unwrap_or("");
    Some((html.get(..start)?.to_string(), html.get(inner..end)?.to_string(), after.to_string()))
}

/// `p` (a saved message as parsed) with `html`, the part of its HTML that holds the original
/// under it ([`QUOTE_CLASS`]), as its formatted text: that part's layout, lines, pictures and
/// text, as the reading pane would show it.
pub fn quoted_part(p: &Parsed, html: &str) -> Parsed {
    let h = crate::layout::inline_css(html);
    let layout = crate::layout::html_layout(&h);
    let mut images = html_images(&h);
    for src in crate::layout::background_pictures(&layout) {
        if !images.contains(&src) && images.len() < 200 {
            images.push(src);
        }
    }
    Parsed { rich: html_to_rich(&h), layout, images, text: html_to_text(&h), had_html: true, ..p.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A draft's copy keeps its Bcc and the message it answers, and reads them back; a folder's
    /// name in the key, whatever it holds, comes back as it was.
    #[test]
    fn a_drafts_copy_keeps_its_bcc_and_what_it_answers() {
        let d = Draft { to: vec![EmailAddress::new("Ann", "ann@x.example")], subject: "Plans".into(), body: "Hi".into(), ..Default::default() };
        let raw = build(&d, &EmailAddress::new("Me", "me@x.example"), 1_700_000_000, "m@x").unwrap_or_default();
        let key = "acc:7:12:Boîte de réception\r\nX-Evil: 1";
        let bcc = [EmailAddress::new("Boss Person", "boss@x.example")];
        let copy = draft_copy(raw, &bcc, Some((ReplyMode::Forward, key)));
        let p = parse(&copy).unwrap_or_default();
        assert_eq!(p.bcc.first().map(|a| a.email.as_str()), Some("boss@x.example"));
        assert_eq!(p.subject, "Plans");
        // Written in hexadecimal, a key with a line break in it can't add a header, and isn't
        // taken back.
        assert!(!String::from_utf8_lossy(&copy).contains("X-Evil"));
        assert_eq!(sending_options(&copy).2, None);
        let ok = draft_copy(copy.clone(), &[], Some((ReplyMode::Reply, "acc:7:12:Boîte")));
        assert_eq!(sending_options(&ok).2, Some((ReplyMode::Reply, "acc:7:12:Boîte".to_string())));
        for junk in ["", "reply", "reply zz", "forward 0", "other 41"] {
            assert_eq!(source_of(junk), None, "{junk}");
        }
    }

    #[test]
    fn a_marked_part_is_found_with_the_divs_inside_it() {
        let html = "<div>Hi</div><div class=\"quatrelle-quote\"><div><div>a</div></div>b</div><p>after</p>";
        let (before, part, after) = split_marked(html, QUOTE_CLASS).unwrap_or_default();
        assert_eq!((before.as_str(), part.as_str(), after.as_str()), ("<div>Hi</div>", "<div><div>a</div></div>b", "<p>after</p>"));
        assert!(split_marked("<div>none</div>", QUOTE_CLASS).is_none());
        let (_, open, after) = split_marked("<div class=\"quatrelle-quote\"><div>never closed", QUOTE_CLASS).unwrap_or_default();
        assert_eq!((open.as_str(), after.as_str()), ("<div>never closed", ""));
    }
}
