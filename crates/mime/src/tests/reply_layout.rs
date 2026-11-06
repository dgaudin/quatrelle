// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! MIME tests: the original under a reply or forward, when it's laid out in tables.

use super::*;

/// A signature laid out in nested tables, as business mail writes one: a column of icons beside
/// the numbers and addresses, a logo on the right, a coloured warning line, then a centred logo
/// under it all.
pub(super) const CARD: &str = r##"<html><body>
<p>Dear Sam,</p><p>We will share the updated invoice soon.</p>
<table width="640" cellpadding="0" cellspacing="0"><tr>
  <td width="40"><table><tr><td><img src="cid:phone@x" width="16" alt="phone"></td></tr><tr><td><img src="cid:web@x" width="16" alt="web"></td></tr></table></td>
  <td width="420"><table>
    <tr><td>+00 1234 5678</td></tr>
    <tr><td><a href="https://example.com/">example.com</a> | <b style="color:#ff0000">A warning in red</b></td></tr>
    <tr><td><b>Office</b>: 1 Example Road, Example Town</td></tr>
  </table></td>
  <td width="180" align="right"><img src="cid:logo@x" width="120" alt="logo"></td>
</tr></table>
<table width="640"><tr><td align="center"><img src="cid:logo2@x" width="90" alt="mill"></td></tr></table>
<hr><p><b>From:</b> Someone</p>
</body></html>"##;

pub(super) fn card() -> Parsed {
    let raw = format!(
        "From: Morgan <morgan@mill.example>\r\nTo: sam@studio.example\r\nSubject: Invoice\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{CARD}\r\n"
    );
    parse(raw.as_bytes()).unwrap()
}

/// The owner's report upstream: Reply turned a signature in layout tables into `│` and `├──`
/// lines padded with spaces. The original goes as its cells one after another, keeping its
/// pictures, link, colour and alignment.
#[test]
fn a_reply_quotes_a_table_layout_cleanly() {
    let p = card();
    assert!(!p.layout.is_empty(), "the card has columns");
    for mode in [ReplyMode::Reply, ReplyMode::ReplyAll, ReplyMode::Forward] {
        let (d, styles) = reply_doc(&p, mode, &[], &Clock::default(), "", &QuoteWords::default());
        let boxy = |c: char| ('\u{2500}'..='\u{257F}').contains(&c);
        assert!(!d.body.chars().any(boxy), "{mode:?}: {}", d.body);
        // No line padded out to a column.
        assert!(d.body.lines().all(|l| !l.starts_with("    ")), "{}", d.body);
        for want in ["Dear Sam,", "+00 1234 5678", "A warning in red", "Office: 1 Example Road, Example Town"] {
            assert!(d.body.lines().any(|l| l.contains(want)), "{want} in {}", d.body);
        }
        let pictures = rich::pictures(&d.body, &styles);
        assert_eq!(pictures, ["phone@x", "web@x", "logo@x", "logo2@x"], "{}", d.body);
        let html = rich::to_html(&d.body, &styles);
        assert!(html.contains("<span style=\"color:#ff0000\"><b>A warning in red</b></span>"), "{html}");
        assert!(html.contains("<a href=\"https://example.com/\""), "{html}");
        assert!(html.contains("<img src=\"cid:logo@x\" width=\"120\""), "{html}");
        // The right-hand and centred cells keep their places.
        assert!(html.contains("<div style=\"text-align:right\"><img src=\"cid:logo@x\""), "{html}");
        assert!(html.contains("<div style=\"text-align:center\"><img src=\"cid:logo2@x\""), "{html}");
        assert!(!html.contains('│'), "{html}");
    }
}

/// The pictures a reply carries on are found in the layout too: one the message keeps by its
/// address (`Content-Location`) goes under the reply's own `cid:`.
#[test]
fn quoted_pictures_reach_the_layout() {
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let html = CARD.replace("cid:logo@x", "logo.png");
    let raw = format!(
        "From: Morgan <morgan@mill.example>\r\nTo: sam@studio.example\r\nSubject: Invoice\r\nMIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=\"R\"\r\n\r\n\
--R\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n\
--R\r\nContent-Type: image/png\r\nContent-Location: logo.png\r\nContent-Transfer-Encoding: base64\r\n\r\n{png}\r\n--R--\r\n"
    );
    let mut p = parse(raw.as_bytes()).unwrap();
    assert!(!p.layout.is_empty());
    let (pictures, _) = quoted_pictures(&mut p, raw.as_bytes());
    let cid = pictures.first().map(|q| q.cid.clone()).unwrap_or_default();
    assert!(!cid.is_empty(), "{pictures:?}");
    let (d, styles) = reply_doc(&p, ReplyMode::Forward, &[], &Clock::default(), "", &QuoteWords::default());
    assert!(rich::pictures(&d.body, &styles).contains(&cid), "{}", d.body);
}
