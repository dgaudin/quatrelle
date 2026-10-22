//! Engine tests: writing, saving, sending and the Outbox.

use super::*;

#[test]
fn compose_reply_and_send_locally() {
    let mut s = sample_session("compose");
    let k = key_of(&s, "Your proof for the festival poster is ready");
    s.run("list.open", &k).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.to, "Print Shop <orders@printshop.example>; ", "ready for another address");
    assert_eq!(c.subject, "Re: Your proof for the festival poster is ready");
    assert!(c.body.contains("\nFrom: Print Shop <orders@printshop.example>\nSent: "));
    assert!(c.body.contains("The proof for order 4471"));
    assert!(c.body.contains("Riley Moreno · Studio Lantern"));
    assert!(!c.window, "replies start in the reading pane");
    s.run("compose.popOut", &json!({})).unwrap();
    assert!(s.compose.as_ref().unwrap().window);
    s.run("compose.popIn", &json!({})).unwrap();
    assert!(!s.compose.as_ref().unwrap().window);
    s.run("compose.update", &json!({"body": "Approved, thank you!\n"})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    assert!(s.compose.is_none());
    assert!(s.list.iter().find(|m| m.subject.starts_with("Your proof")).unwrap().flags.answered);
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    assert!(s.list.iter().any(|m| m.subject == "Re: Your proof for the festival poster is ready"));

    // Validation.
    s.run("mail.new", &json!({})).unwrap();
    assert!(matches!(s.run("compose.send", &json!({})), Err(CmdError::Params(_))));
    s.run("compose.update", &json!({"to": "not an address"})).unwrap();
    assert!(matches!(s.run("compose.send", &json!({})), Err(CmdError::Params(m)) if m.contains("not an address")));
    s.run("compose.update", &json!({"to": "ana@soundlab.example"})).unwrap();
    assert!(matches!(s.run("compose.send", &json!({})), Err(CmdError::Params(m)) if m.contains("subject")));
    s.run("compose.attach", &json!({"name": "notes.txt", "data": [104, 105]})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    assert_eq!(s.list.len(), 1, "saving again replaces the earlier draft");
    s.run("compose.send", &json!({"allowEmptySubject": true})).unwrap();
    assert!(s.list.is_empty(), "the draft goes once sent");

    // Forward carries attachments.
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let k = key_of(&s, "Character turnarounds");
    s.run("list.open", &k).unwrap();
    s.run("mail.forward", &json!({})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().attachments.len(), 1);
    s.run("compose.discard", &json!({})).unwrap();
    assert!(s.compose.is_none());

    // Forward as attachment: the whole original as an .eml, sent as it is (not base64).
    s.run("mail.forwardAsAttachment", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.subject, "Fwd: Character turnarounds");
    assert!(!c.body.contains("Here are the first turnarounds"), "the original isn't quoted");
    let a = &c.attachments[0];
    assert_eq!((c.attachments.len(), a.name.as_str(), a.mime.as_str()), (1, "Character turnarounds.eml", "message/rfc822"));
    assert!(String::from_utf8_lossy(&a.data).contains("Subject: Character turnarounds"));
    s.run("compose.update", &json!({"to": "ana@soundlab.example"})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let k = s.list.iter().find(|m| m.subject == "Fwd: Character turnarounds").unwrap().key.clone();
    let sent = s.store.as_ref().unwrap().read_body(&k).unwrap();
    let sent = String::from_utf8_lossy(&sent);
    let part = sent.split("Content-Type: message/rfc822").nth(1).unwrap();
    assert!(part.contains("Subject: Character turnarounds") && !part[..300.min(part.len())].contains("base64"));
    done(s);
}

#[test]
fn messages_save_as_eml_and_pdf() {
    let mut s = sample_session("export");
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.open", &k).unwrap();
    let dir = s.store.as_ref().unwrap().root().join("export");
    std::fs::create_dir_all(&dir).unwrap();
    let eml = dir.join("m.eml");
    s.run("message.saveAs", &json!({"path": eml.to_string_lossy()})).unwrap();
    assert_eq!(quatrelle_mime::parse(&std::fs::read(&eml).unwrap()).unwrap().subject, "Storyboard review on Thursday");
    if let Some(font) =
        ["C:\\Windows\\Fonts\\segoeui.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"].iter().find_map(|p| std::fs::read(p).ok())
    {
        s.print_fonts = Some((font, None));
    }
    let pdf = dir.join("m.pdf");
    s.run("message.exportPdf", &json!({"path": pdf.to_string_lossy()})).unwrap();
    let bytes = std::fs::read(&pdf).unwrap();
    assert!(bytes.starts_with(b"%PDF-") && bytes.ends_with(b"%%EOF\n"));
    if let Ok(keep) = std::env::var("QUATRELLE_KEEP_PDF") {
        let _ = std::fs::copy(&pdf, keep);
    }
    done(s);
}

#[test]
fn attachments_preview_and_save_all() {
    let mut s = sample_session("attachments");
    let k = key_of(&s, "Your proof for the festival poster is ready");
    s.run("list.open", &k).unwrap();
    let r = s.run("attachment.preview", &json!({"index": 0})).unwrap();
    assert_eq!((r["kind"].as_str(), r["name"].as_str()), (Some("text"), Some("proof-notes.txt")));
    assert!(!r["text"].as_str().unwrap().is_empty());
    let k = key_of(&s, "Character turnarounds");
    s.run("list.open", &k).unwrap();
    assert!(s.run("attachment.preview", &json!({"index": 0})).is_err(), "a picture isn't text");
    let dir = s.store.as_ref().unwrap().root().join("out");
    std::fs::create_dir_all(&dir).unwrap();
    s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy()})).unwrap();
    let again = s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy()})).unwrap();
    assert!(again["saved"][0].as_str().unwrap().contains("(2)"), "names already taken get a number");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    // Only the ones asked for (those selected).
    let only = s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy(), "indices": [9999]})).unwrap();
    assert_eq!(only["saved"].as_array().map(Vec::len), Some(0));
    let index = s.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.attachments[0].index).unwrap();
    let only = s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy(), "indices": [index]})).unwrap();
    assert_eq!(only["saved"].as_array().map(Vec::len), Some(1));
    assert!(s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy(), "indices": "junk"})).is_ok(), "junk means all");
    assert!(s.run("attachment.saveAll", &json!({"dir": "/no/such/folder"})).is_err());
    assert_eq!(quatrelle_mime::craft_app_for("application/pdf", "a.pdf"), Some(("pdfcraft", "PdfCraft")));
    assert_eq!(quatrelle_mime::craft_app_for("application/octet-stream", "q.XLSX").map(|a| a.0), Some("gridcraft"));
    assert_eq!(quatrelle_mime::text_preview_kind("message/rfc822", "fwd.eml"), Some("message"));
    done(s);
}

