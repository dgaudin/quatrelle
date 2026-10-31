//! Replies and forwards quote the pictures of the message (issue #10) and a date for undated mail
//! (from upstream 411d1a8 and d2b25e3).

use super::*;

/// A reply or forward keeps the pictures in the message it quotes (they came out as
/// "[picture]", issue #10), and quotes a message without a `Date` as sent when the server took it in (not
/// on 1 January 1970). Found testing Outlook's `winmail.dat` (issue #18).
#[test]
fn replies_keep_the_quoted_pictures_and_date() {
    let mut s = sample_session("quoted-pictures");
    let raw = b"From: Sam Okafor <sam@studio.example>\r\nTo: riley@studio.example\r\nSubject: Logo\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/related; boundary=\"R\"\r\n\r\n--R\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
<p>Here is the <b>logo</b>:</p><p><img src=\"cid:logo@studio.example\" width=\"40\"></p><p><img src=\"https://cdn.example/poster.jpg\"><img src=\"https://tracker.example/p.gif\" width=\"1\" height=\"1\"></p>\r\n\
--R\r\nContent-Type: image/png\r\nContent-ID: <logo@studio.example>\r\nContent-Disposition: inline; filename=\"logo.png\"\r\n\r\nPNGDATA\r\n--R--\r\n";
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = quatrelle_model::MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9300 };
    inbox.messages.insert(0, quatrelle_mime::summary_from_full(raw, key.clone(), quatrelle_model::Flags::default(), 0, NOW - 3600));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, raw).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    assert_eq!(s.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.date), Some(NOW - 3600));
    for cmd in ["mail.reply", "mail.forward"] {
        s.run(cmd, &json!({})).unwrap();
        let c = s.compose.clone().unwrap();
        assert!(c.attachments.is_empty(), "{cmd}: the picture isn't attached as a file");
        assert_eq!(
            c.inline.iter().map(|p| (p.cid.as_str(), p.data.as_slice())).collect::<Vec<_>>(),
            [("logo@studio.example", &b"PNGDATA"[..])],
            "{cmd}"
        );
        let (text, html, pictures) = c.bodies();
        let html = html.unwrap_or_default();
        assert!(html.contains("<img src=\"cid:logo@studio.example\" width=\"40\""), "{cmd}: {html}");
        // One on the web by its address, never loaded (issue #10); a tracking one left out.
        assert!(html.contains("<img src=\"https://cdn.example/poster.jpg\""), "{cmd}: {html}");
        assert!(!html.contains("tracker.example"), "{cmd}: {html}");
        assert_eq!(pictures.len(), 1);
        assert!(!text.contains("1970"), "{cmd}: {text}");
        s.run("compose.discard", &json!({})).unwrap();
    }
    done(s);
}

/// A picture sent inline beside plain text (as Apple Mail sends one) goes with a forward as a
/// file: the quote can't show it, and it was left out (issue #10).
#[test]
fn a_forward_keeps_an_inline_picture_beside_plain_text() {
    let mut s = sample_session("inline-beside-text");
    let raw = b"From: Sam Okafor <sam@studio.example>\r\nTo: riley@studio.example\r\nSubject: Photo\r\nDate: Thu, 29 Oct 2026 10:00:00 +0000\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"M\"\r\n\r\n--M\r\nContent-Type: text/plain\r\n\r\nThe set, from above.\r\n\
--M\r\nContent-Type: image/jpeg; name=\"set.jpg\"\r\nContent-ID: <set@studio.example>\r\nContent-Disposition: inline; filename=\"set.jpg\"\r\n\r\nJPEGDATA\r\n--M--\r\n";
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = quatrelle_model::MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9301 };
    inbox.messages.insert(0, quatrelle_mime::summary_from_full(raw, key.clone(), quatrelle_model::Flags::default(), 0, NOW));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, raw).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    s.run("mail.forward", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.attachments.iter().map(|a| (a.name.as_str(), a.data.as_slice())).collect::<Vec<_>>(), [("set.jpg", &b"JPEGDATA"[..])]);
    s.run("compose.discard", &json!({})).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    assert!(s.compose.as_ref().unwrap().attachments.is_empty(), "a reply leaves files out, as Outlook's");
    done(s);
}
