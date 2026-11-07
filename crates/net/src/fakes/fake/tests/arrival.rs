// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! When the server received a message (INTERNALDATE): filled at the first sync, and for mail
//! cached before it was kept, once, at the next ordinary sync, without fetching anything else.

use super::*;

const DAY: i64 = 86_400;

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// A cache from before the arrival time was kept gets it at the next sync, from `UID FETCH
/// (UID INTERNALDATE)` alone; a server that refuses it doesn't stop the sync, and the next one
/// asks again.
#[test]
fn a_cache_without_arrival_times_gets_them_at_the_next_sync() {
    let st = State::new();
    let t = now();
    {
        let mut s = st.lock().unwrap();
        s.add_dated("INBOX", "old forwarded", t - 900 * DAY, t - 2 * DAY);
        s.add_dated("INBOX", "plain", t - 3 * DAY, t - 3 * DAY);
    }
    let store = store("arrival");
    let opts = SyncOptions::default();
    let mut c = login(&st);
    sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    let blank = |store: &Store| {
        let mut f = store.load_folder("a1", "INBOX").unwrap();
        for m in f.messages.iter_mut() {
            m.received = 0;
        }
        store.save_folder("a1", "INBOX", &f).unwrap();
    };
    let received = |store: &Store| -> Vec<(String, i64)> {
        let mut v: Vec<_> = store.load_folder("a1", "INBOX").unwrap().messages.iter().map(|m| (m.subject.clone(), m.received)).collect();
        v.sort();
        v
    };
    assert!(received(&store).iter().all(|(_, r)| *r != 0), "the first sync fills it");
    blank(&store);
    st.lock().unwrap().log.clear();
    sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!(received(&store), [("old forwarded".to_string(), t - 2 * DAY), ("plain".to_string(), t - 3 * DAY)]);
    let log = st.lock().unwrap().log.clone();
    assert!(log.iter().any(|l| l.contains("(UID INTERNALDATE)")), "{log:?}");
    assert!(!log.iter().any(|l| l.contains("BODY")), "no header or body fetched again: {log:?}");
    // Once: filled, it isn't asked again.
    st.lock().unwrap().log.clear();
    sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    assert!(!st.lock().unwrap().log.iter().any(|l| l.contains("(UID INTERNALDATE)")));
}