#[test]
fn formatted_messages_go_out_as_html_with_pictures() {
    let mut s = sample_session("rich");
    s.run("mail.new", &json!({})).unwrap();
    // Agents and the command line can write with marks.
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Plan", "body": "Hi **Ann**,\n- frames\n- sound\n", "marks": true})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.body, "Hi Ann,\nframes\nsound\n");
    assert!(c.styles.style(3).bold && c.styles.para(1).list == quatrelle_mime::rich::List::Bullet);
    let png = crate::sample::sample_png();
    let r = s.run("compose.insertPicture", &json!({"name": "board.png", "data": png})).unwrap();
    assert!(r["cid"].as_str().is_some());
    assert!(s.compose.as_ref().unwrap().body.ends_with(quatrelle_mime::rich::PICTURE));
    assert!(s.run("compose.insertPicture", &json!({"name": "notes.txt", "data": [1]})).is_err(), "pictures only");
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let k = key_of(&s, "Plan");
    s.run("list.open", &k).unwrap();
    let p = s.reading.as_ref().unwrap().parsed.clone().unwrap();
    assert!(p.had_html);
    assert!(p.rich.iter().flatten().any(|sp| sp.bold && sp.text == "Ann"));
    assert!(p.text.starts_with("Hi Ann,\n• frames"), "{}", p.text);
    assert_eq!(p.images.len(), 1);
    assert!(p.attachments.iter().any(|a| a.inline && a.content_id.is_some()));

    // Plain text: the marks go as typed, no HTML.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Plain", "body": "Keep **these** stars"})).unwrap();
    s.run("compose.format", &json!({"plain": true})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let k = key_of(&s, "Plain");
    s.run("list.open", &k).unwrap();
    let p = s.reading.as_ref().unwrap().parsed.clone().unwrap();
    assert!(!p.had_html && p.text.contains("**these**"));
    done(s);
}

#[test]
fn offline_sends_wait_in_the_outbox() {
    let store = temp("outbox");
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@example.org", "bcc": "boss@example.org", "subject": "Plans", "body": "See you"})).unwrap();
    s.run("compose.attach", &json!({"name": "a.txt", "data": [104, 105]})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let Some(Job::Send { id: send_id, raw, from, recipients, .. }) = s.take_jobs().into_iter().find(|j| matches!(j, Job::Send { .. })) else {
        panic!()
    };
    // The network layer couldn't send: it put the message in the Outbox.
    let item = quatrelle_store::OutboxItem { id: send_id, from, recipients, queued: NOW, error: Some("offline".into()), ..Default::default() };
    store.outbox_add(&id, &item, &raw).unwrap();
    s.apply(Event::Queued { account: id.clone(), id: send_id, error: "offline".into() });
    assert!(s.compose.is_none(), "a queued message is done for the user");
    assert!(s.notices.last().unwrap().text.contains("Outbox"));
    assert_eq!(s.outbox.len(), 1);

    s.run("folder.outbox", &json!({})).unwrap();
    assert_eq!(s.view_title(), "Outbox");
    assert_eq!(s.list.len(), 1);
    assert_eq!(s.list[0].subject, "Plans");
    let key = s.list[0].key.clone();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    assert!(s.reading.as_ref().unwrap().parsed.as_ref().unwrap().text.contains("See you"));
    // Mail commands leave Outbox entries alone; Send now asks the network layer.
    s.run("mail.flag", &json!({})).unwrap();
    assert!(s.take_jobs().is_empty());
    s.run("outbox.send", &json!({})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::FlushOutbox { .. }]));

    // Edit takes it back out, Bcc and attachment included.
    s.run("outbox.edit", &json!({"key": key.to_id()})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!((c.to.as_str(), c.bcc.as_str(), c.subject.as_str()), ("ann@example.org; ", "boss@example.org; ", "Plans"));
    assert_eq!(c.attachments.len(), 1);
    assert!(s.outbox.is_empty() && store.outbox(&id).is_empty());

    // Deleting from the Outbox cancels the send.
    store.outbox_add(&id, &item, &raw).unwrap();
    s.apply(Event::Queued { account: id.clone(), id: send_id, error: "later".into() });
    assert_eq!(s.list.len(), 1);
    s.run("list.selectAll", &json!({})).unwrap();
    s.run("mail.delete", &json!({})).unwrap();
    assert!(s.list.is_empty() && store.outbox(&id).is_empty());
    // A later success from the Outbox is just a notice.
    s.apply(Event::Sent { account: id.clone(), id: 999 });
    assert_eq!(s.notices.last().unwrap().text, "Message sent");
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn new_mail_opens_in_a_window_and_closing_keeps_a_draft() {
    let mut s = sample_session("compose-window");
    s.run("mail.new", &json!({})).unwrap();
    assert!(s.compose.as_ref().unwrap().window, "new mail opens in its own window");
    // Nothing written (just the signature): closing leaves nothing behind.
    let v = s.run("compose.close", &json!({})).unwrap();
    assert_eq!(v["saved"], false);
    assert!(s.compose.is_none());

    s.run("view.newMailWindow", &json!({"on": false})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    assert!(!s.compose.as_ref().unwrap().window, "unless the option is off");
    s.run("compose.update", &json!({"to": "zoe@studio.example", "subject": "Plans"})).unwrap();
    let v = s.run("compose.close", &json!({})).unwrap();
    assert_eq!(v["saved"], true);
    assert!(s.compose.is_none());
    assert!(s.notices.last().is_some_and(|n| n.text == "Saved in Drafts"));
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    assert!(s.list.iter().any(|m| m.subject == "Plans"));
    // Closing without keeping it.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"subject": "Gone"})).unwrap();
    assert_eq!(s.run("compose.close", &json!({"save": false})).unwrap()["saved"], false);
    done(s);
}

/// View › Arrange by, as Outlook's: sorted and grouped by sender, size, flag…, and the other way
/// round.
#[test]
fn the_list_is_arranged_by_sender_size_flag_and_reversed() {
    let mut s = sample_session("arrange");
    let dates: Vec<i64> = s.list.iter().map(|m| m.date).collect();
    assert!(dates.windows(2).all(|w| w[0] >= w[1]), "newest first by default");
    s.run("view.arrange", &json!({"by": "from"})).unwrap();
    let names: Vec<String> =
        s.list.iter().map(|m| if m.from.name.is_empty() { m.from.email.to_lowercase() } else { m.from.name.to_lowercase() }).collect();
    assert!(names.windows(2).all(|w| w[0] <= w[1]), "senders A to Z: {names:?}");
    assert!(s.rows().iter().any(|r| matches!(r, Row::Group(g) if g == "Zoe Park")), "under each sender");
    s.run("view.reverseSort", &json!({})).unwrap();
    let names: Vec<String> =
        s.list.iter().map(|m| if m.from.name.is_empty() { m.from.email.to_lowercase() } else { m.from.name.to_lowercase() }).collect();
    assert!(names.windows(2).all(|w| w[0] >= w[1]), "Z to A reversed");
    s.run("view.arrange", &json!({"by": "size"})).unwrap();
    assert!(!s.prefs.reverse_sort, "a new arrangement starts in its own order");
    assert!(s.list.windows(2).all(|w| w[0].size >= w[1].size), "largest first");
    assert!(s.rows().iter().any(|r| matches!(r, Row::Header(h) if crate::ARRANGE_HEADINGS.contains(h))));
    s.run("view.arrange", &json!({"by": "flag"})).unwrap();
    let rows = s.rows();
    let at = |h: &'static str| rows.iter().position(|r| *r == Row::Header(h));
    assert!(matches!((at("Flagged"), at("Not flagged")), (Some(a), Some(b)) if a < b), "{rows:?}");
    assert!(s.list.first().is_some_and(|m| m.flags.flagged), "flagged first");
    assert!(s.run("view.arrange", &json!({"by": "colour"})).is_err());
    s.run("view.arrange", &json!({"by": "date"})).unwrap();
    assert_eq!(s.rows()[0], Row::Header("Today"));
}

/// Send / Receive › Work Offline: ahead of anything queued, kept in the settings, and nothing
/// connects meanwhile.
#[test]
fn working_offline_comes_first_and_stops_send_receive() {
    let store = temp("offline");
    let mut s = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap();
    s.run("mail.workOffline", &json!({})).unwrap();
    assert!(s.prefs.work_offline);
    assert!(s.run("mail.sync", &json!({})).is_err(), "nothing connects");
    assert!(matches!(s.take_jobs().first(), Some(Job::WorkOffline { on: true })), "before the account starts");
    s.run("mail.workOffline", &json!({"on": false})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::WorkOffline { on: false }]));
    assert!(s.run("mail.sync", &json!({})).is_ok());
    let _ = std::fs::remove_dir_all(s.store.as_ref().unwrap().root());
}

#[test]
fn delay_delivery_holds_mail_in_the_outbox_until_its_time() {
    let mut s = Session::open(Some(temp("delay")), quatrelle_model::dates::Clock::new(NOW, 0));
    s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap();
    s.take_jobs();
    s.run("mail.new", &json!({})).unwrap();
    s.run(
        "compose.update",
        &json!({"to": "ann@example.org", "subject": "Later", "body": "Hi", "replyTo": "Desk <desk@example.org>", "sendAfter": NOW + 3600}),
    )
    .unwrap();
    assert!(s.run("compose.update", &json!({"replyTo": "not an address"})).is_ok());
    assert!(s.run("compose.send", &json!({})).is_err(), "a bad Replies to address is caught");
    s.run("compose.update", &json!({"replyTo": "Desk <desk@example.org>"})).unwrap();
    assert_eq!(s.run("compose.send", &json!({})).unwrap()["sendAfter"], NOW + 3600);
    assert!(s.compose.is_none());
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Send { .. })), "held back, not sent");
    let (account, item) = s.outbox.first().cloned().unwrap();
    assert_eq!(item.not_before, NOW + 3600);
    let raw = s.store.as_ref().unwrap().outbox_raw(&account, item.id).unwrap();
    assert!(String::from_utf8_lossy(&raw).contains("Reply-To: \"Desk\" <desk@example.org>"));
    // Not yet: nothing asks for it. Its time: the account is asked, once.
    s.tick();
    assert!(s.take_jobs().is_empty());
    s.set_clock(NOW + 3601, 0);
    s.tick();
    s.tick();
    assert_eq!(s.take_jobs().iter().filter(|j| matches!(j, Job::FlushOutbox { .. })).count(), 1);
    // Send now sends a held-back message at once.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@example.org", "subject": "Tomorrow", "body": "x", "sendAfter": NOW + 86_400})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.outbox", &json!({})).unwrap();
    let k = s.list.iter().find(|m| m.subject == "Tomorrow").unwrap().key.clone();
    assert!(s.list.iter().find(|m| m.subject == "Tomorrow").unwrap().preview.starts_with("Will be sent"));
    s.run("outbox.send", &json!({"key": k.to_id()})).unwrap();
    assert!(s.outbox.iter().all(|(_, i)| i.not_before == 0 || i.not_before <= NOW + 3601));
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::FlushOutbox { .. })));
    done(s);
}

