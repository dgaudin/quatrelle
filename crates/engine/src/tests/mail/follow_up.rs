// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a follow-up marked complete (Mark Complete) stays so, with the day it was done,
//! until it's flagged again or cleared; Undo puts back what it changed, the days too.

use quatrelle_model::FOLLOW_UP_COMPLETE;

use super::*;

/// What a message's follow-up days are kept under (its Message-ID, the sample's all have one).
fn id(m: &Summary) -> String {
    m.message_id.trim().to_string()
}

fn turnaround(s: &Session) -> Summary {
    s.list.iter().find(|m| m.subject == "Character turnarounds").map(|m| (**m).clone()).unwrap()
}

/// Mark Complete unflags the message and marks it done today; Undo flags it again, due as it
/// was, and forgets the day done; flagging again or Clear Flag ends the completed state, and
/// their Undo brings back the days it was due and done; the list arranged by the flag shows it
/// under its own heading, and `is:complete` finds it.
#[test]
fn a_follow_up_marked_complete_stays_so() {
    let mut s = sample_session("complete");
    let k = key_of(&s, "Character turnarounds");
    let today = quatrelle_model::dates::today(&s.clock);
    s.run("list.select", &k).unwrap();
    s.run("mail.flag", &json!({"due": "2026-12-01"})).unwrap();
    let due = s.follow_up_due(&turnaround(&s));
    assert!(due.is_some());

    // Complete: not flagged, done today, its due day kept for Undo.
    let r = s.run("mail.flag", &json!({"complete": true})).unwrap();
    assert_eq!(r["complete"], true);
    let m = turnaround(&s);
    assert!(!m.flags.flagged && m.flags.follow_up_complete(), "{:?}", m.flags);
    assert_eq!((s.follow_up_done(&m), s.follow_up_due(&m)), (Some(today), None));
    assert!(!s.categories_of(&m).iter().any(|c| c.keyword.eq_ignore_ascii_case(FOLLOW_UP_COMPLETE)), "not a category");
    assert_eq!(s.run("search.run", &json!({"query": "is:complete"})).unwrap()["results"], 1);
    s.run("search.clear", &json!({})).unwrap();

    // Arranged by the flag: under Completed, after the flagged ones.
    s.run("view.arrange", &json!({"by": "flag"})).unwrap();
    let rows = s.rows().to_vec();
    let at = |h: &'static str| rows.iter().position(|r| *r == Row::Header(h));
    assert!(matches!((at("Flagged"), at("Completed"), at("Not flagged")), (Some(a), Some(b), Some(c)) if a < b && b < c), "{rows:?}");
    s.run("view.arrange", &json!({"by": "date"})).unwrap();

    // Undo, in one step: flagged again, due as it was, no longer complete.
    s.run("edit.undo", &json!({})).unwrap();
    let m = turnaround(&s);
    assert!(m.flags.flagged && !m.flags.has(FOLLOW_UP_COMPLETE), "{:?}", m.flags);
    assert_eq!((s.follow_up_due(&m), s.follow_up_done(&m)), (due, None));
    assert!(s.prefs.follow_up_done.is_empty(), "the day done goes with it: {:?}", s.prefs.follow_up_done);

    // Marked complete again: a second time keeps the day it was first done.
    s.run("list.select", &k).unwrap();
    s.run("mail.flag", &json!({"complete": true})).unwrap();
    s.prefs.follow_up_done.insert(id(&turnaround(&s)), today - 3);
    s.run("mail.flag", &json!({"complete": true})).unwrap();
    assert_eq!(s.follow_up_done(&turnaround(&s)), Some(today - 3));

    // Flagged again (the toggle, as a click on the tick): a fresh flag, without the old day or
    // the done one.
    s.run("mail.flag", &json!({})).unwrap();
    let m = turnaround(&s);
    assert!(m.flags.flagged && !m.flags.has(FOLLOW_UP_COMPLETE));
    assert_eq!((s.follow_up_due(&m), s.prefs.follow_up_done.get(&id(&m))), (None, None));
    // Its Undo marks it complete again, with the days it was due and done.
    s.run("edit.undo", &json!({})).unwrap();
    let m = turnaround(&s);
    assert!(m.flags.follow_up_complete());
    assert_eq!((s.follow_up_done(&m), s.prefs.follow_up.get(&id(&m)).copied()), (Some(today - 3), due));

    // Clear Flag on it, and Undo: complete again, the days back too.
    s.run("list.select", &k).unwrap();
    s.run("mail.flag", &json!({"on": false})).unwrap();
    assert!(!turnaround(&s).flags.has(FOLLOW_UP_COMPLETE) && s.prefs.follow_up_done.is_empty());
    s.run("edit.undo", &json!({})).unwrap();
    let m = turnaround(&s);
    assert!(m.flags.follow_up_complete());
    assert_eq!((s.follow_up_done(&m), s.prefs.follow_up.get(&id(&m)).copied()), (Some(today - 3), due));

    // Clear Flag: neither flagged nor complete.
    s.run("list.select", &k).unwrap();
    s.run("mail.flag", &json!({"on": false})).unwrap();
    let m = turnaround(&s);
    assert!(!m.flags.flagged && !m.flags.has(FOLLOW_UP_COMPLETE));
    assert!(s.prefs.follow_up.is_empty() && s.prefs.follow_up_done.is_empty());

    // An unflagged message can be marked complete too; Undo takes the mark off alone.
    s.run("mail.flag", &json!({"complete": true})).unwrap();
    assert!(turnaround(&s).flags.follow_up_complete());
    s.run("edit.undo", &json!({})).unwrap();
    let m = turnaround(&s);
    assert!(!m.flags.flagged && !m.flags.has(FOLLOW_UP_COMPLETE));
    assert!(s.prefs.follow_up_done.is_empty(), "no day done left behind");
    done(s);
}

