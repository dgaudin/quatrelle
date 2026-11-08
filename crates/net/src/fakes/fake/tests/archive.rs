// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Archiving IMAP mail onto this computer, over the fake IMAP server: the copy, the read-back and
//! the deletion on the server, Gmail's Trash included.

use super::*;
use quatrelle_model::AccountRole;

fn count(st: &Arc<Mutex<State>>, name: &str) -> usize {
    st.lock().unwrap().mailbox(name).map_or(0, |b| b.msgs.len())
}

/// Gmail keeps every message in All Mail: an expunge in a label folder only drops the label, and
/// one in Trash removes the message everywhere. A refused expunge says no, as many times as told.
#[test]
fn the_fake_keeps_gmails_all_mail() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.gmail();
        s.add("INBOX", "a", &[]);
        s.add("INBOX", "b", &[]);
    }
    let mut c = login(&st);
    c.select("INBOX", false).unwrap();
    c.uid_store("1", true, "\\Deleted").unwrap();
    c.expunge("1").unwrap();
    assert_eq!((count(&st, "INBOX"), count(&st, "[Gmail]/All Mail"), count(&st, "[Gmail]/Trash")), (1, 1, 0), "the label goes, the message stays");
    // Moved to Trash and expunged there: gone from Trash, and from All Mail with it.
    let raw = st.lock().unwrap().mailbox("[Gmail]/All Mail").unwrap().msgs[0].raw.clone();
    st.lock().unwrap().add_raw("[Gmail]/Trash", raw);
    c.select("[Gmail]/Trash", false).unwrap();
    c.uid_store("1", true, "\\Deleted").unwrap();
    c.expunge("1").unwrap();
    assert_eq!((count(&st, "[Gmail]/All Mail"), count(&st, "[Gmail]/Trash")), (0, 0));
    // A refused expunge leaves the message where it is, then works again.
    st.lock().unwrap().refuse_expunge(1);
    c.select("INBOX", false).unwrap();
    c.uid_store("2", true, "\\Deleted").unwrap();
    assert!(c.expunge("2").is_err());
    assert_eq!(count(&st, "INBOX"), 1);
    c.expunge("2").unwrap();
    assert_eq!(count(&st, "INBOX"), 0);
}

const ARCHIVES: &str = "a9";

fn archives_account() -> AccountConfig {
    AccountConfig {
        id: ARCHIVES.into(),
        name: "Archives".into(),
        email: "archives@archives.example".into(),
        role: Some(AccountRole::Archives),
        ..Default::default()
    }
}

fn imap_account(port: u16) -> AccountConfig {
    AccountConfig {
        id: "a1".into(),
        name: "Studio".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    }
}

