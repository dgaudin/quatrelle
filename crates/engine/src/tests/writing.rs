//! Engine tests: writing, saving, sending and the Outbox.

use super::*;

mod drafts;
mod names;
mod senders;

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

/// Attach Item: a message, a contact and an appointment on this computer go along as files
/// (`.eml`, `.vcf`, `.ics`) that read back as what they were; words typed narrow the choice.
#[test]
fn items_attach_as_files() {
    use crate::ItemKind;
    let mut s = sample_session("attach-items");
    let account = sample::SAMPLE_ACCOUNT;
    s.run("mail.new", &json!({})).unwrap();
    let mails = s.item_choices(ItemKind::Message, account, "INBOX", "");
    assert!(mails.len() > 1 && mails.windows(2).all(|w| w[0].when >= w[1].when), "newest first");
    let found = s.item_choices(ItemKind::Message, account, "INBOX", "storyboard");
    assert!(!found.is_empty() && found.iter().all(|m| m.title.to_lowercase().contains("storyboard")), "{found:?}");
    let mail = &found[0];
    let contact = s.item_choices(ItemKind::Contact, "", "", "").into_iter().next().expect("the sample has contacts");
    let event = s.item_choices(ItemKind::Event, "", "", "").into_iter().find(|e| !e.title.is_empty()).expect("the sample has appointments");
    for (kind, id) in [("message", &mail.id), ("contact", &contact.id), ("event", &event.id)] {
        s.run("compose.attachItem", &json!({"kind": kind, "id": id})).unwrap();
    }
    let att = s.compose.as_ref().unwrap().attachments.clone();
    assert_eq!(att.iter().map(|a| a.mime.as_str()).collect::<Vec<_>>(), ["message/rfc822", "text/vcard", "text/calendar"]);
    assert_eq!(att[0].name, quatrelle_mime::sanitize_filename(&format!("{}.eml", mail.title)), "named after it, safe to write");
    assert_eq!(quatrelle_mime::parse(&att[0].data).unwrap().subject, mail.title);
    let cards = quatrelle_model::vcard::parse(&String::from_utf8_lossy(&att[1].data));
    assert_eq!(cards.first().map(|c| c.uid.as_str()), Some(contact.id.as_str()));
    assert!(att[2].name.ends_with(".ics"));
    let cal = quatrelle_model::ical::parse(&String::from_utf8_lossy(&att[2].data), 0);
    assert!(cal.events.iter().any(|e| e.summary == event.title), "{:?}", cal.events);
    assert!(s.run("compose.attachItem", &json!({"kind": "folder", "id": "x"})).is_err());
    assert!(s.run("compose.attachItem", &json!({"kind": "contact", "id": "nobody"})).is_err());
    done(s);
}

/// A message's headers, as its source carries them (who really sent it, the servers it passed
/// through): the header lines alone, not its text; nothing without a message open.
#[test]
fn a_message_shows_its_headers() {
    let mut s = sample_session("headers");
    assert!(s.run("message.headers", &json!({})).is_err(), "no message open");
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.open", &k).unwrap();
    let v = s.run("message.headers", &json!({})).unwrap();
    let text = v["text"].as_str().unwrap();
    assert!(text.lines().any(|l| l == "Subject: Storyboard review on Thursday"), "{text}");
    assert!(text.lines().any(|l| l.starts_with("Message-ID:")), "{text}");
    assert!(!text.contains('\r') && !text.ends_with('\n'), "lines as written, without the blank one");
    let body = quatrelle_mime::parse(&s.message_source().unwrap()).unwrap().text;
    let first = body.lines().find(|l| !l.trim().is_empty()).unwrap();
    assert!(!text.contains(first), "the text isn't part of it");
    done(s);
}

