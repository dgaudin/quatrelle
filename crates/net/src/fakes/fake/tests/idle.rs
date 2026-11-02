//! Mail that arrives between the Inbox's sync and IDLE shows at once, not after IDLE's ten
//! minutes.

use super::*;

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
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { port, security: Security::None, host: "127.0.0.1".into() },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
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
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { port, security: Security::None, host: "127.0.0.1".into() },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
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
