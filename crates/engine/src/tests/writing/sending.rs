// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: mail is sent only from an account that can send it. With none set up, or from
//! an Outlook data file, Send is off and says why; nothing goes, nothing says it went, and the
//! message stays open as it was.

use crate::SendBlock;

use super::*;

/// A session with an Outlook data file open, its Inbox and Sent Items in, and no other account.
fn data_file_session(name: &str) -> (Session, String) {
    let mut s = Session::open(Some(temp(name)), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.importPst", &json!({"path": r"C:\Mail\Archive.pst"})).unwrap()["account"].as_str().unwrap().to_string();
    let folder = |path: &str, role| quatrelle_model::Folder {
        account: id.clone(),
        path: path.into(),
        delimiter: Some('/'),
        role,
        selectable: true,
        ..Default::default()
    };
    s.apply(Event::Folders { account: id.clone(), folders: vec![folder("INBOX", FolderRole::Inbox), folder("Sent Items", FolderRole::Sent)] });
    s.apply(Event::Imported {
        account: id.clone(),
        messages: 0,
        skipped: 0,
        total: None,
        folder: String::new(),
        stopped: false,
        done: true,
        error: None,
    });
    s.take_jobs();
    (s, id)
}

/// Put `raw` in the account's Inbox and open it.
fn put_in_inbox(s: &mut Session, account: &str, uid: u32, raw: &[u8]) {
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder(account, "INBOX").unwrap_or_default();
    let key = quatrelle_model::MessageKey { account: account.into(), folder: "INBOX".into(), validity: inbox.validity, uid };
    inbox.messages.insert(0, quatrelle_mime::summary_from_full(raw, key.clone(), quatrelle_model::Flags::default(), 0, NOW));
    store.save_folder(account, "INBOX", &inbox).unwrap();
    store.write_body(&key, raw).unwrap();
    s.apply(Event::FolderChanged { account: account.into(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": account, "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
}

/// Why `compose.send` is off (`None`: on).
fn send_off(s: &Session) -> Option<&'static str> {
    s.registry().get("compose.send").and_then(|c| (c.enabled)(s))
}

/// Nothing was sent, queued or said to be: no send job, nothing in the Outbox or Sent Items,
/// and no notice saying it went.
fn nothing_sent(s: &mut Session, account: &str) {
    let jobs = s.take_jobs();
    assert!(!jobs.iter().any(|j| matches!(j, Job::Send { .. } | Job::FlushOutbox { .. })), "{jobs:?}");
    s.refresh_outbox();
    assert!(s.outbox.is_empty(), "{:?}", s.outbox);
    let sent = s.store.as_ref().unwrap().load_folder(account, "Sent Items").map(|f| f.messages.len()).unwrap_or(0);
    assert_eq!(sent, 0, "nothing filed in Sent Items");
    assert!(!s.notices.iter().any(|n| n.text.contains("sent") && !n.text.contains("not sent")), "{:?}", s.notices);
}

/// The report: with only a data file open, a new message said "Message sent" and went into the
/// file's Sent Items. Send is off, every way of sending says why, and the message stays open as
/// it was.
#[test]
fn mail_isnt_sent_from_a_data_file() {
    let (mut s, id) = data_file_session("send-data-file");
    assert_eq!(s.send_block(&id), Some(SendBlock::DataFile));
    s.run("mail.new", &json!({})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().account, id, "no other account to write from");
    s.run("compose.update", &json!({"to": "ann@example.org", "subject": "Plans", "body": "See you"})).unwrap();
    assert_eq!(send_off(&s), Some(SendBlock::DataFile.reason()));
    let written = s.compose.clone().unwrap();
    // The Send button, Ctrl+Enter, Alt+S and Send anyway all run compose.send; the CLI calls it
    // with an empty subject allowed.
    for params in [json!({}), json!({"allowEmptySubject": true}), json!({"allowEmptySubject": true, "checked": true})] {
        assert!(matches!(s.run("compose.send", &params), Err(CmdError::Disabled(m)) if m.contains("data file")));
    }
    // Called directly, past the command's switch.
    assert!(matches!(s.send_compose(true, true), Err(CmdError::Disabled(_))));
    assert_eq!(s.compose.as_ref(), Some(&written), "open and unchanged");
    nothing_sent(&mut s, &id);
    // Sent later (Delay Delivery): not put in the Outbox either.
    s.run("compose.update", &json!({"sendAfter": NOW + 3600})).unwrap();
    assert!(s.run("compose.send", &json!({})).is_err());
    assert!(!s.compose.as_ref().unwrap().sending);
    nothing_sent(&mut s, &id);
    s.tick();
    nothing_sent(&mut s, &id);
    done(s);
}

/// A reply or forward from a data file: written from the account that can send when there is
/// one (as Outlook does), else refused like a new message.
#[test]
fn replies_from_a_data_file_go_from_an_account_that_can_send() {
    let (mut s, file) = data_file_session("send-reply-data-file");
    let raw = b"From: Ana <ana@example.org>\r\nTo: me@example.org\r\nSubject: Hello\r\nMessage-ID: <h@x>\r\n\r\nHi there\r\n";
    put_in_inbox(&mut s, &file, 1, raw);
    for mode in ["mail.reply", "mail.forward"] {
        s.run(mode, &json!({})).unwrap();
        assert_eq!(s.compose.as_ref().unwrap().account, file);
        s.run("compose.update", &json!({"to": "ann@example.org"})).unwrap();
        assert!(matches!(s.run("compose.send", &json!({})), Err(CmdError::Disabled(_))));
        nothing_sent(&mut s, &file);
        s.run("compose.close", &json!({"save": false})).unwrap();
    }
    // With an account that sends: the reply is written from it, and goes.
    let zoe = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    s.run("folder.open", &json!({"account": file, "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": s.list[0].key.to_id()})).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().account, zoe);
    assert_eq!(send_off(&s), None);
    s.run("compose.send", &json!({})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Send { account, .. } if *account == zoe)));
    // A new message in the data file's folder too.
    s.run("mail.new", &json!({})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().account, zoe);
    // Chosen in From, the data file can't send it.
    s.run("compose.update", &json!({"account": file, "to": "ann@example.org", "subject": "x"})).unwrap();
    assert!(matches!(s.run("compose.send", &json!({})), Err(CmdError::Disabled(_))));
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Send { .. })));
    done(s);
}

