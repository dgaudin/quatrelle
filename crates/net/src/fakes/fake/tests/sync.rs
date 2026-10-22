//! Syncing with the fake IMAP server: sign-in, folders, flags, QRESYNC, the window, moves and keywords.

use super::*;

#[test]
fn wrong_password_is_an_auth_error() {
    let st = State::new();
    let mut c = Client::from_stream(Fake::connect(&st)).unwrap();
    assert!(matches!(c.login("me", "nope"), Err(crate::NetError::Auth(_))));
}

#[test]
fn lists_folders_with_roles() {
    let st = State::new();
    let mut c = login(&st);
    let f = c.list("a1").unwrap();
    let roles: Vec<(String, FolderRole)> = f.iter().map(|f| (f.display_name(), f.role)).collect();
    assert_eq!(
        roles,
        vec![
            ("Inbox".to_string(), FolderRole::Inbox),
            ("Sent Items".to_string(), FolderRole::Sent),
            ("Archive".to_string(), FolderRole::Archive),
            ("Café".to_string(), FolderRole::Other)
        ]
    );
}

#[test]
fn full_sync_cycle() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.add("INBOX", "first", &["\\Seen"]);
        s.add("INBOX", "second", &[]);
        s.add("INBOX", "third", &[]);
    }
    let store = store("cycle");
    let opts = SyncOptions::default();
    let mut c = login(&st);

    // First sync: everything arrives, sources prefetched, previews filled.
    let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!((r.new, r.removed, r.total, r.unread), (3, 0, 3, 2));
    let cache = store.load_folder("a1", "INBOX").unwrap();
    assert_eq!(cache.validity, 100);
    assert_eq!(cache.uid_next, 4);
    let subjects: Vec<&str> = cache.messages.iter().map(|m| m.subject.as_str()).collect();
    assert_eq!(subjects, vec!["third", "second", "first"]);
    assert_eq!(cache.messages[0].preview, "Body of third.");
    assert!(store.has_body(&cache.messages[0].key));

    // Nothing changed: CONDSTORE means no flag fetch at all.
    st.lock().unwrap().log.clear();
    let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    assert!(!r.touched());
    assert!(!st.lock().unwrap().log.iter().any(|l| l.contains("FLAGS")), "{:?}", st.lock().unwrap().log);

    // New mail, a flag change and a deletion on the server.
    {
        let mut s = st.lock().unwrap();
        s.add("INBOX", "fourth", &[]);
        s.modseq += 1;
        let ms = s.modseq;
        let b = s.mailbox("INBOX").unwrap();
        let m = b.msgs.iter_mut().find(|m| m.uid == 2).unwrap();
        m.flags.push("\\Flagged".into());
        m.modseq = ms;
        b.msgs.retain(|m| m.uid != 1);
    }
    let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!((r.new, r.changed, r.removed), (1, 1, 1));
    let cache = store.load_folder("a1", "INBOX").unwrap();
    assert_eq!(cache.messages.len(), 3);
    assert!(cache.find(2).unwrap().flags.flagged);
    assert!(cache.find(1).is_none());

    // UIDVALIDITY changes: the cache is rebuilt from scratch.
    st.lock().unwrap().mailbox("INBOX").unwrap().validity = 101;
    let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    assert!(r.reset);
    let cache = store.load_folder("a1", "INBOX").unwrap();
    assert_eq!(cache.validity, 101);
    assert!(cache.messages.iter().all(|m| m.key.validity == 101));
    assert_eq!(cache.messages.len(), 3);
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn qresync_learns_expunges_and_flags_from_select() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.caps = "IMAP4rev1 UIDPLUS MOVE CONDSTORE QRESYNC ENABLE";
        for i in 0..5 {
            s.add("INBOX", &format!("m{i}"), &[]);
        }
    }
    let store = store("qresync");
    let opts = SyncOptions::default();
    let mut c = login(&st);
    assert!(c.qresync());
    sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    {
        let mut s = st.lock().unwrap();
        s.expunge("INBOX", 2);
        s.expunge("INBOX", 4);
        s.set_flags("INBOX", 3, &["\\Seen", "\\Flagged"]);
        s.add("INBOX", "new", &[]);
        s.log.clear();
    }
    let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!((r.new, r.changed, r.removed), (1, 1, 2));
    let cache = store.load_folder("a1", "INBOX").unwrap();
    let uids: Vec<u32> = cache.messages.iter().map(|m| m.key.uid).collect();
    assert_eq!(uids, vec![6, 5, 3, 1]);
    assert!(cache.find(3).unwrap().flags.flagged);
    let log = st.lock().unwrap().log.clone();
    assert!(log.iter().any(|l| l.contains("(QRESYNC (100 ")), "{log:?}");
    // No flag fetch and no UID search over the cached range: the SELECT said it all.
    assert!(!log.iter().any(|l| l.contains("UID SEARCH UID 1:5") || l.contains("CHANGEDSINCE")), "{log:?}");

    // A new UIDVALIDITY still starts over.
    st.lock().unwrap().mailbox("INBOX").unwrap().validity = 101;
    let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    assert!(r.reset);
    assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 4);
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn without_condstore_flags_are_compared() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.caps = "IMAP4rev1";
        s.add("INBOX", "a", &[]);
    }
    let store = store("nocs");
    let mut c = login(&st);
    sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions::default()).unwrap();
    st.lock().unwrap().mailbox("INBOX").unwrap().msgs[0].flags.push("\\Seen".into());
    let r = sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions::default()).unwrap();
    assert_eq!(r.changed, 1);
    assert!(store.load_folder("a1", "INBOX").unwrap().messages[0].flags.seen);
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn first_sync_takes_a_window() {
    let st = State::new();
    for i in 0..8 {
        st.lock().unwrap().add("INBOX", &format!("m{i}"), &[]);
    }
    let store = store("window");
    let mut c = login(&st);
    let opts = SyncOptions { window: 3, prefetch: 1, ..Default::default() };
    let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!(r.new, 3);
    let cache = store.load_folder("a1", "INBOX").unwrap();
    let uids: Vec<u32> = cache.messages.iter().map(|m| m.key.uid).collect();
    assert_eq!(uids, vec![8, 7, 6]);
    assert!(store.has_body(&cache.messages[0].key));
    assert!(!store.has_body(&cache.messages[1].key));
    // Later syncs only add what's new, not the older backlog.
    st.lock().unwrap().add("INBOX", "m8", &[]);
    assert_eq!(sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap().new, 1);
    // More kept since (Options): the older mail comes in once, up to the new number.
    let more = SyncOptions { window: 6, ..opts };
    assert_eq!(sync_folder(&mut c, &store, "a1", "INBOX", &more).unwrap().new, 2);
    let uids: Vec<u32> = store.load_folder("a1", "INBOX").unwrap().messages.iter().map(|m| m.key.uid).collect();
    assert_eq!(uids, vec![9, 8, 7, 6, 5, 4]);
    assert_eq!(sync_folder(&mut c, &store, "a1", "INBOX", &more).unwrap().new, 0, "once");
    let all = SyncOptions { window: usize::MAX, ..opts };
    assert_eq!(sync_folder(&mut c, &store, "a1", "INBOX", &all).unwrap().new, 3, "all of it");
    let _ = std::fs::remove_dir_all(store.root());
}

