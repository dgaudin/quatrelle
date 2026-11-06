// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What contacts and calendar items carry: a contact's picture, and the files of contacts and
//! events, kept on their card or event; those of a task or note, which have nowhere to keep one,
//! counted.

use outlook_pst::UnicodePstValue as V;

use super::super::super::item::{File, Item};

use super::*;

/// A file attached by value, with these properties of its own.
fn file<'a>(name: &'a str, mime: &'a str, data: &'a [u8], properties: &'a [UnicodePstProperty<'a>]) -> UnicodePstAttachment<'a> {
    UnicodePstAttachment { filename: name, mime_type: mime, data, properties, ..Default::default() }
}

/// The lines of a card or event that hold a file, up to their value.
fn file_lines(extra: &[String]) -> Vec<String> {
    extra
        .iter()
        .filter(|l| l.starts_with("ATTACH") || l.starts_with("X-QUATRELLE-ATTACH"))
        .map(|l| l.split(':').next().unwrap_or_default().to_string())
        .collect()
}

/// A contact comes in with its picture (the attachment Outlook flags as one, hidden or not) and
/// its other files; an appointment with its files; a task's file is counted, as it has nowhere to
/// go.
#[test]
fn pictures_and_files_come_in_on_cards_and_events() {
    let png = b"\x89PNG\r\n\x1a\npicture";
    let photo_flag = [UnicodePstProperty::tagged(0x7FFF, V::Boolean(true))];
    let mut contact = Put::new(&["Contacts"], "IPF.Contact", item("IPM.Contact", "Ana Lima", "", &[]));
    contact.files = vec![file("ContactPicture.png", "image/png", png, &photo_flag), file("cv.pdf", "application/pdf", b"%PDF", &[])];
    let when = [named(APPOINTMENT, 0x820D, V::Time(filetime(1_700_070_400))), named(APPOINTMENT, 0x820E, V::Time(filetime(1_700_074_000)))];
    let mut event = Put::new(&["Calendar"], "IPF.Appointment", item("IPM.Appointment", "Review", "", &when));
    event.files = vec![file("agenda.pdf", "application/pdf", b"%PDF-1.7", &[])];
    let mut task = Put::new(&["Tasks"], "IPF.Task", item("IPM.Task", "Proofs", "", &[]));
    task.files = vec![file("list.txt", "text/plain", b"one", &[])];
    let b = Brought::of("files", &[contact, event, task]);
    assert_eq!((b.done.contacts, b.done.events, b.done.tasks), (1, 1, 1), "{:?}", b.done);
    let pim = b.pim();
    let card = &pim.cards[0].card;
    let photo = card.photo.as_ref().expect("the contact's picture");
    assert_eq!((photo.media.as_str(), photo.bytes(1 << 20).as_deref()), ("image/png", Some(&png[..])));
    assert_eq!(file_lines(&card.extra), ["X-QUATRELLE-ATTACH;X-FILENAME=\"cv.pdf\""]);
    assert_eq!(file_lines(&pim.events[0].event.extra), ["ATTACH;FMTTYPE=application/pdf;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"agenda.pdf\""]);
    assert_eq!(b.done.files_left_out.get(&FileLeftOut::NoPlace), Some(&1), "{:?}", b.done);
    // What the contact's page and the appointment's window show of them, and open (R23).
    use quatrelle_model::pim::files::{card_file_data, card_files, event_file_data, event_files};
    let shown = |f: Vec<quatrelle_model::pim::files::KeptFile>| f.into_iter().map(|f| (f.name, f.mime, f.size)).collect::<Vec<_>>();
    assert_eq!(shown(card_files(card)), [("cv.pdf".to_string(), "application/pdf".to_string(), 4)]);
    assert_eq!(card_file_data(card, 0).as_deref(), Some(&b"%PDF"[..]));
    let event = &pim.events[0].event;
    assert_eq!(shown(event_files(event)), [("agenda.pdf".to_string(), "application/pdf".to_string(), 8)]);
    assert_eq!(event_file_data(event, 0).as_deref(), Some(&b"%PDF-1.7"[..]));
}

/// A file as the import holds it.
fn data(name: &str, mime: &str, n: usize) -> File {
    File::Data { name: name.into(), mime: mime.into(), cid: String::new(), data: vec![b'x'; n] }
}

