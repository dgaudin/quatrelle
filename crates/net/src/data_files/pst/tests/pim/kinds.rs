// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Kinds of item a data file holds beyond plain mail: meeting requests and their answers, posts
//! and documents, contact groups and journal entries; and the categories, Show As, privacy and
//! reminder of a calendar item.

use outlook_pst::ltp::prop_context::GuidValue;
use outlook_pst::messaging::named_prop::PS_PUBLIC_STRINGS;
use outlook_pst::{UnicodePstRecipient, UnicodePstRecipientType, UnicodePstValue as V};
use quatrelle_model::ical::{PrivateKind, ShowAs};

use super::*;

/// 2023-11-15T17:46:40Z, an hour long.
const START: i64 = 1_700_070_400;

/// A meeting's time, as an appointment or a meeting request keeps it.
fn times() -> [UnicodePstProperty<'static>; 2] {
    [named(APPOINTMENT, 0x820D, V::Time(filetime(START))), named(APPOINTMENT, 0x820E, V::Time(filetime(START + 3600)))]
}

/// The calendar a message carries, when it carries one.
fn calendar_of(raw: &[u8]) -> Option<String> {
    let p = mime::parse(raw).unwrap();
    let ics = p.attachments.iter().find(|a| a.mime.starts_with("text/calendar"))?;
    Some(String::from_utf8(mime::attachment_data(raw, ics.index).unwrap()).unwrap())
}

/// A meeting request, cancellation and answer come in with their calendar again, with the method
/// that lets them be answered or applied; another meeting notice carries none.
#[test]
fn meeting_items_come_in_with_their_calendar() {
    let ana = [UnicodePstRecipient { name: "Ana", email: "ana@posteo.example", recipient_type: UnicodePstRecipientType::To, ..Default::default() }];
    let [start, end] = times();
    let props = [start, end, named(MEETING, 0x0003, V::Binary(&[0x04, 0, 0, 0, 0xAB])), named(APPOINTMENT, 0x8201, V::Integer32(2))];
    let classes = [
        "IPM.Schedule.Meeting.Request",
        "IPM.Schedule.Meeting.Canceled",
        "IPM.Schedule.Meeting.Resp.Tent",
        "IPM.Schedule.Meeting.Notification.Forward",
    ];
    let puts: Vec<Put<'_>> =
        classes.iter().map(|c| Put::new(&["Inbox"], "IPF.Note", UnicodePstMessage { recipients: &ana, ..item(c, c, "Room 4", &props) })).collect();
    let b = Brought::of("meetings", &puts);
    assert_eq!(b.done.messages, 4, "{:?}", b.done);
    let by_subject = |s: &str| b.messages("INBOX").into_iter().find(|(m, _)| m.subject == s).map(|(_, raw)| raw).unwrap();
    let ics = calendar_of(&by_subject(classes[0])).expect("a request carries its calendar");
    for line in ["METHOD:REQUEST", "UID:04000000AB", "SEQUENCE:2", "ORGANIZER", "riley@studio.example", "ana@posteo.example", "SUMMARY:"] {
        assert!(ics.contains(line), "{line}: {ics}");
    }
    let ics = calendar_of(&by_subject(classes[1])).expect("a cancellation carries its calendar");
    assert!(ics.contains("METHOD:CANCEL") && ics.contains("STATUS:CANCELLED"), "{ics}");
    let e = quatrelle_model::ical::parse(&calendar_of(&by_subject(classes[2])).expect("an answer carries its calendar"), 0).events.remove(0);
    assert_eq!(e.organizer.as_ref().map(|o| o.email.as_str()), Some("ana@posteo.example"));
    assert_eq!(e.attendees.iter().map(|a| (a.email.as_str(), a.partstat.as_str())).collect::<Vec<_>>(), [("riley@studio.example", "TENTATIVE")]);
    assert!(calendar_of(&by_subject(classes[3])).is_none(), "a forward's notice has none");
}

