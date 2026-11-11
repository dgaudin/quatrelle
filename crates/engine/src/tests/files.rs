// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
//! Engine tests: what the system hands Quatrelle as its mail app, `mailto:` links and the
//! `.eml`, `.ics` and `.vcf` files a user opens with it.

use super::*;

/// A scratch folder for one test, emptied when the test is over (pass or fail).
struct TempFolder(std::path::PathBuf);

impl TempFolder {
    fn new(label: &str) -> TempFolder {
        let folder = std::env::temp_dir().join(format!("quatrelle-open-{}-{label}", std::process::id()));
        if folder.exists() {
            std::fs::remove_dir_all(&folder).unwrap();
        }
        std::fs::create_dir_all(&folder).unwrap();
        TempFolder(folder)
    }

    /// Writes `contents` to `file_name` in the folder, and hands back where it went.
    fn put(&self, file_name: &str, contents: &[u8]) -> std::path::PathBuf {
        let at = self.0.join(file_name);
        std::fs::write(&at, contents).unwrap();
        at
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        // Leftovers in the temporary folder are harmless; a failed clean-up isn't a failed test.
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// Runs `file.open` on `path`.
fn open_path(s: &mut Session, path: &std::path::Path) -> Result<Value, CmdError> {
    s.run("file.open", &json!({ "path": path.display().to_string() }))
}

#[test]
fn a_mailto_link_starts_a_message_and_attaches_nothing() {
    let mut s = sample_session("mailto");
    let link = "mailto:lee@greenhouse.example?subject=Seed%20order&cc=mo@farm.example&body=Dear%20Lee%2C%0D%0Athanks&bcc=ned@farm.example&attach=%2Fetc%2Fpasswd";
    let r = s.run("mail.mailto", &json!({ "url": link })).unwrap();
    let draft = s.compose.as_ref().unwrap();
    assert_eq!(r["compose"], draft.id);
    assert_eq!(draft.to, "lee@greenhouse.example");
    assert_eq!(draft.cc, "mo@farm.example");
    assert_eq!(draft.bcc, "ned@farm.example");
    assert_eq!(draft.subject, "Seed order");
    assert!(draft.body.starts_with("Dear Lee,\nthanks"), "{:?}", draft.body);
    assert!(draft.show_cc, "the copy lines are shown when the link fills them");
    assert!(draft.attachments.is_empty(), "`attach=` in a link is ignored");

    for bad in [json!({ "url": "ftp://farm.example/seeds" }), json!({})] {
        assert!(s.run("mail.mailto", &bad).is_err(), "{bad}");
    }
    let sending = s.take_jobs().into_iter().filter(|j| matches!(j, Job::Send { .. })).count();
    assert_eq!(sending, 0, "the draft waits for the user");
    done(s);
}

#[test]
fn a_message_file_is_shown() {
    let mut s = sample_session("open-eml");
    let scratch = TempFolder::new("eml");
    let source = concat!(
        "Date: Sat, 7 Nov 2026 14:15:00 +0100\r\n",
        "Subject: Greenhouse keys\r\n",
        "From: Lee Park <lee@greenhouse.example>\r\n",
        "To: pat@studio.example\r\n",
        "\r\n",
        "Under the blue pot.\r\n",
    );
    let path = scratch.put("Greenhouse keys.eml", source.as_bytes());
    let shown = open_path(&mut s, &path).unwrap();
    assert_eq!(shown["kind"], "message");
    assert_eq!(shown["name"], "Greenhouse keys.eml");
    assert_eq!(shown["subject"], "Greenhouse keys");
    assert!(shown["from"].as_str().unwrap().contains("lee@greenhouse.example"), "{shown}");
    assert!(shown["text"].as_str().unwrap().contains("blue pot"), "{shown}");

    // Not there, a folder, or nothing at all: each is an error, not a crash.
    assert!(open_path(&mut s, &path.with_file_name("gone.eml")).is_err());
    assert!(open_path(&mut s, &scratch.0).is_err());
    assert!(s.run("file.open", &json!({ "path": "   " })).is_err());
    done(s);
}

/// Runs `file.add` on `path`: what the user says yes to, once `file.open` said what it holds.
fn add_path(s: &mut Session, path: &std::path::Path) -> Result<Value, CmdError> {
    s.run("file.add", &json!({ "path": path.display().to_string() }))
}

#[test]
fn calendar_and_contacts_files_are_added_once_the_user_says_so() {
    let mut s = sample_session("open-pim");
    let scratch = TempFolder::new("pim");
    let calendar = [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "BEGIN:VEVENT",
        "UID:harvest@farm.example",
        "SUMMARY:Harvest day",
        "DTSTART:20261112T120000Z",
        "DTEND:20261112T150000Z",
        "END:VEVENT",
        // An event with nothing but a start is passed over and counted.
        "BEGIN:VEVENT",
        "UID:bare@farm.example",
        "DTSTART:20261113T120000Z",
        "END:VEVENT",
        "END:VCALENDAR",
        "",
    ]
    .join("\r\n");
    // The extension is matched whatever its case. Opening says what the file holds, no more.
    let path = scratch.put("harvest.Ics", calendar.as_bytes());
    let held = open_path(&mut s, &path).unwrap();
    assert_eq!(held["kind"], "calendar", "{held}");
    assert_eq!((held["count"].as_u64(), held["unreadable"].as_u64(), held["already"].as_u64()), (Some(1), Some(1), Some(0)), "{held}");
    assert_eq!(held["titles"], json!(["Harvest day"]), "{held}");
    assert!(held["into"].as_str().is_some_and(|c| !c.is_empty()), "the calendar they would go to is named: {held}");
    assert!(s.find_event("harvest@farm.example").is_none(), "nothing is added by opening");
    let added = add_path(&mut s, &path).unwrap();
    assert_eq!((added["added"].as_u64(), added["skipped"].as_u64()), (Some(1), Some(1)), "{added}");
    assert!(s.find_event("harvest@farm.example").is_some());
    let on_screen = s.run("calendar.goto", &json!({})).unwrap();
    assert_eq!(on_screen["date"], "2026-11-12", "the calendar moves to the first added event");

    let cards = [
        "BEGIN:VCARD",
        "VERSION:3.0",
        "FN:Mo Ortiz",
        "EMAIL:mo@farm.example",
        "END:VCARD",
        "BEGIN:VCARD",
        "VERSION:3.0",
        "FN:Nobody",
        "EMAIL:no at sign here",
        "END:VCARD",
        "",
    ]
    .join("\r\n");
    // A name that says nothing: the first line tells what the file is.
    let path = scratch.put("export.dat", cards.as_bytes());
    let held = open_path(&mut s, &path).unwrap();
    assert_eq!(held["kind"], "contacts", "{held}");
    assert_eq!((held["count"].as_u64(), held["unreadable"].as_u64()), (Some(1), Some(1)), "{held}");
    assert!(!s.people("Ortiz").iter().any(|p| p.card.display() == "Mo Ortiz"), "nobody is added by opening");
    let added = add_path(&mut s, &path).unwrap();
    assert_eq!((added["added"].as_u64(), added["skipped"].as_u64()), (Some(1), Some(1)), "{added}");
    assert!(s.people("Ortiz").iter().any(|p| p.card.display() == "Mo Ortiz"));
    done(s);
}

/// A calendar file is a stranger's text. Opened, it changes nothing; added, it invites nobody
/// (its guests and organizer stay in the file), leaves alone the event that already has one of
/// its identifiers, and brings a bounded number of events.
#[test]
fn a_calendar_file_invites_nobody_and_replaces_nothing() {
    let mut s = sample_session("open-hostile-ics");
    let scratch = TempFolder::new("hostile-ics");
    let mine = s.run("event.create", &json!({"summary": "Board meeting", "start": "2026-11-20T10:00", "end": "2026-11-20T11:00"})).unwrap();
    let uid = mine["uid"].as_str().unwrap().to_string();
    let mut lines = vec!["BEGIN:VCALENDAR".to_string(), "VERSION:2.0".into()];
    let mut event = |uid: &str, title: &str| {
        lines.extend([
            "BEGIN:VEVENT".to_string(),
            format!("UID:{uid}"),
            format!("SUMMARY:{title}"),
            "DTSTART:20261120T100000Z".into(),
            "DTEND:20261120T110000Z".into(),
            "ORGANIZER;CN=Pat:mailto:pat@studio.example".into(),
            "ATTENDEE;CN=Lee;RSVP=TRUE:mailto:lee@greenhouse.example".into(),
            "BEGIN:VALARM".into(),
            "ACTION:EMAIL".into(),
            "ATTENDEE:mailto:mo@farm.example".into(),
            "END:VALARM".into(),
            "END:VEVENT".into(),
        ]);
    };
    event(&uid, "Moved to the car park");
    event("planted@farm.example", "Planted");
    for n in 0..crate::cmd::MAX_FILE_EVENTS + 20 {
        event(&format!("bulk-{n}@farm.example"), "Bulk");
    }
    lines.extend(["END:VCALENDAR".to_string(), String::new()]);
    let path = scratch.put("meeting.ics", lines.join("\r\n").as_bytes());
    let before = s.pim_mut("sample").events.len();

    let held = open_path(&mut s, &path).unwrap();
    assert_eq!(s.pim_mut("sample").events.len(), before, "opening adds nothing");
    assert_eq!(s.find_event(&uid).unwrap().1.event.summary, "Board meeting", "opening replaces nothing");
    assert_eq!(held["already"].as_u64(), Some(1), "{held}");
    assert_eq!(held["count"].as_u64(), Some(crate::cmd::MAX_FILE_EVENTS as u64), "{held}");
    assert!(held["more"].as_u64().is_some_and(|n| n > 0), "what is over the cap is said: {held}");
    assert!(s.take_jobs().is_empty(), "and nothing is asked of a server");

    let added = add_path(&mut s, &path).unwrap();
    assert_eq!(added["added"].as_u64(), Some(crate::cmd::MAX_FILE_EVENTS as u64), "{added}");
    assert_eq!(added["already"].as_u64(), Some(1), "{added}");
    assert_eq!(s.find_event(&uid).unwrap().1.event.summary, "Board meeting", "the user's own event stays as it was");
    let events = &s.pim_mut("sample").events;
    assert_eq!(events.len(), before + crate::cmd::MAX_FILE_EVENTS);
    let planted = events.iter().find(|e| e.event.summary == "Planted").expect("added");
    assert!(planted.event.attendees.is_empty() && planted.event.organizer.is_none(), "{:?}", planted.event);
    assert!(!planted.event.extra.iter().any(|l| l.to_ascii_uppercase().contains("ATTENDEE")), "{:?}", planted.event.extra);
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Send { .. })), "nothing is sent");
    done(s);
}

