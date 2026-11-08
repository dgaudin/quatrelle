// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: archiving by age (the daily look, the bar's offer, what stays, the daily cap).

use quatrelle_model::{AccountConfig, Folder, FolderRole, PINNED, Security, ServerConfig, SyncStatus};

use super::*;

const DAY: i64 = 86_400;
const MONTH: i64 = 2_629_800;

fn folder(account: &str, path: &str, role: FolderRole) -> Folder {
    Folder { account: account.into(), path: path.into(), delimiter: Some('/'), role, selectable: true, ..Default::default() }
}

/// One message in `folder`, received `months` months before `NOW`.
fn aged(account: &str, folder: &str, uid: u32, months: i64, edit: impl FnOnce(&mut Summary)) -> Summary {
    let key = MessageKey { account: account.into(), folder: folder.into(), validity: 1, uid };
    let mut m = Summary {
        key,
        subject: format!("{folder} {uid}"),
        message_id: format!("{folder}-{uid}@studio.example").replace(' ', "-"),
        received: NOW - months * MONTH - DAY,
        date: NOW - months * MONTH - DAY,
        ..Default::default()
    };
    edit(&mut m);
    m
}

fn keep(store: &Store, account: &str, role_of: &[(&str, FolderRole)], mail: &[Summary]) {
    let folders: Vec<Folder> = role_of.iter().map(|(p, r)| folder(account, p, *r)).collect();
    store.save_folders(account, &folders).unwrap();
    for (path, _) in role_of {
        let mut st = store.load_folder(account, path).unwrap();
        st.validity = 1;
        st.messages = mail.iter().filter(|m| m.key.folder == *path).cloned().collect();
        store.save_folder(account, path, &st).unwrap();
    }
}

fn account(months: Option<u32>) -> AccountConfig {
    let server = ServerConfig { host: "imap.studio.example".into(), port: 993, security: Security::Tls };
    AccountConfig {
        id: "a1".into(),
        name: "Studio".into(),
        email: "me@studio.example".into(),
        imap: server.clone(),
        smtp: server,
        archive_after_months: months,
        ..Default::default()
    }
}

const ROLES: [(&str, FolderRole); 7] = [
    ("INBOX", FolderRole::Inbox),
    ("Sent", FolderRole::Sent),
    ("Drafts", FolderRole::Drafts),
    ("Deleted Items", FolderRole::Trash),
    ("Junk", FolderRole::Junk),
    ("Outbox", FolderRole::Other),
    ("Projects/2024", FolderRole::Other),
];

fn session(name: &str, months: Option<u32>, mail: &[Summary]) -> Session {
    let store = temp(name);
    store.save_accounts(&[account(months)]).unwrap();
    keep(&store, "a1", &ROLES, mail);
    Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0))
}

fn online(s: &mut Session) {
    s.apply(Event::Status { account: "a1".into(), status: SyncStatus::Online { at: 1 } });
    // The look is a few folders a frame.
    for _ in 0..200 {
        if !s.archive_scanning() {
            break;
        }
        s.take_jobs();
    }
}

fn archived_keys(jobs: Vec<Job>) -> Vec<Vec<MessageKey>> {
    jobs.into_iter()
        .filter_map(|j| match j {
            Job::Archive { keys, .. } => Some(keys),
            _ => None,
        })
        .collect()
}

#[test]
fn by_age_counts_only_what_should_go() {
    let none = |_: &mut Summary| {};
    let mail = vec![
        aged("a1", "INBOX", 1, 13, none),
        aged("a1", "INBOX", 2, 11, none),
        aged("a1", "INBOX", 3, 13, |m| m.flags.set_keyword(PINNED, true)),
        aged("a1", "INBOX", 4, 13, |m| m.flags.flagged = true),
        aged("a1", "INBOX", 5, 13, |m| m.flags.set_keyword(quatrelle_model::FOLLOW_UP_COMPLETE, true)),
        aged("a1", "Sent", 1, 20, none),
        aged("a1", "Projects/2024", 1, 30, none),
        aged("a1", "Drafts", 1, 20, none),
        aged("a1", "Deleted Items", 1, 20, none),
        aged("a1", "Junk", 1, 20, none),
        aged("a1", "Outbox", 1, 20, none),
    ];
    let mut s = session("age-counts", Some(12), &mail);
    s.take_jobs();
    online(&mut s);
    // Old and plain (Inbox 1), follow-up done (5), Sent and a subfolder: four. Not the recent
    // one, the pinned one, the one with an open flag, nor Drafts, Deleted Items, Junk, Outbox.
    assert_eq!(s.archive_offer("a1"), Some((4, 12)));
    s.run("archive.byAge", &json!({"account": "a1"})).unwrap();
    let batches = archived_keys(s.take_jobs());
    let mut uids: Vec<(String, u32)> = batches.iter().flatten().map(|k| (k.folder.clone(), k.uid)).collect();
    uids.sort();
    assert_eq!(uids, vec![("INBOX".to_string(), 1), ("INBOX".to_string(), 5), ("Projects/2024".to_string(), 1), ("Sent".to_string(), 1)]);
    assert!(s.archive_offer("a1").is_none(), "the bar goes");
    done(s);
}

