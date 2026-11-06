// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a reply in the reading pane set aside for another message, and the copies kept
//! in Drafts.

use super::*;

/// Whether the list marks the message `subject` as having a reply waiting (`[Draft]`).
fn marked(s: &Session, subject: &str) -> bool {
    s.list.iter().find(|m| m.subject == subject).is_some_and(|m| s.has_draft_reply(m))
}

fn drafts(s: &mut Session) -> Vec<String> {
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    let subjects = s.list.iter().map(|m| m.subject.clone()).collect();
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    subjects
}

/// As Outlook's reply in the reading pane: reading another message sets it aside — kept in
/// Drafts — and that message shows; back on the first, the reply comes back as it was left.
/// Discard takes its copy out of Drafts too.
#[test]
fn a_reply_in_the_reading_pane_is_set_aside_for_another_message() {
    let mut s = sample_session("set-aside");
    let poster = key_of(&s, "Your proof for the festival poster is ready");
    let mix = key_of(&s, "Sound mix, version 3");
    s.run("list.open", &poster).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    s.run("compose.update", &json!({"body": "Approved!\n"})).unwrap();
    // The message replied to, read again: the reply stays.
    s.run("list.open", &poster).unwrap();
    assert!(s.compose.is_some());
    s.run("list.open", &mix).unwrap();
    assert!(s.compose.is_none(), "the pane shows the message chosen");
    assert_eq!(s.reading.as_ref().map(|r| r.key.to_id()).as_deref(), mix["key"].as_str());
    assert_eq!(s.set_aside.len(), 1);
    assert!(s.notices.last().is_some_and(|n| n.text == "Draft saved"));
    assert!(marked(&s, "Your proof for the festival poster is ready"), "its message is marked [Draft]");
    assert!(!marked(&s, "Sound mix, version 3"));
    assert!(drafts(&mut s).contains(&"Re: Your proof for the festival poster is ready".to_string()), "kept in Drafts");
    s.run("list.open", &poster).unwrap();
    let c = s.compose.clone().expect("the reply comes back");
    assert!(c.body.starts_with("Approved!") && !c.window, "{:?}", c.body);
    assert!(s.set_aside.is_empty());
    // Another folder sets it aside too.
    s.run("folder.open", &json!({"account": "sample", "folder": "Archive"})).unwrap();
    assert!(s.compose.is_none() && s.set_aside.len() == 1);
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &poster).unwrap();
    assert!(s.compose.is_some());
    s.run("compose.discard", &json!({})).unwrap();
    assert!(s.compose.is_none());
    assert!(!drafts(&mut s).iter().any(|d| d.starts_with("Re: Your proof")), "discarded with its copy in Drafts");
    assert!(!marked(&s, "Your proof for the festival poster is ready"), "and no longer marked");
    // A reply left as it came simply goes.
    s.run("list.open", &poster).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    s.run("list.open", &mix).unwrap();
    assert!(s.compose.is_none() && s.set_aside.is_empty());
    assert!(drafts(&mut s).is_empty(), "nothing kept");
    done(s);
}

/// A draft from Drafts is carried on (Quatrelle closed meanwhile, or the reply's message gone):
/// sent, its copy goes.
#[test]
fn a_draft_from_drafts_is_carried_on_and_goes_once_sent() {
    let mut s = sample_session("edit-draft");
    let poster = key_of(&s, "Your proof for the festival poster is ready");
    s.run("list.open", &poster).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    s.run("compose.update", &json!({"body": "Approved, *thanks*!\n"})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    s.compose = None;
    // Kept in Drafts (Quatrelle closed meanwhile): the message is marked all the same.
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    assert!(marked(&s, "Your proof for the festival poster is ready"));
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    let draft = key_of(&s, "Re: Your proof for the festival poster is ready");
    s.run("draft.edit", &draft).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.to, "Print Shop <orders@printshop.example>; ");
    assert!(c.body.starts_with("Approved,") && !c.saved_id.is_empty());
    assert_eq!(c.source.as_ref().map(|k| k.to_id()).as_deref(), poster["key"].as_str(), "the message it answers, found by its Message-ID");
    s.run("compose.send", &json!({})).unwrap();
    assert!(s.compose.is_none());
    assert!(drafts(&mut s).is_empty(), "sent: the draft goes");
    let original = s.list.iter().find(|m| m.subject == "Your proof for the festival poster is ready").unwrap();
    assert!(original.flags.answered && !s.has_draft_reply(original), "marked Replied, and no longer [Draft]");
    done(s);
}