/// A post (an RSS feed's too), a document, a recall and its report, a sharing invitation and a
/// task request in a mail folder come in as mail, with their subject, text and sender; a contact
/// there goes to the address book as one in a contacts folder does; another kind is counted as
/// left out, not dropped unsaid.
#[test]
fn posts_and_documents_are_mail_and_other_kinds_are_counted() {
    let mail = [
        "IPM.Post",
        "IPM.Post.Rss",
        "IPM.Document.Word.Document.12",
        "IPM.Note",
        "IPM.Outlook.Recall",
        "IPM.Recall.Report.Success",
        "IPM.Sharing",
        "IPM.TaskRequest",
    ];
    let texts: Vec<String> = mail.iter().map(|c| format!("Text of {c}")).collect();
    let mut puts: Vec<Put<'_>> = mail.iter().zip(&texts).map(|(c, t)| Put::new(&["Inbox"], "IPF.Note", item(c, c, t, &[]))).collect();
    let ana = [named(ADDRESS, 0x8083, V::Unicode("ana@posteo.example"))];
    puts.push(Put::new(&["Inbox"], "IPF.Note", item("IPM.Contact", "Ana Lima", "", &ana)));
    puts.push(Put::new(&["Inbox"], "IPF.Note", item("IPM.Configuration.Junk", "x", "", &[])));
    let b = Brought::of("kinds", &puts);
    assert_eq!((b.done.messages, b.done.contacts, b.done.skipped), (mail.len(), 1, 1), "{:?}", b.done);
    let list = b.messages("INBOX");
    for class in mail {
        let (m, raw) = list.iter().find(|(m, _)| m.subject == class).expect(class);
        assert_eq!(m.from.email, "riley@studio.example");
        assert!(mime::parse(raw).unwrap().text.contains(&format!("Text of {class}")), "{class}");
    }
    assert!(b.pim().cards.iter().any(|c| c.card.emails.iter().any(|e| e.value == "ana@posteo.example")));
}

/// A sharing invitation says what it shares, who shares it and an internet calendar's address,
/// from its sharing properties, before its own text.
#[test]
fn a_sharing_invitation_says_what_it_shares() {
    let sharing = GuidValue::new(0x0006_2040, 0, 0, OLE);
    let props = [
        named(sharing, 0x8A05, V::Unicode("Studio bookings")),
        named(sharing, 0x8A1D, V::Unicode("IPF.Appointment")),
        named(sharing, 0x8A07, V::Unicode("Ana")),
        named(sharing, 0x8A08, V::Unicode("ana@posteo.example")),
        named(sharing, 0x8A04, V::Unicode("https://cal.example/ana.ics")),
    ];
    let b = Brought::of("sharing", &[Put::new(&["Inbox"], "IPF.Note", item("IPM.Sharing", "Ana shared a calendar", "Take a look.", &props))]);
    assert_eq!(b.done.messages, 1, "{:?}", b.done);
    let (_, raw) = b.messages("INBOX").remove(0);
    let text = mime::parse(&raw).unwrap().text;
    assert!(
        text.contains("Shared calendar: Studio bookings\nShared by Ana <ana@posteo.example>\nAddress: https://cal.example/ana.ics\n\nTake a look."),
        "{text}"
    );
}

/// The wrapped entry id of a contact group's member that is a one-off address (MS-OXCDATA 2.2.5.3.3).
fn wrapped(one_off: &[u8]) -> Vec<u8> {
    let provider = [0xC0, 0x91, 0xAD, 0xD3, 0x51, 0x9D, 0xCF, 0x11, 0xA4, 0xA9, 0x00, 0xAA, 0x00, 0x47, 0xFA, 0xA4];
    [&[0, 0, 0, 0][..], &provider, &[0x00], one_off].concat()
}

