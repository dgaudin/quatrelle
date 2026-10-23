//! The worker pool over the fake IMAP server: settings and rules, working offline, the Outbox, connections, end to end over TCP.

use super::*;

#[test]
fn mailbox_settings_over_imap_and_managesieve() {
    let st = State::new();
    st.lock().unwrap().mailbox("Archive").unwrap().msgs.push(Msg {
        uid: 1,
        flags: Vec::new(),
        raw: b"Subject: old

x"
        .to_vec(),
        modseq: 1,
        received: RECEIVED,
    });
    st.lock().unwrap().mailbox("Archive").unwrap().uid_next = 2;
    let imap_port = serve_tcp(&st);
    let (sieve_port, sieve) = crate::sieve::tests::serve(crate::sieve::tests::Fake {
        scripts: Vec::new(),
        active: None,
        sieve: "fileinto vacation date relational include",
        log: Vec::new(),
    });
    let store = store("mailbox");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.sieve_port = sieve_port;
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port: imap_port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    let seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert!(seen.iter().any(|e| matches!(e, Event::Quota { limit: 1_048_576, used, .. } if *used >= 1024)), "GETQUOTAROOT, in bytes");

    pool.submit(Job::GetVacation { account: "a1".into() });
    until(&pool, "replies read", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if !v.enabled));
    let away = quatrelle_model::job::Vacation { enabled: true, subject: "Away".into(), text: "Back soon".into(), ..Default::default() };
    pool.submit(Job::SetVacation { account: "a1".into(), vacation: away.clone() });
    until(&pool, "replies on", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == away));
    assert_eq!(sieve.lock().unwrap().active.as_deref(), Some(crate::sieve::SCRIPT));

    // Emptying a folder removes what isn't downloaded too.
    pool.submit(Job::Apply { account: "a1".into(), op: Op::EmptyFolder { path: "Archive".into() } });
    let end = Instant::now() + Duration::from_secs(10);
    while !st.lock().unwrap().mailbox("Archive").unwrap().msgs.is_empty() {
        assert!(Instant::now() < end, "never emptied");
        std::thread::sleep(Duration::from_millis(50));
    }
    pool.submit(Job::Remove { account: "a1".into() });
    // An offline account answers at once.
    pool.submit(Job::GetVacation { account: "a1".into() });
    until(&pool, "offline answer", |e| matches!(e, Event::Vacation { vacation: None, error: Some(_), .. }));
    let _ = std::fs::remove_dir_all(store.root());
}

