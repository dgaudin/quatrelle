// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: the scam check, the language model reading external mail as it arrives (its
//! option, what is sent and what is not, the queue and the cap, hostile or missing answers). The
//! model's server is the test's own: jobs taken, answers applied by hand.

use quatrelle_model::scam::ScamDoc;
use quatrelle_model::{AccountConfig, Flags, Folder};
use quatrelle_store::FolderState;

use super::*;

const ME: &str = "zoe";

/// A session with one account of Zoe Park's at `email`, on Exchange or (`exchange` false) on IMAP,
/// an empty Inbox whose high-water mark is set, and the language model set.
fn session_at(name: &str, email: &str, exchange: bool) -> Session {
    let store = temp(name);
    let config = AccountConfig {
        id: ME.into(),
        name: "Zoe Park".into(),
        email: email.into(),
        username: "zoe".into(),
        ews: exchange.then(|| "https://mail.studio.example/EWS/Exchange.asmx".into()),
        ..Default::default()
    };
    store.save_accounts(&[config]).unwrap();
    let folders = vec![Folder { account: ME.into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() }];
    store.save_folder(ME, "INBOX", &FolderState::<Summary> { validity: 1, uid_next: 1, ..Default::default() }).unwrap();
    store.save_folders(ME, &folders).unwrap();
    let mut s = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "glm"})).unwrap();
    s.apply(Event::FolderChanged { account: ME.into(), folder: "INBOX".into() });
    s.take_jobs();
    s
}

fn zoe(name: &str) -> Session {
    session_at(name, "zoe@studio.example", true)
}

fn on(s: &mut Session) {
    s.run("assistant.scamCheck", &json!({"enabled": true})).unwrap();
}

/// A message source from `from` (name, address) with `subject`; plain text, or HTML when `html`.
fn source(id: &str, from: (&str, &str), subject: &str, body: &str, html: bool) -> Vec<u8> {
    let kind = if html { "text/html" } else { "text/plain" };
    format!(
        "From: {} <{}>\r\nTo: zoe@studio.example\r\nSubject: {subject}\r\nMessage-ID: <{id}@x.example>\r\nDate: Tue, 20 Oct 2026 09:00:00 +0000\r\nMIME-Version: 1.0\r\nContent-Type: {kind}; charset=utf-8\r\n\r\n{body}\r\n",
        from.0, from.1
    )
    .into_bytes()
}

const STRANGER: (&str, &str) = ("Sam Doe", "sam.doe@elsewhere.example");