#[test]
fn the_bar_never_shows_for_an_account_that_did_not_ask_nor_offline() {
    let old = vec![aged("a1", "INBOX", 1, 40, |_| {})];
    // Off (the default): no look, no bar.
    let mut s = session("age-off", None, &old);
    s.take_jobs();
    online(&mut s);
    assert!(s.archive_offer("a1").is_none());
    assert!(!s.archive_scanning());
    // On, but not connected.
    let mut s = session("age-offline", Some(12), &old);
    s.take_jobs();
    for _ in 0..50 {
        s.take_jobs();
    }
    assert!(s.archive_offer("a1").is_none());
    // On and connected: offered; then the connection goes: the bar goes with it.
    online(&mut s);
    assert_eq!(s.archive_offer("a1"), Some((1, 12)));
    s.apply(Event::Status { account: "a1".into(), status: SyncStatus::Offline });
    assert!(s.archive_offer("a1").is_none());
    // Turning it on later (already connected) looks at once; off, it forgets.
    let mut s = session("age-later", None, &old);
    s.take_jobs();
    online(&mut s);
    s.run("archive.settings", &json!({"account": "a1", "afterMonths": 6})).unwrap();
    for _ in 0..50 {
        s.take_jobs();
    }
    assert_eq!(s.archive_offer("a1"), Some((1, 6)));
    s.run("archive.settings", &json!({"account": "a1", "afterMonths": null})).unwrap();
    assert!(s.archive_offer("a1").is_none());
    done(s);
}

#[test]
fn by_age_asks_once_a_day() {
    let old = vec![aged("a1", "INBOX", 1, 40, |_| {})];
    let mut s = session("age-daily", Some(12), &old);
    s.take_jobs();
    online(&mut s);
    assert!(s.archive_offer("a1").is_some());
    // Not now: gone, and not asked again that day however often the account reports Online.
    s.run("archive.notNow", &json!({"account": "a1"})).unwrap();
    assert!(s.archive_offer("a1").is_none());
    online(&mut s);
    assert!(s.archive_offer("a1").is_none(), "not again today");
    // A restart the same day: still not.
    let store = s.store.clone().unwrap();
    drop(s);
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW + 3600, 0));
    s.take_jobs();
    online(&mut s);
    assert!(s.archive_offer("a1").is_none(), "not after a restart either");
    // The next day.
    s.set_clock(NOW + DAY + 10, 0);
    online(&mut s);
    assert_eq!(s.archive_offer("a1"), Some((1, 12)), "asked again tomorrow");
    // Ignoring the bar is not asking again: one offer, however often it is reported.
    online(&mut s);
    assert_eq!(s.archive_offer("a1"), Some((1, 12)));
    done(s);
}

#[test]
fn by_age_stops_at_two_thousand() {
    let many: Vec<Summary> = (1..=2600).map(|uid| aged("a1", "INBOX", uid, 14, |_| {})).collect();
    let mut s = session("age-cap", Some(12), &many);
    s.take_jobs();
    online(&mut s);
    assert_eq!(s.archive_offer("a1"), Some((2000, 12)), "what will go today");
    s.run("archive.byAge", &json!({"account": "a1"})).unwrap();
    let batches = archived_keys(s.take_jobs());
    assert_eq!(batches.iter().map(Vec::len).collect::<Vec<_>>(), vec![500, 500, 500, 500]);
    // The room of the day is used: no more offered today, even asked afresh.
    s.age_forget_for_tests();
    online(&mut s);
    assert!(s.archive_offer("a1").is_none(), "2,000 a day");
    // Tomorrow the room is back; what is still on its way is not offered twice.
    s.set_clock(NOW + DAY + 10, 0);
    online(&mut s);
    assert_eq!(s.archive_offer("a1").map(|o| o.0), Some(600));
    done(s);
}

#[test]
fn a_run_reports_its_progress() {
    let many: Vec<Summary> = (1..=1200).map(|uid| aged("a1", "INBOX", uid, 14, |_| {})).collect();
    let mut s = session("age-progress", Some(12), &many);
    s.take_jobs();
    online(&mut s);
    assert!(s.archive_progress().is_none());
    s.run("archive.byAge", &json!({"account": "a1"})).unwrap();
    assert_eq!(s.archive_progress(), Some((0, 1200)));
    let first: Vec<MessageKey> = archived_keys(s.take_jobs())[0].clone();
    let moved = first.iter().map(|k| (k.clone(), MessageKey { account: "a2".into(), ..k.clone() })).collect();
    s.apply(Event::Archived { account: "a1".into(), moved, kept: vec![] });
    assert_eq!(s.archive_progress(), Some((500, 1200)));
    done(s);
}