/// Save As from the list: one message where asked, several in a folder each under its subject
/// (a name taken gets a number), one not on this computer counted, not saved.
#[test]
fn messages_save_as_from_the_list() {
    let mut s = sample_session("save-list");
    let dir = s.store.as_ref().unwrap().root().join("saved");
    std::fs::create_dir_all(&dir).unwrap();
    let id = |s: &Session, subject: &str| key_of(s, subject)["key"].as_str().unwrap().to_string();
    let (a, b) = (id(&s, "Storyboard review on Thursday"), id(&s, "Your proof for the festival poster is ready"));
    let gone = MessageKey { uid: 99_999, ..MessageKey::from_id(&a).unwrap() }.to_id();
    let one = dir.join("one.eml");
    s.run("messages.saveAs", &json!({"keys": [a], "path": one.to_string_lossy()})).unwrap();
    assert_eq!(quatrelle_mime::parse(&std::fs::read(&one).unwrap()).unwrap().subject, "Storyboard review on Thursday");
    assert!(s.run("messages.saveAs", &json!({"keys": [a, b], "path": one.to_string_lossy()})).is_err(), "a path is for one");
    let v = s.run("messages.saveAs", &json!({"keys": [a, b, a, gone], "dir": dir.to_string_lossy()})).unwrap();
    assert_eq!(v["missing"], 1, "{v}");
    let mut names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    names.sort();
    assert_eq!(
        names,
        ["Storyboard review on Thursday (2).eml", "Storyboard review on Thursday.eml", "Your proof for the festival poster is ready.eml", "one.eml"]
    );
    // Without keys: the messages selected.
    s.run("list.open", &json!({"key": b})).unwrap();
    let v = s.run("messages.saveAs", &json!({"dir": dir.to_string_lossy()})).unwrap();
    assert!(v["saved"][0].as_str().unwrap().ends_with("Your proof for the festival poster is ready (2).eml"), "{v}");
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
    let html = r#"<table align="left" style="float:left;"><tr><td><a href="https://www.proginov.com"><img src="https://www.proginov.com/images/logo.gif" width="194"></a></td></tr></table>
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
    assert!(p.images.iter().any(|u| u.contains("www.proginov.com/images/logo.gif")), "{:?}", p.images);
    assert!(p.text.contains("Hi Zoe") && p.text.contains("Didier GAUDIN") && !p.text.contains("x()"), "{}", p.text);
}

/// Check Names writes out the address of a name typed alone, from the people known, and leaves
/// (and names) the ones it can't tell; the language a message's spelling is checked in is kept
/// with it.
#[test]
fn names_are_checked_and_a_language_is_kept() {
    let mut s = sample_session("check-names");
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "zoe; nobody-of-that-name", "cc": "ann@x.example"})).unwrap();
    let r = s.run("compose.checkNames", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.to, "Zoe Park <zoe@studio.example>; nobody-of-that-name; ", "{r}");
    assert_eq!(c.cc, "ann@x.example; ");
    assert_eq!(r["unresolved"], json!(["nobody-of-that-name"]));
    assert_eq!(r["resolved"], 1);
    // Checked again: nothing changes.
    s.run("compose.checkNames", &json!({})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().to, c.to);
    assert_eq!(s.run("compose.language", &json!({"language": "FR"})).unwrap()["language"], "fr");
    assert_eq!(s.compose.as_ref().unwrap().language, "fr");
    assert!(s.run("compose.language", &json!({"language": "fr; drop"})).is_err());
    s.run("compose.language", &json!({})).unwrap();
    assert!(s.compose.as_ref().unwrap().language.is_empty(), "the interface's language again");
}

