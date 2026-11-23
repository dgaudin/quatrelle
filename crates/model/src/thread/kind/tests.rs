// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
use super::*;
use MessageKind as K;

#[test]
fn exchange_kinds_by_class_then_icon() {
    let k = |class: &str, icon: i64| K::from_exchange(class, Some(icon));
    for (class, want) in [
        ("REPORT.IPM.Note.NDR", K::NotDelivered),
        ("report.ipm.note.ndr", K::NotDelivered),
        ("REPORT.IPM.Schedule.Meeting.Request.NDR", K::NotDelivered),
        ("REPORT.IPM.Note.DelayedDR", K::Delayed),
        ("REPORT.IPM.Note.Delayed.DR", K::Delayed),
        ("REPORT.IPM.Note.DR", K::Delivered),
        ("REPORT.IPM.Note.Relayed.DR", K::Delivered),
        ("REPORT.IPM.Note.IPNRN", K::ReadReceipt),
        ("REPORT.IPM.Note.IPNNRN", K::NotRead),
        ("REPORT.IPM.Note.Other", K::Plain),
        ("IPM.Schedule.Meeting.Request", K::MeetingRequest),
        ("IPM.Schedule.Meeting.Request.Custom", K::MeetingRequest),
        ("IPM.Schedule.Meeting.Resp.Pos", K::MeetingAccepted),
        ("IPM.Schedule.Meeting.Resp.Tent", K::MeetingTentative),
        ("IPM.Schedule.Meeting.Resp.Neg", K::MeetingDeclined),
        ("IPM.Schedule.Meeting.Canceled", K::MeetingCanceled),
        ("IPM.Note.Rules.OofTemplate.Microsoft", K::AutoReply),
        ("IPM.Outlook.Recall", K::Recall),
        // No document names the last part of a recall report's class: its icon says.
        ("IPM.Recall.Report.Success", K::Plain),
        ("IPM.Recall.Report.Failure", K::Plain),
        ("IPM.Note.SMIME", K::Plain),
        ("IPM.Note.SMIME.MultipartSigned", K::Plain),
        ("IPM.Post", K::Plain),
        ("", K::Plain),
        // Read in groups between the periods.
        ("IPM.Schedule.Meeting.Requested", K::Plain),
        ("REPORTS.IPM.Note.NDR", K::Plain),
    ] {
        assert_eq!(k(class, 0x100), want, "{class}");
    }
    for (icon, want) in [
        (0x11B, K::AutoReply),
        (0x108, K::Delivered),
        (0x109, K::ReadReceipt),
        (0x10A, K::NotDelivered),
        (0x10B, K::NotRead),
        (0x404, K::MeetingRequest),
        (0x409, K::MeetingRequest),
        (0x405, K::MeetingAccepted),
        (0x406, K::MeetingDeclined),
        (0x407, K::MeetingTentative),
        (0x408, K::MeetingCanceled),
        (0x11C, K::Recall),
        (0x10C, K::RecallDone),
        (0x10D, K::RecallFailed),
        (0x100, K::Plain),
        (0x101, K::Plain),
        (0x105, K::Plain),
        (0x106, K::Plain),
        (0x40B, K::Plain),
        (-1, K::Plain),
        (0x7FFF_FFFF, K::Plain),
        (i64::MIN, K::Plain),
    ] {
        assert_eq!(k("IPM.Note", icon), want, "{icon:#x}");
        assert_eq!(k("", icon), want);
    }
    assert_eq!(K::from_exchange("IPM.Note", None), K::Plain);
    // The class wins; the icon speaks only for a note.
    assert_eq!(k("IPM.Schedule.Meeting.Canceled", 0x405), K::MeetingCanceled);
    assert_eq!(k("IPM.Post", 0x10A), K::Plain);
    assert_eq!(k("IPM.Notes", 0x10A), K::Plain);
    // A recall's report, by its icon alone ([MS-OXOMSG] 2.2.1.10), and no other.
    assert_eq!(
        (k("IPM.Recall.Report.Success", 0x10C), k("IPM.Recall.Report", 0x10D), k("IPM.Recall.Report.Failure", 0x10A)),
        (K::RecallDone, K::RecallFailed, K::Plain)
    );
    assert_eq!(K::from_exchange("IPM.Recall.Report", None), K::Plain);
    // 10,000 characters, ASCII or not: cut, never a panic.
    assert_eq!(k(&"IPM.Note.".repeat(1112), 0x10A), K::NotDelivered);
    assert_eq!(k(&"é".repeat(10_000), 0x10A), K::Plain);
    assert_eq!(
        (Smime::from_class("ipm.note.smime.multipartsigned"), Smime::from_class("IPM.Note.SMIME"), Smime::from_class("IPM.Note")),
        (Smime::Signed, Smime::Encrypted, Smime::None)
    );
    assert_eq!((Smime::from_class("REPORT.IPM.Note.SMIME.NDR"), Smime::from_class(&"é".repeat(10_000))), (Smime::None, Smime::None));
}

