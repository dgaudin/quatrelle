// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Mail that arrives between the Inbox's sync and IDLE shows at once, not after IDLE's ten
//! minutes.

use super::*;

/// The fake's account, signed in with a password.
fn account(port: u16) -> AccountConfig {
    AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { port, security: Security::None, host: "127.0.0.1".into() },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    }
}

/// The account's calendars and address books looked for today, and none found: the pass after
/// a full sync, which comes before the look at the Inbox and IDLE, asks nothing of the network.
/// Otherwise it looks for `studio.example`'s on this computer's DNS: a tenth of a second, or 3 to
/// 5 s more for each answer lost, and two lost made a ten-second wait after "online" fail.
fn calendars_looked_for(store: &Store) {
    let pim = quatrelle_model::pim::Pim { dav_checked: Some(crate::worker::now()), ..Default::default() };
    store.save_account_doc("a1", quatrelle_model::pim::PIM_DOC, &pim).unwrap();
}

/// An `EXISTS` with a new count in any command's answer is news; the next `SELECT` starts over.
#[test]
fn a_command_saying_the_folder_grew_is_news() {
    let st = State::new();
    st.lock().unwrap().add("INBOX", "one", &[]);
    let mut c = login(&st);
    c.select("INBOX", false).unwrap();
    c.noop().unwrap();
    assert!(!c.take_news(), "the same count is no news");
    st.lock().unwrap().add("INBOX", "two", &[]);
    c.noop().unwrap();
    assert!(c.take_news());
    assert!(!c.take_news(), "said once");
    st.lock().unwrap().add("INBOX", "three", &[]);
    c.noop().unwrap();
    let info = c.select("INBOX", false).unwrap();
    assert!(!c.take_news(), "a SELECT looks at everything");
    assert_eq!(c.seen("INBOX"), Some(&info));
}

