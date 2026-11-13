// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Tests of the jobs, events and changes: what's logged of them, what's never written.

use super::*;

#[test]
fn a_change_is_summed_up_without_a_subject_or_an_address() {
    let key = |folder: &str, uid: u32| MessageKey { account: "me@studio.example".into(), folder: folder.into(), validity: 1, uid };
    let mv = Op::Move { keys: vec![key("INBOX", 1), key("INBOX", 2), key("Sent", 3)], to: "Archive".into() };
    assert_eq!(mv.summary(), "Move of 3 messages in INBOX, Sent");
    let flag = Op::SetFlag { keys: vec![key("INBOX", 1)], flag: Flag::Seen, on: true };
    assert_eq!(flag.summary(), "SetFlag of 1 message in INBOX");
    assert_eq!(Op::EmptyFolder { path: "Trash".into() }.summary(), "EmptyFolder of Trash");
    assert!(!mv.summary().contains("studio.example"));
    let renamed = Op::RenameCalendar { calendar: "AAMk1".into(), name: "Dentist and Ana".into() };
    assert_eq!(renamed.summary(), "RenameCalendar", "a calendar's name stays out of the log");
}

#[test]
fn secrets_never_print() {
    let j = Job::Start { account: AccountConfig::default(), password: Some(Secret("hunter2".into())) };
    assert!(!format!("{j:?}").contains("hunter2"));
    let e = Event::Decrypted { key: MessageKey::default(), content: Some(Private(b"the plan".to_vec())), authenticated: true, error: None };
    assert!(!format!("{e:?}").contains("the plan"), "what decrypted mail says never prints");
    let j = Job::Pgp {
        account: AccountConfig::default(),
        work: PgpWork::Import { file: Private(b"-----BEGIN PGP PRIVATE KEY BLOCK-----".to_vec()), passphrase: Secret("hunter2".into()) },
    };
    let shown = format!("{j:?}");
    assert!(!shown.contains("hunter2") && !shown.contains("45, 45"), "neither the passphrase nor the key file: {shown}");
    let j = Job::Pgp {
        account: AccountConfig::default(),
        work: PgpWork::Backup { fingerprint: "AB".into(), passphrase: Secret("correct horse".into()), path: "/tmp/k.asc".into() },
    };
    assert!(!format!("{j:?}").contains("correct horse") && !serde_json::to_string(&j).unwrap().contains("correct horse"));
}

/// What is written of a job or an event holds no secret either: a log or a file made of one
/// says that there was one, and no more.
#[test]
fn secrets_never_serialize() {
    let meeting = WebexMeeting {
        id: "m1".into(),
        web_link: "https://acme.webex.com/acme/j.php?MTID=m1".into(),
        meeting_number: "2623456701".into(),
        password: Secret("Xy7qT2".into()),
        sip_address: String::new(),
    };
    let j = serde_json::to_string(&Job::SetWebexSecret { secret: Secret("hunter2".into()) }).unwrap();
    let e = serde_json::to_string(&Event::WebexCreated { ask: 1, meeting: Some(meeting), error: None }).unwrap();
    for (written, secret) in [(j, "hunter2"), (e, "Xy7qT2")] {
        assert!(!written.contains(secret), "{written}");
        assert!(written.contains("<redacted>"), "the field is there, masked: {written}");
    }
    assert_eq!(serde_json::to_string(&Secret::default()).unwrap(), "\"\"", "none to hide");
}

#[test]
fn days() {
    for ok in ["2026-10-15", "2024-02-29", "2000-02-29"] {
        assert!(is_day(ok), "{ok}");
    }
    for bad in ["2026-02-29", "1900-02-29", "2026-13-01", "2026-1-01", "26-10-15", "2026-10-15T00", "", "2026-10-00", "abcd-ef-gh"] {
        assert!(!is_day(bad), "{bad}");
    }
}

/// Waiting for the user's answer about another server is no more online than waiting for a
/// password.
#[test]
fn waiting_for_trust_is_not_online() {
    let s = SyncStatus::NeedsTrust { site: "office365.com".into() };
    assert_eq!(s.is_online(), SyncStatus::NeedsPassword { error: String::new() }.is_online());
    assert!(!s.is_online());
    assert_eq!(serde_json::to_value(&s).unwrap()["state"], "needsTrust");
}

#[test]
fn jobs_serialize_tagged() {
    let j = Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() };
    let v = serde_json::to_value(&j).unwrap();
    assert_eq!(v["job"], "syncFolder");
    assert_eq!(serde_json::from_value::<Job>(v).unwrap(), j);
    assert_eq!(j.account(), "a1");
}
