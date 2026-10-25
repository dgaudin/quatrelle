//! Engine tests: search, in the index kept here and on the servers.

use super::*;

#[test]
fn what_the_server_finds_joins_what_the_index_found() {
    let mut s = sample_session("server-search");
    assert_eq!(s.run("search.run", &json!({"query": "deepseek"})).unwrap()["results"], 0);
    // What Exchange answered: a message whose text is on the server only (an old one), so it
    // never reached the index. It's shown with the others, as Outlook merges them.
    let key = MessageKey { account: "sample".into(), folder: "Archive".into(), validity: 1, uid: 9_001 };
    let mut m = Summary {
        key: key.clone(),
        subject: "Deepseek notes".into(),
        from: EmailAddress { name: "Ada".into(), email: "ada@example.org".into() },
        date: 1_107_248_400,
        ..Default::default()
    };
    m.preview = "we tried deepseek in the lab".into();
    s.apply(Event::Found { account: "sample".into(), query: "deepseek".into(), items: vec![m], error: None });
    assert_eq!(s.list.len(), 1);
    assert_eq!(s.list[0].subject, "Deepseek notes");
    assert_eq!(s.summary(&key).map(|m| m.subject.as_str()), Some("Deepseek notes"), "it can be opened from there");
    // Found twice (the index and the server): one row.
    let again = Summary::clone(&s.list[0]);
    s.apply(Event::Found { account: "sample".into(), query: "deepseek".into(), items: vec![again], error: None });
    assert_eq!(s.list.len(), 1);
    // Another search leaves them out.
    s.run("search.run", &json!({"query": "projector"})).unwrap();
    assert!(!s.list.iter().any(|m| m.subject == "Deepseek notes"));
    done(s);
}

/// An IMAP account is asked too: its server's `SEARCH` finds what this computer doesn't keep
/// (the words of a message past the window, or of a body never downloaded), as Exchange's index
/// does for an Exchange account. The sample mailbox, kept here, isn't.
#[test]
fn a_search_asks_an_imap_account_as_well() {
    let mut s = sample_session("imap-search");
    let a = s
        .run("account.add", &json!({"email": "zoe@example.org", "password": "pw", "imapHost": "mail.example.org", "smtpHost": "mail.example.org"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    s.run("search.run", &json!({"query": "needle", "scope": "all"})).unwrap();
    let jobs = s.take_jobs();
    assert!(
        jobs.iter().any(|j| matches!(j, Job::Search { account, query, folder: None } if *account == a && query == "needle")),
        "the account's server is asked for every folder: {jobs:?}"
    );
    assert!(!jobs.iter().any(|j| matches!(j, Job::Search { account, .. } if account == "sample")), "kept here: not asked");
    // A search of the folder on view asks nothing of it while this computer's sample mailbox is
    // the one open; it asks that folder alone once a folder of the account is.
    s.run("search.clear", &json!({})).unwrap();
    s.run("search.run", &json!({"query": "needle", "scope": "current"})).unwrap();
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Search { account, .. } if *account == a)), "the sample is on view");
    done(s);
}

