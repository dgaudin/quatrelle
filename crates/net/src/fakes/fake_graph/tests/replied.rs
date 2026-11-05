// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A message replied to or forwarded over the fake Graph server: Graph has no field for it, but
//! gives and takes the properties Outlook keeps (`singleValueExtendedProperties`: `Integer
//! 0x1081` the last verb, `SystemTime 0x1082` when, `Integer 0x1080` the icon).

use super::*;

/// Replied to or forwarded in Outlook, a message comes in with its arrow and time, at its first
/// sync and at the next one of a cache from before this version; replied to or forwarded here,
/// Graph gets the verb, the time and the icon; the last thing done shows.
#[test]
fn a_reply_or_a_forward_goes_both_ways_over_graph() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    let replied = d.deliver(&inbox, b"From: a@x.example\r\nSubject: replied\r\n\r\nx\r\n", true, "2024-05-01T09:00:00Z");
    let plain = d.deliver(&inbox, b"From: a@x.example\r\nSubject: plain\r\n\r\nx\r\n", true, "2024-05-02T09:00:00Z");
    for m in d.messages.iter_mut().filter(|m| m.id == replied) {
        (m.verb, m.verb_time) = (Some("102".into()), Some("2026-10-30T09:12:00Z".into()));
    }
    let server = serve(d);
    let store = crate::fake::tests_store("graph-replied");
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let mut st = crate::jmap::State::default();
    crate::graph::list_folders(&c, &mut st, "a1", &[]).unwrap();
    let opts = crate::sync::SyncOptions { prefetch: 0, ..Default::default() };
    let sync = |st: &mut crate::jmap::State| crate::graph::sync_folder(&c, st, &store, "a1", "INBOX", &opts).unwrap();
    let msg = |subject: &str| store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == subject).cloned().unwrap();
    let marks = |subject: &str| {
        let f = msg(subject).flags;
        (f.answered, f.forwarded, f.acted)
    };
    let at = |text: &str| crate::jmap::utc_date(text).unwrap();

    sync(&mut st);
    assert_eq!(marks("replied"), (true, false, at("2026-10-30T09:12:00Z")), "listed with its arrow");
    // As the version before left it: the next ordinary sync brings it (Graph lists every kept
    // message's marks at each sync).
    let mut fs = store.load_folder("a1", "INBOX").unwrap();
    for m in fs.messages.iter_mut() {
        (m.flags.answered, m.flags.acted) = (false, 0);
    }
    store.save_folder("a1", "INBOX", &fs).unwrap();
    sync(&mut st);
    assert_eq!(marks("replied"), (true, false, at("2026-10-30T09:12:00Z")), "a cache from before");

    // Forwarded here: the verb, the time and the icon, as Outlook writes them.
    let keys = vec![msg("plain").key];
    for op in [Op::SetFlag { keys: keys.clone(), flag: Flag::Forwarded, on: true }] {
        crate::graph::apply_remote(&c, &st, &op).unwrap();
    }
    let (verb, time, icon) = server.data.lock().unwrap().message(&plain).map(|m| (m.verb.clone(), m.verb_time.clone(), m.icon.clone())).unwrap();
    assert_eq!((verb.as_deref(), icon.as_deref()), (Some("104"), Some("262")));
    assert!(time.as_deref().and_then(crate::jmap::utc_date).is_some_and(|t| t > at("2026-01-01T00:00:00Z")), "{time:?}");
    sync(&mut st);
    assert!(matches!(marks("plain"), (false, true, t) if t > 0), "{:?}", msg("plain").flags);
    // Replied to in Outlook afterwards: the reply shows.
    if let Some(m) = server.data.lock().unwrap().messages.iter_mut().find(|m| m.id == plain) {
        (m.verb, m.verb_time) = (Some("103".into()), Some("2026-11-02T08:00:00Z".into()));
    }
    sync(&mut st);
    assert_eq!(marks("plain"), (true, false, at("2026-11-02T08:00:00Z")));
    let _ = std::fs::remove_dir_all(store.root());
}
