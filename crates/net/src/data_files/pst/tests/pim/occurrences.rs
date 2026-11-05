// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A repeating appointment's changed occurrences: each comes in as an event of its own that
//! replaces its occurrence (`RECURRENCE-ID`), and the fork's calendar shows the series with them.

use outlook_pst::{UnicodePstAttachmentContent, UnicodePstEmbeddedMessage, UnicodePstValue as V};
use quatrelle_model::dates::days_from_civil;
use quatrelle_model::ical;

use super::*;

/// The class of an appointment's changed occurrence, attached to it (MS-OXOCAL 2.2.10.1.1).
const EXCEPTION: &str = "IPM.OLE.CLASS.{00061055-0000-0000-C000-000000000046}";

/// 2026-10-`d` at `h`:00 UTC.
fn at(d: u32, h: i64) -> i64 {
    days_from_civil(2026, 10, d) * 86_400 + h * 3600
}

/// A changed occurrence's item as Outlook attaches it to its appointment: hidden, marked as an
/// exception (afException), with its own times, the occurrence it replaces and its own text.
fn exception<'a>(props: &'a [UnicodePstProperty<'a>], body: &'a str, flags: &'a [UnicodePstProperty<'a>]) -> UnicodePstAttachment<'a> {
    UnicodePstAttachment {
        filename: "Review",
        content: UnicodePstAttachmentContent::Message(Box::new(UnicodePstEmbeddedMessage {
            message: item(EXCEPTION, "Review", body, props),
            attachments: Vec::new(),
        })),
        properties: flags,
        ..Default::default()
    }
}

/// A repeating appointment comes in with its changed occurrences, each an event replacing its
/// occurrence with its own text; it counts as one calendar item; the calendar shows the series
/// with the moved one moved, the changed one changed and the deleted one gone, and so does the
/// calendar written as a file and read back.
#[test]
fn changed_occurrences_come_in_and_show_in_the_calendar() {
    let (pattern, start) = crate::pst_pim::tests::series(0x3009);
    let series = [
        named(APPOINTMENT, 0x820D, V::Time(filetime(start))),
        named(APPOINTMENT, 0x820E, V::Time(filetime(start + 3600))),
        named(APPOINTMENT, 0x8216, V::Binary(&pattern)),
        named(APPOINTMENT, 0x8223, V::Boolean(true)),
    ];
    // The quiet one's item, which says the hour exactly (summer time ended: 08:00Z).
    let quiet = [
        named(APPOINTMENT, 0x820D, V::Time(filetime(at(19, 8)))),
        named(APPOINTMENT, 0x820E, V::Time(filetime(at(19, 9)))),
        named(APPOINTMENT, 0x8228, V::Time(filetime(at(19, 8)))),
    ];
    let flags = [UnicodePstProperty::tagged(0x7FFE, V::Boolean(true)), UnicodePstProperty::tagged(0x7FFD, V::Integer32(0x2))];
    let mut put = Put::new(&["Calendar"], "IPF.Appointment", item("IPM.Appointment", "Review", "Weekly.", &series));
    put.files.push(exception(&quiet, "Bring the proofs.", &flags));
    let b = Brought::of("changed", &[put]);
    assert_eq!((b.done.events, b.done.skipped), (1, 0), "{:?}", b.done);
    let pim = b.pim();
    let events: Vec<ical::CalEvent> = pim.events.iter().map(|e| e.event.clone()).collect();
    assert_eq!(events.len(), 3, "{events:#?}");
    let master = &events[0];
    assert!(master.rrule.is_some() && master.recurrence_id.is_none());
    assert_eq!(master.exdates, [at(7, 7)]);
    let (moved, changed) = (&events[1], &events[2]);
    assert_eq!((moved.uid.as_str(), moved.recurrence_id, moved.summary.as_str()), (master.uid.as_str(), Some(at(12, 7)), "Review (moved) \u{e9}"));
    assert_eq!((moved.start, moved.end, moved.location.as_str()), (at(13, 8), at(13, 9), "Room 2"));
    assert_eq!((changed.recurrence_id, changed.start, changed.description.as_str()), (Some(at(19, 7)), at(19, 8), "Bring the proofs."));
    assert_eq!(changed.show_as, ical::ShowAs::Free);
    // In the calendar: the moved one on the 13th, the changed one an hour later, the deleted one
    // gone.
    let shown = |events: &[ical::CalEvent]| -> Vec<i64> { ical::occurrences(events, at(5, 0), at(22, 0)).iter().map(|o| o.start).collect() };
    let expected = [at(5, 7), at(13, 8), at(14, 7), at(19, 8), at(21, 7)];
    assert_eq!(shown(&events), expected);
    // Written as a calendar file, the series and its changed occurrences read back together.
    let back = ical::parse(&ical::write(&events, None, 0), 0).events;
    assert_eq!(back.iter().map(|e| e.recurrence_id).collect::<Vec<_>>(), [None, Some(at(12, 7)), Some(at(19, 7))]);
    assert_eq!(shown(&back), expected);
}

/// An appointment attached to a mail keeps its changed occurrences hidden: the mail carries the
/// appointment, not its calendar bookkeeping as messages inside it.
#[test]
fn an_attached_appointment_keeps_its_changed_occurrences_hidden() {
    let (pattern, start) = crate::pst_pim::tests::series(0x3009);
    let series = [
        named(APPOINTMENT, 0x820D, V::Time(filetime(start))),
        named(APPOINTMENT, 0x820E, V::Time(filetime(start + 3600))),
        named(APPOINTMENT, 0x8216, V::Binary(&pattern)),
    ];
    let quiet = [named(APPOINTMENT, 0x820D, V::Time(filetime(at(19, 8)))), named(APPOINTMENT, 0x8228, V::Time(filetime(at(19, 8))))];
    let hidden = [UnicodePstProperty::tagged(0x7FFE, V::Boolean(true))];
    let appointment = UnicodePstAttachment {
        filename: "Review",
        content: UnicodePstAttachmentContent::Message(Box::new(UnicodePstEmbeddedMessage {
            message: item("IPM.Appointment", "Review", "Weekly.", &series),
            attachments: vec![exception(&quiet, "Bring the proofs.", &hidden)],
        })),
        ..Default::default()
    };
    let mut put = Put::new(&["Inbox"], "IPF.Note", item("IPM.Note", "Our weekly review", "See the meeting.", &[]));
    put.files.push(appointment);
    let b = Brought::of("attached-appointment", &[put]);
    assert_eq!((b.done.messages, b.done.events), (1, 0), "{:?}", b.done);
    let (_, raw) = b.messages("INBOX").remove(0);
    let p = mime::parse(&raw).unwrap();
    assert_eq!(p.attachments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Review.eml"]);
    let inner = mime::attachment_data(&raw, p.attachments[0].index).unwrap();
    let inner = mime::parse(&inner).unwrap();
    assert!(inner.attachments.is_empty(), "{:?}", inner.attachments.iter().map(|a| &a.name).collect::<Vec<_>>());
}
