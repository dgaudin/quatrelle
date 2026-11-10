// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Tests of the pool and the workers' bookkeeping.

use super::*;

mod drafts;

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
        archive: false,
        archive_tag: None,
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
    // Calendars just ticked: read by the sync waiting, else together.
    push_job(&mut q, Job::SyncCalendars { account: "a".into(), calendars: vec!["shared:ana@contoso.example".into()] });
    assert_eq!(q.len(), 3, "the sync waiting reads them");
    let mut q = VecDeque::new();
    let ticked = |c: &str| Job::SyncCalendars { account: "a".into(), calendars: vec![c.to_string()] };
    push_job(&mut q, ticked("shared:ana@contoso.example"));
    push_job(&mut q, ticked("shared:tom@contoso.example"));
    push_job(&mut q, ticked("shared:ana@contoso.example"));
    let both = Job::SyncCalendars { account: "a".into(), calendars: vec!["shared:ana@contoso.example".into(), "shared:tom@contoso.example".into()] };
    assert_eq!(q, [both]);
    // The calendar opened (every calendar ticked) takes them all in, before or after.
    let opened = Job::SyncCalendars { account: "a".into(), calendars: Vec::new() };
    push_job(&mut q, opened.clone());
    push_job(&mut q, ticked("shared:eve@contoso.example"));
    assert_eq!(q, [opened]);
}