/// A new message to someone (a contact card's Send email): To filled in, ready for more.
#[test]
fn new_mail_to_someone() {
    let mut s = sample_session("new-mail-to");
    s.run("mail.new", &json!({"to": "Zoe Park <zoe@studio.example>"})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().to, "Zoe Park <zoe@studio.example>; ");
    s.compose = None;
    assert!(s.run("mail.new", &json!({"to": "not an address"})).is_err());
    assert!(s.compose.is_none(), "nothing opens for a bad address");
    s.run("mail.new", &json!({"to": "  "})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().to, "");
    done(s);
}

/// Several messages written at once, as Outlook keeps every New Email window: starting another
/// sets the one being written aside (in a window of its own), never drops it; an empty one goes.
#[test]
fn new_mail_keeps_the_message_being_written() {
    let mut s = sample_session("several-composes");
    let first = s.run("mail.new", &json!({})).unwrap()["compose"].as_u64().unwrap();
    // Nothing typed yet: a second New mail replaces it.
    let second = s.run("mail.new", &json!({})).unwrap()["compose"].as_u64().unwrap();
    assert!(s.composes.is_empty(), "an empty message isn't kept");
    assert_ne!(first, second);
    s.run("compose.update", &json!({"to": "ana@soundlab.example", "subject": "One"})).unwrap();
    let third = s.run("mail.new", &json!({})).unwrap()["compose"].as_u64().unwrap();
    s.run("compose.update", &json!({"subject": "Two", "body": "Second text"})).unwrap();
    // A reply too: both written ones stay.
    let k = key_of(&s, "Sound mix, version 3");
    s.run("list.open", &k).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    let list = s.run("compose.list", &json!({})).unwrap();
    let subjects: Vec<String> = list.as_array().unwrap().iter().map(|c| c["subject"].as_str().unwrap().to_string()).collect();
    assert_eq!(subjects.len(), 3, "{subjects:?}");
    assert!(subjects.contains(&"One".into()) && subjects.contains(&"Two".into()));
    assert!(s.composes.iter().all(|c| c.window), "set aside in windows of their own");
    // Back to the first: edits go to it; the reply is set aside in turn.
    s.run("compose.activate", &json!({"id": second})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().subject, "One");
    s.run("compose.update", &json!({"subject": "One, changed"})).unwrap();
    assert!(s.composes.iter().any(|c| c.id == third && c.subject == "Two"));
    // Closed (kept in Drafts): the others stay.
    s.run("compose.close", &json!({})).unwrap();
    assert_eq!(s.all_composes().count(), 2);
    assert!(s.run("compose.activate", &json!({"id": second})).is_err(), "closed");
    assert!(s.run("compose.activate", &json!({})).is_err());
    assert!(s.activate_compose(third));
    assert_eq!(s.compose.as_ref().unwrap().subject, "Two");
}