#[test]
fn one_icon_by_priority() {
    let f = |answered, forwarded, all| {
        let mut f = Flags { answered, forwarded, ..Default::default() };
        f.set_keyword(REPLIED_ALL, all);
        f
    };
    let icon = |f: &Flags, k, s, with| list_icon(f, k, s, with);
    let (none, both) = (f(false, false, false), f(true, true, true));
    for k in [K::NotDelivered, K::Delayed, K::MeetingRequest, K::MeetingCanceled, K::MeetingAccepted, K::MeetingTentative, K::MeetingDeclined] {
        assert_eq!(icon(&both, k, Smime::Encrypted, true), Some(ListIcon::Kind(k)), "{k:?}");
    }
    assert_eq!(icon(&both, K::AutoReply, Smime::Signed, true), Some(ListIcon::RepliedAll));
    assert_eq!(icon(&f(true, true, false), K::ReadReceipt, Smime::None, true), Some(ListIcon::Replied));
    assert_eq!(icon(&f(false, true, false), K::Delivered, Smime::None, true), Some(ListIcon::Forwarded));
    // The keyword alone, without \Answered, is no reply.
    assert_eq!(icon(&f(false, false, true), K::Plain, Smime::None, true), None);
    for k in [K::AutoReply, K::ReadReceipt, K::NotRead, K::Delivered, K::Recall, K::RecallDone, K::RecallFailed] {
        assert_eq!(icon(&none, k, Smime::Encrypted, true), Some(ListIcon::Kind(k)));
    }
    assert_eq!(icon(&none, K::Plain, Smime::Encrypted, true), Some(ListIcon::Encrypted));
    assert_eq!(icon(&none, K::Unknown, Smime::Signed, true), Some(ListIcon::Signed));
    assert_eq!(icon(&none, K::Plain, Smime::Encrypted, false), None, "the two-line view has its own S/MIME mark");
    // Encrypted and replied to: the arrow, and everything is said.
    assert_eq!(icon(&f(true, false, false), K::Plain, Smime::Encrypted, true), Some(ListIcon::Replied));
    assert_eq!(
        list_said(&f(false, true, false), K::MeetingRequest, Smime::Signed),
        vec![ListIcon::Kind(K::MeetingRequest), ListIcon::Forwarded, ListIcon::Signed]
    );
    assert_eq!(list_said(&both, K::Plain, Smime::None), vec![ListIcon::RepliedAll, ListIcon::Forwarded]);
    assert_eq!(list_said(&none, K::Unknown, Smime::None), vec![]);
    let m = Summary { flags: f(true, false, false), kind: K::NotDelivered, ..Default::default() };
    assert_eq!((m.list_icon(false), Summary::default().list_icon(true)), (Some(ListIcon::Kind(K::NotDelivered)), None));
    assert!(ListIcon::Kind(K::NotDelivered).urgent() && ListIcon::Kind(K::MeetingCanceled).urgent());
    assert!(!ListIcon::Kind(K::Delayed).urgent() && !ListIcon::Replied.urgent() && !ListIcon::Encrypted.urgent());
}

/// The probe of a page of the cache holding a kind this version doesn't know.
#[test]
fn a_page_with_any_kind_value_still_reads() {
    let read = |kind: &str| {
        serde_json::from_str::<Summary>(&format!(r#"{{"key":{{"account":"a","folder":"INBOX","validity":1,"uid":7}},"subject":"s"{kind}}}"#))
    };
    for kind in ["", r#","kind":"autreChose""#, r#","kind":3"#, r#","kind":null"#, r#","kind":{"a":[1]}"#, r#","kind":"""#] {
        let m = read(kind).unwrap_or_else(|e| panic!("{kind}: {e}"));
        assert_eq!((m.kind, m.key.uid, m.subject.as_str()), (K::Unknown, 7, "s"), "{kind}");
    }
    assert_eq!(read(r#","kind":"notDelivered""#).unwrap().kind, K::NotDelivered);
    // Not said yet: nothing is written (1.5.0 and a cache from before stay byte for byte).
    assert!(!serde_json::to_string(&Summary::default()).unwrap().contains("kind"));
}

#[test]
fn who_says_what_a_message_is() {
    assert_eq!(K::Unknown.told_by_server(K::Plain), K::Unknown);
    assert_eq!(K::AutoReply.told_by_server(K::Plain), K::AutoReply);
    assert_eq!(K::Plain.told_by_server(K::MeetingRequest), K::MeetingRequest);
    assert_eq!(K::NotRead.told_by_server(K::Unknown), K::NotRead);
    assert_eq!((K::Unknown.settled_by_source(K::Plain), K::Plain.settled_by_source(K::NotRead)), (K::Plain, K::NotRead));
    assert_eq!((K::AutoReply.settled_by_source(K::NotDelivered), K::AutoReply.settled_by_source(K::Plain)), (K::NotDelivered, K::AutoReply));
    assert_eq!(K::MeetingRequest.settled_by_source(K::Plain), K::MeetingRequest);
    assert_eq!(K::Plain.settled_by_source(K::Unknown), K::Plain, "a source that says nothing undoes nothing");
    let mut f = Flags::default();
    Flag::Answered.apply(&mut f, true, true);
    assert!(f.replied_all());
    Flag::Answered.apply(&mut f, true, false);
    assert!(f.replied_all(), "IMAP keeps each mark as a fact of its own");
    Flag::Answered.apply(&mut f, false, false);
    assert!(!f.answered && !f.has(REPLIED_ALL));
    Flag::Forwarded.apply(&mut f, true, true);
    assert!(f.forwarded && !f.has(REPLIED_ALL), "only a reply is to all");
    assert!(Flags::from_imap(["\\Answered", "$repliedall"]).replied_all());
}