/// Permanent delete on a Gmail-style server: moved into Trash and expunged there. Other
/// servers get a plain EXPUNGE.
#[test]
fn gmail_permanent_delete_moves_to_trash_then_expunges() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.caps = "IMAP4rev1 UIDPLUS MOVE X-GM-EXT-1";
        for (name, attr, validity) in [("[Gmail]/All Mail", "\\All", 500), ("[Gmail]/Trash", "\\Trash", 600)] {
            s.boxes.push(Mailbox { name: name.into(), attr, validity, uid_next: 1, msgs: Vec::new(), expunged: Vec::new() });
        }
        for subject in ["a", "b", "c", "d"] {
            s.add("INBOX", subject, &[]);
        }
        s.add("[Gmail]/Trash", "binned", &[]);
    }
    let mut c = login(&st);
    let key = |folder: &str, validity, uid| MessageKey { account: "a1".into(), folder: folder.into(), validity, uid };
    let count = |name: &str| st.lock().unwrap().mailbox(name).map_or(0, |b| b.msgs.len());
    // No Trash path given: plain EXPUNGE, which Gmail turns into an archive.
    apply_remote(&mut c, &Op::Expunge { keys: vec![key("INBOX", 100, 1)] }).unwrap();
    assert_eq!((count("INBOX"), count("[Gmail]/All Mail")), (3, 1));
    // With the Trash path: moved, then expunged; nothing left behind.
    let trash = Some("[Gmail]/Trash");
    crate::worker::apply_remote_with(&mut c, &Op::Expunge { keys: vec![key("INBOX", 100, 2), key("INBOX", 100, 3)] }, trash).unwrap();
    assert_eq!((count("INBOX"), count("[Gmail]/All Mail"), count("[Gmail]/Trash")), (1, 1, 1));
    // Keys already in Trash: expunged in place.
    crate::worker::apply_remote_with(&mut c, &Op::Expunge { keys: vec![key("[Gmail]/Trash", 600, 1)] }, trash).unwrap();
    assert_eq!((count("[Gmail]/All Mail"), count("[Gmail]/Trash")), (1, 0));
    // Non-Gmail server: the Trash path is ignored.
    st.lock().unwrap().caps = "IMAP4rev1 UIDPLUS MOVE";
    let mut c = login(&st);
    crate::worker::apply_remote_with(&mut c, &Op::Expunge { keys: vec![key("INBOX", 100, 4)] }, trash).unwrap();
    assert_eq!((count("INBOX"), count("[Gmail]/All Mail"), count("[Gmail]/Trash")), (0, 1, 0));
}

