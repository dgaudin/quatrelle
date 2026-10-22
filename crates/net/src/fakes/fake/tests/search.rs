//! Searching with the fake IMAP server: queries as IMAP keys, mail past the window found and kept, Show more.

use super::*;

/// Issue #4: the search box's words as IMAP SEARCH keys; what IMAP can't ask is left out.
#[test]
fn a_query_becomes_imap_search_keys() {
    use crate::sync::{SearchArg, imap_criteria};
    let a = |s: &str| SearchArg::Atom(s.into());
    let st = |s: &str| SearchArg::Str(s.into());
    assert_eq!(imap_criteria("quarterly"), Some(vec![a("TEXT"), st("quarterly")]));
    assert_eq!(
        imap_criteria("from:ana subject:\"the plan\" -draft is:unread"),
        Some(vec![a("FROM"), st("ana"), a("SUBJECT"), st("the plan"), a("NOT"), a("TEXT"), st("draft"), a("UNSEEN")])
    );
    assert_eq!(imap_criteria("is:flagged -is:unread"), Some(vec![a("FLAGGED"), a("SEEN")]));
    assert_eq!(imap_criteria("has:attachment"), None, "nothing IMAP can ask");
    assert_eq!(imap_criteria(""), None);
    assert_eq!(imap_criteria("Grüße"), Some(vec![a("TEXT"), st("grüße")]));
    // Sizes; a list as nested ORs (left to the list when one name is several words).
    assert_eq!(imap_criteria("larger:1KB -smaller:10"), Some(vec![a("LARGER"), a("1024"), a("NOT"), a("SMALLER"), a("10")]));
    assert_eq!(
        imap_criteria("from:ana,jun,\"zoe park\""),
        Some(vec![a("OR"), a("FROM"), st("ana"), a("OR"), a("FROM"), st("jun"), a("FROM"), st("zoe park")])
    );
    assert_eq!(imap_criteria("-to:ana,jun"), Some(vec![a("NOT"), a("OR"), a("TO"), st("ana"), a("TO"), st("jun")]));
    assert_eq!(
        imap_criteria("from:ana,jun lee"),
        Some(vec![a("TEXT"), st("lee")]).map(|v| [vec![a("OR"), a("FROM"), st("ana"), a("FROM"), st("jun")], v].concat())
    );
}

/// Gap #7: the query as Exchange's KQL/AQS words with the read, flag and attachment
/// conditions apart, and as a JMAP filter. What a user types can't break out of a word: quotes
/// and backslashes are dropped, a word spelled like an operator is quoted.
#[test]
fn a_query_becomes_exchange_words_and_a_jmap_filter() {
    use crate::sync::{WordQuery, word_query};
    use serde_json::json;
    let w = |text: &str| WordQuery { text: text.into(), ..Default::default() };
    assert_eq!(word_query("quarterly"), Some(w("quarterly")));
    assert_eq!(
        word_query("from:ana subject:\"the plan\" -draft is:unread"),
        Some(WordQuery { text: "from:ana subject:\"the plan\" NOT draft".into(), unread: Some(true), ..Default::default() })
    );
    assert_eq!(
        word_query("is:flagged -is:unread has:attachment"),
        Some(WordQuery { unread: Some(false), flagged: Some(true), attachment: Some(true), ..Default::default() })
    );
    assert_eq!(word_query(""), None);
    assert_eq!(word_query("is:important"), None, "nothing Exchange's searches ask");
    assert_eq!(word_query("OR NOT"), Some(w("or not")), "words, lowercased by the parser: not operators");
    assert_eq!(word_query("a(b) ana@x.example").map(|q| q.text), Some("a b ana x.example".into()), "the parser's own words");
    for hostile in ["subject:\"a\\\" OR from:x\" a(b)", "\"\"", "\\\"", "from:\"", "x\" OR \"y", "\u{0}\"\r\n", "-\"-\""] {
        let Some(q) = word_query(hostile) else { continue };
        assert_eq!(q.text.matches('"').count() % 2, 0, "{hostile:?} → {:?}", q.text);
        assert!(!q.text.contains('\\') && !q.text.contains("\"\"") && !q.text.chars().any(char::is_control), "{hostile:?} → {:?}", q.text);
    }
    let long = "x".repeat(5000);
    assert!(word_query(&long).unwrap().text.len() <= 202);
    let read = WordQuery { unread: Some(true), ..Default::default() };
    assert!(read.admits(&quatrelle_model::Flags::default(), false));
    assert!(!read.admits(&quatrelle_model::Flags { seen: true, ..Default::default() }, false));

    assert_eq!(
        quatrelle_jmap::search_filter("from:ana -draft is:unread is:flagged"),
        Some(json!({"operator": "AND", "conditions": [
            {"from": "ana"},
            {"operator": "NOT", "conditions": [{"text": "draft"}]},
            {"notKeyword": "$seen"},
            {"hasKeyword": "$flagged"},
        ]}))
    );
    assert_eq!(
        quatrelle_jmap::search_filter("-is:unread has:attachment"),
        Some(json!({"operator": "AND", "conditions": [{"hasKeyword": "$seen"}, {"hasAttachment": true}]}))
    );
    assert_eq!(quatrelle_jmap::search_filter("is:important"), None);
    assert_eq!(quatrelle_jmap::search_filter(""), None);
    // Sizes (minSize is at least, so one more byte), and a list as OR.
    assert_eq!(
        quatrelle_jmap::search_filter("larger:1KB smaller:2KB from:ana,\"zoe park\""),
        Some(json!({"operator": "AND", "conditions": [
            {"minSize": 1025},
            {"maxSize": 2048},
            {"operator": "OR", "conditions": [{"from": "ana"}, {"from": "zoe park"}]},
        ]}))
    );
}