/// A colleague's calendar just ticked goes on the second connection (on-premises Exchange), not
/// behind the main one's sync (15 s, 29/10); the calendar opened (every calendar ticked) stays on
/// the main one.
#[test]
fn a_calendar_ticked_goes_on_the_second_connection() {
    let ews = AccountConfig { ews: Some("https://mail.contoso.example/EWS/Exchange.asmx".into()), ..Default::default() };
    let ticked = Job::SyncCalendars { account: "a".into(), calendars: vec!["shared:ana@contoso.example".into()] };
    let opened = Job::SyncCalendars { account: "a".into(), calendars: Vec::new() };
    assert!(quick_job(&ticked, &ews));
    assert!(!quick_job(&opened, &ews));
    assert!(!quick_job(&ticked, &AccountConfig::default()), "not over CalDAV");
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

/// A server on a site the account doesn't trust, met while connecting: the account waits for
/// the user's answer (`NeedsTrust`), as for a password. No back-off, and no try again by
/// itself: it starts again once answered.
#[test]
fn untrusted_at_connect_needs_trust_with_no_backoff() {
    let store = crate::fake::tests_store("untrusted-connect");
    let p = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc =
        AccountConfig { id: "a1".into(), email: "riley@contoso.example".into(), ews: Some(crate::ews::AUTODISCOVER.into()), ..Default::default() };
    let mut w = Worker::new(acc, Auth::shared("pw"), store, p.sink.clone(), SyncOptions::default());
    let (backoff, next) = (w.backoff, w.next_attempt);
    w.connect_failed(NetError::Untrusted { site: "office365.com".into() });
    assert!(w.conn.is_none() && w.auth_failed, "waits to be started again");
    assert_eq!((w.backoff, w.next_attempt), (backoff, next), "no back-off");
    assert!(
        matches!(p.poll().as_slice(), [Event::Status { status: SyncStatus::NeedsTrust { site }, .. }] if site == "office365.com"),
        "the question is the account's status"
    );
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
    assert!(ahead::enough_room(26 * GB, 394 * GB), "a nearly full disk, 26 GB free of 394");
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

/// A new mail told of here, set beside when it came to the server by the server's clock, that
/// clock behind or ahead of this computer's.
#[test]
fn how_long_after_a_new_mail_came_it_was_told_of() {
    let came = 1_793_273_424;
    assert!((listening::told_after(came, 0, 1_793_273_427_500) - 3.5).abs() < 1e-9);
    // The server's clock 2 s ahead: it came at 22 by this one's.
    assert!((listening::told_after(came, 2, 1_793_273_424_000) - 2.0).abs() < 1e-9);
    // Hostile times give a number, never a panic.
    assert!(listening::told_after(i64::MIN, i64::MAX, u128::MAX).is_finite());
}

/// The calendars' and contacts' homes saved in the account's PIM doc are trusted: on another
/// site, they still sync, with no question. A site the server leads to that the account doesn't
/// trust is asked about once, and the PIM sync then waits for the account's next start.
#[test]
fn dav_trusts_the_saved_homes_and_asks_once_about_another_site() {
    use quatrelle_model::pim::{PIM_DOC, Pim, Tombstone};

    use crate::fakes::fake_dav;
    let s = fake_dav::serve();
    let there = s.base.replace("127.0.0.1", "localhost");
    let store = crate::fake::tests_store("dav-trust");
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc = AccountConfig {
        id: "a1".into(),
        email: fake_dav::USER.into(),
        imap: quatrelle_model::ServerConfig { host: "127.0.0.1".into(), port: 1, security: quatrelle_model::Security::None },
        ..Default::default()
    };
    let events = |pool: &Pool| std::iter::from_fn(|| pool.wait(Duration::from_millis(100))).collect::<Vec<_>>();
    let worker = || Worker::new(acc.clone(), Auth::shared(fake_dav::PASSWORD), store.clone(), pool.sink.clone(), SyncOptions::default());

    // Homes found and saved before on `localhost`, a site the account's settings don't name.
    let saved = Pim { caldav_home: Some(format!("{there}/dav/cal/")), carddav_home: Some(format!("{there}/dav/card/")), ..Default::default() };
    store.save_account_doc("a1", PIM_DOC, &saved).unwrap();
    worker().sync_dav();
    let seen = events(&pool);
    assert!(seen.iter().any(|e| matches!(e, Event::DavSynced { .. })), "{seen:?}");
    assert!(!seen.iter().any(|e| matches!(e, Event::TrustAsked { .. })), "{seen:?}");

    // Homes on the account's own server, and a deletion on `localhost`, which nothing trusts.
    let pim = Pim {
        caldav_home: Some(format!("{}/dav/cal/", s.base)),
        carddav_home: Some(format!("{}/dav/card/", s.base)),
        deleted: vec![Tombstone { href: format!("{there}{}x.ics", fake_dav::CAL), ..Default::default() }],
        ..Default::default()
    };
    store.save_account_doc("a1", PIM_DOC, &pim).unwrap();
    let before = s.data.lock().unwrap().log.len();
    let mut w = worker();
    w.sync_dav();
    assert_eq!(events(&pool), [Event::TrustAsked { account: "a1".into(), site: "localhost".into() }]);
    // Until the next start: nothing more, neither asked nor sent.
    w.sync_dav();
    assert_eq!(events(&pool), []);
    assert_eq!(s.data.lock().unwrap().log.len(), before, "{:?}", s.data.lock().unwrap().log);
    // The next start (a new worker) asks again; the engine keeps one question per run.
    worker().sync_dav();
    assert_eq!(events(&pool), [Event::TrustAsked { account: "a1".into(), site: "localhost".into() }]);
}

/// A site removed while its home was saved: the home that came back (a sync already on its way
/// saved it after the removal) goes for nothing. The worker sends nothing there and asks again.
#[test]
fn a_removed_sites_home_gets_nothing_and_is_asked_about_again() {
    use quatrelle_model::pim::{PIM_DOC, Pim};

    use crate::fakes::fake_dav;
    let s = fake_dav::serve();
    let there = s.base.replace("127.0.0.1", "localhost");
    let store = crate::fake::tests_store("dav-removed");
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc = AccountConfig { id: "a1".into(), email: fake_dav::USER.into(), ..Default::default() };
    let events = |pool: &Pool| std::iter::from_fn(|| pool.wait(Duration::from_millis(100))).collect::<Vec<_>>();
    let worker =
        |a: &AccountConfig| Worker::new(a.clone(), Auth::shared(fake_dav::PASSWORD), store.clone(), pool.sink.clone(), SyncOptions::default());
    let saved =
        Pim { caldav_home: Some(format!("{there}/dav/cal/")), carddav_home: Some(String::new()), dav_checked: Some(now()), ..Default::default() };
    store.save_account_doc("a1", PIM_DOC, &saved).unwrap();

    // Allowed: it syncs.
    worker(&AccountConfig { trusted_domains: vec!["localhost".into()], ..acc.clone() }).sync_dav();
    assert!(events(&pool).iter().any(|e| matches!(e, Event::DavSynced { .. })));
    assert!(!s.data.lock().unwrap().log.is_empty(), "the home was read");

    // Removed.
    s.data.lock().unwrap().log.clear();
    worker(&AccountConfig { withdrawn_domains: vec!["localhost".into()], ..acc }).sync_dav();
    assert_eq!(events(&pool), [Event::TrustAsked { account: "a1".into(), site: "localhost".into() }]);
    assert_eq!(s.data.lock().unwrap().log, Vec::<String>::new(), "nothing went there");
}

/// A worker told to stop (the account starts again: a site removed, its settings changed) doesn't
/// begin the calendars' pass under the settings it was started with.
#[test]
fn a_worker_told_to_stop_begins_no_calendar_pass() {
    use quatrelle_model::pim::{PIM_DOC, Pim};

    use crate::fakes::fake_dav;
    let s = fake_dav::serve();
    let store = crate::fake::tests_store("dav-stopping");
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc = AccountConfig {
        id: "a1".into(),
        email: fake_dav::USER.into(),
        imap: quatrelle_model::ServerConfig { host: "127.0.0.1".into(), port: 1, security: quatrelle_model::Security::None },
        ..Default::default()
    };
    let saved =
        Pim { caldav_home: Some(format!("{}/dav/cal/", s.base)), carddav_home: Some(String::new()), dav_checked: Some(now()), ..Default::default() };
    store.save_account_doc("a1", PIM_DOC, &saved).unwrap();
    let mut w = Worker::new(acc, Auth::shared(fake_dav::PASSWORD), store, pool.sink.clone(), SyncOptions::default());

    let (tx, rx) = mpsc::channel();
    w.pim_pass(&rx);
    assert!(!s.data.lock().unwrap().log.is_empty(), "running: the pass is made");
    s.data.lock().unwrap().log.clear();
    drop(tx);
    w.pim_pass(&rx);
    assert_eq!(s.data.lock().unwrap().log, Vec::<String>::new(), "stopping: nothing sent");
}

/// A sync whose look for a missing home stopped at another site is merged all the same, and
/// that site asked about once, however many syncs bring it back.
#[test]
fn a_look_stopped_elsewhere_is_merged_and_asked_once() {
    use quatrelle_model::pim::DavSync;
    let store = crate::fake::tests_store("dav-looked");
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc = AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() };
    let mut w = Worker::new(acc, Auth::shared("pw"), store, pool.sink.clone(), SyncOptions::default());
    let stopped = || Ok(DavSync { looked: true, untrusted: Some("localhost".into()), ..Default::default() });
    w.dav_done(stopped());
    w.dav_done(stopped());
    let seen: Vec<Event> = std::iter::from_fn(|| pool.wait(Duration::from_millis(100))).collect();
    let synced = seen.iter().filter(|e| matches!(e, Event::DavSynced { .. })).count();
    let asked: Vec<&Event> = seen.iter().filter(|e| matches!(e, Event::TrustAsked { .. })).collect();
    assert_eq!(synced, 2, "{seen:?}");
    assert_eq!(asked, [&Event::TrustAsked { account: "a1".into(), site: "localhost".into() }]);
    assert!(!w.dav_waits, "the rest of the sync goes on");
}

#[test]
fn what_someone_waits_on_jumps_the_queue() {
    let key = MessageKey::default();
    for j in [Job::FetchBody { key: key.clone() }, Job::FetchTextFirst { key: key.clone() }, Job::ReadOnline { key: key.clone() }] {
        assert!(jumps_queue(&j) && between_folders(&j), "{j:?}");
    }
}

#[test]
fn a_text_asked_for_twice_is_asked_once() {
    let mut q = VecDeque::new();
    let j = Job::FetchTextFirst { key: MessageKey::default() };
    push_job(&mut q, j.clone());
    push_job(&mut q, j);
    assert_eq!(q.len(), 1);
}

/// A pending-drafts list that can't be read isn't saved over with just the newest draft: the
/// drafts waiting in it stay.
#[test]
fn a_draft_kept_offline_doesnt_write_over_a_list_that_cant_be_read() {
    let store = Store::failing();
    keep_draft(&store, "a1", "Drafts", None, b"Subject: first\r\n\r\nOne\r\n".to_vec()).unwrap();
    store.fail(Some(PENDING_DRAFTS), None);
    assert!(keep_draft(&store, "a1", "Drafts", None, b"Subject: second\r\n\r\nTwo\r\n".to_vec()).is_err());
    store.fail(None, None);
    let kept: Vec<PendingDraft> = store.load_account_doc("a1", PENDING_DRAFTS).unwrap();
    assert_eq!(kept.iter().map(|d| d.raw.clone()).collect::<Vec<_>>(), [b"Subject: first\r\n\r\nOne\r\n".to_vec()]);
}

/// `n` messages, with their sources, in the Inbox of account `a1` of a memory cache.
fn indexed_mail(n: u32) -> Store {
    let s = Store::memory();
    fill(&s, n);
    s
}

/// `n` messages in `s`'s account `a1`, as [`indexed_mail`] has them.
fn fill(s: &Store, n: u32) {
    let messages = (1..=n)
        .map(|uid| quatrelle_model::Summary {
            key: MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid },
            message_id: format!("m{uid}@x"),
            size: 40,
            date: i64::from(uid),
            ..Default::default()
        })
        .collect();
    let inbox = Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    s.save_folders("a1", &[inbox]).unwrap();
    s.save_folder("a1", "INBOX", &quatrelle_store::FolderState { validity: 7, uid_next: n + 1, messages, ..Default::default() }).unwrap();
    for uid in 1..=n {
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid };
        s.write_body(&key, format!("Subject: m\r\nMessage-ID: <m{uid}@x>\r\n\r\nword{uid} common\r\n").as_bytes()).unwrap();
    }
}

