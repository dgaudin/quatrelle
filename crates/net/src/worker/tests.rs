//! Tests of the pool and the workers' bookkeeping.

use super::*;

/// A full sync walks the Inbox first, then what mail is about, then the rest, the archive and
/// the public folders followed last: a mailbox of public folders and years of archives must not
/// hold the Inbox back, nor the user's folders named after "Public Folders". The public folders
/// not followed aren't walked: they're listed when opened.
#[test]
fn a_full_sync_starts_with_the_inbox() {
    use quatrelle_model::FolderRole;
    let f = |path: &str, role: FolderRole| Folder {
        account: "a1".into(),
        path: path.into(),
        delimiter: Some('/'),
        role,
        selectable: true,
        unread: 0,
        total: 0,
        name: None,
        mailbox: None,
        mailbox_role: quatrelle_model::FolderRole::Other,
    };
    let folders = vec![
        f("ASP", FolderRole::Other),
        f("Archives/2025", FolderRole::Archive),
        f("Public Folders", FolderRole::Other),
        f("Public Folders/Reports", FolderRole::Other),
        f("Public Folders/Team", FolderRole::Other),
        f("Support", FolderRole::Other),
        f("Éléments envoyés", FolderRole::Sent),
        f("Brouillons", FolderRole::Drafts),
        f("Corbeille", FolderRole::Trash),
        f("INBOX", FolderRole::Inbox),
        Folder { selectable: false, ..f("Conteneur", FolderRole::Other) },
    ];
    assert_eq!(
        sync_order(&folders, |f| !is_public_folder(&f.path) || f.path == "Public Folders/Team"),
        ["INBOX", "Brouillons", "Éléments envoyés", "Corbeille", "ASP", "Support", "Archives/2025", "Public Folders/Team"],
        "Inbox, drafts, sent, trash, the rest (by name), the archive, the public folder followed; no containers"
    );
}

/// Sent Items asked for after each of several messages is read once.
#[test]
fn a_folder_waiting_to_be_read_isnt_queued_twice() {
    let mut q = VecDeque::new();
    let sent = Job::SyncFolder { account: "a".into(), folder: "Sent".into() };
    push_job(&mut q, sent.clone());
    push_job(&mut q, Job::SyncFolder { account: "a".into(), folder: "INBOX".into() });
    push_job(&mut q, sent.clone());
    push_job(&mut q, Job::GetVacation { account: "a".into() });
    push_job(&mut q, Job::GetVacation { account: "a".into() });
    assert_eq!(q.len(), 4, "only the folder read is merged");
}

/// A dozen calendars ticked, or tasks changed, in a row each ask for the calendars' and contacts'
/// sync: one waiting reads them all (it reads what's kept when it starts), where each ran a full
/// pass in turn, half a minute each, for a quarter of an hour (28/10).
#[test]
fn a_calendar_sync_waiting_isnt_queued_twice() {
    let mut q = VecDeque::new();
    let dav = Job::SyncDav { account: "a".into() };
    push_job(&mut q, dav.clone());
    push_job(&mut q, Job::SyncFolder { account: "a".into(), folder: "INBOX".into() });
    push_job(&mut q, dav.clone());
    push_job(&mut q, dav.clone());
    assert_eq!(q.len(), 2, "{q:?}");
    // Another account's is its own.
    push_job(&mut q, Job::SyncDav { account: "b".into() });
    assert_eq!(q.len(), 3);
}