/// An IMAP account's rules go to its Sieve script: the rule as Sieve, the automatic replies
/// it already held kept, and the id the script keeps for each rule coming back.
#[test]
fn rules_reach_an_imap_accounts_sieve_script() {
    let st = State::new();
    st.lock().unwrap().mailbox("Archive").unwrap();
    let imap_port = serve_tcp(&st);
    let (sieve_port, sieve) = crate::sieve::tests::serve(crate::sieve::tests::Fake {
        // A script of the user's own, which ours leaves to them.
        scripts: vec![("filters".into(), "require \"fileinto\";".into())],
        active: None,
        sieve: "fileinto imap4flags vacation include",
        log: Vec::new(),
    });
    let store = store("sieve-rules");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.sieve_port = sieve_port;
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port: imap_port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let away = quatrelle_model::job::Vacation { enabled: true, text: "Back soon".into(), ..Default::default() };
    pool.submit(Job::SetVacation { account: "a1".into(), vacation: away.clone() });
    until(&pool, "replies on", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == away));
    use quatrelle_model::organize::{Condition, Rule, RuleAction};
    let rule = Rule {
        name: "Invoices".into(),
        enabled: true,
        account: Some("a1".into()),
        conditions: vec![Condition::Subject("invoice".into())],
        match_all: true,
        actions: vec![RuleAction::MoveTo("Archive".into()), RuleAction::MarkRead],
        stop: true,
        server: true,
        remote_id: None,
        priority: None,
        refused: None,
    };
    pool.submit(Job::SetServerRules { account: "a1".into(), rules: vec![rule.clone()], delete: Vec::new(), categories: Vec::new() });
    let seen = until(&pool, "rules written", |e| matches!(e, Event::ServerRules { ids, error: None, .. } if !ids.is_empty()));
    let ids = seen
        .into_iter()
        .find_map(|e| match e {
            Event::ServerRules { ids, .. } => Some(ids),
            _ => None,
        })
        .unwrap_or_default();
    assert_eq!(ids.len(), 1, "one id for the one rule: {ids:?}");
    {
        let f = sieve.lock().unwrap();
        let script = &f.scripts.iter().find(|(n, _)| n == crate::sieve::SCRIPT).expect("our script is written").1;
        assert!(script.contains("if header :contains \"subject\" \"invoice\" {"), "{script}");
        assert!(script.contains("fileinto \"Archive\";"), "{script}");
        assert!(script.contains("addflag \"\\\\Seen\";"), "{script}");
        assert!(script.contains("vacation :days 1"), "the automatic replies it held are kept: {script}");
        assert_eq!(f.active.as_deref(), Some(crate::sieve::SCRIPT));
    }
    // What the server keeps is read back: our rule by the id the script holds for it, and
    // the user's own script as a rule Quatrelle didn't make.
    pool.submit(Job::GetServerRules { account: "a1".into(), ours: vec!["sieve:Invoices".into()] });
    let seen = until(&pool, "rules read", |e| matches!(e, Event::ServerRules { error: None, .. } if ids_of(e).len() == 1));
    let read = seen.into_iter().find_map(|e| match e {
        Event::ServerRules { ids, others, cleared, error: None, .. } => Some((ids, others, cleared)),
        _ => None,
    });
    let (ids, others, cleared) = read.unwrap_or_default();
    assert_eq!(ids, vec![("Invoices".to_string(), "sieve:Invoices".to_string())]);
    assert_eq!(
        others.iter().map(|r| r.name.clone()).collect::<Vec<_>>(),
        vec!["filters".to_string()],
        "the user's own script is listed, and left alone"
    );
    assert!(cleared.is_empty());
    // The rule turned off: the script keeps the replies and loses the rule.
    pool.submit(Job::SetServerRules { account: "a1".into(), rules: Vec::new(), delete: vec![ids[0].1.clone()], categories: Vec::new() });
    until(&pool, "rules removed", |e| matches!(e, Event::ServerRules { ids, error: None, .. } if ids.is_empty()));
    {
        let f = sieve.lock().unwrap();
        let script = &f.scripts.iter().find(|(n, _)| n == crate::sieve::SCRIPT).expect("our script stays").1;
        assert!(!script.contains("fileinto"), "the rule is gone: {script}");
        assert!(script.contains("vacation :days 1"), "the replies stay: {script}");
        assert_eq!(f.active.as_deref(), Some(crate::sieve::SCRIPT), "still ours, for the replies");
    }
    // A rule turned off here isn't written to the script, and its id still comes back (so
    // it isn't sent again until it changes).
    let off = Rule { enabled: false, ..rule.clone() };
    pool.submit(Job::SetServerRules { account: "a1".into(), rules: vec![off], delete: vec![ids[0].1.clone()], categories: Vec::new() });
    let seen = until(&pool, "off rule kept", |e| matches!(e, Event::ServerRules { ids, error: None, .. } if !ids.is_empty()));
    assert!(seen.iter().any(|e| matches!(e, Event::ServerRules { ids, .. } if ids.iter().any(|(_, id)| id == "sieve:Invoices"))));
    {
        let f = sieve.lock().unwrap();
        let script = &f.scripts.iter().find(|(n, _)| n == crate::sieve::SCRIPT).expect("our script stays").1;
        assert!(!script.contains("if "), "a rule that's off isn't in the script: {script}");
    }
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// Send / Receive › Work Offline: connections close; changes, sent mail and drafts wait on
/// this computer; turned off, the account connects and the draft reaches the server.
#[test]
fn working_offline_keeps_drafts_and_mail_until_back() {
    let st = State::new();
    let imap_port = serve_tcp(&st);
    let store = store("offline");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port: imap_port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    };
    // Working offline before it starts: it never connects.
    pool.submit(Job::WorkOffline { on: true });
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "offline", |e| matches!(e, Event::Status { status: SyncStatus::Offline, .. }));
    let draft = b"From: me@studio.example