/// A server account's copy is numbered by the server once it's listed: found in Drafts by its
/// `Message-ID`, it's replaced at the next save and goes when the message is discarded (it used
/// to stay there for good).
#[test]
fn a_server_accounts_draft_is_found_by_its_message_id() {
    let store = temp("server-draft");
    let mut s = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let f = |p: &str, role| quatrelle_model::Folder { account: id.clone(), path: p.into(), role, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![f("INBOX", FolderRole::Inbox), f("Drafts", FolderRole::Drafts)] });
    s.take_jobs();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@soundlab.example", "subject": "Plans"})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::SaveDraft { replaces: None, .. }]));
    let mid = s.compose.as_ref().unwrap().saved_id.clone();
    assert!(!mid.is_empty());
    // The server lists the copy.
    let listed = |s: &mut Session, uid: u32| {
        let key = MessageKey { account: id.clone(), folder: "Drafts".into(), validity: 1, uid };
        let m = Summary { key: key.clone(), message_id: mid.clone(), subject: "Plans".into(), ..Default::default() };
        let st = quatrelle_store::FolderState { validity: 1, uid_next: uid + 1, messages: vec![m], ..Default::default() };
        s.store.as_ref().unwrap().save_folder(&id, "Drafts", &st).unwrap();
        s.apply(Event::FolderChanged { account: id.clone(), folder: "Drafts".into() });
        key
    };
    let first = listed(&mut s, 7);
    s.run("compose.saveDraft", &json!({})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::SaveDraft { replaces: Some(k), .. }] if *k == first), "saved again in its place");
    assert_eq!(s.compose.as_ref().unwrap().saved_id, mid, "under the same Message-ID");
    let second = listed(&mut s, 8);
    s.run("compose.discard", &json!({})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::Apply { op: Op::Expunge { keys }, .. }] if *keys == vec![second]), "discarded: its copy goes");
    let _ = std::fs::remove_dir_all(s.store.as_ref().unwrap().root());
}

/// A server account that lists no Drafts (GreenMail, a new account on many hosts) refused every
/// draft ("this account has no Drafts folder"), so a message being written was lost as Quatrelle
/// closed. Its draft goes to `Drafts`, which the connection makes there.
#[test]
fn a_draft_on_a_server_without_drafts_is_kept() {
    let store = temp("no-drafts");
    let mut s = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let inbox =
        quatrelle_model::Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![inbox] });
    s.take_jobs();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@soundlab.example", "subject": "Plans", "body": "To finish"})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::SaveDraft { folder, .. }] if folder == "Drafts"));
    // As Quatrelle closes: kept on this computer, for Drafts at the next connection.
    assert_eq!(s.keep_drafts(), 1);
    assert!(matches!(s.take_jobs().as_slice(), [Job::KeepDraft { folder, .. }] if folder == "Drafts"));
    let _ = std::fs::remove_dir_all(s.store.as_ref().unwrap().root());
}