/// A move says where its messages went (COPYUID), with MOVE and without.
#[test]
fn a_move_says_the_new_uids() {
    let st = State::new();
    for s in ["a", "b", "c"] {
        st.lock().unwrap().add("INBOX", s, &[]);
    }
    let mut c = login(&st);
    c.select("INBOX", false).unwrap();
    assert_eq!(c.uid_move_to("1,3", "Archive").unwrap(), vec![1, 2]);
    st.lock().unwrap().caps = "IMAP4rev1 UIDPLUS";
    let mut c = login(&st);
    c.select("INBOX", false).unwrap();
    assert_eq!(c.uid_move_to("2", "Archive").unwrap(), vec![3]);
    assert_eq!(st.lock().unwrap().mailbox("INBOX").map(|b| b.msgs.len()), Some(0));
}

#[test]
fn remote_changes_flags_moves_deletes_and_appends() {
    let st = State::new();
    for s in ["a", "b", "c"] {
        st.lock().unwrap().add("INBOX", s, &[]);
    }
    let mut c = login(&st);
    let key = |uid| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 100, uid };
    apply_remote(&mut c, &Op::SetFlag { keys: vec![key(1), key(2)], flag: Flag::Seen, on: true }).unwrap();
    apply_remote(&mut c, &Op::Move { keys: vec![key(2)], to: "Archive".into() }).unwrap();
    apply_remote(&mut c, &Op::Expunge { keys: vec![key(3)] }).unwrap();
    // A stale key (old UIDVALIDITY) is skipped, not applied to whatever has that UID now.
    let mut stale = key(1);
    stale.validity = 99;
    apply_remote(&mut c, &Op::Expunge { keys: vec![stale] }).unwrap();
    let uid = c.append("Sent", "\\Seen", b"Subject: sent\r\n\r\nx\r\n").unwrap();
    assert_eq!(uid, Some(1));
    let mut s = st.lock().unwrap();
    let inbox: Vec<(u32, Vec<String>)> = s.mailbox("INBOX").unwrap().msgs.iter().map(|m| (m.uid, m.flags.clone())).collect();
    assert_eq!(inbox, vec![(1, vec!["\\Seen".to_string()])]);
    let arch = s.mailbox("Archive").unwrap();
    assert_eq!(arch.msgs.len(), 1);
    assert!(arch.msgs[0].flags.contains(&"\\Seen".to_string()));
    assert_eq!(s.mailbox("Sent").unwrap().msgs[0].raw, b"Subject: sent\r\n\r\nx\r\n");
}

/// For looking at the app against a mail server that acts as Outlook.com does, on
/// 127.0.0.1:1143 without TLS (user name anything, password `pw`, OAuth token `good-token`):
/// `QUATRELLE_FAKE_REFUSE` refuses every password with its text ("Basic authentication is
/// disabled"); `QUATRELLE_FAKE_CLOSED` (an address) takes that address's token but won't
/// open its mailbox. Run: `cargo test -p quatrelle-net serve_imap_for_the_app -- --ignored`.
#[test]
#[ignore]
fn serve_imap_for_the_app() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.refuse_passwords = std::env::var("QUATRELLE_FAKE_REFUSE").ok().filter(|v| !v.is_empty());
        s.not_connected.extend(std::env::var("QUATRELLE_FAKE_CLOSED").ok().filter(|v| !v.is_empty()));
        s.add("INBOX", "Welcome", &[]);
        // `QUATRELLE_FAKE_COUNT`: that many more in the Inbox, oldest first ("Message 1" is
        // the oldest; each body says `needle-<n>`), for more mail than is kept (issue #4).
        let count: u32 = std::env::var("QUATRELLE_FAKE_COUNT").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
        for n in 1..=count.min(20_000) {
            let day = 1 + n / 400;
            s.add_raw(
                "INBOX",
                format!(
                    "From: Ana <ana@studio.example>\r\nTo: me@studio.example\r\nSubject: Message {n}\r\nDate: Mon, {day:02} Jun 2025 10:00:00 +0000\r\nMessage-ID: <m{n}@studio.example>\r\n\r\nBody of message {n}: needle-{n}.\r\n"
                )
                .into_bytes(),
            );
        }
        // `QUATRELLE_FAKE_EML`: messages to put in the Inbox (paths, separated by `;`).
        for path in std::env::var("QUATRELLE_FAKE_EML").unwrap_or_default().split(';').filter(|p| !p.is_empty()) {
            if let Ok(raw) = std::fs::read(path) {
                s.add_raw("INBOX", raw);
            }
        }
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:1143").unwrap();
    for sock in listener.incoming().flatten() {
        let st = st.clone();
        std::thread::spawn(move || {
            let mut fake = Fake::connect(&st);
            let mut sock = sock;
            let mut buf = [0u8; 8192];
            // Counted so a test can see that a sync really used several connections.
            if let Ok(mut s) = st.lock() {
                s.open += 1;
                s.peak = s.peak.max(s.open);
            }
            struct Open(Arc<Mutex<State>>);
            impl Drop for Open {
                fn drop(&mut self) {
                    if let Ok(mut s) = self.0.lock() {
                        s.open = s.open.saturating_sub(1);
                    }
                }
            }
            let _open = Open(st.clone());
            loop {
                let out: Vec<u8> = fake.out.drain(..).collect();
                if !out.is_empty() && sock.write_all(&out).is_err() {
                    return;
                }
                match sock.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        let _ = fake.write(&buf[..n]);
                    }
                }
            }
        });
    }
}

