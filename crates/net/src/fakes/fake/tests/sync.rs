//! Syncing with the fake IMAP server: sign-in, folders, flags, QRESYNC, the window and the age
//! kept, downloading ahead, moves and keywords.

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

const DAY: i64 = 86_400;

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap())
}

/// The subjects kept of a folder, sorted.
fn kept_subjects(store: &Store, folder: &str) -> Vec<String> {
    let mut v: Vec<String> = store.load_folder("a1", folder).unwrap().messages.iter().map(|m| m.subject.clone()).collect();
    v.sort();
    v
}

/// Mail kept for an age, as Outlook's Cached Exchange Mode keeps it: the first sync lists what
/// the server received since the cutoff (`UID SEARCH SINCE`, by INTERNALDATE), the newest
/// window of it. A message's own date doesn't count (an old one forwarded yesterday is kept, a
/// recent one that came long ago isn't), nor what the cutoff's day lists from before it.
#[test]
fn a_first_sync_keeps_what_came_in_since_the_age() {
    // The fake says when it received a message as IMAP writes it (RFC 3501 `date-time`).
    assert_eq!(internal_date(RECEIVED), "14-Oct-2026 09:30:00 +0000");
    assert_eq!(internal_date(3 * DAY + 5), " 4-Jan-1970 00:00:05 +0000");
    let st = State::new();
    let t = now();
    let cutoff = t - 30 * DAY;
    {
        let mut s = st.lock().unwrap();
        s.add_dated("INBOX", "a year ago", t - 400 * DAY, t - 400 * DAY);
        s.add_dated("INBOX", "forty days ago", t - 40 * DAY, t - 40 * DAY);
        s.add_dated("INBOX", "dated lately, came long ago", t - 2 * DAY, t - 60 * DAY);
        // The cutoff's day, a few hours before it (unless the cutoff is that day's first second).
        let early = cutoff.div_euclid(DAY) * DAY;
        if early + 60 < cutoff {
            s.add_dated("INBOX", "the cutoff's day, earlier", early, early);
        }
        s.add_dated("INBOX", "ten days ago", t - 10 * DAY, t - 10 * DAY);
        s.add_dated("INBOX", "old, forwarded yesterday", t - 900 * DAY, t - DAY);
        s.add_dated("INBOX", "today", t - 60, t - 60);
    }
    let wide = store("since");
    let mut c = login(&st);
    let r = sync_folder(&mut c, &wide, "a1", "INBOX", &SyncOptions { since_days: 30, ..Default::default() }).unwrap();
    assert_eq!(r.new, 3);
    assert_eq!(kept_subjects(&wide, "INBOX"), ["old, forwarded yesterday", "ten days ago", "today"]);
    let log = st.lock().unwrap().log.clone();
    assert!(log.contains(&format!("UID SEARCH SINCE {}", crate::sync::imap_date(cutoff))), "{log:?}");
    assert!(!log.iter().any(|l| l.contains("SEARCH ALL")), "{log:?}");

    // Still the newest window of it.
    let small = store("since-window");
    sync_folder(&mut c, &small, "a1", "INBOX", &SyncOptions { since_days: 30, window: 2, ..Default::default() }).unwrap();
    assert_eq!(kept_subjects(&small, "INBOX"), ["old, forwarded yesterday", "today"]);
    let _ = std::fs::remove_dir_all(wide.root());
    let _ = std::fs::remove_dir_all(small.root());
}