/// A made-up message laid out in nested tables, as business mail writes a signature: an icon
/// column beside the numbers, a coloured warning, a logo cell; with a style sheet, a script, a
/// handler and a tracking picture to leave out.
pub(super) const MILL: &[u8] = b"From: Morgan Lee <morgan@mill.example>\r\nTo: riley@studio.example\r\nSubject: Invoice\r\n\
Message-ID: <invoice-1@mill.example>\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/related; boundary=\"R\"\r\n\r\n--R\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
<html><head><style>.warn{color:#ff0000}</style><script>alert(1)</script></head><body>\
<p>We will share the updated invoice soon.</p>\
<table width=\"640\"><tr><td width=\"40\"><img src=\"cid:phone@mill.example\" width=\"16\" alt=\"phone\"></td>\
<td width=\"420\"><table><tr><td>+00 1234 5678</td></tr><tr><td><b class=\"warn\" onclick=\"steal()\">A warning in red</b></td></tr></table></td>\
<td width=\"180\" align=\"right\">Mill</td></tr></table>\
<img src=\"https://tracker.example/open.gif\" width=\"1\" height=\"1\"></body></html>\r\n\
--R\r\nContent-Type: image/png\r\nContent-ID: <phone@mill.example>\r\nContent-Disposition: inline; filename=\"phone.png\"\r\n\r\nPNGDATA\r\n--R--\r\n";

/// [`MILL`] in the sample Inbox, opened in the reading pane; its key.
pub(super) fn open_mill(s: &mut Session) -> MessageKey {
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9400 };
    inbox.messages.insert(0, quatrelle_mime::summary_from_full(MILL, key.clone(), quatrelle_model::Flags::default(), 0, NOW - 3600));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, MILL).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    key
}

/// The draft in Drafts with `subject`.
fn draft_key(s: &mut Session, subject: &str) -> MessageKey {
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    s.list.iter().find(|m| m.subject == subject).map(|m| m.key.clone()).unwrap_or_else(|| panic!("{subject} in Drafts"))
}