/// A changed occurrence's files go on its own event, the series' on the series; a file past what
/// the contacts and calendar keep is counted as too big, and the files left out as an item was read
/// are counted too; an exception item that is no occurrence (it says no time) is counted.
#[test]
fn an_occurrence_s_files_go_on_its_event_and_big_ones_are_counted() {
    use quatrelle_model::dates::days_from_civil;
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-pim-out-files-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut progress = |_| {};
    let mut out = Out::new(&store, "d1", &mut progress);
    let at = |d: u32, h: i64| days_from_civil(2026, 10, d) * 86_400 + h * 3600;
    let (pattern, start) = crate::pst_pim::tests::series(0x3009);
    let own = File::Message(Box::new(Item {
        class: "IPM.OLE.CLASS.{00061055-0000-0000-C000-000000000046}".into(),
        subject: "Review".into(),
        body: "Bring the proofs.".into(),
        files: vec![data("proofs.pdf", "application/pdf", 4)],
        extra: Some(crate::pst_pim::Extra { start: Some(at(19, 8)), end: Some(at(19, 9)), replaces: Some(at(19, 8)), ..Default::default() }),
        ..Default::default()
    }));
    let timeless = File::Message(Box::new(Item {
        class: "IPM.OLE.CLASS.{00061055-0000-0000-C000-000000000046}".into(),
        files: vec![data("lost.pdf", "application/pdf", 1)],
        extra: Some(crate::pst_pim::Extra { replaces: Some(at(26, 7)), ..Default::default() }),
        ..Default::default()
    }));
    out.add_pim(Item {
        class: "IPM.Appointment".into(),
        subject: "Review".into(),
        files: vec![data("agenda.pdf", "application/pdf", 3), own, data("huge.bin", "application/octet-stream", 5 << 20), timeless],
        left_out: BTreeMap::from([(FileLeftOut::Link, 1)]),
        extra: Some(crate::pst_pim::Extra { start: Some(start), end: Some(start + 3600), recurrence: pattern, ..Default::default() }),
        ..Default::default()
    });
    let done = out.finish();
    assert_eq!((done.events, done.skipped), (1, 1));
    assert_eq!(done.files_left_out.into_iter().collect::<Vec<_>>(), [(FileLeftOut::Link, 1), (FileLeftOut::TooBig, 1)]);
    let pim: Pim = store.load_account_doc("d1", PIM_DOC).unwrap();
    let events: Vec<_> = pim.events.iter().map(|e| &e.event).collect();
    assert_eq!(file_lines(&events[0].extra), ["ATTACH;FMTTYPE=application/pdf;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"agenda.pdf\""]);
    assert!(file_lines(&events[1].extra).is_empty());
    assert_eq!(file_lines(&events[2].extra), ["ATTACH;FMTTYPE=application/pdf;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"proofs.pdf\""]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The files kept on cards and events stay within what Quatrelle reads back: the contacts and
/// calendar written as one `.vcf` and one `.ics` read back whole, every card and event with every
/// file that was kept; those past the cap are counted.
#[test]
fn kept_files_read_back_from_calendar_and_address_book_files() {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-pim-files-back-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut progress = |_| {};
    let mut out = Out::new(&store, "d1", &mut progress);
    // Files of 700,000 bytes: about 0.93 MB each in base64.
    for i in 0..6 {
        out.add_pim(Item {
            class: "IPM.Contact".into(),
            files: vec![data("cv.pdf", "application/pdf", 700_000)],
            extra: Some(crate::pst_pim::Extra { given: format!("Person {i}"), ..Default::default() }),
            ..Default::default()
        });
        let start = 1_790_000_000 + i * 86_400;
        out.add_pim(Item {
            class: "IPM.Appointment".into(),
            subject: format!("Review {i}"),
            files: vec![data("proofs.pdf", "application/pdf", 700_000)],
            extra: Some(crate::pst_pim::Extra { start: Some(start), end: Some(start + 3600), ..Default::default() }),
            ..Default::default()
        });
    }
    let done = out.finish();
    assert_eq!((done.contacts, done.events), (6, 6));
    let lost = done.files_left_out.get(&FileLeftOut::TooBig).copied().unwrap_or(0);
    let pim: Pim = store.load_account_doc("d1", PIM_DOC).unwrap();
    let vcf: String = pim.cards.iter().map(|c| quatrelle_model::vcard::write(&c.card)).collect();
    let ics = quatrelle_model::ical::write(&pim.events.iter().map(|e| e.event.clone()).collect::<Vec<_>>(), None, 0);
    let cards = quatrelle_model::vcard::parse(&vcf);
    let events = quatrelle_model::ical::parse(&ics, 0).events;
    assert_eq!((cards.len(), events.len()), (6, 6), "read back whole: {} and {} bytes", vcf.len(), ics.len());
    let kept = cards.iter().map(|c| file_lines(&c.extra).len()).sum::<usize>() + events.iter().map(|e| file_lines(&e.extra).len()).sum::<usize>();
    assert_eq!((kept, lost), (8, 4), "eight fit in what's kept");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A contact's picture past what a card keeps of one is counted as too big: hidden, it would
/// otherwise go unsaid.
#[test]
fn a_picture_past_the_cap_is_counted() {
    let mut big = b"\x89PNG\r\n\x1a\n".to_vec();
    big.resize(quatrelle_model::vcard::MAX_PHOTO + 1, 0);
    let flags = [UnicodePstProperty::tagged(0x7FFF, V::Boolean(true)), UnicodePstProperty::tagged(0x7FFE, V::Boolean(true))];
    let mut contact = Put::new(&["Contacts"], "IPF.Contact", item("IPM.Contact", "Ana Lima", "", &[]));
    contact.files = vec![file("ContactPicture.png", "image/png", &big, &flags)];
    let b = Brought::of("big-picture", &[contact]);
    assert_eq!(b.done.contacts, 1);
    assert!(b.pim().cards[0].card.photo.is_none());
    assert_eq!(b.done.files_left_out.get(&FileLeftOut::TooBig), Some(&1), "{:?}", b.done);
}