/// Issue #4: mail past what's kept is found on the server. A folder keeps its newest 10 of
/// 30; searching the server finds an old one by a word of its body (and one by a word in
/// UTF-8, sent as a literal), brings it into the list, and then shows more fills in the
/// newest of the rest without skipping what's around the found one.
#[test]
fn mail_past_whats_kept_is_found_and_shown_more() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        for n in 1..=30 {
            let body = if n == 3 { "Grüße und needle-three".to_string() } else { format!("body {n}") };
            s.add_raw("INBOX", format!("Subject: Message {n}\r\nMessage-ID: <m{n}@x>\r\n\r\n{body}\r\n").into_bytes());
        }
    }
    let store = store("past-kept");
    let mut c = login(&st);
    sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions { window: 10, prefetch: 0, ..Default::default() }).unwrap();
    assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 10);

    let criteria = crate::sync::imap_criteria("needle-three").unwrap();
    let (keys, new) = crate::sync::find_on_server(&mut c, &store, "a1", "INBOX", &criteria, 500).unwrap();
    assert_eq!((keys.iter().map(|k| k.uid).collect::<Vec<_>>(), new), (vec![3], 1));
    let kept = store.load_folder("a1", "INBOX").unwrap();
    assert!(kept.messages.iter().any(|m| m.subject == "Message 3") && kept.messages.len() == 11);
    // Found again: nothing new to bring in.
    assert_eq!(crate::sync::find_on_server(&mut c, &store, "a1", "INBOX", &criteria, 500).unwrap(), (keys.clone(), 0));
    // A word in UTF-8: sent as a literal with CHARSET UTF-8.
    let (keys, _) = crate::sync::find_on_server(&mut c, &store, "a1", "INBOX", &crate::sync::imap_criteria("Grüße").unwrap(), 500).unwrap();
    assert_eq!(keys.iter().map(|k| k.uid).collect::<Vec<_>>(), [3]);
    assert!(st.lock().unwrap().log.iter().any(|l| l.ends_with("UID SEARCH CHARSET UTF-8 TEXT ")), "a literal follows");
    // Nothing found: nothing changes.
    assert_eq!(crate::sync::find_on_server(&mut c, &store, "a1", "INBOX", &crate::sync::imap_criteria("absent").unwrap(), 500).unwrap(), (vec![], 0));

    // Show more: 10 more, the newest of what isn't kept (20 down to 11), the found one kept.
    sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions { window: 20, prefetch: 0, ..Default::default() }).unwrap();
    let mut uids: Vec<u32> = store.load_folder("a1", "INBOX").unwrap().messages.iter().map(|m| m.key.uid).collect();
    uids.sort_unstable();
    assert_eq!(uids, [3].into_iter().chain(12..=30).collect::<Vec<u32>>(), "the newest of the rest, up to 20 in all");
    // The usual window again: nothing is dropped.
    sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions { window: 10, prefetch: 0, ..Default::default() }).unwrap();
    assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 20);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Issue #4 through the worker: Show more keeps the folder's larger window across syncs