/// Closing with messages written: each one with something in it is kept as a draft (on this
/// computer at once; a server account's goes to Drafts at the next connection).
#[test]
fn closing_keeps_every_message_being_written() {
    let mut s = sample_session("keep-on-close");
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"subject": "Kept one"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"subject": "Kept two"})).unwrap();
    s.run("mail.new", &json!({})).unwrap(); // nothing written in this one
    assert_eq!(s.run("compose.keepAll", &json!({})).unwrap()["kept"], 2);
    let drafts = s.account("sample").and_then(|a| a.folder(FolderRole::Drafts)).map(|f| f.path.clone()).unwrap();
    let subjects: Vec<String> = s.folder_state("sample", &drafts).messages.iter().map(|m| m.subject.clone()).collect();
    assert!(subjects.contains(&"Kept one".into()) && subjects.contains(&"Kept two".into()), "{subjects:?}");
    // A server account's: handed to the network side, kept before Quatrelle goes.
    s.accounts[0].config.imap.host = "imap.studio.example".into();
    s.take_jobs();
    s.activate_compose(s.composes[0].id);
    s.keep_drafts();
    assert!(s.take_jobs().iter().any(|j| matches!(j, quatrelle_model::Job::KeepDraft { .. })));
}

#[test]
fn a_correction_replaces_the_word_and_keeps_its_look() {
    let mut s = sample_session("replace");
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Plan", "body": "Le **serveur** est lent", "marks": true})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.body, "Le serveur est lent");
    let at = c.body.find("serveur").unwrap();
    assert!(c.styles.style(3).bold, "the word is bold");
    // The suggestion takes the word's place, the look with it.
    s.run("compose.replaceText", &json!({"from": at, "to": at + "serveur".len(), "text": "serveur mail"})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.body, "Le serveur mail est lent");
    assert_eq!(c.styles.chars.len(), c.body.chars().count(), "one style per character");
    assert!(c.styles.style(3).bold && c.styles.style(14).bold, "the replacement stays bold");
    assert!(!c.styles.style(15).bold, "what follows it keeps its own look");
    // What isn't there is refused, and the text is left as it was.
    for bad in [json!({"from": 4, "to": 2, "text": "x"}), json!({"from": 0, "to": 9_999, "text": "x"}), json!({"from": -1, "to": 2, "text": "x"})] {
        assert!(s.run("compose.replaceText", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.compose.as_ref().unwrap().body, "Le serveur mail est lent");
    // In the middle of a character: refused, not a panic.
    s.run("compose.update", &json!({"body": "Le café est prêt"})).unwrap();
    let e = s.compose.as_ref().unwrap().body.find('é').unwrap();
    assert!(s.run("compose.replaceText", &json!({"from": e + 1, "to": e + 3, "text": "e"})).is_err());
    assert_eq!(s.compose.as_ref().unwrap().body, "Le café est prêt");
    // Taking a word out.
    let (from, to) = (3, 3 + "café".len());
    s.run("compose.replaceText", &json!({"from": from, "to": to, "text": ""})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.body, "Le  est prêt");
    assert_eq!(c.styles.chars.len(), c.body.chars().count());
    // The subject, with offsets into it.
    s.run("compose.update", &json!({"subject": "Le servveur est là"})).unwrap();
    s.run("compose.replaceText", &json!({"from": 3, "to": 11, "text": "serveur", "field": "subject"})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().subject, "Le serveur est là");
    assert!(s.run("compose.replaceText", &json!({"from": 0, "to": 99, "text": "x", "field": "subject"})).is_err());
    assert!(s.run("compose.replaceText", &json!({"from": 0, "to": 1, "text": "x", "field": "nowhere"})).is_err());
    // Underlining as you type, on or off (Outlook's "Check spelling as you type").
    assert!(s.prefs.spell_check);
    assert_eq!(s.run("view.spelling", &json!({"on": false})).unwrap()["on"], false);
    assert!(!s.prefs.spell_check);
    s.run("view.spelling", &json!({})).unwrap();
    assert!(s.prefs.spell_check);
    // The words added to the dictionary, once each, whatever the case.
    assert_eq!(s.run("spelling.add", &json!({"word": "  Quatrelle "})).unwrap()["word"], "quatrelle");
    s.run("spelling.add", &json!({"word": "QUATRELLE"})).unwrap();
    s.run("spelling.add", &json!({"word": "serveur"})).unwrap();
    assert_eq!(s.prefs.spelling, vec!["quatrelle".to_string(), "serveur".to_string()]);
    assert!(s.run("spelling.add", &json!({"word": "   "})).is_err());
    done(s);
}

/// The signature goes into new messages, replies and forwards, or only some, as the account
/// says (Outlook's New messages and Replies/forwards); the choice is kept with the account.
#[test]
fn a_signature_goes_where_the_account_says() {
    let mut s = sample_session("signature-where");
    let first = s.list.first().map(|m| m.key.clone()).unwrap();
    let body = |s: &Session| s.compose.as_ref().map(|c| c.body.clone()).unwrap_or_default();
    s.run("account.setSignature", &json!({"account": "sample", "signature": "Riley", "newMessages": false})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    assert!(!body(&s).contains("Riley"), "not in new messages");
    s.compose = None;
    s.open_message(&first);
    s.run("mail.reply", &json!({})).unwrap();
    assert!(body(&s).contains("Riley"), "in replies");
    s.compose = None;
    // Changing the text alone keeps the choices.
    s.run("account.setSignature", &json!({"account": "sample", "signature": "R.", "replies": false})).unwrap();
    s.run("mail.forward", &json!({})).unwrap();
    assert!(!body(&s).contains("R."), "not in forwards");
    s.compose = None;
    s.run("mail.new", &json!({})).unwrap();
    assert!(!body(&s).contains("R."), "still not in new messages");
    let config = s.account("sample").unwrap().config.clone();
    assert!(config.signature_not_new && config.signature_not_replies);
}

/// A signature's logo is kept with the account and goes into each message with it, as a
/// picture of the message (never fetched from the web); a picture the HTML no longer names
/// goes.
#[test]
fn a_signature_keeps_its_logo() {
    let mut s = sample_session("signature-logo");
    let logo = json!({"cid": "logo@mc", "name": "logo.gif", "mime": "image/gif", "data": [71, 73, 70, 56, 57, 97]});
    let html = "<p><img src=\"cid:logo@mc\"></p><p><b>Didier GAUDIN</b><br>Tél. 02 51 70 93 93</p>";
    s.run("account.setSignature", &json!({"account": "sample", "signature": "x", "html": html, "pictures": [logo]})).unwrap();
    let config = s.account("sample").unwrap().config.clone();
    assert_eq!(config.signature_pictures.len(), 1);
    assert!(config.signature_html.contains("cid:logo@mc"), "{}", config.signature_html);
    s.run("mail.new", &json!({})).unwrap();
    let (text, html, pictures) = s.compose.as_ref().unwrap().bodies();
    assert!(text.contains("Didier GAUDIN"));
    assert!(html.unwrap_or_default().contains("cid:logo@mc"));
    assert_eq!(pictures.len(), 1);
    // Changed without its picture: it goes.
    s.run("account.setSignature", &json!({"account": "sample", "signature": "Didier", "html": "<p><b>Didier</b></p>"})).unwrap();
    assert!(s.account("sample").unwrap().config.signature_pictures.is_empty());
    // A picture on the web isn't kept (nor fetched).
    s.run("account.setSignature", &json!({"account": "sample", "signature": "x", "html": "<img src=\"https://www.example.com/logo.gif\"><b>D</b>"}))
        .unwrap();
    assert!(!s.account("sample").unwrap().config.signature_html.contains("https://www.example.com"));
}

/// A signature written in HTML goes as it was written (its logo beside the name, cleaned of
/// what mustn't go), where its text stands in the message; the text part has its text.
#[test]
fn an_html_signature_goes_as_written() {
    let mut s = Session::open(Some(temp("signature-html")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "dgaudin@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let html = r#"<table align="left" style="float:left;"><tr><td><a href="https://www.contoso.example"><img src="https://www.contoso.example/images/logo.gif" width="194"></a></td></tr></table>
<table style="float:left;border-left:1px solid #ccc;"><tr><td><p><span style="font-weight:bold">Didier GAUDIN</span><br>T&eacute;l. 02 51 70 93 93</p></td></tr></table><script>x()</script>"#;
    s.run("account.setSignature", &json!({"account": id, "signature": "", "rawHtml": html})).unwrap();
    let config = s.account(&id).unwrap().config.clone();
    assert!(config.signature.contains("Didier GAUDIN"), "{}", config.signature);
    assert!(!config.signature_raw_html.contains("<script"));
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "zoe@example.org", "subject": "Hello", "body": "Hi Zoe"})).unwrap();
    // The body set anew: the signature again, as a new message gets it.
    s.run("compose.insertSignature", &json!({})).unwrap();
    s.take_jobs();
    s.run("compose.send", &json!({})).unwrap();
    let raw = s
        .take_jobs()
        .into_iter()
        .find_map(|j| match j {
            Job::Send { raw, .. } => Some(raw),
            _ => None,
        })
        .unwrap();
    let p = quatrelle_mime::parse(&raw).unwrap();
    // Read back: the logo and the name side by side, the logo on the web, the message's text.
    assert!(p.had_html);
    let side_by_side = p.layout.iter().any(|b| matches!(b, quatrelle_mime::layout::Block::Table(t) if t.rows.iter().any(|r| r.len() == 2)));
    assert!(side_by_side, "{:?}", p.layout);
    assert!(p.images.iter().any(|u| u.contains("www.contoso.example/images/logo.gif")), "{:?}", p.images);
    assert!(p.text.contains("Hi Zoe") && p.text.contains("Didier GAUDIN") && !p.text.contains("x()"), "{}", p.text);
}