/// The account online (its mail listed, none downloaded ahead) with the Archives account beside it.
fn online(st: &Arc<Mutex<State>>, name: &str, store: Store) -> crate::worker::Pool {
    let port = serve_tcp(st);
    store.save_accounts(&[imap_account(port), archives_account()]).unwrap();
    let mut pool = crate::worker::Pool::new(store, Arc::new(crate::worker::MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    pool.submit(Job::Start { account: imap_account(port), password: Some(Secret("pw".into())) });
    until(&pool, &format!("{name} online"), |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    pool
}

fn keys(store: &Store, folder: &str) -> Vec<MessageKey> {
    let mut v: Vec<MessageKey> = store.load_folder("a1", folder).unwrap().messages.into_iter().map(|m| m.key).collect();
    v.sort_by_key(|k| k.uid);
    v
}

fn archived(pool: &crate::worker::Pool) -> (Vec<(MessageKey, MessageKey)>, Vec<(MessageKey, String)>) {
    let seen = until(pool, "archived", |e| matches!(e, Event::Archived { .. }));
    match seen.last() {
        Some(Event::Archived { moved, kept, .. }) => (moved.clone(), kept.clone()),
        other => panic!("{other:?}"),
    }
}

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while !f() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn archive(pool: &mut crate::worker::Pool, keys: Vec<MessageKey>) {
    pool.submit(Job::Archive { account: "a1".into(), keys, to_account: ARCHIVES.into() });
}

fn originals(st: &Arc<Mutex<State>>, folder: &str) -> Vec<Vec<u8>> {
    st.lock().unwrap().mailbox(folder).unwrap().msgs.iter().map(|m| m.raw.clone()).collect()
}

#[test]
fn archived_mail_is_copied_checked_then_gone_from_the_server() {
    let st = State::new();
    for s in ["one", "two", "three"] {
        st.lock().unwrap().add("INBOX", s, &["\\Seen", "\\Flagged"]);
    }
    let raws = originals(&st, "INBOX");
    let store = store("archive-basic");
    let mut pool = online(&st, "basic", store.clone());
    let all = keys(&store, "INBOX");
    archive(&mut pool, all[..2].to_vec());
    let (moved, kept) = archived(&pool);
    assert_eq!((moved.len(), kept.len()), (2, 0), "{kept:?}");
    // In Archives, as the server had them, marks kept, under the account's name.
    let there = store.load_folder(ARCHIVES, "Studio/INBOX").unwrap();
    assert_eq!(there.messages.len(), 2);
    for (from, to) in &moved {
        assert_eq!((to.account.as_str(), to.folder.as_str()), (ARCHIVES, "Studio/INBOX"));
        let i = all.iter().position(|k| k == from).unwrap();
        assert_eq!(store.load_body(to).unwrap().as_deref(), Some(raws[i].as_slice()));
        let m = there.messages.iter().find(|m| m.key == *to).unwrap();
        assert!(m.flags.seen && m.flags.flagged);
    }
    // Gone from the server, and from the list here; the third is untouched.
    assert_eq!(count(&st, "INBOX"), 1);
    assert_eq!(keys(&store, "INBOX"), all[2..].to_vec());
    assert!(store.load_ops("a1").unwrap().is_empty(), "the journal is done");
    // The space is read again.
    until(&pool, "the quota read again", |e| matches!(e, Event::Quota { .. }));
    pool.submit(Job::Remove { account: "a1".into() });
}

#[test]
fn a_copy_that_fails_deletes_nothing() {
    let st = State::new();
    for s in ["one", "two"] {
        st.lock().unwrap().add("INBOX", s, &[]);
    }
    let store = Store::failing();
    let mut pool = online(&st, "failing", store.clone());
    let all = keys(&store, "INBOX");
    // The disk refuses message files now.
    store.fail(None, Some(".eml"));
    archive(&mut pool, all.clone());
    let (moved, kept) = archived(&pool);
    assert!(moved.is_empty(), "{moved:?}");
    assert_eq!(kept.len(), 2, "the failed one and the rest");
    store.fail(None, None);
    assert_eq!(count(&st, "INBOX"), 2, "nothing deleted on the server");
    assert_eq!(keys(&store, "INBOX"), all);
    assert!(store.load_ops("a1").unwrap().is_empty());
    assert!(store.load_folder(ARCHIVES, "Studio/INBOX").unwrap().messages.is_empty());
    pool.submit(Job::Remove { account: "a1".into() });
}

#[test]
fn a_lost_connection_after_the_copy_deletes_at_the_next_connection_once() {
    let st = State::new();
    st.lock().unwrap().add("INBOX", "one", &[]);
    let raw = originals(&st, "INBOX").remove(0);
    let store = store("archive-offline");
    let mut pool = online(&st, "offline", store.clone());
    let key = keys(&store, "INBOX").remove(0);
    // Its source is kept here, then the connection goes.
    pool.submit(Job::FetchBody { key: key.clone() });
    until(&pool, "its source", |e| matches!(e, Event::Body { .. }));
    pool.submit(Job::WorkOffline { on: true });
    until(&pool, "offline", |e| matches!(e, Event::Status { status: SyncStatus::Offline, .. }));
    archive(&mut pool, vec![key.clone()]);
    let (moved, kept) = archived(&pool);
    assert_eq!((moved.len(), kept.len()), (1, 0), "{kept:?}");
    assert_eq!(store.load_body(&moved[0].1).unwrap().as_deref(), Some(raw.as_slice()));
    // The server still has it; the deletion waits in the journal.
    assert_eq!(count(&st, "INBOX"), 1);
    assert!(matches!(store.load_ops("a1").unwrap().as_slice(), [Op::Expunge { .. }]));
    // The next connection carries it out, once.
    pool.submit(Job::WorkOffline { on: false });
    wait_for("the deletion", || count(&st, "INBOX") == 0);
    wait_for("the journal done", || store.load_ops("a1").unwrap().is_empty());
    let expunges = st.lock().unwrap().log.iter().filter(|l| l.contains("EXPUNGE")).count();
    assert_eq!(expunges, 1);
    assert_eq!(store.load_folder(ARCHIVES, "Studio/INBOX").unwrap().messages.len(), 1);
    pool.submit(Job::Remove { account: "a1".into() });
}

#[test]
fn gmail_mail_goes_through_its_trash() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.gmail();
        let uid = s.add("INBOX", "one", &[]);
        let raw = s.mailbox("INBOX").unwrap().msgs.iter().find(|m| m.uid == uid).unwrap().raw.clone();
        s.add_raw("[Gmail]/All Mail", raw);
    }
    let store = store("archive-gmail");
    let mut pool = online(&st, "gmail", store.clone());
    archive(&mut pool, keys(&store, "INBOX"));
    let (moved, kept) = archived(&pool);
    assert_eq!((moved.len(), kept.len()), (1, 0), "{kept:?}");
    assert_eq!((count(&st, "INBOX"), count(&st, "[Gmail]/Trash"), count(&st, "[Gmail]/All Mail")), (0, 0, 0), "gone, not in All Mail either");
    assert!(st.lock().unwrap().log.iter().any(|l| l.contains("MOVE") && l.contains("[Gmail]/Trash")), "through its Trash");
    pool.submit(Job::Remove { account: "a1".into() });
}

#[test]
fn gmail_without_a_trash_folder_keeps_the_mail_and_says_so() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.gmail();
        s.boxes.retain(|b| b.name != "[Gmail]/Trash");
        s.add("INBOX", "one", &[]);
    }
    let store = store("archive-gmail-no-trash");
    let mut pool = online(&st, "gmail without trash", store.clone());
    archive(&mut pool, keys(&store, "INBOX"));
    let (moved, kept) = archived(&pool);
    assert!(moved.is_empty());
    assert_eq!(kept.len(), 1);
    assert!(kept[0].1.contains("Trash"), "{}", kept[0].1);
    assert_eq!(count(&st, "INBOX"), 1, "still on the server");
    assert!(store.load_ops("a1").unwrap().is_empty());
    assert_eq!(store.load_folder(ARCHIVES, "Studio/INBOX").unwrap().messages.len(), 1, "the copy is kept");
    pool.submit(Job::Remove { account: "a1".into() });
}