/// As Outlook opens a draft from Drafts: as it was saved. A reply comes back with its
/// recipients (Bcc too), importance and read receipt, its text and formatting, the line over the
/// original, its picture in place (it came back as a file, beside a broken picture), and the
/// message it answers; a forward, with the message it forwards (it had none). Saved again it
/// replaces its copy; sent, it leaves Drafts and marks the original replied to, and what's sent
/// carries neither the Bcc nor what it answers.
#[test]
fn a_draft_opens_again_as_it_was_saved() {
    let mut s = sample_session("draft-reopen");
    let mill = open_mill(&mut s);
    s.run("mail.reply", &json!({})).unwrap();
    s.run("compose.update", &json!({"bcc": "boss@studio.example", "importance": "high", "readReceipt": true})).unwrap();
    {
        let c = s.compose.as_mut().unwrap();
        c.styles.fit(&c.body);
        let typed = quatrelle_mime::rich::Styles::plain("All paid.");
        quatrelle_mime::rich::splice(&mut c.body, &mut c.styles, 0..0, "All paid.", &typed);
        c.styles.restyle(&c.body, 4..8, |st| quatrelle_mime::rich::Style { bold: true, ..st.clone() });
    }
    s.run("compose.saveDraft", &json!({})).unwrap();
    s.compose = None;
    let key = draft_key(&mut s, "Re: Invoice");
    s.run("draft.edit", &json!({"key": key.to_id()})).unwrap();
    let c = s.compose.clone().expect("being written again");
    assert_eq!(c.draft.as_ref(), Some(&key), "its copy, to replace");
    assert!(c.to.contains("morgan@mill.example") && c.bcc.contains("boss@studio.example") && c.show_cc, "{} / {}", c.to, c.bcc);
    assert_eq!((c.subject.as_str(), c.importance, c.read_receipt), ("Re: Invoice", 1, true));
    assert!(c.body.starts_with("All paid.") && c.styles.style(5).bold && !c.styles.style(0).bold, "{}", c.body);
    let rule = c.body.split('\n').position(|l| l.starts_with("From: ")).and_then(|l| l.checked_sub(1)).map(|l| c.styles.para(l).rule);
    assert_eq!(rule, Some(true), "the line over the original: {}", c.body);
    assert!(
        c.inline.iter().any(|p| p.cid == "phone@mill.example"),
        "its picture in place: {:?}",
        c.inline.iter().map(|p| &p.cid).collect::<Vec<_>>()
    );
    assert!(c.attachments.is_empty(), "not a file: {:?}", c.attachments.iter().map(|a| &a.name).collect::<Vec<_>>());
    assert!(c.bodies().1.unwrap_or_default().contains("cid:phone@mill.example") && c.bodies().2.len() == 1);
    assert_eq!((c.source.as_ref(), c.mode), (Some(&mill), Some(quatrelle_mime::ReplyMode::Reply)), "the message it answers");
    // Opened again while it's being written: the same one.
    s.run("compose.popOut", &json!({})).unwrap_or_default();
    assert_eq!(s.run("draft.edit", &json!({"key": key.to_id()})).unwrap()["compose"], json!(c.id));
    // Saved again: one copy in Drafts.
    s.run("compose.saveDraft", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    assert_eq!(s.list.iter().filter(|m| m.subject == "Re: Invoice").count(), 1, "replaced, not added");
    s.run("compose.send", &json!({"checked": true})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    assert!(!s.list.iter().any(|m| m.subject == "Re: Invoice"), "gone from Drafts once sent");
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    assert!(s.list.iter().find(|m| m.key == mill).is_some_and(|m| m.flags.answered), "the original replied to");
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let sent = s.list.iter().find(|m| m.subject == "Re: Invoice").map(|m| m.key.clone()).expect("sent");
    let raw = String::from_utf8_lossy(&s.store.as_ref().unwrap().read_body(&sent).unwrap()).to_string();
    assert!(!raw.contains("boss@studio.example") && !raw.contains(quatrelle_mime::SOURCE_HEADER), "{raw}");
    // A forward's draft comes back a forward of the same message.
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": mill.to_id()})).unwrap();
    s.run("mail.forward", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@x.example"})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    s.compose = None;
    let key = draft_key(&mut s, "Fwd: Invoice");
    s.run("draft.edit", &json!({"key": key.to_id()})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert_eq!((c.source.as_ref(), c.mode), (Some(&mill), Some(quatrelle_mime::ReplyMode::Forward)));
    assert!(s.run("draft.edit", &json!({"key": "nonsense"})).is_err());
    done(s);
}

/// A signature written in HTML comes back from Drafts as it went in: where it stood, the same
/// HTML, its picture with it once (it came back as text, the HTML lost).
#[test]
fn a_draft_keeps_its_html_signature() {
    let mut s = sample_session("draft-html-signature");
    let png = crate::sample::sample_png();
    let code = "<table><tr><td><img src=\"cid:photo@sig\" width=\"80\"></td><td><b>Riley Moreno</b><br>Producer</td></tr></table>";
    s.run("account.setSignature", &json!({"account": "sample", "signature": "Riley Moreno\nProducer", "rawHtml": code})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    {
        let c = s.compose.as_mut().unwrap();
        c.inline.push(quatrelle_mime::InlinePicture { cid: "photo@sig".into(), name: "photo.png".into(), mime: "image/png".into(), data: png });
    }
    let written = s.compose.clone().unwrap();
    assert!(written.body.contains(quatrelle_mime::rich::SIGNATURE), "the HTML signature stands in the text: {:?}", written.body);
    let body = format!("Hello Ann,{}", written.body);
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Card draft", "body": body})).unwrap();
    let before = s.compose.as_ref().unwrap().bodies().1.unwrap_or_default();
    s.run("compose.saveDraft", &json!({})).unwrap();
    s.compose = None;
    let key = draft_key(&mut s, "Card draft");
    s.run("draft.edit", &json!({"key": key.to_id()})).unwrap();
    let c = s.compose.clone().unwrap();
    assert!(c.body.starts_with("Hello Ann,") && c.body.contains(quatrelle_mime::rich::SIGNATURE), "{:?}", c.body);
    assert_eq!(c.signature_html.matches("<table").count(), 1, "the same HTML: {}", c.signature_html);
    let (_, html, pictures) = c.bodies();
    let html = html.unwrap_or_default();
    assert_eq!(html.matches("<table").count(), before.matches("<table").count(), "{html}");
    assert_eq!(html.matches("cid:photo@sig").count(), 1, "its photo once: {html}");
    assert_eq!(pictures.iter().filter(|p| p.cid == "photo@sig").count(), 1);
    done(s);
}