/// Put messages in the Inbox (`uid`, source, whether its source is already kept) as one sync does,
/// their date `age` seconds before now, and open the Inbox.
fn arrive(s: &mut Session, msgs: &[(u32, Vec<u8>, bool)], age: i64) -> Vec<MessageKey> {
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder(ME, "INBOX").unwrap();
    let mut keys = Vec::new();
    for (uid, raw, kept) in msgs {
        let key = MessageKey { account: ME.into(), folder: "INBOX".into(), validity: inbox.validity, uid: *uid };
        let mut sum = quatrelle_mime::summary_from_full(raw, key.clone(), Flags::default(), 0, NOW);
        sum.date = NOW - age;
        sum.received = NOW - age;
        inbox.messages.insert(0, sum);
        inbox.uid_next = inbox.uid_next.max(uid + 1);
        if *kept {
            store.write_body(&key, raw).unwrap();
        }
        keys.push(key);
    }
    store.save_folder(ME, "INBOX", &inbox).unwrap();
    s.forget_echoes(ME);
    s.apply(Event::FolderChanged { account: ME.into(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": ME, "folder": "INBOX"})).unwrap();
    keys
}

/// The questions put to the model among `jobs`: (id, the mail's text).
fn asks(jobs: &[Job]) -> Vec<(u64, String)> {
    jobs.iter()
        .filter_map(|j| match j {
            Job::AskLlm { id, messages, .. } => Some((*id, messages.last().map(|m| m.1.clone()).unwrap_or_default())),
            _ => None,
        })
        .collect()
}

fn answer(s: &mut Session, id: u64, text: &str) {
    s.apply(Event::LlmAnswered { id, text: text.into(), error: None });
}

const SCAM: &str = r#"{"score": 85, "reasons": ["Urgent transfer to a new account"]}"#;

fn verdict(s: &mut Session, key: &MessageKey) -> Value {
    s.run("scam.verdict", &json!({"key": key.to_id()})).unwrap()
}

#[test]
fn the_scam_option_needs_a_model_and_exchange() {
    let store = temp("scam-option");
    let config = AccountConfig { id: ME.into(), email: "zoe@studio.example".into(), username: "zoe".into(), ..Default::default() };
    store.save_accounts(&[config]).unwrap();
    let mut s = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    assert!(s.run("assistant.scamCheck", &json!({"enabled": true})).is_err(), "no model set");
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "glm"})).unwrap();
    assert!(s.run("assistant.scamCheck", &json!({"accounts": [ME]})).is_err(), "an IMAP account");
    assert!(s.run("assistant.scamCheck", &json!({"accounts": "zoe"})).is_err());
    assert!(s.run("assistant.scamCheck", &json!({"dailyCap": 0})).is_err() && s.run("assistant.scamCheck", &json!({"dailyCap": 2001})).is_err());
    let r = s.run("assistant.scamCheck", &json!({"enabled": true, "dailyCap": 50})).unwrap();
    assert_eq!((r["enabled"].as_bool(), r["dailyCap"].as_u64()), (Some(true), Some(50)));
    assert!(!s.prefs.scam_check || s.prefs.scam_accounts.is_empty(), "nothing is ticked when no account is Exchange");
    done(s);
    let mut s = zoe("scam-option-ews");
    assert!(!s.prefs.scam_check, "off until turned on");
    let r = s.run("assistant.scamCheck", &json!({"enabled": true})).unwrap();
    assert_eq!(r["accounts"], json!([ME]), "every Exchange account, ticked");
    done(s);
}

#[test]
fn an_unknown_external_senders_mail_is_judged() {
    let mut s = zoe("scam-judged");
    on(&mut s);
    let raw = source("a1", STRANGER, "Overdue invoice", "Pay now to the new account.", false);
    let keys = arrive(&mut s, &[(5, raw.clone(), false)], 60);
    let jobs = s.take_jobs();
    assert!(jobs.iter().any(|j| matches!(j, Job::FetchBody { key } if *key == keys[0])), "the source is asked for");
    assert!(asks(&jobs).is_empty());
    s.apply(Event::Body { key: keys[0].clone(), raw });
    let q = asks(&s.take_jobs());
    assert_eq!(q.len(), 1);
    assert!(q[0].1.contains("Overdue invoice") && q[0].1.contains("sam.doe@elsewhere.example"));
    assert_eq!(verdict(&mut s, &keys[0]), Value::Null, "nothing before the answer");
    answer(&mut s, q[0].0, SCAM);
    assert_eq!(verdict(&mut s, &keys[0])["score"], 85);
    assert_eq!(verdict(&mut s, &keys[0])["reasons"], json!(["Urgent transfer to a new account"]));
    let kept: ScamDoc = s.store.as_ref().unwrap().load_account_doc(ME, "scam.json").unwrap();
    assert_eq!(kept.verdicts.len(), 1, "kept on disk");
    done(s);
}