/// Mail that grows older than the age kept goes with its source at the next sync, but for what
/// a search of the server found; new mail comes in. Age is when the server received it (an
/// old message forwarded lately stays). A longer age lists the folder again, once; a find the
/// server no longer has is forgotten. With QRESYNC too.
#[test]
fn mail_older_than_the_age_goes_with_its_source() {
    for caps in ["IMAP4rev1 UIDPLUS MOVE CONDSTORE ENABLE", "IMAP4rev1 UIDPLUS MOVE CONDSTORE QRESYNC ENABLE"] {
        let st = State::new();
        let t = now();
        {
            let mut s = st.lock().unwrap();
            s.caps = caps;
            s.add_dated("INBOX", "two years ago", t - 700 * DAY, t - 700 * DAY);
            s.add_dated("INBOX", "forty days ago", t - 40 * DAY, t - 40 * DAY);
            s.add_dated("INBOX", "ten days ago", t - 10 * DAY, t - 10 * DAY);
            s.add_dated("INBOX", "old, forwarded lately", t - 900 * DAY, t - 5 * DAY);
            s.add_dated("INBOX", "a year and a half ago", t - 550 * DAY, t - 550 * DAY);
        }
        let store = store(if caps.contains("QRESYNC") { "age-qresync" } else { "age" });
        let mut c = login(&st);
        let days = |since_days| SyncOptions { since_days, ..Default::default() };
        sync_folder(&mut c, &store, "a1", "INBOX", &days(60)).unwrap();
        assert_eq!(kept_subjects(&store, "INBOX"), ["forty days ago", "old, forwarded lately", "ten days ago"], "{caps}");
        let forty = store.load_folder("a1", "INBOX").unwrap().find(2).unwrap().key.clone();
        assert!(store.has_body(&forty));
        // Searches of the server bring in older ones (the search box's, and the one by ranges).
        let found = crate::sync::search_on_server(&mut c, &store, "a1", "INBOX", &crate::sync::imap_criteria("two").unwrap(), 500).unwrap();
        assert_eq!(found.len(), 1);
        let half = crate::sync::imap_criteria("half").unwrap();
        assert_eq!(crate::sync::find_on_server(&mut c, &store, "a1", "INBOX", &half, 500, &mut |_, _| true).unwrap(), 1);

        // Thirty days: the one forty days old goes, its source too; the find stays; new mail comes.
        st.lock().unwrap().add_dated("INBOX", "new", t, t);
        let r = sync_folder(&mut c, &store, "a1", "INBOX", &days(30)).unwrap();
        assert_eq!((r.new, r.removed), (1, 1), "{caps}");
        let now_kept = ["a year and a half ago", "new", "old, forwarded lately", "ten days ago", "two years ago"];
        assert_eq!(kept_subjects(&store, "INBOX"), now_kept, "{caps}");
        assert!(!store.has_body(&forty));
        // Nothing more goes, and nothing is listed again.
        st.lock().unwrap().log.clear();
        assert!(!sync_folder(&mut c, &store, "a1", "INBOX", &days(30)).unwrap().touched(), "{caps}");
        let log = st.lock().unwrap().log.clone();
        assert!(!log.iter().any(|l| l.contains("SINCE") || l.contains("ALL")), "{log:?}");

        // A year: the folder is listed again, once.
        assert_eq!(sync_folder(&mut c, &store, "a1", "INBOX", &days(365)).unwrap().new, 1, "{caps}");
        let now_kept = ["a year and a half ago", "forty days ago", "new", "old, forwarded lately", "ten days ago", "two years ago"];
        assert_eq!(kept_subjects(&store, "INBOX"), now_kept, "{caps}");
        assert_eq!(sync_folder(&mut c, &store, "a1", "INBOX", &days(365)).unwrap().new, 0, "once");

        // A find goes on the server: here too, and it's no longer marked.
        st.lock().unwrap().expunge("INBOX", 1);
        sync_folder(&mut c, &store, "a1", "INBOX", &days(365)).unwrap();
        let now_kept = ["a year and a half ago", "forty days ago", "new", "old, forwarded lately", "ten days ago"];
        assert_eq!(kept_subjects(&store, "INBOX"), now_kept, "{caps}");
        let kept: serde_json::Value = store.load_folder_doc("a1", "INBOX", "kept.json").unwrap();
        assert_eq!(kept["found"], serde_json::json!([5]), "{kept}");
        assert_eq!(kept["days"], 365);
        let _ = std::fs::remove_dir_all(store.root());
    }
}

/// No age (0: all of it) keeps what it kept before, however old: the newest window, listed
/// with `SEARCH ALL`, nothing let go.
#[test]
fn without_an_age_all_of_it_is_kept() {
    let st = State::new();
    let t = now();
    for (n, age) in [("ten years", 3650), ("a year", 400), ("a day", 1)] {
        st.lock().unwrap().add_dated("INBOX", n, t - age * DAY, t - age * DAY);
    }
    let store = store("all-ages");
    let mut c = login(&st);
    let r = sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions::default()).unwrap();
    assert_eq!(r.new, 3);
    assert!(!sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions::default()).unwrap().touched());
    assert_eq!(kept_subjects(&store, "INBOX"), ["a day", "a year", "ten years"]);
    let log = st.lock().unwrap().log.clone();
    assert!(log.iter().any(|l| l == "UID SEARCH ALL") && !log.iter().any(|l| l.contains("SINCE")), "{log:?}");
    let _ = std::fs::remove_dir_all(store.root());
}