/// (windows.json), and a server search answers with what it found, in the list.
#[test]
fn show_more_and_search_the_server_through_the_worker() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        for n in 1..=25 {
            s.add_raw("INBOX", format!("Subject: Message {n}\r\nMessage-ID: <w{n}@x>\r\n\r\nbody needle-{n}\r\n").into_bytes());
        }
    }
    let port = serve_tcp(&st);
    let store = store("worker-more");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::SetSync { messages: 10, offline: 0 });
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
    assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 10);

    pool.submit(Job::SearchServer { account: "a1".into(), folders: vec!["INBOX".into()], query: "needle-2".into() });
    let seen = until(&pool, "searched", |e| matches!(e, Event::ServerSearched { .. }));
    let found = seen.iter().find_map(|e| match e {
        Event::ServerSearched { keys, error: None, query, .. } if query == "needle-2" => Some(keys.iter().map(|k| k.uid).collect::<Vec<_>>()),
        _ => None,
    });
    // "needle-2" is the words "needle" and "2", each found anywhere as IMAP's TEXT finds them
    // (so "needle-12" too).
    assert_eq!(found, Some(vec![2, 12, 20, 21, 22, 23, 24, 25]), "{seen:?}");
    assert!(seen.iter().any(|e| matches!(e, Event::FolderChanged { folder, .. } if folder == "INBOX")));
    assert!(store.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.key.uid == 2));

    pool.submit(Job::MoreMail { account: "a1".into(), folder: "INBOX".into(), more: 5 });
    let end = Instant::now() + Duration::from_secs(20);
    while store.load_folder("a1", "INBOX").unwrap().messages.len() < 17 {
        assert!(Instant::now() < end, "never showed more");
        let _ = pool.wait(Duration::from_millis(100));
    }
    let windows: std::collections::HashMap<String, usize> = store.load_account_doc("a1", "windows.json").unwrap();
    assert_eq!(windows.get("INBOX"), Some(&17), "12 kept (10, and 2 found), 5 more");
    // A later sync keeps them.
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    until(&pool, "synced", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 17);
    pool.submit(Job::Remove { account: "a1".into() });
    // Offline: answered at once.
    pool.submit(Job::SearchServer { account: "a1".into(), folders: vec!["INBOX".into()], query: "x".into() });
    until(&pool, "offline answer", |e| matches!(e, Event::ServerSearched { error: Some(_), .. }));
    let _ = std::fs::remove_dir_all(store.root());
}