#[test]
fn xoauth2_sign_in() {
    for caps in ["IMAP4rev1 AUTH=XOAUTH2 SASL-IR", "IMAP4rev1 AUTH=XOAUTH2"] {
        let st = State::new();
        st.lock().unwrap().caps = caps;
        let mut c = Client::from_stream(Fake::connect(&st)).unwrap();
        c.capability().unwrap();
        c.login_xoauth2("me@studio.example", GOOD_TOKEN).unwrap();
        assert!(c.list("a1").is_ok(), "signed in with {caps}");
        let mut c = Client::from_stream(Fake::connect(&st)).unwrap();
        c.capability().unwrap();
        assert!(matches!(c.login_xoauth2("me@studio.example", "expired"), Err(crate::NetError::Auth(_))), "{caps}");
    }
}

#[test]
fn keywords_round_trip_with_the_server() {
    let st = State::new();
    st.lock().unwrap().add("INBOX", "tagged", &[]);
    let store = store("keywords");
    let mut c = login(&st);
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 100, uid: 1 };
    apply_remote(&mut c, &Op::SetKeyword { keys: vec![key.clone()], keyword: "$label2".into(), on: true }).unwrap();
    apply_remote(&mut c, &Op::SetKeyword { keys: vec![key.clone()], keyword: "$Pinned".into(), on: true }).unwrap();
    // Not an atom: never sent.
    apply_remote(&mut c, &Op::SetKeyword { keys: vec![key], keyword: "two words".into(), on: true }).unwrap();
    assert!(!st.lock().unwrap().log.iter().any(|l| l.contains("two words")));
    sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions::default()).unwrap();
    let m = store.load_folder("a1", "INBOX").unwrap().messages[0].clone();
    assert!(m.flags.has("$label2") && m.flags.pinned(), "{:?}", m.flags);
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn remote_folder_changes() {
    let st = State::new();
    st.lock().unwrap().add("Archive", "kept", &[]);
    let mut c = login(&st);
    apply_remote(&mut c, &Op::CreateFolder { path: "Clients".into() }).unwrap();
    // Creating an existing folder is not an error.
    apply_remote(&mut c, &Op::CreateFolder { path: "Clients".into() }).unwrap();
    c.select("Archive", false).unwrap();
    apply_remote(&mut c, &Op::RenameFolder { from: "Archive".into(), to: "Clients/Old".into() }).unwrap();
    assert!(c.selected().is_none(), "renaming the selected folder deselects it first");
    apply_remote(&mut c, &Op::DeleteFolder { path: "Projects/Caf&AOk-".into() }).unwrap();
    // Deleting a folder that's already gone is fine; a refused rename is an error.
    apply_remote(&mut c, &Op::DeleteFolder { path: "Nowhere".into() }).unwrap();
    assert!(matches!(apply_remote(&mut c, &Op::RenameFolder { from: "Nope".into(), to: "X".into() }), Err(crate::NetError::No(_))));
    let names: Vec<String> = c.list("a1").unwrap().into_iter().map(|f| f.path).collect();
    assert_eq!(names, vec!["INBOX", "Sent", "Clients/Old", "Clients"]);
    let s = st.lock().unwrap();
    assert_eq!(s.boxes.iter().find(|b| b.name == "Clients/Old").unwrap().msgs.len(), 1, "the mail moved with the folder");
    assert!(s.log.iter().any(|l| l.starts_with("SUBSCRIBE \"Clients\"")));
}
