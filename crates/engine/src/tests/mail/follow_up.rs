// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a follow-up marked complete (Mark Complete) stays so, with the day it was done,
//! until it's flagged again or cleared; Undo puts back what it changed, the days too.

use quatrelle_model::{FOLLOW_UP_COMPLETE, FieldChange};

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

/// Follow Up › Custom… sets a start day, a due day and a reminder; the reminder rings in the
/// reminders list with the message to open, and is dismissed or snoozed as an appointment's.
#[test]
fn a_flag_s_reminder_rings_and_is_dismissed_or_snoozed() {
    let mut s = sample_session("follow-up-reminder");
    let k = key_of(&s, "Character turnarounds");
    let day = quatrelle_model::dates::days_from_civil(2026, 12, 1);
    let start = quatrelle_model::dates::days_from_civil(2026, 11, 20);
    let now = s.clock.now;
    s.run("list.select", &k).unwrap();
    let r = s.run("mail.flag", &json!({"start": "2026-11-20", "due": "2026-12-01", "reminder": now - 60})).unwrap();
    assert_eq!(r["flagged"], true);
    let m = turnaround(&s);
    assert!(m.flags.flagged, "{:?}", m.flags);
    assert_eq!((s.follow_up_start(&m), s.follow_up_due(&m), s.follow_up_reminder(&m)), (Some(start), Some(day), Some(now - 60)));

    // Due: listed with the message to open.
    let due = s.due_reminders(now).into_iter().find(|r| r.message.as_ref() == Some(&m.key)).expect("the flag's reminder is due");
    assert_eq!(due.summary, m.subject);
    assert!(!due.task);
    s.run("reminders.dismiss", &json!({"key": due.key})).unwrap();
    assert!(s.due_reminders(now).iter().all(|r| r.message.as_ref() != Some(&m.key)), "dismissed");

    // Set again: it rings again (a dismissal doesn't outlive a new reminder), and a snooze
    // waits.
    s.run("mail.flag", &json!({"reminder": now - 30})).unwrap();
    let due = s.due_reminders(now).into_iter().find(|r| r.message.as_ref() == Some(&m.key)).expect("set anew, due again");
    s.run("reminders.snooze", &json!({"key": due.key, "minutes": 5})).unwrap();
    assert!(s.due_reminders(now).iter().all(|r| r.message.as_ref() != Some(&m.key)), "snoozed");
    assert!(s.due_reminders(now + 301).iter().any(|r| r.message.as_ref() == Some(&m.key)), "back after the snooze");

    // Add Reminder… takes it off (`false`), and the flag stays.
    s.run("mail.flag", &json!({"reminder": false})).unwrap();
    let m = turnaround(&s);
    assert_eq!(s.follow_up_reminder(&m), None);
    assert!(m.flags.flagged);
    done(s);
}

/// What Exchange keeps wins over what's kept here: its start, due day and reminder are the
/// follow-up's, the preferences only the fallback for what it doesn't have. The server's dates
/// are the day's floating midnight (`day * 86_400`, as `PidLidTask*Date` holds it), read as a
/// UTC day: west of UTC, reading one in the local zone would name the day before.
#[test]
fn the_server_s_follow_up_dates_win_over_the_preferences() {
    let mut s = sample_session("follow-up-server-wins");
    // A clock west of UTC (New York), where the local-zone read of a floating midnight shifts.
    s.clock = quatrelle_model::dates::Clock::new(NOW, -5 * 3600).in_zone(Some(quatrelle_model::zone::Zone::Fixed(-5 * 3600)));
    let mut m = turnaround(&s);
    let id = id(&m);
    let day = |y, mo, d| quatrelle_model::dates::days_from_civil(y, mo, d);
    m.flags.flagged = true;
    m.flags.follow_up.start = day(2026, 11, 30).saturating_mul(86_400);
    m.flags.follow_up.due = day(2026, 12, 3).saturating_mul(86_400);
    m.flags.follow_up.reminder = s.clock.now + 3600;
    s.prefs.follow_up_start.insert(id.clone(), day(2020, 1, 1));
    s.prefs.follow_up.insert(id.clone(), day(2020, 1, 2));
    s.prefs.follow_up_reminder.insert(id, s.clock.now + 60);
    assert_eq!(s.follow_up_start(&m), Some(day(2026, 11, 30)));
    assert_eq!(s.follow_up_due(&m), Some(day(2026, 12, 3)));
    assert_eq!(s.follow_up_reminder(&m), Some(s.clock.now + 3600));
    assert_eq!(m.flags.follow_up.due, day(2026, 12, 3) * 86_400, "the day's floating midnight, not a local one");
    done(s);
}