/// No account at all (the one written from was removed): Send is off, and nothing goes.
#[test]
fn mail_isnt_sent_with_no_account() {
    let mut s = Session::open(Some(temp("send-no-account")), quatrelle_model::dates::Clock::new(NOW, 0));
    assert!(s.run("mail.new", &json!({})).is_err(), "nothing to write from");
    assert_eq!(s.send_block("a1"), Some(SendBlock::NoAccount));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@example.org", "subject": "Plans", "body": "See you"})).unwrap();
    // The account goes; the message being written is put back as it was, there being nowhere to
    // keep it.
    s.accounts.retain(|a| a.config.id != id);
    s.take_jobs();
    let written = s.compose.clone().unwrap();
    assert_eq!(send_off(&s), Some(SendBlock::NoAccount.reason()));
    assert!(matches!(s.run("compose.send", &json!({"allowEmptySubject": true})), Err(CmdError::Disabled(m)) if m == "add an account to send mail"));
    assert!(matches!(s.send_compose(true, true), Err(CmdError::Disabled(_))));
    assert_eq!(s.compose.as_ref(), Some(&written));
    nothing_sent(&mut s, &id);
    done(s);
}

/// Held in the Outbox for an account that can't send (written by an older Quatrelle): Send now
/// says why, and nothing asks for it to go.
#[test]
fn the_outbox_doesnt_send_from_a_data_file() {
    let (mut s, id) = data_file_session("send-outbox-data-file");
    let raw = b"From: x@example.org\r\nTo: ann@example.org\r\nSubject: Held\r\n\r\nhi\r\n";
    let item =
        quatrelle_store::OutboxItem { id: 9, recipients: vec!["ann@example.org".into()], queued: NOW, not_before: NOW + 60, ..Default::default() };
    s.store.as_ref().unwrap().outbox_add(&id, &item, raw).unwrap();
    s.run("folder.outbox", &json!({})).unwrap();
    let k = s.list.first().map(|m| m.key.to_id()).unwrap();
    assert!(matches!(s.run("outbox.send", &json!({"key": k})), Err(CmdError::Disabled(_))));
    assert!(matches!(s.run("outbox.send", &json!({})), Err(CmdError::Disabled(_))));
    s.set_clock(NOW + 120, 0);
    s.tick();
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::FlushOutbox { .. })));
    done(s);
}