#[test]
fn mail_arriving_after_the_inbox_sync_shows_before_idle() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.caps = "IMAP4rev1 UIDPLUS MOVE CONDSTORE ENABLE QUOTA IDLE";
        s.add("INBOX", "First", &[]);
        // Delivered once the Inbox is synced, while the sync is on another folder.
        s.deliver_on = Some(("SELECT \"Archive\"".into(), "Late".into()));
    }
    let port = serve_tcp(&st);
    let store = store("late-mail");
    calendars_looked_for(&store);
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: account(port), password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert!(st.lock().unwrap().deliver_on.is_none(), "delivered during the sync");
    let has_late = || store.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.subject == "Late");
    let end = Instant::now() + Duration::from_secs(10);
    while !has_late() {
        assert!(Instant::now() < end, "the late mail waited for IDLE to end");
        let _ = pool.wait(Duration::from_millis(100));
    }
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// The same when the Inbox was read on a connection of its own, as the app reads a full sync's
/// folders: the connection that waits in IDLE never selected the Inbox before, and its `SELECT`
/// just before IDLE is told of the late mail, which the server then never pushes. It waited for
/// the next full sync, a quarter of an hour or more.
#[test]
fn mail_arriving_after_the_inbox_was_read_elsewhere_shows_before_idle() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.caps = "IMAP4rev1 UIDPLUS MOVE CONDSTORE ENABLE QUOTA IDLE";
        s.add("INBOX", "First", &[]);
        // Delivered once every folder is read, as the sync asks for the quota on the main
        // connection.
        s.deliver_on = Some(("GETQUOTAROOT".into(), "Late".into()));
    }
    let port = serve_tcp(&st);
    let store = store("late-mail-elsewhere");
    calendars_looked_for(&store);
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.opts.parallel = 2;
    pool.submit(Job::Start { account: account(port), password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert!(st.lock().unwrap().deliver_on.is_none(), "delivered during the sync");
    assert!(st.lock().unwrap().peak >= 3, "the folders were read on connections of their own");
    let has_late = || store.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.subject == "Late");
    let end = Instant::now() + Duration::from_secs(10);
    while !has_late() {
        assert!(Instant::now() < end, "the late mail waited for IDLE to end");
        let _ = pool.wait(Duration::from_millis(100));
    }
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// The same when the connection that waits in IDLE selected the Inbox after the Inbox was read
/// elsewhere, for a change made meanwhile (a message read as the app opens): that `SELECT` was
/// told of the late mail, and nothing read it.
#[test]
fn mail_told_of_to_a_change_after_the_inbox_was_read_elsewhere_shows_before_idle() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.caps = "IMAP4rev1 UIDPLUS MOVE CONDSTORE ENABLE QUOTA IDLE";
        s.add("INBOX", "First", &[]);
        // The last folder takes a while: the change is sent meanwhile.
        s.stall_on = Some(("SELECT \"Archive\"".into(), Duration::from_secs(3)));
    }
    let port = serve_tcp(&st);
    let store = store("late-mail-change");
    calendars_looked_for(&store);
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.opts.parallel = 2;
    pool.submit(Job::Start { account: account(port), password: Some(Secret("pw".into())) });
    // The Inbox read on a connection of its own, then mail comes.
    let end = Instant::now() + Duration::from_secs(10);
    while store.load_folder("a1", "INBOX").map_or(true, |f| f.messages.is_empty()) {
        assert!(Instant::now() < end, "the Inbox was never read");
        let _ = pool.wait(Duration::from_millis(10));
    }
    st.lock().unwrap().add("INBOX", "Late", &[]);
    // Read here, as the engine's command does, then sent on the main connection.
    let first = store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone();
    let read = Op::SetFlag { keys: vec![first], flag: Flag::Seen, on: true };
    store.add_op("a1", read.clone()).unwrap();
    store.apply_op("a1", &read).unwrap();
    pool.submit(Job::Apply { account: "a1".into(), op: read });
    let read_there = || st.lock().unwrap().mailbox("INBOX").is_some_and(|m| m.msgs.iter().any(|m| m.flags.iter().any(|f| f == "\\Seen")));
    let end = Instant::now() + Duration::from_secs(10);
    while !read_there() {
        assert!(Instant::now() < end, "the change was never sent");
        let _ = pool.wait(Duration::from_millis(10));
    }
    assert!(!st.lock().unwrap().log.iter().any(|c| c.starts_with("GETQUOTAROOT")), "sent while the folders were read");
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let has_late = || store.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.subject == "Late");
    let end = Instant::now() + Duration::from_secs(10);
    while !has_late() {
        assert!(Instant::now() < end, "the late mail waited for IDLE to end");
        let _ = pool.wait(Duration::from_millis(100));
    }
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// The Inbox read again before IDLE, and that sync refused by the server: IDLE follows, not the
/// same sync over and over.
#[test]
fn an_inbox_sync_refused_before_idle_is_not_tried_over_and_over() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.caps = "IMAP4rev1 UIDPLUS MOVE CONDSTORE ENABLE QUOTA IDLE";
        s.add("INBOX", "First", &[]);
        s.deliver_on = Some(("GETQUOTAROOT".into(), "Late".into()));
        // Only the Inbox's sync after the late mail asks for what came after the first message.
        s.refuse_on = vec![("UID SEARCH UID 2:*".into(), "not now".into(), 100)];
    }
    let port = serve_tcp(&st);
    let store = store("late-mail-refused");
    calendars_looked_for(&store);
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: account(port), password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let end = Instant::now() + Duration::from_secs(10);
    while !st.lock().unwrap().log.iter().any(|c| c == "IDLE") {
        assert!(Instant::now() < end, "never waited in IDLE: {:?}", st.lock().unwrap().log);
        let _ = pool.wait(Duration::from_millis(10));
    }
    let tries = st.lock().unwrap().log.iter().filter(|c| c.starts_with("UID SEARCH UID 2:*")).count();
    assert_eq!(tries, 1, "{:?}", st.lock().unwrap().log);
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// A connection lost while waiting in IDLE (the office's gateway changed over on 02/11, and
/// Gmail's connection died in silence) is made again at once, saying nothing: the account showed
/// an error each time, with nothing for the user to do. The mail that came meanwhile is read.
#[test]
fn a_connection_lost_while_waiting_comes_back_without_an_error() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.caps = "IMAP4rev1 UIDPLUS MOVE CONDSTORE ENABLE QUOTA IDLE";
        s.add("INBOX", "First", &[]);
        s.hang_up_idle = Some(Duration::from_millis(500));
    }
    let port = serve_tcp(&st);
    let store = store("lost-idle");
    calendars_looked_for(&store);
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: account(port), password: Some(Secret("pw".into())) });
    let mut seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    // Waiting in IDLE: mail comes, which the fake never tells of; only a new connection reads it.
    let end = Instant::now() + Duration::from_secs(10);
    while !st.lock().unwrap().log.iter().any(|c| c == "IDLE") {
        assert!(Instant::now() < end, "never waited in IDLE");
        std::thread::sleep(Duration::from_millis(10));
    }
    st.lock().unwrap().add("INBOX", "While cut off", &[]);
    let has = || store.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.subject == "While cut off");
    let end = Instant::now() + Duration::from_secs(15);
    while !has() {
        assert!(Instant::now() < end, "the mail never came; saw {seen:?}");
        if let Some(e) = pool.wait(Duration::from_millis(100)) {
            seen.push(e);
        }
    }
    // What was said before the mail came, to the last.
    let end = Instant::now() + Duration::from_secs(2);
    while let Some(e) = pool.wait(Duration::from_millis(200)).filter(|_| Instant::now() < end) {
        seen.push(e);
    }
    assert!(st.lock().unwrap().hang_up_idle.is_none(), "the connection was cut");
    let errors: Vec<&Event> =
        seen.iter().filter(|e| matches!(e, Event::Status { status: SyncStatus::Error { .. } | SyncStatus::NeedsPassword { .. }, .. })).collect();
    assert!(errors.is_empty(), "{errors:?}");
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}
