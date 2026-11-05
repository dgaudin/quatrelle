// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: search reads accents away, and the servers are asked for what was typed.

use super::*;

/// Search reads accents away, both ways round, in the words looked for and the words left out
/// (the accent part of upstream's `search_folds_accents_and_reads_dates`, 41464ec1).
#[test]
fn search_folds_accents() {
    let mut s = sample_session("search-fold");
    let all = |s: &mut Session, q: &str| s.run("search.run", &json!({"query": q, "scope": "all"})).unwrap()["results"].as_u64().unwrap() as usize;
    assert_eq!(all(&mut s, "PRÓJECTOR"), 1, "an accent typed finds the word written without");
    assert_eq!(all(&mut s, "projector"), 1);
    assert_eq!(all(&mut s, "turnaróunds -fóx"), 0, "a word left out, folded too");
    assert_eq!(all(&mut s, "\"thé projéctor\""), 1, "a phrase too");
    done(s);
}

/// A server is asked for the words as they were typed (the owner's decision, 04/11): the
/// accents stay, the server reads them its own way.
#[test]
fn a_server_is_asked_for_the_words_as_typed() {
    let mut s = sample_session("imap-search-typed");
    let a = s
        .run("account.add", &json!({"email": "zoe@example.org", "password": "pw", "imapHost": "mail.example.org", "smtpHost": "mail.example.org"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    s.run("search.run", &json!({"query": "Réunion d'équipe", "scope": "all"})).unwrap();
    let jobs = s.take_jobs();
    assert!(
        jobs.iter().any(|j| matches!(j, Job::Search { account, query, .. } if *account == a && query == "Réunion d'équipe")),
        "asked as typed: {jobs:?}"
    );
    done(s);
}