#[test]
fn without_uidplus_the_server_expunge_is_used() {
    let st = State::new();
    st.lock().unwrap().caps = "IMAP4rev1 MOVE CONDSTORE ENABLE QUOTA";
    st.lock().unwrap().add("INBOX", "one", &[]);
    let store = store("archive-no-uidplus");
    let mut pool = online(&st, "no uidplus", store.clone());
    archive(&mut pool, keys(&store, "INBOX"));
    let (moved, _) = archived(&pool);
    assert_eq!(moved.len(), 1);
    assert_eq!(count(&st, "INBOX"), 0);
    let log = st.lock().unwrap().log.clone();
    assert!(log.iter().any(|l| l.eq_ignore_ascii_case("EXPUNGE")), "{log:?}");
    assert!(!log.iter().any(|l| l.contains("UID EXPUNGE")), "{log:?}");
    pool.submit(Job::Remove { account: "a1".into() });
}

#[test]
fn archiving_twice_copies_once() {
    let st = State::new();
    st.lock().unwrap().add("INBOX", "one", &[]);
    let store = store("archive-twice");
    let mut pool = online(&st, "twice", store.clone());
    let key = keys(&store, "INBOX").remove(0);
    // The server says no to the deletion (or ignores it): the message is still there.
    st.lock().unwrap().refuse_expunge(1);
    archive(&mut pool, vec![key.clone()]);
    let (moved, _) = archived(&pool);
    assert_eq!(moved.len(), 1);
    wait_for("the refused deletion given up and the list read again", || {
        count(&st, "INBOX") == 1 && store.load_ops("a1").unwrap().is_empty() && keys(&store, "INBOX") == vec![key.clone()]
    });
    // Archived again: the copy is found, not made again, and the deletion goes through.
    archive(&mut pool, vec![key.clone()]);
    let (again, kept) = archived(&pool);
    assert_eq!((again.len(), kept.len()), (1, 0), "{kept:?}");
    assert_eq!(again[0].1, moved[0].1, "the same copy");
    assert_eq!(store.load_folder(ARCHIVES, "Studio/INBOX").unwrap().messages.len(), 1);
    wait_for("the deletion", || count(&st, "INBOX") == 0);
    pool.submit(Job::Remove { account: "a1".into() });
}