/// An account removed while changes to its mail wait on the pool's thread: its files go at
/// once, on the window's thread, and the changes queued before aren't made after, which wrote
/// its folder and its queue for the server back.
#[test]
fn changes_waiting_for_a_removed_account_are_dropped() {
    // Fewer than upstream's 2,000: each change here is read back over the whole list, and the
    // removal still comes with nearly all of them waiting.
    let s = indexed_mail(300);
    let mut pool = Pool::new(s.clone(), Arc::new(MemoryCredentials::default()));
    for uid in 1..=300 {
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid };
        pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key], flag: Flag::Seen, on: true } });
    }
    s.delete_account("a1").unwrap();
    pool.submit(Job::Remove { account: "a1".into() });
    pool.shutdown();
    assert_eq!(s.account_size("a1"), 0, "nothing of it written back");
    assert!(s.load_ops("a1").unwrap_or_default().is_empty());
    // Added again under the same id: its changes are made.
    fill(&s, 3);
    let mut pool = Pool::new(s.clone(), Arc::new(MemoryCredentials::default()));
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid: 2 };
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key], flag: Flag::Seen, on: true } });
    pool.shutdown();
    assert!(s.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.key.uid == 2 && m.flags.seen));
}

/// Compact Now through the pool, on its own thread: the step that has the file to itself is told
/// before it starts (the window puts its veil up then, not before), then what it gave back. A
/// data file this version made has no attachments to make smaller: nothing saved there, and room
/// enough on the drive.
#[test]
fn compact_now_tells_its_steps_and_what_it_gave_back() {
    let dir = std::env::temp_dir().join(format!("quatrelle-compact-now-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open_packed(dir.join("app"), dir.join("Files")).unwrap();
    store.save_accounts(&[AccountConfig { id: "a1".into(), ..Default::default() }]).unwrap();
    let key = |uid: u32| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
    for uid in 1..=40 {
        store.write_body(&key(uid), format!("Subject: {uid}\r\n\r\n{}", "x".repeat(200_000 + uid as usize)).as_bytes()).unwrap();
    }
    for uid in 1..=30 {
        store.remove_body(&key(uid));
    }
    let mut p = Pool::new(store, Arc::new(MemoryCredentials::default()));
    p.submit(Job::CompactDataFile { account: "a1".into() });
    let (mut heard, began) = (Vec::new(), std::time::Instant::now());
    while !heard.iter().any(|e| matches!(e, Event::Compacted { .. })) && began.elapsed() < std::time::Duration::from_secs(60) {
        heard.extend(p.poll());
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let steps: Vec<bool> =
        heard.iter().filter_map(|e| if let Event::CompactProgress { in_place, .. } = e { Some(*in_place) } else { None }).collect();
    assert_eq!(steps.last(), Some(&true), "the step in place told: {heard:?}");
    let Some(Event::Compacted { before, after, recompressed, no_room, error, .. }) = heard.last() else { panic!("{heard:?}") };
    assert_eq!((error, *recompressed, *no_room), (&None, 0, false));
    // The room given back, where the file system tells the room the file takes (144 KiB before,
    // 52 after on Linux): one that compresses it again (ZFS, FreeBSD's runner) says one block of
    // 512 bytes, before and after.
    if *before > 64 * 1024 {
        assert!(after < before, "{before} → {after}");
    }
    assert_eq!(p.store.read_body(&key(35)).map(|b| b.len()), Some(200_000 + 35 + "Subject: 35\r\n\r\n".len()));
    drop(p);
    let _ = std::fs::remove_dir_all(dir);
}

/// A message marked read a moment before an export is read in the data file. The mark goes into
/// the cache on the edit writer's thread, and the export read the cache on its own without
/// waiting for it: the writer is held here until the export has had time to start, as a busy
/// disk would hold it, and the export waits for it now.
#[test]
fn an_export_has_the_marks_made_just_before_it() {
    let dir = std::env::temp_dir().join(format!("quatrelle-export-marks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(dir.join("cache")).unwrap();
    let account = pool::HELD_ACCOUNT;
    let key = MessageKey { account: account.into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let raw = b"From: Ana <ana@example.com>\r\nTo: riley@example.com\r\nSubject: Marked\r\nMessage-ID: <marked@example.com>\r\nDate: Mon, 1 Jan 2024 00:00:00 +0000\r\n\r\nHello.\r\n";
    store.write_body(&key, raw).unwrap();
    let summary = quatrelle_mime::summary_from_full(raw, key.clone(), quatrelle_model::Flags::default(), 10, 0);
    store
        .save_folder(account, "INBOX", &quatrelle_store::FolderState { validity: 1, uid_next: 2, messages: vec![summary], ..Default::default() })
        .unwrap();
    let inbox = Folder { account: account.into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    store.save_folders(account, &[inbox]).unwrap();
    let mut pool = Pool::new(store, Arc::new(MemoryCredentials::default()));
    let file = dir.join("Out.pst");
    let exported = |pool: &Pool, within: Duration| {
        let began = Instant::now();
        while began.elapsed() < within {
            if let Some(e) = pool.poll().into_iter().find(|e| matches!(e, Event::Exported { done: true, .. })) {
                return Some(e);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        None
    };
    let held = pool::HOLD_EDITS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    pool.submit(Job::Apply { account: account.into(), op: Op::SetFlag { keys: vec![key], flag: Flag::Seen, on: true } });
    pool.submit(Job::ExportPst { account: account.into(), path: file.to_string_lossy().into(), name: "Riley".into(), marks: Default::default() });
    let early = exported(&pool, Duration::from_secs(3));
    drop(held);
    let done = early.or_else(|| exported(&pool, Duration::from_secs(60)));
    assert!(matches!(done, Some(Event::Exported { messages: 1, error: None, .. })), "{done:?}");
    let back = Store::open(dir.join("back")).unwrap();
    crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap();
    let inbox = back.load_folders("b1").unwrap().into_iter().find(|f| f.role == FolderRole::Inbox).unwrap();
    let read = back.load_folder("b1", &inbox.path).unwrap().messages.iter().find(|m| m.subject == "Marked").map(|m| m.flags.seen);
    assert_eq!(read, Some(true), "the mark made just before the export is in the file");
    drop(pool);
    let _ = std::fs::remove_dir_all(&dir);
}