/// A contact group comes in as a group card with its members; a journal entry as a note with
/// its subject, its time and its text.
#[test]
fn contact_groups_and_journal_entries_come_in() {
    let ana = super::super::one_off("Ana", "SMTP", "ana@posteo.example", true);
    let ben = wrapped(&super::super::one_off("", "SMTP", "ben@studio.example", true));
    let one_offs = [&ana[..]];
    let members = [&ben[..]];
    let group = [named(ADDRESS, 0x8054, V::MultipleBinary(&one_offs)), named(ADDRESS, 0x8055, V::MultipleBinary(&members))];
    let entry = [named(LOG, 0x8706, V::Time(filetime(1_700_000_000))), named(LOG, 0x8708, V::Time(filetime(1_700_000_000 + 45 * 60)))];
    let b = Brought::of(
        "groups",
        &[
            Put::new(&["Contacts"], "IPF.Contact", item("IPM.DistList", "Crew", "", &group)),
            Put::new(&["Journal"], "IPF.Journal", item("IPM.Activity", "Call with the printer", "Proofs on Monday.\r\nPaper: matte.", &entry)),
        ],
    );
    assert_eq!((b.done.contacts, b.done.notes), (1, 1), "{:?}", b.done);
    let pim = b.pim();
    let card = &pim.cards[0].card;
    assert_eq!(card.name, "Crew");
    assert_eq!(card.extra, ["KIND:group", "MEMBER:mailto:ana@posteo.example", "MEMBER:mailto:ben@studio.example"]);
    assert_eq!(card.note, "Ana <ana@posteo.example>\nben@studio.example");
    let text = quatrelle_model::vcard::write(card);
    assert!(text.contains("KIND:group\r\n") && text.contains("MEMBER:mailto:ben@studio.example\r\n"), "{text}");
    assert_eq!(pim.notes[0].note.text, "Call with the printer\n2023-11-14 22:13–22:58 UTC\nProofs on Monday.\nPaper: matte.");
}

/// An appointment keeps its categories, how it shows its time, its privacy, its reminder and its
/// version.
#[test]
fn an_event_keeps_its_categories_show_as_privacy_and_reminder() {
    let names = ["Print", "Studio"];
    let event = |busy: i32, sensitivity: i32, reminder: Option<i32>| {
        let [start, end] = times();
        let mut props = vec![
            start,
            end,
            named(APPOINTMENT, 0x8205, V::Integer32(busy)),
            UnicodePstProperty::tagged(0x0036, V::Integer32(sensitivity)),
            named(APPOINTMENT, 0x8201, V::Integer32(3)),
            UnicodePstProperty::named(PS_PUBLIC_STRINGS, UnicodePstName::String("Keywords"), V::MultipleUnicode(&names)),
        ];
        if let Some(m) = reminder {
            props.extend([named(COMMON, 0x8503, V::Boolean(true)), named(COMMON, 0x8501, V::Integer32(m))]);
        }
        props
    };
    let (a, b, c) = (event(0, 2, Some(30)), event(3, 3, None), event(2, 1, Some(-5)));
    let b = Brought::of(
        "event",
        &[
            Put::new(&["Calendar"], "IPF.Appointment", item("IPM.Appointment", "Press check", "", &a)),
            Put::new(&["Calendar"], "IPF.Appointment", item("IPM.Appointment", "Away", "", &b)),
            Put::new(&["Calendar"], "IPF.Appointment", item("IPM.Appointment", "Mine", "", &c)),
        ],
    );
    let pim = b.pim();
    let e = |s: &str| pim.events.iter().map(|e| &e.event).find(|e| e.summary == s).unwrap().clone();
    let press = e("Press check");
    assert_eq!((press.categories.as_slice(), press.sequence), (&["Print".to_string(), "Studio".to_string()][..], 3));
    assert_eq!((press.show_as, press.private, press.private_kind, press.reminder), (ShowAs::Free, true, PrivateKind::Private, Some(30)));
    let away = e("Away");
    assert_eq!((away.show_as, away.private, away.private_kind, away.reminder), (ShowAs::OutOfOffice, true, PrivateKind::Confidential, None));
    let mine = e("Mine");
    assert_eq!((mine.show_as, mine.private, mine.private_kind), (ShowAs::Busy, true, PrivateKind::Personal));
    assert_eq!(mine.reminder, Some(15), "a reminder that makes no sense is Outlook's own");
    let written = quatrelle_model::ical::write(&[press], None, 0);
    assert!(written.contains("BEGIN:VALARM") && written.contains("CATEGORIES:Print,Studio"), "{written}");
}