/// A server's rules split by whose they are: the ones Quatrelle holds the id of, and the
/// ones it didn't make (Outlook's own, the server's web page).
#[test]
fn server_rules_split_by_whose_they_are() {
    let rule = |name: &str, id: &str| quatrelle_model::organize::ForeignRule {
        account: String::new(),
        id: id.into(),
        name: name.into(),
        enabled: true,
        runnable: false,
        rule: None,
        summary: String::new(),
    };
    let (mine, others) = split_rules(vec![rule("Outlook rule", "R9"), rule("Invoices", "R1")], &["R1".to_string()]);
    assert_eq!(mine, vec![("Invoices".to_string(), "R1".to_string())]);
    assert_eq!(others.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Outlook rule"]);
    let (mine, others) = split_rules(vec![rule("Theirs", "R2")], &[]);
    assert!(mine.is_empty());
    assert_eq!(others.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Theirs"], "nothing of ours there: all of it is theirs");
}

/// Automatic replies are answered between two folders of a long first sync, not after it
/// (File › Info said "Checking with the server…" all the while).
#[test]
fn automatic_replies_jump_the_full_syncs_queue() {
    assert!(jumps_queue(&Job::GetVacation { account: "a".into() }));
    assert!(jumps_queue(&Job::SetVacation { account: "a".into(), vacation: Default::default() }));
    assert!(jumps_queue(&Job::SyncDav { account: "a".into() }));
    assert!(jumps_queue(&Job::SyncFolder { account: "a".into(), folder: "INBOX".into() }));
    assert!(!jumps_queue(&Job::SyncAll { account: "a".into() }));
}

#[test]
fn memory_credentials() {
    let m = MemoryCredentials::default();
    let a = AccountConfig { id: "a1".into(), ..Default::default() };
    assert_eq!(m.get(&a), None);
    m.set(&a, "pw").unwrap();
    assert_eq!(m.get(&a).as_deref(), Some("pw"));
    m.delete(&a);
    assert_eq!(m.get(&a), None);
}

/// Microsoft can take a token and then refuse the mailbox ("User is authenticated but not
/// connected"). The address the browser signed in as is tried; otherwise it's a refusal that
/// says what to do, not a sign-in to ask for again.
#[test]
fn a_mailbox_microsoft_wont_open_says_why() {
    let st = crate::fake::State::new();
    st.lock().unwrap().not_connected.push("zoe@hotmail.example".into());
    let port = crate::fake::serve_tcp(&st);
    let store = crate::fake::tests_store("closed-mailbox");
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut acc = AccountConfig {
        id: "a1".into(),
        email: "zoe@hotmail.example".into(),
        imap: quatrelle_model::ServerConfig { host: "127.0.0.1".into(), port, security: quatrelle_model::Security::None },
        oauth: Some("microsoft".into()),
        ..Default::default()
    };
    let w = Worker::new(acc.clone(), Auth::shared("refresh"), store.clone(), pool.sink.clone(), SyncOptions::default());
    let e = w.imap_with_token("zoe@hotmail.example", crate::fake::GOOD_TOKEN).err().unwrap();
    assert!(e.to_string().starts_with(quatrelle_model::MICROSOFT_IMAP_CLOSED) && e.to_string().contains("IMAP"), "{e:?}");
    // The browser signed in as the mailbox's own address: that one opens it.
    acc.oauth_user = Some("me@studio.example".into());
    let w = Worker::new(acc.clone(), Auth::shared("refresh"), store.clone(), pool.sink.clone(), SyncOptions::default());
    assert!(w.imap_with_token("zoe@hotmail.example", crate::fake::GOOD_TOKEN).is_ok());
    // Signed in as another account that can't open it either: named in the refusal.
    acc.oauth_user = Some("someone@else.example".into());
    let w = Worker::new(acc, Auth::shared("refresh"), store, pool.sink.clone(), SyncOptions::default());
    let e = w.imap_with_token("zoe@hotmail.example", crate::fake::GOOD_TOKEN).err().unwrap();
    assert!(matches!(&e, NetError::Refused(m) if m.contains("signed in as someone@else.example")), "{e:?}");
    // A token that's simply refused is still a sign-in to ask for again.
    let w = Worker::new(
        AccountConfig { oauth_user: None, ..w.acc.clone() },
        Auth::shared("refresh"),
        crate::fake::tests_store("closed-mailbox-2"),
        pool.sink.clone(),
        SyncOptions::default(),
    );
    assert!(matches!(w.imap_with_token("zoe@hotmail.example", "expired"), Err(NetError::Auth(_))));
}

/// A Quatrelle data file (`.quatrelle`) comes in through the same job as a `.pst`.
#[test]
fn a_quatrelle_data_file_is_imported() {
    let t = std::env::temp_dir().join(format!("quatrelle-worker-datafile-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    let old = Store::open_packed(t.join("old"), t.join("Old Files")).unwrap();
    old.save_accounts(&[AccountConfig { id: "a1".into(), email: "zoe@studio.example".into(), ..Default::default() }]).unwrap();
    old.save_folders("a1", &[Folder { path: "INBOX".into(), role: FolderRole::Inbox, ..Default::default() }]).unwrap();
    let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 3 };
    let mut st = old.load_folder("a1", "INBOX").unwrap();
    st.messages.push(quatrelle_model::Summary { key: k, subject: "Kept".into(), ..Default::default() });
    old.save_folder("a1", "INBOX", &st).unwrap();
    let file = old.data_file("a1").unwrap();
    drop(old);
    let store = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    let mut p = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    p.submit(Job::ImportPst { account: "p1".into(), path: file.to_string_lossy().into_owned() });
    let end = Instant::now() + Duration::from_secs(20);
    let mut seen = Vec::new();
    while Instant::now() < end && !seen.iter().any(|e| matches!(e, Event::Imported { done: true, .. })) {
        seen.extend(p.wait(Duration::from_millis(100)));
    }
    assert!(seen.iter().any(|e| matches!(e, Event::Imported { messages: 1, done: true, error: None, .. })), "{seen:?}");
    assert!(seen.iter().any(|e| matches!(e, Event::Folders { folders, .. } if folders.len() == 1)));
    assert_eq!(store.load_folder("p1", "INBOX").unwrap().messages[0].key.account, "p1");
    let _ = std::fs::remove_dir_all(&t);
}

#[test]
fn pool_without_password_asks_for_one() {
    let dir = std::env::temp_dir().join(format!("quatrelle-pool-{}", std::process::id()));
    let store = Store::open(&dir).unwrap();
    let mut p = Pool::new(store, Arc::new(MemoryCredentials::default()));
    let a = AccountConfig { id: "a1".into(), ..Default::default() };
    p.submit(Job::Start { account: a, password: None });
    assert!(matches!(p.poll().as_slice(), [Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }]));
    p.submit(Job::SyncAll { account: "a1".into() });
    assert!(matches!(p.poll().as_slice(), [Event::Notice { .. }]));
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    p.submit(Job::FetchBody { key: key.clone() });
    assert!(matches!(p.poll().as_slice(), [Event::BodyFailed { .. }]));
    p.store.write_body(&key, b"Subject: cached\r\n\r\nx").unwrap();
    p.submit(Job::FetchBody { key });
    assert!(matches!(p.poll().as_slice(), [Event::Body { .. }]));
    let _ = std::fs::remove_dir_all(dir);
}

/// Mail downloaded ahead never fills the disk: the larger of 5 GB and a fiftieth of it stays
/// free; and the disk looked at is the one the mail is kept on.
#[test]
fn downloading_ahead_leaves_room_on_the_disk() {
    const GB: u64 = 1 << 30;
    assert!(ahead::enough_room(26 * GB, 394 * GB), "the owner's disk, 26 GB free of 394");
    assert!(!ahead::enough_room(7 * GB, 394 * GB), "less than a fiftieth of a big disk");
    assert!(!ahead::enough_room(4 * GB, 100 * GB), "less than 5 GB of a small one");
    assert!(ahead::enough_room(6 * GB, 100 * GB));
    let dir = std::env::temp_dir().join(format!("quatrelle-room-{}", std::process::id()));
    let packed = Store::open_packed(dir.join("app"), dir.join("Files")).unwrap();
    assert_eq!(packed.data_dir(), Some(dir.join("Files")), "a data file's folder");
    assert_eq!(Store::open(dir.join("plain")).unwrap().data_dir(), Some(dir.join("plain")));
    assert_eq!(Store::memory().data_dir(), None);
    assert!(fs4::statvfs(dir.join("Files")).is_ok_and(|s| s.total_space() > 0), "its disk is read");
    let _ = std::fs::remove_dir_all(dir);
}