/// A big mailbox's search is indexed a slice at a time: each call answers with what it has and
/// says whether more is coming, so the window keeps drawing between the calls instead of
/// freezing while the whole mailbox is read.
#[test]
fn a_search_is_indexed_a_slice_at_a_time() {
    let store = temp("slice-search");
    store.save_accounts(&[quatrelle_model::AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
    store
        .save_folders(
            "a1",
            &[quatrelle_model::Folder {
                account: "a1".into(),
                path: "INBOX".into(),
                role: FolderRole::Inbox,
                selectable: true,
                ..Default::default()
            }],
        )
        .unwrap();
    let mut st = store.load_folder("a1", "INBOX").unwrap();
    for uid in 1..=60u32 {
        let key = quatrelle_model::MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        st.messages.push(quatrelle_model::Summary { key, subject: format!("Report {uid}"), ..Default::default() });
    }
    st.messages[59].subject = "needle in the last one".into();
    store.save_folder("a1", "INBOX", &st).unwrap();
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    // Ten messages a call: the first answers with what it has indexed and says more is coming.
    let v = s.run("search.run", &json!({"query": "needle", "index": 10})).unwrap();
    assert_eq!(v["pending"], true, "a slice is left: {v}");
    assert_eq!(v["results"], 0, "the matching message isn't indexed yet: {v}");
    // The window keeps asking, a slice a frame, until the index is done.
    let mut calls = 0;
    loop {
        let v = s.run("search.run", &json!({"query": "needle", "index": 10})).unwrap();
        calls += 1;
        if v["pending"] == false {
            assert_eq!(v["results"], 1, "the match is found once the index is done: {v}");
            break;
        }
        assert!(calls < 100, "the scan must finish");
    }
    // A later search of the same scope uses the index it has: nothing left to do.
    let v = s.run("search.run", &json!({"query": "report", "index": 10})).unwrap();
    assert_eq!(v["pending"], false, "the index is built: {v}");
    assert!(v["results"].as_u64().unwrap_or(0) > 0, "the whole folder is searchable: {v}");
    let _ = std::fs::remove_dir_all(store.root());
}

/// A folder's index is kept when the folder is read: the next run searches it again without
/// reading the messages (with the bodies out of the cache, only the kept index finds the words).
#[test]
fn a_search_index_is_kept_for_the_next_run() {
    let store = temp("kept-index");
    store.save_accounts(&[quatrelle_model::AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
    store
        .save_folders(
            "a1",
            &[quatrelle_model::Folder {
                account: "a1".into(),
                path: "INBOX".into(),
                role: FolderRole::Inbox,
                selectable: true,
                ..Default::default()
            }],
        )
        .unwrap();
    let mut st = store.load_folder("a1", "INBOX").unwrap();
    st.validity = 1;
    for uid in 1..=40u32 {
        let key = quatrelle_model::MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        st.messages.push(quatrelle_model::Summary { key, subject: format!("Report {uid}"), ..Default::default() });
    }
    // A word only the body holds: the summary alone can't find it.
    let key = st.messages[39].key.clone();
    store.write_body(&key, b"From: a@x.example\r\nSubject: Report 40\r\n\r\nwatermelon in the body\r\n").unwrap();
    store.save_folder("a1", "INBOX", &st).unwrap();
    {
        let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
        s.take_jobs();
        assert_eq!(s.run("search.run", &json!({"query": "watermelon"})).unwrap()["results"], 1);
        assert!(store.load_search_index("a1", "INBOX").is_some(), "the folder's index is kept");
    }
    // The body goes (a cache that let go of it), and the next run still finds the word.
    store.remove_body(&key);
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    let v = s.run("search.run", &json!({"query": "watermelon"})).unwrap();
    assert_eq!(v["results"], 1, "found in the kept index, not by reading the messages: {v}");
    assert_eq!(v["pending"], false);
    assert_eq!(s.run("search.run", &json!({"query": "report"})).unwrap()["results"], 40);
    done(s);
}

/// A body that arrives after the index was kept joins it: the same run finds it, and the next
/// run reads that body only (the folder itself isn't read again).
#[test]
fn a_downloaded_body_joins_a_kept_index() {
    let store = temp("kept-delta");
    store.save_accounts(&[quatrelle_model::AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
    store
        .save_folders(
            "a1",
            &[quatrelle_model::Folder {
                account: "a1".into(),
                path: "INBOX".into(),
                role: FolderRole::Inbox,
                selectable: true,
                ..Default::default()
            }],
        )
        .unwrap();
    let mut st = store.load_folder("a1", "INBOX").unwrap();
    st.validity = 1;
    for uid in 1..=20u32 {
        let key = quatrelle_model::MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        st.messages.push(quatrelle_model::Summary { key, subject: format!("Report {uid}"), ..Default::default() });
    }
    let key = st.messages[7].key.clone();
    store.save_folder("a1", "INBOX", &st).unwrap();
    {
        let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
        s.take_jobs();
        assert_eq!(s.run("search.run", &json!({"query": "report"})).unwrap()["results"], 20);
        // The body comes in, as "Download all" brings it.
        let raw = b"From: a@x.example\r\nSubject: Report 8\r\n\r\nwatermelon in the body\r\n".to_vec();
        store.write_body(&key, &raw).unwrap();
        s.apply(Event::Body { key: key.clone(), raw });
        assert_eq!(s.run("search.run", &json!({"query": "watermelon"})).unwrap()["results"], 1);
    }
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    let v = s.run("search.run", &json!({"query": "watermelon"})).unwrap();
    assert_eq!(v["results"], 1, "the body read since is added to the kept index: {v}");
    done(s);
}

/// A folder that changed since its index was kept is read again, on its own: the messages that
/// arrived are searchable, and nothing else is read.
#[test]
fn a_changed_folder_is_indexed_again() {
    let store = temp("kept-change");
    store.save_accounts(&[quatrelle_model::AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
    store
        .save_folders(
            "a1",
            &[quatrelle_model::Folder {
                account: "a1".into(),
                path: "INBOX".into(),
                role: FolderRole::Inbox,
                selectable: true,
                ..Default::default()
            }],
        )
        .unwrap();
    let mut st = store.load_folder("a1", "INBOX").unwrap();
    st.validity = 1;
    for uid in 1..=20u32 {
        let key = quatrelle_model::MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        st.messages.push(quatrelle_model::Summary { key, subject: format!("Report {uid}"), ..Default::default() });
    }
    store.save_folder("a1", "INBOX", &st).unwrap();
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    assert_eq!(s.run("search.run", &json!({"query": "report"})).unwrap()["results"], 20);
    // Mail arrives while the window is open: the folder is read again (the index no longer fits
    // it), and the new subject is searchable.
    let mut st = store.load_folder("a1", "INBOX").unwrap();
    let key = quatrelle_model::MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 21 };
    st.messages.push(quatrelle_model::Summary { key, subject: "kitchen sink".into(), ..Default::default() });
    store.save_folder("a1", "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: "a1".into(), folder: "INBOX".into() });
    assert_eq!(s.run("search.run", &json!({"query": "kitchen"})).unwrap()["results"], 1);
    assert_eq!(s.run("search.run", &json!({"query": "report"})).unwrap()["results"], 20);
    done(s);
}

/// The language model set in Options summarizes the message being read: the job carries its
/// text; the answer (or why there's none) is kept for that question only.
#[test]
fn the_assistant_summarizes_the_message_being_read() {
    let mut s = sample_session("assistant");
    assert!(s.run("assistant.summarize", &json!({})).is_err(), "no model set");
    assert!(s.run("assistant.settings", &json!({"url": "ftp://x"})).is_err());
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "llama3", "key": "k1"})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SetLlmKey { key } if key.0 == "k1")), "the key goes to the keychain");
    assert_eq!((s.prefs.llm_url.as_str(), s.prefs.llm_model.as_str()), ("http://localhost:11434/v1", "llama3"));
    assert!(s.run("assistant.summarize", &json!({})).is_err(), "no message open");
    let key = s.list.first().map(|m| m.key.clone()).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    s.take_jobs();
    let id = s.run("assistant.summarize", &json!({})).unwrap()["asked"].as_u64().unwrap();
    let asked = s.take_jobs().into_iter().find_map(|j| match j {
        Job::AskLlm { id, model, messages, .. } => Some((id, model, messages)),
        _ => None,
    });
    let (job_id, model, messages) = asked.unwrap();
    assert_eq!((job_id, model.as_str(), messages.len()), (id, "llama3", 2));
    assert!(messages[1].1.contains("Subject: "), "{:?}", messages[1]);
    assert_eq!(s.assistant.as_ref().map(|a| (a.key.clone(), a.answer.is_none())), Some((key.clone(), true)));
    // An answer to an older question is dropped; this one's kept.
    s.apply(Event::LlmAnswered { id: id + 1000, text: "old".into(), error: None });
    assert!(s.assistant.as_ref().unwrap().answer.is_none());
    s.apply(Event::LlmAnswered { id, text: "- Review on Thursday".into(), error: None });
    assert_eq!(s.assistant.as_ref().unwrap().answer, Some(Ok("- Review on Thursday".to_string())));
    // No model any more: nothing shown.
    s.run("assistant.settings", &json!({"url": ""})).unwrap();
    assert!(s.assistant.is_none());
}
