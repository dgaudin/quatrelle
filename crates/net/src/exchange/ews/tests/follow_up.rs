// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Tests of the EWS transport: a follow-up marked complete, as Outlook keeps it on Exchange
//! (`PidTagFlagStatus` 1, and `PidTagFlagCompleteTime` the time it was done), both ways.

use quatrelle_model::FOLLOW_UP_COMPLETE;

use super::*;
use crate::fakes::fake_ews::{self, Data};

/// Marked complete in Outlook, a message comes in complete, on the day Exchange says; marked
/// complete here, Exchange gets status 1 and the time; flagged again, status 2 without the
/// time; cleared, neither. Each change made elsewhere comes in at the next sync.
#[test]
fn a_follow_up_marked_complete_goes_both_ways_with_its_day() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let done = d.deliver(&inbox, b"From: a@x.example\r\nSubject: done\r\n\r\nx\r\n", false, "2024-05-01T09:00:00Z");
    let flagged = d.deliver(&inbox, b"From: a@x.example\r\nSubject: flagged\r\n\r\nx\r\n", false, "2024-05-02T09:00:00Z");
    for i in d.items.iter_mut() {
        if i.id == done {
            i.complete = true;
            i.complete_time = Some("2026-10-30T09:12:00Z".into());
        }
        i.flagged = i.id == flagged;
    }
    let server = fake_ews::serve(d);
    let dir = std::env::temp_dir().join(format!("quatrelle-ews-follow-up-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let user = format!("{}\\{}", fake_ews::DOMAIN, fake_ews::USER);
    let c = Client::connect(&server.url, &user, fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    list_folders(&c, &mut st, "a1").unwrap();
    let opts = SyncOptions { prefetch: 0, ..Default::default() };
    let msg = |subject: &str| store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == subject).cloned().unwrap();
    let on_server = |id: &str| {
        let d = server.data.lock().unwrap();
        d.items.iter().find(|i| i.id == id).map(|i| (i.flagged, i.complete, i.complete_time.clone())).unwrap()
    };
    let at = |text: &str| jmap::utc_date(text).unwrap();

    // Complete in Outlook: complete here, on its day.
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let m = msg("done");
    assert!(m.flags.follow_up_complete() && m.flags.completed == at("2026-10-30T09:12:00Z"), "{:?}", m.flags);
    assert!(msg("flagged").flags.flagged && !msg("flagged").flags.has(FOLLOW_UP_COMPLETE));

    // Mark Complete here (the flag off, then the keyword on): status 1 and the time.
    let keys = vec![msg("flagged").key];
    let flag = |on| Op::SetFlag { keys: keys.clone(), flag: Flag::Flagged, on };
    let complete = |on| Op::SetKeyword { keys: keys.clone(), keyword: FOLLOW_UP_COMPLETE.into(), on, name: None };
    for op in [flag(false), complete(true)] {
        apply_remote(&c, &st, &op).unwrap();
    }
    let (f, cmp, time) = on_server(&flagged);
    assert!(!f && cmp && time.as_deref().and_then(jmap::utc_date).is_some_and(|t| t > at("2026-01-01T00:00:00Z")), "{time:?}");
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    assert!(msg("flagged").flags.follow_up_complete() && msg("flagged").flags.completed > 0, "{:?}", msg("flagged").flags);

    // Its Undo (the keyword off, then the flag on): flagged, the time gone.
    for op in [complete(false), flag(true)] {
        apply_remote(&c, &st, &op).unwrap();
    }
    assert_eq!(on_server(&flagged), (true, false, None));
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let m = msg("flagged");
    assert!(m.flags.flagged && !m.flags.has(FOLLOW_UP_COMPLETE) && m.flags.completed == 0, "{:?}", m.flags);

    // Cleared: neither.
    apply_remote(&c, &st, &flag(false)).unwrap();
    assert_eq!(on_server(&flagged), (false, false, None));

    // In Outlook: the one done flagged again, the other marked complete; the next sync (only
    // what changed) says so.
    {
        let mut d = server.data.lock().unwrap();
        for i in d.items.iter_mut() {
            if i.id == done {
                (i.flagged, i.complete, i.complete_time) = (true, false, None);
            } else if i.id == flagged {
                (i.complete, i.complete_time) = (true, Some("2026-11-02T16:30:00Z".into()));
            }
        }
    }
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let m = msg("done");
    assert!(m.flags.flagged && !m.flags.has(FOLLOW_UP_COMPLETE) && m.flags.completed == 0, "{:?}", m.flags);
    let m = msg("flagged");
    assert!(m.flags.follow_up_complete() && m.flags.completed == at("2026-11-02T16:30:00Z"), "{:?}", m.flags);
    let _ = std::fs::remove_dir_all(&dir);
}

/// What the server says of a follow-up is read with care: a status other than 1 or 2, a time
/// that isn't one, or a time without the status.
#[test]
fn a_follow_up_read_from_exchange_is_read_with_care() {
    let item = |props: &str| {
        let doc = parse(&format!("<t:Message><t:IsRead>true</t:IsRead>{props}</t:Message>"));
        doc.find("Message").map(flags).unwrap_or_default()
    };
    let prop = |tag: &str, kind: &str, v: &str| {
        format!("<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"{tag}\" PropertyType=\"{kind}\"/><t:Value>{v}</t:Value></t:ExtendedProperty>")
    };
    let status = |v: &str| prop("0x1090", "Integer", v);
    let time = |v: &str| prop("0x1091", "SystemTime", v);
    let f = item(&format!("{}{}", status("1"), time("2026-10-30T09:12:00Z")));
    assert!(f.follow_up_complete() && f.completed > 0, "{f:?}");
    let f = item(&format!("{}{}", status("1"), time("not a time")));
    assert!(f.follow_up_complete() && f.completed == 0, "{f:?}");
    let f = item(&format!("{}{}", status("1"), time("1601-01-01T00:00:00Z")));
    assert!(f.follow_up_complete() && f.completed == 0, "a time before 1970 isn't kept: {f:?}");
    let f = item(&format!("{}{}", status("2"), time("2026-10-30T09:12:00Z")));
    assert!(f.flagged && !f.has(FOLLOW_UP_COMPLETE) && f.completed == 0, "{f:?}");
    let f = item(&time("2026-10-30T09:12:00Z"));
    assert!(!f.flagged && !f.has(FOLLOW_UP_COMPLETE) && f.completed == 0, "{f:?}");
    let f = item(&status("7"));
    assert!(!f.flagged && !f.has(FOLLOW_UP_COMPLETE), "{f:?}");
}