#[test]
fn known_senders_internal_mail_and_the_backlog_are_not_sent() {
    let asked = |name: &str, setup: &dyn Fn(&mut Session), from: (&str, &str), age: i64, exchange: bool, enabled: bool| {
        let mut s = session_at(name, "zoe@studio.example", exchange);
        if enabled {
            on(&mut s);
        }
        setup(&mut s);
        s.take_jobs();
        arrive(&mut s, &[(5, source("b1", from, "Hello", "Plain text, nothing odd.", false), true)], age);
        let n = asks(&s.take_jobs()).len();
        done(s);
        n
    };
    let nothing: &dyn Fn(&mut Session) = &|_| {};
    assert_eq!(asked("scam-ctl", nothing, STRANGER, 60, true, true), 1, "the control case is asked");
    let known: &dyn Fn(&mut Session) = &|s| {
        s.run("contact.create", &json!({"name": "Sam Doe", "emails": [STRANGER.1]})).unwrap();
    };
    assert_eq!(asked("scam-known", known, STRANGER, 60, true, true), 0, "a known sender, no sign");
    assert_eq!(asked("scam-inside", nothing, ("Bob", "bob@studio.example"), 60, true, true), 0, "internal");
    assert_eq!(asked("scam-old", nothing, STRANGER, 3 * 86_400, true, true), 0, "older than a day");
    assert_eq!(asked("scam-imap", nothing, STRANGER, 60, false, true), 0, "an IMAP account");
    assert_eq!(asked("scam-off", nothing, STRANGER, 60, true, false), 0, "the option off");
    let safe: &dyn Fn(&mut Session) = &|s| {
        s.prefs.safe_senders.insert(STRANGER.1.into());
    };
    assert_eq!(asked("scam-safe", safe, STRANGER, 60, true, true), 0, "a safe sender");
    // The backlog of a first sync: a message already there when the mark is first set.
    let mut s = zoe("scam-backlog");
    on(&mut s);
    let store = s.store.clone().unwrap();
    s.seen_uid.clear();
    let mut inbox = store.load_folder(ME, "INBOX").unwrap();
    let raw = source("b2", STRANGER, "Old news", "text", false);
    let key = MessageKey { account: ME.into(), folder: "INBOX".into(), validity: 1, uid: 3 };
    inbox.messages.insert(0, quatrelle_mime::summary_from_full(&raw, key.clone(), Flags::default(), 0, NOW));
    store.save_folder(ME, "INBOX", &inbox).unwrap();
    store.write_body(&key, &raw).unwrap();
    s.apply(Event::FolderChanged { account: ME.into(), folder: "INBOX".into() });
    assert!(asks(&s.take_jobs()).is_empty(), "a first sync sends nothing");
    done(s);
}

#[test]
fn a_known_sender_with_a_misleading_link_is_sent() {
    let mut s = zoe("scam-link");
    s.run("contact.create", &json!({"name": "Sam Doe", "emails": [STRANGER.1]})).unwrap();
    on(&mut s);
    s.take_jobs();
    let html = r#"<p>Your statement:</p><a href="https://evil.example/login">www.my-bank.example</a>"#;
    arrive(&mut s, &[(5, source("c1", STRANGER, "Statement", html, true), true)], 60);
    let q = asks(&s.take_jobs());
    assert_eq!(q.len(), 1);
    assert!(q[0].1.contains("evil.example") && q[0].1.contains("Signs found") && q[0].1.contains("known to the reader: yes"), "{}", q[0].1);
    done(s);
}

#[test]
fn the_same_message_is_judged_once() {
    let mut s = zoe("scam-twice");
    on(&mut s);
    let raw = source("d1", STRANGER, "Twice", "Pay now.", false);
    arrive(&mut s, &[(5, raw.clone(), true), (6, raw, true)], 60);
    let q = asks(&s.take_jobs());
    assert_eq!(q.len(), 1, "one question for one Message-ID");
    answer(&mut s, q[0].0, SCAM);
    assert!(asks(&s.take_jobs()).is_empty(), "the second copy is not sent after the first is judged");
    done(s);
}