/// An answer to a meeting invitation in a data file: the calendar takes it, the answer isn't
/// sent (nor filed in Sent Items as if it had been), and that's said.
#[test]
fn invitation_answers_arent_sent_from_a_data_file() {
    let (mut s, id) = data_file_session("send-invite-data-file");
    let event = quatrelle_model::ical::CalEvent {
        uid: "m1@example.org".into(),
        summary: "Review".into(),
        start: NOW + 86_400,
        end: NOW + 90_000,
        organizer: Some(quatrelle_model::ical::Attendee { email: "ana@example.org".into(), name: "Ana".into(), ..Default::default() }),
        ..Default::default()
    };
    let ics = quatrelle_model::ical::write(&[event], Some("REQUEST"), NOW);
    let d = quatrelle_mime::Draft {
        to: vec![quatrelle_model::EmailAddress::new("Me", "me@example.org")],
        subject: "Review".into(),
        body: "See the calendar.".into(),
        attachments: vec![quatrelle_mime::OutAttachment {
            name: "invite.ics".into(),
            mime: "text/calendar; method=REQUEST".into(),
            data: ics.into_bytes(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let raw = quatrelle_mime::build(&d, &quatrelle_model::EmailAddress::new("Ana", "ana@example.org"), NOW, "<i1@x>").unwrap();
    put_in_inbox(&mut s, &id, 1, &raw);
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert!(s.pim.get(&id).is_some_and(|p| p.events.iter().any(|e| e.event.uid == "m1@example.org")), "in the calendar");
    assert!(s.notices.iter().any(|n| n.error && n.text.contains("not sent")), "{:?}", s.notices);
    nothing_sent(&mut s, &id);
    done(s);
}

/// The sample mailbox, a demonstration, still files what it sends in Sent; another mailbox kept
/// only here can't send.
#[test]
fn the_sample_mailbox_can_send_and_a_local_one_cant() {
    let mut s = sample_session("send-sample");
    assert_eq!(s.send_block(sample::SAMPLE_ACCOUNT), None);
    // A mailbox kept only here, with an address: no server, so no sending.
    let local = s.run("account.add", &json!({"email": "me@example.org", "local": true})).unwrap()["account"].as_str().unwrap().to_string();
    assert_eq!(s.send_block(&local), Some(SendBlock::NoServer));
    done(s);
}

/// Forwarded as an attachment from a data file: it goes from the account that can send, with
/// that account's signature for replies and forwards (it had the data file's, none).
#[test]
fn forward_as_attachment_from_a_data_file_has_the_senders_signature() {
    let (mut s, file) = data_file_session("send-forward-attachment");
    let raw = b"From: Ana <ana@example.org>\r\nTo: me@example.org\r\nSubject: Hello\r\nMessage-ID: <h@x>\r\n\r\nHi there\r\n";
    put_in_inbox(&mut s, &file, 1, raw);
    let zoe = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.run("account.setSignature", &json!({"account": zoe, "signature": "Zoe x"})).unwrap();
    s.take_jobs();
    s.run("folder.open", &json!({"account": file, "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": s.list[0].key.to_id()})).unwrap();
    s.run("mail.forwardAsAttachment", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.account, zoe);
    assert!(c.body.contains("Zoe x"), "{:?}", c.body);
    assert_eq!(c.attachments.len(), 1);
    done(s);
}

/// The bar's Add an account: the messages being written from the data file (the open one and
/// one set aside) go From the account added, its signature put in, and can be sent.
#[test]
fn adding_an_account_lets_an_open_message_go() {
    let (mut s, _file) = data_file_session("send-add-account");
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@example.org", "subject": "First", "body": "One"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@example.org", "subject": "Plans", "body": "See you"})).unwrap();
    assert_eq!(s.composes.len(), 1, "the first set aside");
    assert_eq!(send_off(&s), Some(SendBlock::DataFile.reason()));
    // Its signature is in the account's first config, as one saved with the account.
    let mut config = quatrelle_model::AccountConfig::guess("zoe@example.org");
    config.signature = "Zoe x".into();
    let zoe = s.add_account(config, Some("pw".into())).unwrap();
    s.take_jobs();
    for c in s.all_composes() {
        assert_eq!(c.account, zoe, "{}", c.subject);
        assert!(c.body.contains("Zoe x"), "{:?}", c.body);
        assert!(c.body.starts_with(if c.subject == "Plans" { "See you" } else { "One" }), "{:?}", c.body);
    }
    assert_eq!(s.compose.as_ref().unwrap().subject, "Plans", "the same one still open");
    assert_eq!(send_off(&s), None);
    s.run("compose.send", &json!({})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Send { account, .. } if *account == zoe)));
    done(s);
}