/// Downloading ahead: the sources of the newest kept messages not downloaded yet, `budget` at a
/// time, up to the size downloaded ahead, or any size when every message kept is to be
/// (`full`); how many are left after.
#[test]
fn downloading_ahead_goes_newest_first_within_its_budget() {
    let st = State::new();
    let t = now();
    {
        let mut s = st.lock().unwrap();
        for i in 0..5 {
            s.add_dated("INBOX", &format!("m{i}"), t - (10 - i) * DAY, t - (10 - i) * DAY);
        }
        let date = internal_date(t).trim().replace('-', " ");
        s.add_raw("INBOX", format!("Subject: big\r\nDate: {date}\r\n\r\n{}\r\n", "x".repeat(5000)).into_bytes());
    }
    let store = store("ahead");
    let mut c = login(&st);
    let opts = SyncOptions { prefetch: 0, prefetch_max_bytes: 1000, ..Default::default() };
    sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    let has = |subject: &str| store.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.subject == subject && store.has_body(&m.key));
    assert!(!has("m4") && !has("big"));

    // Another folder selected: it's opened. Two, the newest small ones; three small ones left.
    c.select("Sent", false).unwrap();
    assert_eq!(crate::sync::download_ahead(&mut c, &store, "a1", "INBOX", &opts, 2).unwrap(), 3);
    assert!(has("m4") && has("m3") && !has("m2") && !has("big"));
    let folder = store.load_folder("a1", "INBOX").unwrap();
    assert_eq!(folder.messages.iter().find(|m| m.subject == "m4").map(|m| m.preview.as_str()), Some("Body of m4."));
    assert_eq!(crate::sync::download_ahead(&mut c, &store, "a1", "INBOX", &opts, 10).unwrap(), 0);
    assert!(has("m0") && !has("big"));

    // Every message kept: the big one too.
    let full = SyncOptions { full: true, ..opts };
    assert_eq!(crate::sync::download_ahead(&mut c, &store, "a1", "INBOX", &full, 0).unwrap(), 1, "none asked, one left");
    assert_eq!(crate::sync::download_ahead(&mut c, &store, "a1", "INBOX", &full, 10).unwrap(), 0);
    assert!(has("big"));
    // All here: nothing asked of the server.
    st.lock().unwrap().log.clear();
    assert_eq!(crate::sync::download_ahead(&mut c, &store, "a1", "INBOX", &full, 10).unwrap(), 0);
    assert!(st.lock().unwrap().log.is_empty());
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
    apply_remote(&mut c, &Op::SetKeyword { keys: vec![key.clone()], keyword: "$label2".into(), on: true, name: None }).unwrap();
    apply_remote(&mut c, &Op::SetKeyword { keys: vec![key.clone()], keyword: "$Pinned".into(), on: true, name: None }).unwrap();
    // Not an atom: never sent.
    apply_remote(&mut c, &Op::SetKeyword { keys: vec![key], keyword: "two words".into(), on: true, name: None }).unwrap();
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

/// A sync says when the server received the newest message that came in, for the log to say how
/// soon new mail was told of: an IMAP account's said nothing (a Gmail message seemed not to come,
/// 29/10).
#[test]
fn a_sync_says_when_the_newest_message_came_to_the_server() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.add("INBOX", "first", &[]);
        s.add("INBOX", "second", &[]);
        // The first listed came to the server last.
        s.mailbox("INBOX").unwrap().msgs[0].received = RECEIVED + 60;
    }
    let store = store("newest");
    let opts = SyncOptions::default();
    let mut c = login(&st);
    assert_eq!(sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap().newest, Some(RECEIVED + 60));
    assert_eq!(sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap().newest, None, "nothing came");
    st.lock().unwrap().add("INBOX", "third", &[]);
    assert_eq!(sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap().newest, Some(RECEIVED));
}