#[test]
fn two_archives_into_one_folder_keep_both() {
    let st = State::new();
    for s in ["one", "two", "three", "four"] {
        st.lock().unwrap().add("INBOX", s, &[]);
    }
    let store = store("archive-two");
    let mut pool = online(&st, "two", store.clone());
    let all = keys(&store, "INBOX");
    archive(&mut pool, all[..2].to_vec());
    let (first, _) = archived(&pool);
    // A move inside Archives meanwhile (the window's own write to the same account).
    store.apply_op(ARCHIVES, &Op::CreateFolder { path: "Studio/Elsewhere".into() }).unwrap();
    archive(&mut pool, all[2..].to_vec());
    let (second, _) = archived(&pool);
    assert_eq!((first.len(), second.len()), (2, 2));
    let there = store.load_folder(ARCHIVES, "Studio/INBOX").unwrap();
    let mut uids: Vec<u32> = there.messages.iter().map(|m| m.key.uid).collect();
    uids.sort_unstable();
    uids.dedup();
    assert_eq!(uids.len(), 4, "four distinct messages");
    for (_, to) in first.iter().chain(&second) {
        assert!(store.load_body(to).unwrap().is_some());
    }
    assert!(store.load_folders(ARCHIVES).unwrap().iter().any(|f| f.path == "Studio/Elsewhere"), "the other write survives");
    pool.submit(Job::Remove { account: "a1".into() });
}

fn put_back(pool: &mut crate::worker::Pool, keys: Vec<MessageKey>, folder: &str) {
    pool.submit(Job::PutBack { account: "a1".into(), keys, to_folder: folder.into() });
}

type PutBack = (Vec<(MessageKey, Option<u32>)>, Vec<(MessageKey, String)>);

fn put_back_done(pool: &crate::worker::Pool) -> PutBack {
    let seen = until(pool, "put back", |e| matches!(e, Event::PutBack { .. }));
    match seen.last() {
        Some(Event::PutBack { back, kept, .. }) => (back.clone(), kept.clone()),
        other => panic!("{other:?}"),
    }
}

/// Mail archived, then put back: on the server again, with its marks, keywords and arrival time,
/// and only then gone from Archives.
#[test]
fn put_back_mail_is_on_the_server_before_it_leaves_archives() {
    let st = State::new();
    let uid = st.lock().unwrap().add("INBOX", "one", &["\\Seen", "\\Flagged", "$Forwarded", "work"]);
    let raw = originals(&st, "INBOX").remove(0);
    let received = 1_700_000_000;
    st.lock().unwrap().mailbox("INBOX").unwrap().msgs.iter_mut().find(|m| m.uid == uid).unwrap().received = received;
    let store = store("archive-back");
    let mut pool = online(&st, "back", store.clone());
    archive(&mut pool, keys(&store, "INBOX"));
    let (moved, _) = archived(&pool);
    wait_for("the deletion", || count(&st, "INBOX") == 0);
    // Put back into the Archive folder of the server.
    put_back(&mut pool, vec![moved[0].1.clone()], "Archive");
    let (back, kept) = put_back_done(&pool);
    assert_eq!((back.len(), kept.len()), (1, 0), "{kept:?}");
    assert_eq!(back[0].0, moved[0].1);
    assert!(back[0].1.is_some(), "the server said where");
    let there = st.lock().unwrap().mailbox("Archive").unwrap().msgs.iter().map(|m| (m.raw.clone(), m.flags.clone(), m.received)).collect::<Vec<_>>();
    assert_eq!(there.len(), 1);
    assert_eq!(there[0].0, raw);
    for f in ["\\Seen", "\\Flagged", "$Forwarded", "work"] {
        assert!(there[0].1.iter().any(|x| x == f), "{f} in {:?}", there[0].1);
    }
    assert_eq!(there[0].2, received, "its arrival time");
    // Gone from Archives, and the folder it was in counts none.
    assert!(store.load_folder(ARCHIVES, "Studio/INBOX").unwrap().messages.is_empty());
    assert!(store.load_body(&moved[0].1).unwrap().is_none());
    pool.submit(Job::Remove { account: "a1".into() });
}