Subject: Kept

written offline
"
    .to_vec();
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: draft, folder: "INBOX".into(), replaces: None });
    until(&pool, "draft kept", |e| matches!(e, Event::Notice { message, .. } if message.contains("kept on this computer")));
    pool.submit(Job::Send {
        account: "a1".into(),
        id: 7,
        raw: b"Subject: x

y"
        .to_vec(),
        from: "me@studio.example".into(),
        recipients: vec!["a@b.example".into()],
        sent_folder: None,
    });
    until(&pool, "queued", |e| matches!(e, Event::Queued { id: 7, .. }));
    pool.submit(Job::SyncAll { account: "a1".into() });
    assert!(st.lock().unwrap().mailbox("INBOX").unwrap().msgs.is_empty(), "nothing reached the server");
    // Back online: the kept draft goes up first.
    pool.submit(Job::WorkOffline { on: false });
    let end = Instant::now() + Duration::from_secs(20);
    while st.lock().unwrap().mailbox("INBOX").unwrap().msgs.is_empty() {
        assert!(Instant::now() < end, "the draft never reached the server");
        std::thread::sleep(Duration::from_millis(50));
    }
    let saved = st.lock().unwrap().mailbox("INBOX").unwrap().msgs[0].raw.clone();
    assert!(String::from_utf8_lossy(&saved).contains("written offline"));
    assert!(store.outbox("a1").iter().any(|i| i.id == 7), "sent mail waits in the Outbox");
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// Issue #4: an account kept online only syncs its folders and opens its mail as any other,
/// but none of it reaches the disk: no data file, nothing downloaded ahead of reading, and
/// once Quatrelle closes, nothing of its mail is anywhere under the data folders. The other
/// account beside it keeps its mail as before (and shows the search finds what's kept).
#[test]
fn an_online_only_account_syncs_and_reads_with_nothing_on_disk() {
    fn on_disk(dir: &std::path::Path, needle: &[u8]) -> bool {
        let Ok(rd) = std::fs::read_dir(dir) else { return false };
        rd.flatten().any(|e| {
            let p = e.path();
            if p.is_dir() { on_disk(&p, needle) } else { std::fs::read(&p).is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle)) }
        })
    }
    let mail = |subject: &str, body: &str| {
        format!("From: Ana <ana@studio.example>\r\nTo: me@studio.example\r\nSubject: {subject}\r\nMessage-ID: <{subject}@studio.example>\r\n\r\n{body}\r\n").into_bytes()
    };
    let (online, kept) = (State::new(), State::new());
    for i in 1..=3 {
        online.lock().unwrap().add_raw("INBOX", mail(&format!("ONLINE-SUBJECT-{i}"), &format!("ONLINE-BODY-{i}")));
        kept.lock().unwrap().add_raw("INBOX", mail(&format!("KEPT-SUBJECT-{i}"), &format!("KEPT-BODY-{i}")));
    }
    let t = std::env::temp_dir().join(format!("quatrelle-fake-online-only-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    let store = Store::open_packed(t.join("app"), t.join("Quatrelle Files")).unwrap();
    let account = |id: &str, port: u16, online_only: bool| AccountConfig {
        id: id.into(),
        email: format!("{id}@studio.example"),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        online_only,
        ..Default::default()
    };
    let (a1, a2) = (account("a1", serve_tcp(&online), true), account("a2", serve_tcp(&kept), false));
    store.save_accounts(&[a1.clone(), a2.clone()]).unwrap();
    // As the engine does when it opens.
    store.set_online_only("a1", true).unwrap();
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: a1, password: Some(Secret("pw".into())) });
    pool.submit(Job::Start { account: a2, password: Some(Secret("pw".into())) });
    let end = Instant::now() + Duration::from_secs(20);
    let synced = |a: &str| store.load_folder(a, "INBOX").is_ok_and(|f| f.messages.len() == 3);
    while !(synced("a1") && synced("a2") && store.load_folder("a2", "INBOX").unwrap().messages.iter().all(|m| store.has_body(&m.key))) {
        assert!(Instant::now() < end, "never synced");
        let _ = pool.wait(Duration::from_millis(100));
    }
    let inbox = store.load_folder("a1", "INBOX").unwrap();
    assert!(inbox.messages.iter().any(|m| m.subject == "ONLINE-SUBJECT-2"));
    assert!(inbox.messages.iter().all(|m| !store.has_body(&m.key)), "nothing downloaded ahead of reading");
    // Opening one downloads it, into memory.
    let key = inbox.messages[0].key.clone();
    pool.submit(Job::FetchBody { key: key.clone() });
    let seen = until(&pool, "the opened message", |e| matches!(e, Event::Body { key: k, .. } if *k == key));
    assert!(seen.iter().any(|e| matches!(e, Event::Body { raw, .. } if String::from_utf8_lossy(raw).contains("ONLINE-BODY"))));
    assert!(store.has_body(&key));
    assert_eq!(store.data_file("a1"), None, "no data file");

    // Quatrelle closes (its threads let the data files go).
    let kept_file = store.data_file("a2").unwrap();
    drop(pool);
    drop(store);
    let end = Instant::now() + Duration::from_secs(20);
    while std::fs::read(&kept_file).is_err() {
        assert!(Instant::now() < end, "the data file stayed open");
        std::thread::sleep(Duration::from_millis(50));
    }
    for i in 1..=3 {
        assert!(!on_disk(&t, format!("ONLINE-SUBJECT-{i}").as_bytes()) && !on_disk(&t, format!("ONLINE-BODY-{i}").as_bytes()), "{i}");
        assert!(on_disk(&t, format!("KEPT-SUBJECT-{i}").as_bytes()) && on_disk(&t, format!("KEPT-BODY-{i}").as_bytes()), "{i}");
    }
    let files: Vec<String> =
        std::fs::read_dir(t.join("Quatrelle Files")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(files, ["a2@studio.example.quatrelle"], "only the other account has a data file");
    let _ = std::fs::remove_dir_all(&t);
}