#[test]
fn the_cap_and_the_queue_hold() {
    let mut s = zoe("scam-cap");
    on(&mut s);
    s.run("assistant.scamCheck", &json!({"dailyCap": 2})).unwrap();
    let mut asked = 0;
    for n in 0..3u32 {
        arrive(&mut s, &[(10 + n, source(&format!("e{n}"), STRANGER, &format!("Capped {n}"), "Pay now.", false), true)], 60);
        let q = asks(&s.take_jobs());
        asked += q.len();
        for (id, _) in q {
            answer(&mut s, id, SCAM);
        }
    }
    assert_eq!(asked, 2, "the third is past the day's cap");
    done(s);
    let mut s = zoe("scam-queue");
    on(&mut s);
    let batch: Vec<(u32, Vec<u8>, bool)> =
        (0..60u32).map(|n| (100 + n, source(&format!("f{n}"), STRANGER, &format!("Burst {n}"), "Pay now.", false), true)).collect();
    arrive(&mut s, &batch, 60);
    let mut subjects = Vec::new();
    let mut jobs = s.take_jobs();
    while let Some((id, text)) = asks(&jobs).into_iter().next() {
        subjects.push(text);
        answer(&mut s, id, r#"{"score": 10}"#);
        jobs = s.take_jobs();
    }
    assert_eq!(subjects.len(), 50, "the queue holds 50");
    assert!(
        subjects.iter().all(|t| !t.contains("Subject: Burst 9\n")) && subjects.iter().any(|t| t.contains("Subject: Burst 59\n")),
        "the newest are kept"
    );
    done(s);
}

#[test]
fn a_failing_or_silent_server_keeps_nothing() {
    let mut s = zoe("scam-fail");
    on(&mut s);
    let msgs: Vec<(u32, Vec<u8>, bool)> =
        (0..4u32).map(|n| (20 + n, source(&format!("g{n}"), STRANGER, &format!("Try {n}"), "Pay now.", false), true)).collect();
    let keys = arrive(&mut s, &msgs, 60);
    let first = asks(&s.take_jobs());
    assert_eq!(first.len(), 1, "one at a time");
    s.apply(Event::LlmAnswered { id: first[0].0, text: String::new(), error: Some("HTTP 500".into()) });
    let second = asks(&s.take_jobs());
    assert_eq!(second.len(), 1, "the next is asked after a failure");
    answer(&mut s, second[0].0, "<html>nonsense</html>");
    let third = asks(&s.take_jobs());
    assert_eq!(third.len(), 1);
    // Silence: 61 seconds on, the next is asked.
    s.set_clock(NOW + 61, 0);
    s.tick();
    let fourth = asks(&s.take_jobs());
    assert_eq!(fourth.len(), 1, "no answer within 60 s: the next one");
    for k in &keys {
        assert_eq!(verdict(&mut s, k), Value::Null);
    }
    // A late answer to the one given up on changes nothing.
    answer(&mut s, third[0].0, SCAM);
    assert!(keys.iter().all(|k| s.run("scam.verdict", &json!({"key": k.to_id()})).unwrap().is_null()));
    done(s);
}

#[test]
fn an_organisation_unknown_sends_nothing() {
    let mut s = session_at("scam-noorg", "zoe.park@gmail.com", true);
    on(&mut s);
    arrive(&mut s, &[(5, source("h1", STRANGER, "Hello", "Pay now.", false), true)], 60);
    assert!(asks(&s.take_jobs()).is_empty(), "a mail service's address says nothing of the organisation");
    done(s);
}

#[test]
fn not_a_scam_trusts_the_sender() {
    let mut s = zoe("scam-notscam");
    on(&mut s);
    let keys = arrive(&mut s, &[(5, source("i1", STRANGER, "Invoice", "Pay now.", false), true)], 60);
    let q = asks(&s.take_jobs());
    answer(&mut s, q[0].0, SCAM);
    assert_eq!(verdict(&mut s, &keys[0])["score"], 85);
    s.run("list.open", &json!({"key": keys[0].to_id()})).unwrap();
    s.run("scam.notScam", &json!({})).unwrap();
    assert_eq!(verdict(&mut s, &keys[0]), Value::Null);
    let kept: ScamDoc = s.store.as_ref().unwrap().load_account_doc(ME, "scam.json").unwrap();
    assert!(kept.trusted.contains(STRANGER.1) && kept.verdicts.is_empty(), "saved");
    // Never sent again.
    arrive(&mut s, &[(6, source("i2", STRANGER, "Another", "Pay now.", false), true)], 60);
    assert!(asks(&s.take_jobs()).is_empty());
    // Taken back in Options.
    assert_eq!(s.run("scam.untrust", &json!({"address": "SAM.DOE@elsewhere.example"})).unwrap()["removed"], true);
    arrive(&mut s, &[(7, source("i3", STRANGER, "Third", "Pay now.", false), true)], 60);
    assert_eq!(asks(&s.take_jobs()).len(), 1);
    done(s);
}

/// A source over what the check holds in memory is read back from the cache; one the cache
/// doesn't have either is given up on, never asked for again (no loop of downloads).
#[test]
fn a_source_that_cant_be_read_is_asked_for_once() {
    let mut s = zoe("scam-source-once");
    on(&mut s);
    let raw = source("a9", STRANGER, "Big invoice", &"Pay now. ".repeat(40_000), false);
    let keys = arrive(&mut s, &[(9, raw.clone(), false)], 60);
    let jobs = s.take_jobs();
    assert_eq!(jobs.iter().filter(|j| matches!(j, Job::FetchBody { key } if *key == keys[0])).count(), 1);
    // It comes, too big to hold, and the cache doesn't have it (the worker would have kept it).
    s.apply(Event::Body { key: keys[0].clone(), raw });
    let jobs = s.take_jobs();
    assert!(asks(&jobs).is_empty(), "nothing to send");
    assert!(!jobs.iter().any(|j| matches!(j, Job::FetchBody { .. })), "not asked for again");
    done(s);
}

/// "Try it on examples": the twenty are put to the model one at a time; the counts, the wrong
/// ones and the seconds a message took come out. Nothing of the user's is sent.
#[test]
fn the_test_counts_hits_and_misses() {
    use quatrelle_model::scam::EXAMPLES;
    let mut s = session_at("scam-trial-nomodel", "zoe@studio.example", true);
    s.prefs.llm_model.clear();
    assert!(s.run("assistant.testScam", &json!({})).is_err(), "no model set");
    done(s);
    let mut s = zoe("scam-trial");
    let r = s.run("assistant.testScam", &json!({})).unwrap();
    assert_eq!((r["running"].as_bool(), r["total"].as_u64()), (Some(true), Some(20)));
    let mut missed = None;
    for (i, e) in EXAMPLES.iter().enumerate() {
        let jobs = s.take_jobs();
        let q = asks(&jobs);
        assert_eq!(q.len(), 1, "one question at a time ({i})");
        assert!(q[0].1.contains(e.subject) && q[0].1.contains(e.from), "the example's own message goes: {}", q[0].1);
        assert!(!q[0].1.contains("zoe@studio.example"), "nothing of the user's");
        // A scam let through (the third), a normal message taken for one (the twelfth), one
        // answered with nonsense (the fifth), each answer 4 s after the question.
        s.clock.now += 4;
        let text = match i {
            2 => r#"{"score": 10, "reasons": []}"#.to_string(),
            4 => "<html>".to_string(),
            11 => r#"{"score": 55, "reasons": ["Odd"]}"#.to_string(),
            _ if e.scam => SCAM.to_string(),
            _ => r#"{"score": 3, "reasons": []}"#.to_string(),
        };
        if i == 2 {
            missed = Some(e.subject);
        }
        answer(&mut s, q[0].0, &text);
    }
    assert!(asks(&s.take_jobs()).is_empty(), "twenty only");
    let t = s.scam_test().cloned().expect("the result is kept");
    assert!(!t.running);
    assert_eq!((t.scams, t.spotted, t.normals, t.left_alone, t.unanswered), (10, 8, 10, 9, 1));
    assert_eq!(t.misses.len(), 2, "{:?}", t.misses);
    assert!(t.misses.iter().any(|m| Some(m.as_str()) == missed) && t.misses.iter().any(|m| m == EXAMPLES[11].subject));
    assert_eq!(t.secs_per_message(), 4);
    // Asked again, it starts over; a real arrival is not mixed with it.
    let r = s.run("assistant.testScam", &json!({})).unwrap();
    assert_eq!((r["running"].as_bool(), r["done"].as_u64()), (Some(true), Some(0)));
    // A server that never answers: each question is given up on after a minute, the run ends.
    for _ in 0..20 {
        assert_eq!(asks(&s.take_jobs()).len(), 1);
        s.clock.now += 61;
        s.tick();
    }
    let t = s.scam_test().cloned().expect("the result");
    assert!(!t.running && t.unanswered == 20 && t.spotted == 0, "{t:?}");
    done(s);
}
