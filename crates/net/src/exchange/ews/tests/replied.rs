// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Tests of the EWS transport: a message replied to or forwarded, as Outlook keeps it on
//! Exchange (`PidTagLastVerbExecuted` 102, 103 or 104, `PidTagLastVerbExecutionTime` when, and
//! `PidTagIconIndex` the icon its list draws), both ways.

use super::*;
use crate::fakes::fake_ews::{self, Data, Verb};

/// A last verb as Outlook writes it.
fn verb(last: &str, time: &str) -> Verb {
    Verb { last: Some(last.into()), time: Some(time.into()), icon: None }
}

/// Replied to or forwarded in Outlook, a message comes in with its arrow and the time; replied
/// to or forwarded here, Exchange gets the verb, the time and the icon, as Outlook writes them;
/// the next sync keeps the arrow, the last thing done showing (forwarded after a reply shows
/// forwarded); a reply made in Outlook afterwards comes in at the next sync.
#[test]
fn a_reply_or_a_forward_goes_both_ways_with_its_time() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let mail = |s: &str| format!("From: a@x.example\r\nSubject: {s}\r\n\r\nx\r\n").into_bytes();
    let replied = d.deliver(&inbox, &mail("replied"), true, "2024-05-01T09:00:00Z");
    let all = d.deliver(&inbox, &mail("replied to all"), true, "2024-05-01T10:00:00Z");
    let forwarded = d.deliver(&inbox, &mail("forwarded"), true, "2024-05-02T09:00:00Z");
    let plain = d.deliver(&inbox, &mail("plain"), false, "2024-05-03T09:00:00Z");
    for i in d.items.iter_mut() {
        if i.id == replied {
            i.verb = verb("102", "2026-10-30T09:12:00Z");
        } else if i.id == all {
            i.verb = verb("103", "2026-10-30T09:13:00Z");
        } else if i.id == forwarded {
            i.verb = verb("104", "2026-10-31T16:30:00Z");
        }
    }
    let server = fake_ews::serve(d);
    let dir = std::env::temp_dir().join(format!("quatrelle-ews-replied-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let user = format!("{}\\{}", fake_ews::DOMAIN, fake_ews::USER);
    let c = Client::connect(&server.url, &user, fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    list_folders(&c, &mut st, "a1").unwrap();
    let opts = SyncOptions { prefetch: 0, ..Default::default() };
    let msg = |subject: &str| store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == subject).cloned().unwrap();
    let on_server = |id: &str| server.data.lock().unwrap().items.iter().find(|i| i.id == id).map(|i| i.verb.clone()).unwrap();
    let at = |text: &str| jmap::utc_date(text).unwrap();
    let marks = |subject: &str| {
        let f = msg(subject).flags;
        (f.answered, f.forwarded, f.acted)
    };

    // Done in Outlook: the arrow here, with its time.
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!(marks("replied"), (true, false, at("2026-10-30T09:12:00Z")));
    assert_eq!(marks("replied to all"), (true, false, at("2026-10-30T09:13:00Z")));
    assert_eq!(marks("forwarded"), (false, true, at("2026-10-31T16:30:00Z")));
    assert_eq!(marks("plain"), (false, false, 0));

    // Replied to here: the verb, the time and the icon, as Outlook writes them.
    let keys = vec![msg("plain").key];
    let set = |flag, on| Op::SetFlag { keys: keys.clone(), flag, on };
    apply_remote(&c, &st, &set(Flag::Answered, true)).unwrap();
    let v = on_server(&plain);
    assert_eq!((v.last.as_deref(), v.icon.as_deref()), (Some("102"), Some("261")), "{v:?}");
    assert!(v.time.as_deref().and_then(jmap::utc_date).is_some_and(|t| t > at("2026-01-01T00:00:00Z")), "{v:?}");
    // The next sync (only what changed) keeps the arrow, with the server's time.
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let (answered, fwd, acted) = marks("plain");
    assert!(answered && !fwd && acted > at("2026-01-01T00:00:00Z"), "{:?}", msg("plain").flags);

    // Then forwarded here (the engine takes the reply's arrow off first, which Exchange has
    // no use for): the forward is the last thing done.
    for op in [set(Flag::Answered, false), set(Flag::Forwarded, true)] {
        apply_remote(&c, &st, &op).unwrap();
    }
    let v = on_server(&plain);
    assert_eq!((v.last.as_deref(), v.icon.as_deref()), (Some("104"), Some("262")), "{v:?}");
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    assert!(matches!(marks("plain"), (false, true, t) if t > 0), "{:?}", msg("plain").flags);

    // In Outlook: the one forwarded is replied to; the next sync says so.
    {
        let mut d = server.data.lock().unwrap();
        if let Some(i) = d.items.iter_mut().find(|i| i.id == forwarded) {
            i.verb = verb("102", "2026-11-02T08:00:00Z");
        }
    }
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!(marks("forwarded"), (true, false, at("2026-11-02T08:00:00Z")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Replied to here before Exchange was told (or by a version that never told it): the arrow kept
/// here isn't taken off by a sync that says nothing of it, a change made in Outlook or a folder
/// read again in full.
#[test]
fn an_arrow_kept_here_survives_the_syncs_that_say_nothing_of_it() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let id = d.deliver(&inbox, b"From: a@x.example\r\nSubject: kept\r\n\r\nx\r\n", false, "2024-05-01T09:00:00Z");
    let server = fake_ews::serve(d);
    let store = crate::fake::tests_store("ews-replied-kept");
    let user = format!("{}\\{}", fake_ews::DOMAIN, fake_ews::USER);
    let c = Client::connect(&server.url, &user, fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    list_folders(&c, &mut st, "a1").unwrap();
    let opts = SyncOptions { prefetch: 0, ..Default::default() };
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let key = store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone();
    store.apply_op("a1", &Op::SetFlag { keys: vec![key], flag: Flag::Answered, on: true }).unwrap();
    let answered = || store.load_folder("a1", "INBOX").unwrap().messages[0].flags.answered;
    // Read in Outlook: the change comes in, the arrow stays.
    if let Some(i) = server.data.lock().unwrap().items.iter_mut().find(|i| i.id == id) {
        i.read = true;
        i.flagged = true;
    }
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    assert!(answered(), "after the changes");
    // The folder read again in full (its sync state forgotten).
    st.marks.clear();
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    assert!(answered(), "after a full read");
    let _ = std::fs::remove_dir_all(store.root());
}

/// What the server says of the last thing done is read with care: a verb other than a reply or
/// a forward (a vote), one that isn't a number, a time that isn't one, before 1970 or MAPI's
/// "none" (4501), a time without its verb.
#[test]
fn a_reply_read_from_exchange_is_read_with_care() {
    let item = |props: &str| {
        let doc = parse(&format!("<t:Message><t:IsRead>true</t:IsRead>{props}</t:Message>"));
        doc.find("Message").map(flags).unwrap_or_default()
    };
    let prop = |tag: &str, kind: &str, v: &str| {
        format!("<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"{tag}\" PropertyType=\"{kind}\"/><t:Value>{v}</t:Value></t:ExtendedProperty>")
    };
    let last = |v: &str| prop("0x1081", "Integer", v);
    let time = |v: &str| prop("0x1082", "SystemTime", v);
    let marks = |props: &str| {
        let f = item(props);
        (f.answered, f.forwarded, f.acted)
    };
    let when = jmap::utc_date("2026-10-30T09:12:00Z").unwrap();
    assert_eq!(marks(&format!("{}{}", last("102"), time("2026-10-30T09:12:00Z"))), (true, false, when));
    assert_eq!(marks(&format!("{}{}", last("103"), time("2026-10-30T09:12:00Z"))), (true, false, when));
    assert_eq!(marks(&format!("{}{}", last("104"), time("2026-10-30T09:12:00Z"))), (false, true, when));
    // Exchange's own spelling of the tag.
    assert_eq!(marks(&format!("{}{}", prop("0x1081", "Integer", " 104 "), prop("4226", "SystemTime", "2026-10-30T09:12:00Z"))).1, true);
    assert_eq!(marks(&format!("{}{}", last("102"), time("not a time"))), (true, false, 0));
    assert_eq!(marks(&format!("{}{}", last("102"), time("1601-01-01T00:00:00Z"))), (true, false, 0), "before 1970");
    assert_eq!(marks(&format!("{}{}", last("104"), time("4501-01-01T00:00:00Z"))), (false, true, 0), "MAPI's none");
    for other in ["0", "1", "7", "105", "261", "-102", "102.0", "abc", "99999999999999999999999", ""] {
        assert_eq!(marks(&format!("{}{}", last(other), time("2026-10-30T09:12:00Z"))), (false, false, 0), "{other:?}");
    }
    assert_eq!(marks(&time("2026-10-30T09:12:00Z")), (false, false, 0), "a time without its verb");
}
