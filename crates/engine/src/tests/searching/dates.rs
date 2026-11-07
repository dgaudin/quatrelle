// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: dates in a search (adapted from upstream 41464ec1 and 427337f4).

use super::*;

/// `received:` days are placed where the user is, from the session's clock, and answered from
/// the summaries alone; a date that isn't one is left out rather than looked for as digits.
#[test]
fn search_reads_dates() {
    let mut s = sample_session("search-dates");
    let all = |s: &mut Session, q: &str| s.run("search.run", &json!({"query": q, "scope": "all"})).unwrap()["results"].as_u64().unwrap() as usize;
    let c = quatrelle_model::dates::civil(s.clock.now, &s.clock);
    let day = format!("{:04}-{:02}-{:02}", c.year, c.month, c.day);
    let start = crate::parse_local(&day, &s.clock).map(|(t, _)| t).unwrap_or_default();
    let on_day = |m: &Summary| (start..start + 86_400).contains(&m.date);
    let today = summaries::counted(&mut s, on_day);
    let before = summaries::counted(&mut s, |m| m.date < start);
    assert!(today > 0 && before > 0, "the sample has mail today and before");
    let reads = s.store.as_ref().map_or(0, |st| st.reads());
    assert_eq!(all(&mut s, "received:today"), today);
    assert_eq!(all(&mut s, "reçu:aujourd'hui"), today, "in French too");
    assert_eq!(all(&mut s, &format!("received:{day}")), today);
    assert_eq!(all(&mut s, &format!("before:{day}")), before);
    assert_eq!(all(&mut s, &format!("avant:{day}")), before);
    assert_eq!(all(&mut s, &format!("-received:{day}")), summaries::counted(&mut s, |m| !on_day(m)));
    assert_eq!(s.store.as_ref().map_or(0, |st| st.reads()), reads, "dates are the summaries': no message was read");
    assert_eq!(all(&mut s, "received:2026-13"), 0, "not a date: left out, nothing to search");
    done(s);
}

/// The servers are asked for words, not dates: what they find outside a search's dates isn't
/// listed (a found message as the server sent it, or a key it answered).
#[test]
fn server_finds_are_checked_against_the_dates() {
    let mut s = sample_session("server-dates");
    let now = s.clock.now;
    let found = |uid: u32, subject: &str, date: i64| Summary {
        key: MessageKey { account: "sample".into(), folder: "Archive".into(), validity: 1, uid },
        subject: subject.into(),
        date,
        ..Default::default()
    };
    for (q, want) in [("needle received:today", "New one"), ("needle -received:today", "Old one")] {
        s.run("search.run", &json!({"query": q, "scope": "all"})).unwrap();
        s.apply(Event::Found {
            account: "sample".into(),
            query: q.into(),
            items: vec![found(9_001, "Old one", 86_400), found(9_002, "New one", now)],
            error: None,
        });
        let shown: Vec<&str> = s.list.iter().map(|m| m.subject.as_str()).collect();
        assert_eq!(shown, [want], "{q}");
    }
    done(s);
}