/// Found closing Quatrelle: a message sent, and another started before it had gone. It was set
/// aside still sending, and news of it looked only at the one on screen, so it stayed "being
/// sent" for good. Delivered, kept for later or refused, the news finds it.
#[test]
fn a_message_set_aside_as_it_goes_hears_what_became_of_it() {
    let mut s = Session::open(Some(temp("set-aside-sending")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "riley@studio.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let inbox =
        quatrelle_model::Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![inbox] });
    let send = |s: &mut Session, subject: &str| -> u64 {
        s.run("mail.new", &json!({"account": id})).unwrap();
        s.run("compose.update", &json!({"to": "ben@x.example", "subject": subject, "body": "hi"})).unwrap();
        s.run("compose.send", &json!({})).unwrap();
        s.compose.as_ref().map(|c| c.id).unwrap()
    };
    let delivered = send(&mut s, "Delivered");
    let later = send(&mut s, "Later");
    let refused = send(&mut s, "Refused");
    s.run("mail.new", &json!({"account": id})).unwrap();
    s.run("compose.update", &json!({"subject": "Still writing"})).unwrap();
    let writing = s.compose.as_ref().map(|c| c.id).unwrap();
    let sending = |s: &Session| s.all_composes().filter(|c| c.sending).count();
    assert_eq!(sending(&s), 3);

    s.apply(Event::Sent { account: id.clone(), id: delivered });
    s.apply(Event::Queued { account: id.clone(), id: later, error: "offline".into() });
    assert_eq!(sending(&s), 1, "gone from those being sent");
    assert!(s.all_composes().all(|c| c.id != delivered && c.id != later));

    s.apply(Event::SendFailed { account: id.clone(), id: refused, error: "no such person".into() });
    assert_eq!(sending(&s), 0);
    let back = s.all_composes().find(|c| c.id == refused).expect("kept, to be put right");
    assert_eq!(back.error.as_deref(), Some("no such person"));
    assert_eq!(s.compose.as_ref().map(|c| c.id), Some(writing), "the one on screen is left alone");
}

/// A message selected in the Outbox stays the one selected when the message above it is sent:
/// keys there count rows, so the selection slid to the next message, and Delete took that one.
#[test]
fn an_outbox_selection_follows_its_message() {
    let (mut s, id) = data_file_session("outbox-follows");
    let store = s.store.clone().unwrap();
    for (n, subject) in [(7u64, "First"), (8, "Second"), (9, "Third")] {
        let raw = format!("From: x@studio.example\r\nTo: ann@studio.example\r\nSubject: {subject}\r\n\r\nhi\r\n");
        let item = quatrelle_store::OutboxItem {
            id: n,
            recipients: vec!["ann@studio.example".into()],
            queued: NOW + n as i64,
            not_before: NOW + 600,
            ..Default::default()
        };
        store.outbox_add(&id, &item, raw.as_bytes()).unwrap();
    }
    s.run("folder.outbox", &json!({})).unwrap();
    let second = s.list.iter().find(|m| m.subject == "Second").map(|m| m.key.clone()).unwrap();
    s.open_message(&second);
    // The first one goes.
    store.outbox_remove(&id, 7).unwrap();
    s.apply(Event::Sent { account: id.clone(), id: 7 });
    let chosen: Vec<u64> = s.selection.iter().filter_map(|k| s.outbox_entry(k)).map(|(_, i)| i.id).collect();
    assert_eq!(chosen, [8]);
    assert_eq!(s.reading.as_ref().and_then(|r| s.outbox_entry(&r.key)).map(|(_, i)| i.id), Some(8));
    s.run("mail.delete", &json!({})).unwrap();
    assert_eq!(store.outbox(&id).iter().map(|i| i.id).collect::<Vec<_>>(), [9]);
    done(s);
}

