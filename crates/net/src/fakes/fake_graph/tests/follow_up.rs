// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A follow-up marked complete over the fake Graph server: Graph's one `flagStatus` holds both
//! the flag and the follow-up marked complete (`complete`, with its `completedDateTime`).

use quatrelle_model::{FOLLOW_UP_COMPLETE, Flag, Op};

use super::*;

/// Marked complete in Outlook, a message comes in complete, on the day Graph says; marked
/// complete here (the flag off, then the keyword on), Graph says complete; its Undo (the keyword
/// off, then the flag on) flagged; flagged again in Outlook, flagged here at the next sync.
#[test]
fn a_follow_up_marked_complete_goes_both_ways() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    let done = d.deliver(&inbox, b"From: a@x.example\r\nSubject: done\r\n\r\nx\r\n", false, "2024-05-01T09:00:00Z");
    let flagged = d.deliver(&inbox, b"From: a@x.example\r\nSubject: flagged\r\n\r\nx\r\n", false, "2024-05-02T09:00:00Z");
    for m in d.messages.iter_mut() {
        if m.id == done {
            m.complete = Some("2026-10-30T09:12:00.0000000".into());
        }
        m.flagged = m.id == flagged;
    }
    let server = serve(d);
    let dir = std::env::temp_dir().join(format!("quatrelle-graph-follow-up-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let mut st = crate::jmap::State::default();
    crate::graph::list_folders(&c, &mut st, "a1", &[]).unwrap();
    let opts = crate::sync::SyncOptions { prefetch: 0, ..Default::default() };
    let sync = |st: &mut crate::jmap::State| crate::graph::sync_folder(&c, st, &store, "a1", "INBOX", &opts).unwrap();
    let msg = |subject: &str| store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == subject).cloned().unwrap();
    let on_server = |id: &str| server.data.lock().unwrap().message(id).map(|m| (m.flagged, m.complete.is_some())).unwrap();
    let at = |text: &str| crate::jmap::utc_date(text).unwrap();

    sync(&mut st);
    let m = msg("done");
    assert!(m.flags.follow_up_complete() && m.flags.completed == at("2026-10-30T09:12:00Z"), "{:?}", m.flags);
    assert!(msg("flagged").flags.flagged);

    let keys = vec![msg("flagged").key];
    let flag = |on| Op::SetFlag { keys: keys.clone(), flag: Flag::Flagged, on };
    let complete = |on| Op::SetKeyword { keys: keys.clone(), keyword: FOLLOW_UP_COMPLETE.into(), on, name: None };
    let apply = |st: &crate::jmap::State, op: Op| crate::graph::apply_remote(&c, st, &op).unwrap();
    apply(&st, flag(false));
    apply(&st, complete(true));
    assert_eq!(on_server(&flagged), (false, true), "complete");
    sync(&mut st);
    let m = msg("flagged");
    assert!(m.flags.follow_up_complete() && m.flags.completed == at("2026-11-05T10:00:00Z"), "{:?}", m.flags);
    apply(&st, complete(false));
    apply(&st, flag(true));
    assert_eq!(on_server(&flagged), (true, false), "flagged again");
    sync(&mut st);
    let m = msg("flagged");
    assert!(m.flags.flagged && !m.flags.has(FOLLOW_UP_COMPLETE) && m.flags.completed == 0, "{:?}", m.flags);

    // In Outlook: the one done flagged again.
    if let Some(m) = server.data.lock().unwrap().messages.iter_mut().find(|m| m.id == done) {
        (m.flagged, m.complete) = (true, None);
    }
    sync(&mut st);
    let m = msg("done");
    assert!(m.flags.flagged && !m.flags.has(FOLLOW_UP_COMPLETE) && m.flags.completed == 0, "{:?}", m.flags);
    let _ = std::fs::remove_dir_all(&dir);
}