/// A contacts file adds nobody twice, replaces nobody, and brings a bounded number of people.
#[test]
fn a_contacts_file_replaces_nobody() {
    let mut s = sample_session("open-hostile-vcf");
    let scratch = TempFolder::new("hostile-vcf");
    let card = |uid: &str, name: &str, email: &str| format!("BEGIN:VCARD\r\nVERSION:3.0\r\nUID:{uid}\r\nFN:{name}\r\nEMAIL:{email}\r\nEND:VCARD\r\n");
    let first = scratch.put("one.vcf", card("mo-1", "Mo Ortiz", "mo@farm.example").as_bytes());
    add_path(&mut s, &first).unwrap();
    let mut text = card("mo-1", "Mo Ortiz", "mo@elsewhere.example");
    for n in 0..crate::cmd::MAX_FILE_CARDS + 5 {
        text.push_str(&card(&format!("bulk-{n}"), &format!("Bulk {n}"), "bulk@farm.example"));
    }
    let path = scratch.put("many.vcf", text.as_bytes());
    let held = open_path(&mut s, &path).unwrap();
    assert_eq!((held["already"].as_u64(), held["count"].as_u64()), (Some(1), Some(crate::cmd::MAX_FILE_CARDS as u64)), "{held}");
    let added = add_path(&mut s, &path).unwrap();
    assert_eq!((added["added"].as_u64(), added["already"].as_u64()), (Some(crate::cmd::MAX_FILE_CARDS as u64), Some(1)), "{added}");
    let mo: Vec<_> = s.people("Ortiz").into_iter().filter(|p| p.card.display() == "Mo Ortiz").collect();
    assert_eq!(mo.len(), 1);
    assert!(mo[0].card.emails.iter().any(|e| e.value == "mo@farm.example"), "the card kept is the one that was there");
    done(s);
}