/// The day the server says it was done (Exchange keeps it) wins over the one kept here; a
/// change made here forgets it until the server says again.
#[test]
fn the_day_exchange_says_it_was_done_wins() {
    let mut s = sample_session("complete-server-day");
    let k = key_of(&s, "Character turnarounds");
    let today = quatrelle_model::dates::today(&s.clock);
    s.run("list.select", &k).unwrap();
    s.run("mail.flag", &json!({"complete": true})).unwrap();
    let key = turnaround(&s).key.clone();
    // As a sync from Exchange leaves it: the time it was done, ten days ago.
    let st = s.cache.get_mut(&(key.account.clone(), key.folder.clone())).unwrap();
    let m = st.messages.iter_mut().find(|m| m.key == key).unwrap();
    std::sync::Arc::make_mut(m).flags.completed = s.clock.now - 10 * 86_400;
    let m = st.messages.iter().find(|m| m.key == key).cloned().unwrap();
    assert_eq!(s.follow_up_done(&m), Some(today - 10));
    // Flagged again here: the server's time goes with the mark.
    s.run("mail.flag", &json!({"on": true})).unwrap();
    let m = turnaround(&s);
    assert_eq!((m.flags.completed, s.follow_up_done(&m)), (0, None));
    done(s);
}

/// Flagged again by another program, the keyword left behind: flagged, as the list shows it.
/// Flagging it here keeps its due day and takes the stale keyword off; `is:complete` doesn't
/// find it.
#[test]
fn a_flag_set_again_elsewhere_wins_over_a_stale_complete_mark() {
    let mut s = sample_session("stale-complete");
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    s.run("mail.flag", &json!({"due": "2026-12-01"})).unwrap();
    let due = s.follow_up_due(&turnaround(&s));
    assert!(due.is_some());
    let key = turnaround(&s).key.clone();
    s.apply_op(&key.account.clone(), Op::SetKeyword { keys: vec![key], keyword: FOLLOW_UP_COMPLETE.into(), on: true, name: None });
    let m = turnaround(&s);
    assert!(m.flags.flagged && m.flags.has(FOLLOW_UP_COMPLETE) && !m.flags.follow_up_complete());
    assert_eq!(s.run("search.run", &json!({"query": "is:complete"})).unwrap()["results"], 0);
    s.run("search.clear", &json!({})).unwrap();

    s.run("list.select", &k).unwrap();
    s.run("mail.flag", &json!({"on": true})).unwrap();
    let m = turnaround(&s);
    assert!(m.flags.flagged && !m.flags.has(FOLLOW_UP_COMPLETE), "{:?}", m.flags);
    assert_eq!(s.follow_up_due(&m), due, "still due as it was");
    done(s);
}

/// The days kept in the preferences come from disk: what isn't a day (out of range, as a
/// damaged or hostile file has it, a key too long or empty) is left out as they're read, so
/// the reading pane never labels it.
#[test]
fn the_preferences_days_read_from_disk_are_days() {
    let long = "x".repeat(999);
    let read = json!({
        "followUp": {"due@x": 20_400, "far@x": i64::MAX, (long.clone()): 20_400},
        "followUpDone": {"done@x": 20_390, "early@x": i64::MIN, "": 20_390},
    });
    let p: crate::Prefs = serde_json::from_value(read).unwrap();
    assert_eq!(p.follow_up, std::collections::BTreeMap::from([("due@x".to_string(), 20_400)]));
    assert_eq!(p.follow_up_done, std::collections::BTreeMap::from([("done@x".to_string(), 20_390)]));
}