/// A day a follow-up is due goes to the server as the day's floating midnight (`day * 86_400`),
/// the convention Exchange's `PidLidTask*Date` holds ([MS-OXOTASK] 2.2.2.2.4): a local midnight
/// written there shows the day before in Outlook (in France, 22/11 leaves as the 21st).
#[test]
fn a_follow_up_day_goes_to_exchange_as_its_floating_midnight() {
    let mut s = sample_session("follow-up-floating");
    // A clock east of UTC (Paris), where a local midnight is not the floating one.
    s.clock = quatrelle_model::dates::Clock::new(NOW, 3600).in_zone(Some(quatrelle_model::zone::Zone::Fixed(3600)));
    let account = s
        .run("account.add", &json!({"email": "riley@contoso.example", "password": "pw", "ews": "https://ews.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let mut m = turnaround(&s);
    m.key.account = account.clone();
    let day = quatrelle_model::dates::days_from_civil(2026, 12, 1);
    s.follow_up_for(&[std::sync::Arc::new(m)], Some(day)).unwrap();
    let due = s
        .take_jobs()
        .iter()
        .find_map(|j| match j {
            Job::Apply { op: Op::SetFollowUp { due, .. }, .. } => Some(*due),
            _ => None,
        })
        .expect("the follow-up change went to the server");
    assert_eq!(due, Some(FieldChange::Set(day.saturating_mul(86_400))), "the floating midnight, not the local one ({})", s.clock.midnight(day));
    done(s);
}

/// Clear Flag on a message whose server keeps its follow-up (Exchange, Graph) takes the dates
/// and the reminder off there too, as Outlook's does — a SetFollowUp of its own, so Mark
/// Complete (which takes the flag off without it) keeps them; Undo puts them back.
#[test]
fn clear_flag_takes_the_follow_up_off_the_server_too() {
    let mut s = sample_session("clear-flag-server");
    let account = s
        .run("account.add", &json!({"email": "riley@contoso.example", "password": "pw", "ews": "https://ews.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let mut m = turnaround(&s);
    m.key.account = account.clone();
    m.flags.flagged = true;
    m.flags.follow_up.start = 20_000 * 86_400;
    m.flags.follow_up.due = 20_010 * 86_400;
    m.flags.follow_up.reminder = s.clock.now + 3600;
    let m = std::sync::Arc::new(m);
    // The message as the account's cache has it (`mail.flag`'s `key` reads the cache).
    s.cache.entry((account.clone(), m.key.folder.clone())).or_default().messages.push(m.clone());
    s.run("mail.flag", &json!({"key": m.key.to_id(), "on": false})).unwrap();
    let jobs = s.take_jobs();
    let cleared = jobs.iter().any(|j| {
        matches!(
            j,
            Job::Apply {
                op: Op::SetFollowUp { start: Some(FieldChange::Clear), due: Some(FieldChange::Clear), reminder: Some(FieldChange::Clear), .. },
                ..
            }
        )
    });
    assert!(cleared, "a SetFollowUp taking all three off went to the server: {jobs:?}");
    // Undo puts the server's dates and reminder back.
    s.run("edit.undo", &json!({})).unwrap();
    let jobs = s.take_jobs();
    let now = s.clock.now;
    let restored = jobs.iter().any(|j| match j {
        Job::Apply {
            op: Op::SetFollowUp { start: Some(FieldChange::Set(a)), due: Some(FieldChange::Set(b)), reminder: Some(FieldChange::Set(c)), .. },
            ..
        } => *a == 20_000 * 86_400 && *b == 20_010 * 86_400 && *c == now + 3600,
        _ => false,
    });
    assert!(restored, "the Undo's SetFollowUp puts the dates back: {jobs:?}");
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

/// A partial follow-up change names only its fields: Follow Up › Tomorrow on a message whose
/// server already has a start and a reminder sends only the due day, leaving the other two as
/// the server has them (a change that rewrote all three from the cache would erase them when
/// the cache didn't know them).
#[test]
fn a_partial_follow_up_change_names_only_its_fields() {
    let mut s = sample_session("partial-follow-up");
    let account = s
        .run("account.add", &json!({"email": "riley@contoso.example", "password": "pw", "ews": "https://ews.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let mut m = turnaround(&s);
    m.key.account = account.clone();
    m.flags.flagged = true;
    // As Outlook followed it: the server has all three (the cache read them).
    m.flags.follow_up.start = 20_000 * 86_400;
    m.flags.follow_up.due = 20_010 * 86_400;
    m.flags.follow_up.reminder = s.clock.now + 3600;
    let m = std::sync::Arc::new(m);
    s.cache.entry((account.clone(), m.key.folder.clone())).or_default().messages.push(m.clone());
    let day = quatrelle_model::dates::days_from_civil(2026, 12, 1);
    s.run("mail.flag", &json!({"key": m.key.to_id(), "due": "2026-12-01"})).unwrap();
    let op = s
        .take_jobs()
        .iter()
        .find_map(|j| match j {
            Job::Apply { op: Op::SetFollowUp { start, due, reminder, .. }, .. } => Some((*start, *due, *reminder)),
            _ => None,
        })
        .expect("the follow-up change went to the server");
    assert_eq!(op.0, None, "start unnamed: left as the server has it");
    assert_eq!(op.1, Some(FieldChange::Set(day.saturating_mul(86_400))), "due named");
    assert_eq!(op.2, None, "reminder unnamed: left as the server has it");
    done(s);
}

/// Flagging a selection that includes an already-followed message writes the flag only on the
/// others: the followed one keeps the text and icon Outlook gave its flag ([MS-OXOFLAG]
/// 3.1.4.1.3 — `PidLidFlagRequest` is set only if it doesn't already exist), not "Follow up"
/// and the plain red flag again.
#[test]
fn flagging_a_selection_leaves_an_already_flagged_message_s_flag_alone() {
    let mut s = sample_session("flag-selection");
    let account = s
        .run("account.add", &json!({"email": "riley@contoso.example", "password": "pw", "ews": "https://ews.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let followed = {
        let mut m = turnaround(&s);
        m.key.account = account.clone();
        m.flags.flagged = true;
        m.flags.follow_up.title = "Review".into();
        std::sync::Arc::new(m)
    };
    let fresh = {
        let mut m = turnaround(&s);
        m.key.account = account.clone();
        m.key.uid += 1;
        m.subject = "Fresh".into();
        std::sync::Arc::new(m)
    };
    s.set_flagged(&[followed.clone(), fresh.clone()], true);
    let jobs = s.take_jobs();
    let flagged_keys: Vec<u32> = jobs
        .iter()
        .filter_map(|j| match j {
            Job::Apply { op: Op::SetFlag { keys, flag: Flag::Flagged, on: true, .. }, .. } => Some(keys.iter().map(|k| k.uid).collect::<Vec<_>>()),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(flagged_keys, vec![fresh.key.uid], "only the message whose flag changed got the write: {jobs:?}");
    done(s);
}

/// Reminder times and answers kept on this computer come back from disk as the Unix times they
/// are: read with a day count, every one of them (billions of seconds) would be refused and the
/// reminders lost at each start. They go back as they were.
#[test]
fn reminder_times_kept_on_this_computer_read_back_from_disk() {
    let at = 1_791_979_140; // 2026-11-25T09:19:00Z
    let read = json!({
        "followUpReminder": {"ring@x": at, "far@x": i64::MAX, "early@x": -1, "": at},
        "reminders": {"a1|mail|k1": 0, "a1|t1|1700000000": at, "a1|t1|bad": i64::MAX},
    });
    let p: crate::Prefs = serde_json::from_value(read).unwrap();
    assert_eq!(p.follow_up_reminder, std::collections::BTreeMap::from([("ring@x".to_string(), at)]));
    assert_eq!(p.reminders, std::collections::BTreeMap::from([("a1|mail|k1".to_string(), 0), ("a1|t1|1700000000".to_string(), at)]));
    // And they are saved as they were read.
    let saved = serde_json::to_value(&p).unwrap();
    assert_eq!(saved["followUpReminder"]["ring@x"], json!(at));
    assert_eq!(saved["reminders"]["a1|t1|1700000000"], json!(at));
}