/// A server that refuses the message: the archived copy stays, the user is told why.
#[test]
fn a_refused_append_keeps_the_archived_copy() {
    let st = State::new();
    st.lock().unwrap().add("INBOX", "one", &[]);
    let store = store("archive-back-refused");
    let mut pool = online(&st, "back refused", store.clone());
    archive(&mut pool, keys(&store, "INBOX"));
    let (moved, _) = archived(&pool);
    wait_for("the deletion", || count(&st, "INBOX") == 0);
    st.lock().unwrap().refuse_append(1);
    put_back(&mut pool, vec![moved[0].1.clone()], "INBOX");
    let (back, kept) = put_back_done(&pool);
    assert!(back.is_empty());
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].0, moved[0].1);
    assert_eq!(count(&st, "INBOX"), 0);
    assert_eq!(store.load_folder(ARCHIVES, "Studio/INBOX").unwrap().messages.len(), 1, "still in Archives");
    assert!(store.load_body(&moved[0].1).unwrap().is_some());
    // And once the server takes it, it goes.
    put_back(&mut pool, vec![moved[0].1.clone()], "INBOX");
    let (back, kept) = put_back_done(&pool);
    assert_eq!((back.len(), kept.len()), (1, 0), "{kept:?}");
    assert_eq!(count(&st, "INBOX"), 1);
    pool.submit(Job::Remove { account: "a1".into() });
}

/// A server without UIDPLUS doesn't say where the message went: it is found by its Message-ID.
/// A message with none can't be found: it is on the server, and the archived copy stays.
#[test]
fn without_appenduid_the_message_is_found_by_its_message_id() {
    let st = State::new();
    st.lock().unwrap().caps = "IMAP4rev1 MOVE CONDSTORE ENABLE QUOTA";
    st.lock().unwrap().add("INBOX", "withid", &[]);
    st.lock()
        .unwrap()
        .add_raw("INBOX", b"From: Zoe <zoe@studio.example>\r\nSubject: bare\r\nDate: Wed, 14 Oct 2026 09:30:00 +0000\r\n\r\nNo id.\r\n".to_vec());
    let store = store("archive-back-no-appenduid");
    let mut pool = online(&st, "no appenduid", store.clone());
    archive(&mut pool, keys(&store, "INBOX"));
    let (moved, kept) = archived(&pool);
    assert_eq!((moved.len(), kept.len()), (2, 0), "{kept:?}");
    wait_for("the deletions", || count(&st, "INBOX") == 0);
    put_back(&mut pool, moved.iter().map(|(_, to)| to.clone()).collect(), "INBOX");
    let (back, kept) = put_back_done(&pool);
    assert_eq!(count(&st, "INBOX"), 2, "both appended");
    assert_eq!((back.len(), kept.len()), (1, 1), "{back:?} {kept:?}");
    assert!(back[0].1.is_some(), "found by its Message-ID");
    assert!(kept[0].1.contains("Message-ID"), "{}", kept[0].1);
    let left = store.load_folder(ARCHIVES, "Studio/INBOX").unwrap();
    assert_eq!(left.messages.len(), 1, "the one without an id stays in Archives");
    assert_eq!(left.messages[0].key, kept[0].0);
    pool.submit(Job::Remove { account: "a1".into() });
}
