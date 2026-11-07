// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: mail is listed, headed and threaded by when it came, not by its `Date:`
//! (adapted from upstream 62a459d3).

use super::*;

/// Mail sent long before it came (a data file's, a forward) is listed, headed and threaded by when
/// it came; alone or in its conversation, it goes under Today. The search reads `received:` by
/// that time and `sent:` by its date.
#[test]
fn received_mail_is_listed_when_it_came() {
    let store = Store::memory();
    sample::seed_if_empty(&store, NOW).unwrap();
    let mut st = store.load_folder("sample", "Projects").unwrap();
    let m = |uid: u32, subject: &str, date: i64, received: i64| Summary {
        key: MessageKey { account: "sample".into(), folder: "Projects".into(), validity: st.validity, uid },
        message_id: format!("m{uid}@x"),
        subject: subject.into(),
        date,
        received,
        ..Default::default()
    };
    let mut late = m(1, "Late", NOW - 20 * 86_400, NOW - 60);
    late.in_reply_to = "m3@x".into();
    st.messages = vec![late, m(2, "Yesterday's", NOW - 86_400, 0), m(3, "First", NOW - 30 * 86_400, 0)];
    store.save_folder("sample", "Projects", &st).unwrap();
    let mut s = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    s.run("folder.open", &json!({"account": "sample", "folder": "Projects"})).unwrap();
    let subjects: Vec<&str> = s.list.iter().map(|m| m.subject.as_str()).collect();
    assert_eq!(subjects, ["Late", "Yesterday's", "First"]);
    let rows = s.rows();
    assert_eq!(rows[..2], [Row::Header("Today"), Row::Message { index: 0, count: 1, unread: 1 }]);
    assert_eq!(rows[2], Row::Header("Yesterday"));
    assert_eq!(s.run("list.messages", &json!({})).ok().and_then(|v| v["messages"][0]["received"].as_i64()), Some(NOW - 60));
    s.run("view.conversations", &json!({"on": true})).unwrap();
    let rows = s.rows();
    assert_eq!(rows[..2], [Row::Header("Today"), Row::Message { index: 0, count: 2, unread: 2 }]);
    assert_eq!(s.threads[0].latest, NOW - 60);
    // Searched in this folder: it came today, and was sent twenty days before.
    let found = |s: &mut Session, q: &str| s.run("search.run", &json!({"query": q, "scope": "all"})).unwrap()["results"].as_u64().unwrap_or(0);
    assert_eq!((found(&mut s, "received:today Late"), found(&mut s, "sent:today Late")), (1, 0));
    assert_eq!(found(&mut s, "date:2026 Late"), 1);
}