/// Mail past what's kept is found on the server: a folder keeps its newest 10 of 30, and a
/// word of the third's body brings it into the folder's cache, headers and all, once.
#[test]
fn the_server_finds_mail_past_whats_kept() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        for n in 1..=30 {
            let body = if n == 3 { "Grüße und needle-three".to_string() } else { format!("body {n}") };
            s.add_raw("INBOX", format!("Subject: Message {n}\r\nMessage-ID: <m{n}@x>\r\n\r\n{body}\r\n").into_bytes());
        }
    }
    let store = store("found-past-kept");
    let mut c = login(&st);
    sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions { window: 10, prefetch: 0, ..Default::default() }).unwrap();
    assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 10);

    let criteria = crate::sync::imap_criteria("needle-three").unwrap();
    let found = crate::sync::search_on_server(&mut c, &store, "a1", "INBOX", &criteria, 500).unwrap();
    assert_eq!(found.iter().map(|m| m.key.uid).collect::<Vec<_>>(), [3]);
    assert_eq!(found.first().map(|m| m.subject.as_str()), Some("Message 3"), "its headers came too");
    let kept = store.load_folder("a1", "INBOX").unwrap();
    assert!(kept.messages.iter().any(|m| m.subject == "Message 3") && kept.messages.len() == 11);
    // Found again: it's here now, so it's read from the cache rather than fetched again.
    let again = crate::sync::search_on_server(&mut c, &store, "a1", "INBOX", &criteria, 500).unwrap();
    assert_eq!(again.iter().map(|m| m.key.uid).collect::<Vec<_>>(), [3]);
    // A word in UTF-8: sent as a literal with CHARSET UTF-8.
    let found = crate::sync::search_on_server(&mut c, &store, "a1", "INBOX", &crate::sync::imap_criteria("Grüße").unwrap(), 500).unwrap();
    assert_eq!(found.iter().map(|m| m.key.uid).collect::<Vec<_>>(), [3]);
    assert!(st.lock().unwrap().log.iter().any(|l| l.ends_with("UID SEARCH CHARSET UTF-8 TEXT ")), "a literal follows");
    // Nothing found: nothing changes.
    assert!(crate::sync::search_on_server(&mut c, &store, "a1", "INBOX", &crate::sync::imap_criteria("absent").unwrap(), 500).unwrap().is_empty());
    // The newest of a word found anywhere: only the newest `cap` are kept.
    let many = crate::sync::search_on_server(&mut c, &store, "a1", "INBOX", &crate::sync::imap_criteria("body").unwrap(), 3).unwrap();
    assert_eq!(many.len(), 3);
    assert_eq!(many.iter().map(|m| m.key.uid).collect::<Vec<_>>(), [28, 29, 30]);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Through the worker: a search asks the IMAP server (`SEARCH`) and its finds arrive as
/// `Event::Found`, with headers; Show more keeps the folder's larger window across syncs
/// (`windows.json`).
#[test]
fn a_search_asks_the_imap_server_and_show_more_keeps() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        for n in 1..=25 {
            s.add_raw("INBOX", format!("Subject: Message {n}\r\nMessage-ID: <w{n}@x>\r\n\r\nbody needle-{n}\r\n").into_bytes());
        }
    }
    let port = serve_tcp(&st);
    let store = store("worker-search");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::SetSync { messages: 10, offline: 0 });
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
    assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 10);

    pool.submit(Job::Search { account: "a1".into(), query: "needle-2".into(), folder: None });
    let seen = until(&pool, "found", |e| matches!(e, Event::Found { .. }));
    let found = seen.iter().find_map(|e| match e {
        Event::Found { query, items, error: None, .. } if query == "needle-2" => Some(items.iter().map(|m| m.key.uid).collect::<Vec<_>>()),
        _ => None,
    });
    // "needle-2" is the words "needle" and "2", each found anywhere as IMAP's TEXT finds
    // them (so "needle-12" too), newest first.
    assert_eq!(found, Some(vec![25, 24, 23, 22, 21, 20, 12, 2]), "{seen:?}");
    assert!(store.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.key.uid == 2), "kept, to be opened");

    // Show more: 8 more than the folder keeps now (the 10, and the 2 the search brought in).
    let kept = store.load_folder("a1", "INBOX").unwrap().messages.len();
    assert_eq!(kept, 12, "the window of 10, and two found older");
    pool.submit(Job::MoreMail { account: "a1".into(), folder: "INBOX".into(), more: 8 });
    let end = Instant::now() + Duration::from_secs(20);
    while store.load_folder("a1", "INBOX").unwrap().messages.len() < kept + 8 {
        assert!(Instant::now() < end, "never showed more");
        let _ = pool.wait(Duration::from_millis(100));
    }
    let windows: std::collections::HashMap<String, usize> = store.load_account_doc("a1", "windows.json").unwrap();
    assert_eq!(windows.get("INBOX"), Some(&(kept + 8)), "12 kept, 8 more");
    // A later sync keeps them.
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    until(&pool, "synced", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert!(store.load_folder("a1", "INBOX").unwrap().messages.len() >= kept + 8);
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}
