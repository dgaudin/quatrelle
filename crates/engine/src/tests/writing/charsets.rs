// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a text file sent again (a forward, a draft reopened) keeps the charset it names.

use super::*;

/// Put this message in the sample Inbox and open it.
fn open_raw(s: &mut Session, uid: u32, raw: &[u8]) {
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = quatrelle_model::MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid };
    inbox.messages.insert(0, quatrelle_mime::summary_from_full(raw, key.clone(), quatrelle_model::Flags::default(), 0, NOW));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, raw).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
}

const MENU: &[u8] = b"From: Sam Okafor <sam@studio.example>\r\nTo: riley@studio.example\r\nSubject: Menu\r\nDate: Thu, 29 Oct 2026 10:00:00 +0000\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"M\"\r\n\r\n--M\r\nContent-Type: text/plain\r\n\r\nThe menu.\r\n\
--M\r\nContent-Type: text/plain; charset=windows-1252\r\nContent-Disposition: attachment; filename=\"menu.txt\"\r\n\r\nCaf\xe9 au lait\r\n--M--\r\n";

/// Sent as the message being written holds it, its first attachment read back as the reading
/// pane reads it (in the charset its part names).
fn sent_text(c: &crate::Compose) -> Option<String> {
    let d = quatrelle_mime::Draft { subject: c.subject.clone(), body: c.body.clone(), attachments: c.attachments.clone(), ..Default::default() };
    let sent = quatrelle_mime::build(&d, &quatrelle_model::EmailAddress::new("Riley", "riley@studio.example"), NOW, "<fwd@x>").unwrap();
    quatrelle_mime::attachment_text(&sent, 0)
}

/// A forwarded text file in Windows-1252 goes as it came and keeps its charset: it went out as
/// bare `text/plain`, so the copy read as UTF-8, with replacement characters.
#[test]
fn a_forward_keeps_a_text_files_charset() {
    let mut s = sample_session("forward-charset");
    open_raw(&mut s, 9302, MENU);
    s.run("mail.forward", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    let a = &c.attachments[0];
    assert_eq!((c.attachments.len(), a.name.as_str(), a.mime.as_str()), (1, "menu.txt", "text/plain; charset=windows-1252"));
    assert!(a.data.starts_with(b"Caf\xe9 au lait"));
    assert!(sent_text(&c).is_some_and(|t| t.starts_with("Caf\u{e9} au lait")), "{:?}", sent_text(&c));
    done(s);
}

/// A draft reopened from Drafts keeps a text file's charset too (it came back as bare
/// `text/plain`, and went so when sent).
#[test]
fn a_reopened_draft_keeps_a_text_files_charset() {
    let mut s = sample_session("draft-charset");
    open_raw(&mut s, 9303, MENU);
    let key = s.reading.as_ref().unwrap().key.clone();
    s.edit_draft(&key).unwrap();
    let c = s.compose.clone().unwrap();
    let a = &c.attachments[0];
    assert_eq!((a.name.as_str(), a.mime.as_str()), ("menu.txt", "text/plain; charset=windows-1252"));
    assert!(sent_text(&c).is_some_and(|t| t.starts_with("Caf\u{e9} au lait")), "{:?}", sent_text(&c));
    done(s);
}