/// A signature made in the editor comes back exactly as set, in every new message and in what's
/// sent: its size and typeface, its underline and its centring (it was read back from its HTML
/// before, which kept none of them). Outlook puts no `-- ` line before a signature: one goes
/// in only when the preferences ask for it.
#[test]
fn a_signature_keeps_every_bit_of_its_look() {
    use quatrelle_mime::rich::{Align, Style, Styles};
    let mut s = Session::open(Some(temp("signature-look")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "dgaudin@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let text = "Didier GAUDIN\nProginov";
    let mut styles = Styles::plain(text);
    styles.restyle(text, 0..13, |st| Style { size: Some(16), font: Some("Arial".into()), underline: true, ..st.clone() });
    styles.relayout(text, 0..1, |p| p.align = Align::Center);
    s.run("account.setSignature", &json!({"account": id, "signature": text, "styles": styles})).unwrap();
    let config = s.account(&id).unwrap().config.clone();
    assert!(!config.signature_styles.is_empty() && config.signature_html.contains("16pt"), "{config:?}");
    // Saved, then read again: the styles themselves, not a rendering of them.
    let (again, again_styles) = s.signature_rich(&id);
    assert_eq!(again, text);
    assert_eq!(again_styles.style(0), styles.style(0));
    assert_eq!(again_styles.para(0).align, Align::Center);
    // Into a new message, with no `-- ` line before it.
    s.run("mail.new", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.body, "\n\nDidier GAUDIN\nProginov");
    assert_eq!(c.signature, "Didier GAUDIN\nProginov");
    let st = c.styles.style(2);
    assert_eq!((st.size, st.font.as_deref(), st.underline), (Some(16), Some("Arial"), true), "{st:?}");
    assert_eq!(c.styles.para(2).align, Align::Center);
    assert!(!s.compose_written(), "a message with only its signature has nothing written");
    // Written and sent: the HTML says it all, the text twin has no `-- ` line.
    s.run("compose.update", &json!({"to": "zoe@example.org", "subject": "Hello", "body": "Hi Zoe"})).unwrap();
    s.run("compose.insertSignature", &json!({})).unwrap();
    let (plain, html, _) = s.compose.as_ref().unwrap().bodies();
    let html = html.unwrap_or_default();
    assert!(html.contains("<span style=\"font-family:'Arial',sans-serif; font-size:16pt\"><u>Didier GAUDIN</u></span>"), "{html}");
    assert!(html.contains("<div style=\"text-align:center\">"), "{html}");
    assert!(!plain.contains("-- "), "{plain}");
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
    assert!(p.had_html && p.text.contains("Hi Zoe") && p.text.contains("Didier GAUDIN") && !p.text.contains("-- "), "{}", p.text);
    // The `-- ` line asked for: it goes in before the signature, which keeps its look.
    s.run("view.signatureDashes", &json!({"on": true})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!((c.body.as_str(), c.signature.as_str()), ("\n\n-- \nDidier GAUDIN\nProginov", "-- \nDidier GAUDIN\nProginov"));
    assert!(c.styles.style(6).underline && c.styles.para(3).align == Align::Center, "{:?}", c.styles);
    // Junk styles are refused, not trusted.
    assert!(s.run("account.setSignature", &json!({"account": id, "signature": "x", "styles": "junk"})).is_err());
    let mut junk = Styles::plain("x");
    junk.chars = vec![9999];
    s.run("account.setSignature", &json!({"account": id, "signature": "x", "styles": junk})).unwrap();
    assert!(s.signature_rich(&id).1.is_plain());
}

/// The address book over a big mailbox (16 folders of 5,000 messages, more than the cache keeps):
/// the window's suggestions never read it whole in one go. The first name typed starts the
/// build and reads nothing; each slice reads at most the one folder it is in (a slice of 5,000
/// over folders of 5,000: never more than two folders cached beyond those before), and the
/// book is complete in as many slices as the mailbox has; what it then suggests is what the
/// whole book (a command's, read in one go) suggests. Built whole, as it was, the first name
/// typed read every folder from disk at once: seconds, with the window frozen.
#[test]
fn recipient_suggestions_read_a_big_mailbox_a_slice_at_a_time() {
    let store = temp("contacts-slices");
    sample::seed_big_mailbox(&store, 16, 5000, NOW).unwrap();
    let mut s = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    s.set_cache_limit(12_000);
    s.run("folder.open", &json!({"account": "big", "folder": "INBOX"})).unwrap();
    let began = std::time::Instant::now();
    let first = s.suggest_contacts_now("ann1", 8);
    let asked = began.elapsed();
    assert!(first.is_empty() && s.contacts_pending(), "nothing known yet: the build starts, reading nothing");
    assert_eq!(s.cache_order.len(), 1, "the folder on screen only; the first name typed read no folder");
    let revision = s.contacts_revision();
    let mut slices = 0;
    let mut longest = std::time::Duration::ZERO;
    let mut most = 0;
    while s.contacts_pending() {
        let before: Vec<(String, String)> = s.cache_order.clone();
        let began = std::time::Instant::now();
        let done = s.contacts_step(5000);
        longest = longest.max(began.elapsed());
        most = most.max(s.cache_order.iter().filter(|k| !before.contains(k)).count());
        slices += 1;
        assert!(slices <= 17, "16 folders of a slice each (and one more to finish)");
        assert_eq!(done, !s.contacts_pending());
    }
    assert!(most <= 2, "a slice reads the folder it is in, at most the next too: {most}");
    assert_ne!(s.contacts_revision(), revision, "what the book answers changed");
    let built = s.suggest_contacts_now("ann1", 8);
    assert_eq!(built.len(), 8);
    assert!(built.iter().all(|c| c.address.name.starts_with("Ann1")), "{built:?}");
    assert_eq!(s.suggest_contacts("ann1", 8), built, "the whole book, read in one go, answers the same");
    println!("asked {asked:?}; {slices} slices, the longest {longest:?}");
    done(s);
}

/// The address book is kept on disk, as Outlook's AutoComplete list: a start reads no folder,
/// and a folder a sync changed has only its new mail read (not the whole mailbox again every few
/// minutes, as it was). An address taken out of the suggestions (the ×) stays out — after a
/// restart, and whatever mail comes from it — until it's written to. The Focused Inbox follows
/// what the book learns.
#[test]
fn the_address_book_is_kept_and_reads_only_new_mail() {
    let store = temp("contacts-kept");
    sample::seed_big_mailbox(&store, 4, 100, NOW).unwrap();
    let open = || {
        let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
        s.take_jobs();
        s.run("folder.open", &json!({"account": "big", "folder": "INBOX"})).unwrap();
        s
    };
    let mut s = open();
    assert_eq!(s.suggest_contacts("ann1", 8).len(), 8, "a first book, read whole");
    drop(s);
    // Started again: the book is there at once, read from no folder.
    let mut s = open();
    let before = s.cached_folders();
    assert_eq!(s.suggest_contacts_now("ann1", 8).len(), 8);
    assert!(s.contacts_step(usize::MAX));
    assert_eq!(s.cached_folders(), before, "no folder read for it");
    // New mail in a folder: only that folder is read, only its new message counted.
    let arrive = |s: &mut Session, uid: u32| {
        let mut st = store.load_folder("big", "Folder2").unwrap();
        let mut m = st.messages[0].clone();
        m.key.uid = uid;
        m.from = EmailAddress::new("Zed New", "zed@new.example");
        m.date = NOW;
        st.messages.insert(0, m);
        store.save_folder("big", "Folder2", &st).unwrap();
        s.apply(Event::FolderChanged { account: "big".into(), folder: "Folder2".into() });
    };
    s.run("view.focused", &json!({"on": true})).unwrap();
    let revision = s.contacts_revision();
    let list = s.revision;
    arrive(&mut s, 1000);
    assert!(s.contacts_pending());
    assert!(s.contacts_step(usize::MAX) && !s.contacts_pending());
    assert_eq!(s.suggest_contacts_now("zed", 8).len(), 1, "the new sender");
    assert_eq!(s.known_contact("p201@big.example").map(|c| c.received), Some(1), "the folder's old mail isn't counted again");
    assert_ne!(s.contacts_revision(), revision);
    assert_ne!(s.revision, list, "the Focused Inbox's list made again once read");
    // Taken out with its ×: gone, and still gone after a start and more mail from it.
    let r = s.run("contacts.forget", &json!({"email": "Zed@New.example"})).unwrap();
    assert_eq!(r["removed"], true);
    assert!(s.suggest_contacts_now("zed", 8).is_empty());
    assert!(s.run("contacts.forget", &json!({"email": "nobody"})).is_err());
    drop(s);
    let mut s = open();
    arrive(&mut s, 1001);
    assert!(s.contacts_step(usize::MAX));
    assert!(s.suggest_contacts_now("zed", 8).is_empty(), "mail from it doesn't bring it back");
    // Written to: back, first.
    s.contacts_sent(&[EmailAddress::new("Zed New", "zed@new.example")]);
    let back = s.suggest_contacts_now("zed", 8);
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].sent, 1);
    done(s);
}

/// Attaching from a path reads a plain file only: a folder, a device that reads forever
/// (`/dev/zero`, which reports a size of 0) or a pipe that blocks are refused, at once.
#[test]
fn attaching_reads_plain_files_only() {
    let mut s = sample_session("attach-paths");
    s.run("mail.new", &json!({})).unwrap();
    let dir = s.store.as_ref().unwrap().root().to_path_buf();
    let e = s.run("compose.attach", &json!({"path": dir.to_string_lossy()})).unwrap_err().to_string();
    assert!(e.contains("isn't a file"), "{e}");
    let device = if cfg!(windows) { r"\.\NUL" } else { "/dev/zero" };
    let started = std::time::Instant::now();
    assert!(s.run("compose.attach", &json!({"path": device})).is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "refused, not read");
    let file = dir.join("ok.txt");
    std::fs::write(&file, b"hi").unwrap();
    s.run("compose.attach", &json!({"path": file.to_string_lossy()})).unwrap();
    assert_eq!(s.compose.as_ref().map(|c| c.attachments.len()), Some(1));
    done(s);
}