#[test]
fn outbox_holds_mail_until_smtp_is_back() {
    let st = State::new();
    let imap_port = serve_tcp(&st);
    // An SMTP port with nothing listening yet: sending fails at once (connection refused).
    let smtp_port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let store = store("outbox");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port: imap_port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: smtp_port, security: Security::None },
        ..Default::default()
    };
    // Not started yet: a send goes straight to the Outbox.
    let raw = b"From: me@studio.example\r\nTo: zoe@studio.example\r\nSubject: queued\r\nMessage-ID: <q@x>\r\n\r\nhi\r\n".to_vec();
    let send = |id: u64| Job::Send {
        account: "a1".into(),
        id,
        raw: raw.clone(),
        from: "me@studio.example".into(),
        recipients: vec!["zoe@studio.example".into()],
        sent_folder: Some("Sent".into()),
    };
    pool.submit(send(1));
    until(&pool, "queued offline", |e| matches!(e, Event::Queued { id: 1, .. }));
    assert_eq!(store.outbox("a1").len(), 1);

    // Online, but the SMTP server is down: the next message joins the queue.
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    pool.submit(send(2));
    until(&pool, "queued on failure", |e| matches!(e, Event::Queued { id: 2, .. }));
    assert_eq!(store.outbox("a1").len(), 2);
    assert!(store.outbox("a1")[1].error.as_deref().is_some_and(|e| e.contains("network")), "{:?}", store.outbox("a1"));

    // A message held back for tomorrow (Delay Delivery) isn't sent with them.
    let later = quatrelle_store::OutboxItem {
        id: 3,
        from: "me@studio.example".into(),
        recipients: vec!["zoe@studio.example".into()],
        not_before: crate::worker::now() + 86_400,
        ..Default::default()
    };
    store.outbox_add("a1", &later, &raw).unwrap();

    // The server comes back: Send now delivers both, oldest first, and files copies.
    let (_, log1) = crate::smtp::tests::serve_once_on(std::net::TcpListener::bind(("127.0.0.1", smtp_port)).unwrap());
    pool.submit(Job::FlushOutbox { account: "a1".into() });
    until(&pool, "first sent", |e| matches!(e, Event::Sent { id: 1, .. }));
    assert!(log1.join().unwrap().contains("Subject: queued"));
    // serve_once takes one message; the second waits for the next server.
    let (_, log2) = crate::smtp::tests::serve_once_on(std::net::TcpListener::bind(("127.0.0.1", smtp_port)).unwrap());
    pool.submit(Job::FlushOutbox { account: "a1".into() });
    until(&pool, "second sent", |e| matches!(e, Event::Sent { id: 2, .. }));
    assert!(log2.join().unwrap().contains("RCPT TO:<zoe@studio.example>"));
    assert_eq!(store.outbox("a1").iter().map(|i| i.id).collect::<Vec<_>>(), vec![3], "the held-back one waits");
    let end = Instant::now() + Duration::from_secs(10);
    while st.lock().unwrap().mailbox("Sent").unwrap().msgs.len() < 2 {
        assert!(Instant::now() < end, "copies never reached Sent");
        std::thread::sleep(Duration::from_millis(50));
    }
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// A full sync reads folders on connections of their own, the main one staying free: three
/// connections at once (the main one and two readers), and every folder read.
#[test]
fn folders_are_read_on_connections_of_their_own() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        for (name, subject) in [("Sent", "sent today"), ("Archive", "old"), ("Projects/Caf&AOk-", "plan")] {
            s.add(name, subject, &[]);
        }
        s.add("INBOX", "hello", &[]);
    }
    let imap_port = serve_tcp(&st);
    let store = store("parallel");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.opts.parallel = 2;
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port: imap_port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    for folder in ["INBOX", "Sent", "Archive", "Projects/Caf&AOk-"] {
        let f = store.load_folder("a1", folder).unwrap();
        assert_eq!(f.messages.len(), 1, "{folder} was read");
    }
    // The readers opened their own connections: the main one and two of them.
    assert!(st.lock().unwrap().peak >= 3, "peak connections: {}", st.lock().unwrap().peak);
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn worker_pool_end_to_end_over_tcp() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.add("INBOX", "hello", &[]);
        s.add("INBOX", "second", &["\\Seen"]);
    }
    let imap_port = serve_tcp(&st);
    let (smtp_port, smtp_log) = crate::smtp::tests::serve_once();
    let store = store("e2e");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        name: "Me".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port: imap_port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: smtp_port, security: Security::None },
        ..Default::default()
    };

    // Wrong password first: the worker reports it and stops retrying.
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret("nope".into())) });
    until(&pool, "password prompt", |e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }));

    // Right password: folders, the Inbox, then online.
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret("pw".into())) });
    let seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert!(seen.iter().any(|e| matches!(e, Event::Folders { folders, .. } if folders.len() == 4)));
    assert!(seen.iter().any(|e| matches!(e, Event::FolderChanged { folder, .. } if folder == "INBOX")));
    let inbox = store.load_folder("a1", "INBOX").unwrap();
    assert_eq!(inbox.messages.len(), 2);

    // Open a message (already prefetched: answered from the cache).
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 100, uid: 1 };
    pool.submit(Job::FetchBody { key: key.clone() });
    until(&pool, "body", |e| matches!(e, Event::Body { key: k, raw } if *k == key && raw.starts_with(b"From:")));

    // Mark read: cache at once, server right after.
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true } });
    until(&pool, "flag change", |e| matches!(e, Event::FolderChanged { .. }));
    assert!(store.load_folder("a1", "INBOX").unwrap().find(1).unwrap().flags.seen);
    let end = Instant::now() + Duration::from_secs(10);
    while !st.lock().unwrap().mailbox("INBOX").unwrap().msgs[0].flags.iter().any(|f| f == "\\Seen") {
        assert!(Instant::now() < end, "the server never got the flag");
        std::thread::sleep(Duration::from_millis(50));
    }

    // Send: SMTP submission, then a copy filed in Sent.
    let raw = b"From: me@studio.example\r\nTo: zoe@studio.example\r\nSubject: e2e\r\nMessage-ID: <e2e@x>\r\n\r\nhi\r\n".to_vec();
    pool.submit(Job::Send {
        account: "a1".into(),
        id: 7,
        raw,
        from: "me@studio.example".into(),
        recipients: vec!["zoe@studio.example".into()],
        sent_folder: Some("Sent".into()),
    });
    until(&pool, "sent", |e| matches!(e, Event::Sent { id: 7, .. }));
    until(&pool, "sent copy", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Sent"));
    assert_eq!(st.lock().unwrap().mailbox("Sent").unwrap().msgs.len(), 1);
    assert!(smtp_log.join().unwrap().contains("Subject: e2e"));
    assert_eq!(store.load_folder("a1", "Sent").unwrap().messages[0].subject, "e2e");

    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// Issue #3: an outgoing server whose user name and password aren't the incoming server's.
/// Without its password kept, sending says so (and nothing goes as the IMAP user); once kept,
/// SMTP signs in with its own user name and password while IMAP keeps its own.
#[test]
fn the_outgoing_server_signs_in_with_its_own_user_name_and_password() {
    let st = State::new();
    let imap_port = serve_tcp(&st);
    let (smtp_port, smtp_log) = crate::smtp::tests::serve_once();
    let store = store("smtp-own-sign-in");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port: imap_port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: smtp_port, security: Security::None },
        smtp_username: Some("relay-user".into()),
        ..Default::default()
    };
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let send = |id: u64| Job::Send {
        account: "a1".into(),
        id,
        raw: b"From: me@studio.example\r\nTo: zoe@studio.example\r\nSubject: relay\r\n\r\nhi\r\n".to_vec(),
        from: "me@studio.example".into(),
        recipients: vec!["zoe@studio.example".into()],
        sent_folder: None,
    };
    pool.submit(send(1));
    until(&pool, "no password", |e| match e {
        Event::SendFailed { id: 1, error, .. } | Event::Queued { id: 1, error, .. } => error.contains("outgoing server's password"),
        _ => false,
    });
    pool.submit(Job::SmtpPassword { account: account.clone(), password: Secret("smtp-secret".into()) });
    pool.submit(send(2));
    until(&pool, "sent", |e| matches!(e, Event::Sent { id: 2, .. }));
    let log = smtp_log.join().unwrap();
    let own = crate::imap::base64(b"\0relay-user\0smtp-secret");
    assert!(log.contains(&own), "SMTP signed in as relay-user: {log}");
    assert!(!log.contains(&crate::imap::base64(b"\0me@studio.example\0pw")), "not as the IMAP user");
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// An IMAP account keeps its mail whole, as Outlook's Cached Exchange Mode keeps it offline:
/// the newest few with the folder's list, the rest downloaded ahead through the worker while
/// nothing else is asked; and new mail still comes in meanwhile: the Inbox is looked at between
/// downloads (IDLE and polls waited until they were done).
#[test]
fn imap_mail_kept_is_downloaded_whole_through_the_worker() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        for n in 1..=1000 {
            s.add_raw("INBOX", format!("Subject: Message {n}\r\nMessage-ID: <w{n}@x>\r\n\r\nbody {n}\r\n").into_bytes());
        }
    }
    let port = serve_tcp(&st);
    let store = store("worker-ahead-imap");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::SetSync { days: 0, messages: 0, full: true });
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    // New mail while the rest downloads: it comes before they're all in (a look at the Inbox
    // between downloads, not the next poll two minutes on).
    st.lock().unwrap().add_raw("INBOX", b"Subject: Just now\r\nMessage-ID: <now@x>\r\n\r\nhi\r\n".to_vec());
    let missing =
        |store: &Store| store.load_folder("a1", "INBOX").map(|f| f.messages.iter().filter(|m| !store.has_body(&m.key)).count()).unwrap_or(0);
    let end = Instant::now() + Duration::from_secs(20);
    while !store.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.subject == "Just now") {
        assert!(Instant::now() < end, "new mail never came while downloading");
        let _ = pool.wait(Duration::from_millis(20));
    }
    let end = Instant::now() + Duration::from_secs(60);
    while missing(&store) > 0 {
        assert!(Instant::now() < end, "never downloaded whole: {} missing", missing(&store));
        let _ = pool.wait(Duration::from_millis(50));
    }
}
