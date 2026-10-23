//! What a JMAP account keeps, as Outlook's Cached Exchange Mode does: the mail received in the
//! time kept, and every message kept downloaded whole in the end.

use quatrelle_model::MessageKey;

use super::*;
use crate::sync::SyncOptions;

const DAY: i64 = 86_400;

/// When the syncs below run: 2026-10-23 at noon.
fn now() -> i64 {
    crate::jmap::utc_date("2026-10-23T12:00:00Z").unwrap()
}

/// A message received `days` ago; `date` is its own `Date` header, when it has one.
fn deliver(d: &mut Data, inbox: &str, subject: &str, days: i64, date: Option<&str>) -> String {
    let date = date.map(|d| format!("Date: {d}\r\n")).unwrap_or_default();
    let id = subject.replace(' ', "-");
    let raw =
        format!("From: Zoe Park <zoe@studio.example>\r\nTo: {USER}\r\n{date}Subject: {subject}\r\nMessage-ID: <{id}@x>\r\n\r\nAbout {subject}.\r\n");
    d.deliver(inbox, raw.as_bytes(), &["$seen"], &quatrelle_jmap::utc_date_string(now() - days * DAY))
}

/// A store of its own, signed in, the folders listed.
fn open(server: &Server, name: &str) -> (std::path::PathBuf, Store, crate::jmap::Client, crate::jmap::State) {
    let dir = std::env::temp_dir().join(format!("quatrelle-jmap-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let c = crate::jmap::Client::connect(&server.base, USER, PASSWORD, false).unwrap();
    let mut st = crate::jmap::State::default();
    crate::jmap::list_folders(&c, &mut st, "a1").unwrap();
    (dir, store, c, st)
}

fn subjects(store: &Store) -> Vec<String> {
    let mut s: Vec<String> = store.load_folder("a1", "INBOX").unwrap().messages.iter().map(|m| m.subject.clone()).collect();
    s.sort();
    s
}

fn key(store: &Store, subject: &str) -> MessageKey {
    store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == subject).unwrap().key.clone()
}

/// Mail from last week, a forward of an old message that came two days ago, a message dated
/// yesterday that came two months ago, and last year's: what's kept is measured by when the
/// server received each, as the server measures it.
fn four() -> (Data, String) {
    let mut d = Data::default();
    let inbox = d.mailbox("Inbox", None, Some("inbox"));
    deliver(&mut d, &inbox, "Last week", 7, None);
    deliver(&mut d, &inbox, "Forwarded", 2, Some("Mon, 01 Jan 2018 09:00:00 +0000"));
    deliver(&mut d, &inbox, "Backdated", 60, Some("Thu, 22 Oct 2026 09:00:00 +0000"));
    deliver(&mut d, &inbox, "Last year", 365, None);
    (d, inbox)
}

/// A first sync keeps the mail received in the last 30 days, asking the server for it alone.
#[test]
fn a_jmap_folder_keeps_the_mail_received_in_the_time_kept() {
    let (d, inbox) = four();
    let server = serve(d);
    let (dir, store, c, mut st) = open(&server, "since");
    let opts = SyncOptions { since_days: 30, prefetch: 0, ..Default::default() };
    let r = crate::jmap::sync_folder_at(&c, &mut st, &store, "a1", "INBOX", &opts, now()).unwrap();
    assert_eq!(subjects(&store), ["Forwarded", "Last week"], "received in the last 30 days, whatever their own date");
    assert_eq!((r.new, r.total), (2, 4), "the folder's count is the server's");
    let fwd = store.load_folder("a1", "INBOX").unwrap().messages.into_iter().find(|m| m.subject == "Forwarded").unwrap();
    assert_eq!((fwd.received, fwd.date), (now() - 2 * DAY, crate::jmap::utc_date("2018-01-01T09:00:00Z").unwrap()), "received is the server's");
    assert_eq!(server.data.lock().unwrap().queries, [json!({"inMailbox": inbox, "after": "2026-09-23T12:00:00Z"})], "only that is listed");
    let _ = std::fs::remove_dir_all(&dir);
}

/// With no age set, all of it, asked as before.
#[test]
fn a_jmap_folder_keeps_all_its_mail_with_no_age_set() {
    let (d, inbox) = four();
    let server = serve(d);
    let (dir, store, c, mut st) = open(&server, "all");
    let opts = SyncOptions { prefetch: 0, ..Default::default() };
    crate::jmap::sync_folder_at(&c, &mut st, &store, "a1", "INBOX", &opts, now()).unwrap();
    assert_eq!(subjects(&store), ["Backdated", "Forwarded", "Last week", "Last year"]);
    assert_eq!(server.data.lock().unwrap().queries, [json!({"inMailbox": inbox})]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Mail that comes to be older than the time kept leaves the cache with its source at the next
/// sync, though nothing changed on the server; what a search of the server found stays, and so
/// does an old message forwarded lately (it isn't let go and listed again, sync after sync).
#[test]
fn jmap_mail_ages_out_at_the_next_sync_even_unchanged() {
    let mut d = Data::default();
    let inbox = d.mailbox("Inbox", None, Some("inbox"));
    deliver(&mut d, &inbox, "Recent", 1, None);
    deliver(&mut d, &inbox, "Forwarded", 2, Some("Mon, 01 Jan 2018 09:00:00 +0000"));
    deliver(&mut d, &inbox, "Older", 25, None);
    deliver(&mut d, &inbox, "Archive needle", 400, None);
    let server = serve(d);
    let (dir, store, c, mut st) = open(&server, "ageing");
    let opts = SyncOptions { since_days: 30, prefetch: 10, ..Default::default() };
    crate::jmap::sync_folder_at(&c, &mut st, &store, "a1", "INBOX", &opts, now()).unwrap();
    assert_eq!(subjects(&store), ["Forwarded", "Older", "Recent"]);
    let older = key(&store, "Older");
    assert!(store.has_body(&older), "downloaded with the list");
    // A search of the server brings last year's in, opened.
    let filter = quatrelle_jmap::search_filter("needle").unwrap();
    let (found, new) = crate::jmap::find_on_server(&c, &mut st, &store, "a1", "INBOX", &filter, quatrelle_jmap::FOUND_MAX).unwrap();
    assert_eq!((found.len(), new), (1, 1));
    crate::jmap::fetch_body(&c, &st, &store, &found[0]).unwrap();

    // Ten days on, nothing changed on the server: "Older" came 35 days ago.
    let later = now() + 10 * DAY;
    let r = crate::jmap::sync_folder_at(&c, &mut st, &store, "a1", "INBOX", &opts, later).unwrap();
    assert_eq!((r.new, r.removed, r.changed), (0, 1, 0));
    assert_eq!(subjects(&store), ["Archive needle", "Forwarded", "Recent"]);
    assert!(!store.has_body(&older), "its source goes with it");
    assert!(store.has_body(&found[0]) && st.was_found("INBOX", found[0].uid), "what a search found stays, with its source");
    // And again: nothing comes back, nothing goes.
    let r = crate::jmap::sync_folder_at(&c, &mut st, &store, "a1", "INBOX", &opts, later + DAY).unwrap();
    assert!(!r.touched(), "{r:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every message kept is downloaded in the end, a few at a time, the newest first: up to the
/// size for downloading ahead, or all of them when kept whole. One the server no longer has is
/// left for when it's opened, and isn't counted as still to come.
#[test]
fn jmap_mail_kept_is_downloaded_ahead_newest_first() {
    let mut d = Data::default();
    let inbox = d.mailbox("Inbox", None, Some("inbox"));
    let mut ids = Vec::new();
    for n in 1..=6 {
        ids.push(deliver(&mut d, &inbox, &format!("M{n}"), 7 - n, None));
    }
    // M5 is big.
    let big = d.email(&ids[4]).unwrap().blob.clone();
    d.blobs.get_mut(&big).unwrap().extend(std::iter::repeat_n(b'x', 4000));
    let server = serve(d);
    let (dir, store, c, mut st) = open(&server, "ahead");
    let capped = SyncOptions { prefetch: 0, prefetch_max_bytes: 1000, ..Default::default() };
    let whole = SyncOptions { full: true, ..capped };
    crate::jmap::sync_folder_at(&c, &mut st, &store, "a1", "INBOX", &capped, now()).unwrap();
    let ahead = |opts: &SyncOptions, budget| crate::jmap::download_ahead(&c, &st, &store, "a1", "INBOX", opts, budget).unwrap();
    let has = |names: &[&str]| names.iter().map(|n| store.has_body(&key(&store, n))).collect::<Vec<bool>>();
    assert_eq!((ahead(&capped, 0), ahead(&whole, 0)), (5, 6), "nothing downloaded yet: the big one only when kept whole");

    assert_eq!(ahead(&capped, 2), 3, "two of five");
    assert_eq!(has(&["M6", "M5", "M4", "M3"]), [true, false, true, false], "the newest first, the big one passed over");
    assert_eq!(ahead(&whole, 2), 2);
    assert_eq!(has(&["M5", "M3", "M2"]), [true, true, false], "whole: the big one comes too");

    // M2 is deleted on the server: M1 comes in its place, and nothing is left to come.
    server.data.lock().unwrap().emails.retain(|e| e.id != ids[1]);
    assert_eq!(ahead(&whole, 1), 0);
    assert_eq!(has(&["M2", "M1"]), [false, true]);
    let _ = std::fs::remove_dir_all(&dir);
}