/// A session with an account that sends through its server (the jobs are only looked at), its
/// Inbox and Drafts in.
fn server_session(name: &str) -> (Session, String) {
    let mut s = Session::open(Some(temp(name)), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "riley@studio.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let folder = |path: &str, role| quatrelle_model::Folder { account: id.clone(), path: path.into(), role, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![folder("INBOX", FolderRole::Inbox), folder("Drafts", FolderRole::Drafts)] });
    s.take_jobs();
    (s, id)
}

/// A message to Ben written and sent, its copy in Drafts at `uid` (kept there before it was
/// sent): its id, and that copy.
fn send_one(s: &mut Session, account: &str, subject: &str, uid: u32) -> (u64, MessageKey) {
    s.run("mail.new", &json!({"account": account})).unwrap();
    s.run("compose.update", &json!({"to": "ben@studio.example", "subject": subject, "body": "hi"})).unwrap();
    let draft = MessageKey { account: account.to_string(), folder: "Drafts".into(), validity: 1, uid };
    if let Some(c) = s.compose.as_mut() {
        c.draft = Some(draft.clone());
    }
    s.run("compose.send", &json!({})).unwrap();
    let id = s.compose.as_ref().filter(|c| c.sending).map(|c| c.id).unwrap_or_default();
    s.take_jobs();
    (id, draft)
}

/// Whether the jobs asked for since the last look take `key` out of its folder.
fn expunged(s: &mut Session, key: &MessageKey) -> bool {
    s.take_jobs().iter().any(|j| matches!(j, Job::Apply { op: Op::Expunge { keys, .. }, .. } if keys.contains(key)))
}

/// A message closed (or discarded) while it's being sent isn't lost when the server then
/// refuses it: it was in neither Drafts nor the Outbox, with only "Not sent" said. It opens
/// again in a window of its own with the reason, beside the message being written, which is
/// left alone; its copy in Drafts stays while it goes.
#[test]
fn a_message_closed_as_it_goes_comes_back_if_refused() {
    let (mut s, id) = server_session("closed-sending");
    for (close, params) in [("compose.close", json!({"save": false})), ("compose.close", json!({"save": true})), ("compose.discard", json!({}))] {
        let (sent, draft) = send_one(&mut s, &id, "Refused", 40);
        s.run(close, &params).unwrap();
        assert!(s.all_composes().next().is_none(), "{close} {params}: off the screen");
        assert!(!expunged(&mut s, &draft), "{close} {params}: its copy in Drafts is kept while it goes");
        assert_eq!(s.still_sending(), 1, "{close} {params}: still on its way");
        s.run("mail.new", &json!({"account": id})).unwrap();
        s.run("compose.update", &json!({"subject": "Still writing"})).unwrap();
        let writing = s.compose.as_ref().map(|c| c.id);
        s.apply(Event::SendFailed { account: id.clone(), id: sent, error: "no such person".into() });
        let back = s.all_composes().find(|c| c.id == sent).cloned();
        assert!(back.is_some(), "{close} {params}: open again");
        let back = back.unwrap_or_default();
        assert_eq!(
            (back.sending, back.window, back.error.as_deref(), back.subject.as_str(), back.draft.as_ref()),
            (false, true, Some("no such person"), "Refused", Some(&draft)),
            "{close} {params}"
        );
        assert_eq!(s.compose.as_ref().map(|c| c.id), writing, "{close} {params}: the one being written is left alone");
        assert_eq!(s.still_sending(), 0, "{close} {params}");
        assert!(!expunged(&mut s, &draft), "{close} {params}: its copy in Drafts is still there");
        s.compose = None;
        s.composes.clear();
    }
}

/// A message closed as it goes that is sent after all, or kept in the Outbox to go later: nothing
/// is kept of it, and its copy in Drafts goes then.
#[test]
fn a_message_closed_as_it_goes_and_delivered_leaves_nothing_behind() {
    let (mut s, id) = server_session("closed-sent");
    for queued in [false, true] {
        let (sent, draft) = send_one(&mut s, &id, "Fine", 41);
        s.run("compose.discard", &json!({})).unwrap();
        s.take_jobs();
        if queued {
            s.apply(Event::Queued { account: id.clone(), id: sent, error: "offline".into() });
        } else {
            s.apply(Event::Sent { account: id.clone(), id: sent });
        }
        assert!(s.sent_away.is_empty() && s.all_composes().next().is_none(), "queued {queued}: nothing kept");
        assert_eq!(s.still_sending(), 0, "queued {queued}");
        assert!(expunged(&mut s, &draft), "queued {queued}: its copy in Drafts goes once it's done");
    }
}

/// Messages closed as they go are kept 64 at most: past that the oldest goes, the newest stays.
/// Their account removed, they no longer count as on their way (no word of them will come).
#[test]
fn messages_closed_as_they_go_are_kept_64_at_most() {
    let (mut s, id) = server_session("closed-many");
    let mut last = 0;
    for n in 0..70 {
        (last, _) = send_one(&mut s, &id, &format!("Message {n}"), 50 + n);
        s.run("compose.close", &json!({"save": false})).unwrap();
    }
    assert_eq!(s.sent_away.len(), 64);
    assert!(s.sent_away.iter().any(|c| c.id == last), "the last one closed is kept");
    assert_eq!(s.still_sending(), 64);
    s.run("account.remove", &json!({"account": id})).unwrap();
    assert_eq!(s.still_sending(), 0, "its account gone");
}