/// With no account there's nowhere to put them: said, not crashed.
#[test]
fn without_an_account_nothing_is_added() {
    let mut s = Session::open(None, quatrelle_model::dates::Clock::new(NOW, 0));
    let scratch = TempFolder::new("no-account");
    let card = scratch.put("one.vcf", b"BEGIN:VCARD\r\nFN:Solo\r\nEND:VCARD\r\n");
    assert!(matches!(open_path(&mut s, &card), Err(CmdError::Disabled(_))));
    let link = s.run("mail.mailto", &json!({ "url": "mailto:solo@farm.example" }));
    assert!(matches!(link, Err(CmdError::Disabled(_))));
}

/// Issue #38: message files dropped on the window go into the folder open, filed by the network
/// layer, which says how many went in; anything but a folder open (All Inboxes), or no files,
/// is refused. An Outlook `.msg` opened with Quatrelle is read by the network layer and shown
/// when it comes.
#[test]
fn message_files_go_into_the_folder_open_and_a_msg_is_shown_when_read() {
    let mut s = sample_session("message-files");
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let r = s.run("file.addToFolder", &json!({"paths": ["C:/mail/Quarterly figures.msg", "  ", "C:/mail/hello.eml"]})).unwrap();
    assert_eq!((r["folder"].as_str(), r["files"].as_u64()), (Some("INBOX"), Some(2)));
    let jobs = s.take_jobs();
    assert!(
        jobs.iter()
            .any(|j| matches!(j, Job::FileMessages { account, folder, paths } if account == "sample" && folder == "INBOX" && paths.len() == 2)),
        "{jobs:?}"
    );
    assert!(s.run("file.addToFolder", &json!({"paths": []})).is_err());
    assert!(s.run("file.addToFolder", &json!({"paths": ["a.msg"], "folder": "Nowhere"})).is_err());
    s.run("folder.unified", &json!({"role": "inbox"})).unwrap();
    assert!(s.run("file.addToFolder", &json!({"paths": ["a.msg"]})).is_err(), "no one folder open");
    s.apply(Event::MessagesFiled {
        account: "sample".into(),
        folder: "INBOX".into(),
        added: 1,
        failed: vec![("hello.eml".into(), "not a message".into())],
    });
    assert_eq!(s.take_filed().iter().map(|f| (f.added, f.failed.len())).collect::<Vec<_>>(), [(1, 1)]);
    assert!(s.take_filed().is_empty());
    assert!(is_message_file("x.MSG") && is_message_file("y.eml") && !is_message_file("z.pdf"));

    // A .msg opened: read elsewhere, then shown.
    let t = TempFolder::new("msg");
    let at = t.put("Quarterly figures.msg", &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0]);
    let r = open_path(&mut s, &at).unwrap();
    assert_eq!(r["kind"], "reading");
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::ReadMessageFile { path } if path.ends_with("Quarterly figures.msg"))));
    let raw = b"From: Sam <sam@studio.test>\r\nTo: me@studio.test\r\nSubject: Quarterly figures\r\n\r\nHello.\r\n".to_vec();
    s.apply(Event::MessageFileRead { path: at.display().to_string(), raw: Ok(raw) });
    let shown = s.opened_file.take().unwrap().unwrap();
    assert_eq!(
        (shown["kind"].as_str(), shown["subject"].as_str(), shown["name"].as_str()),
        (Some("message"), Some("Quarterly figures"), Some("Quarterly figures.msg"))
    );
    s.apply(Event::MessageFileRead { path: at.display().to_string(), raw: Err("the file is damaged".into()) });
    assert!(s.opened_file.take().unwrap().is_err());
}

fn is_message_file(path: &str) -> bool {
    crate::cmd::is_message_file(path)
}
