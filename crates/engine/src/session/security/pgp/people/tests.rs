// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! People's keys, as kept: a newer message's key replaces an older one's (and says so once), an
//! older message's doesn't, a key imported by hand stays, and a flood of keys from mail stops at
//! [`PEOPLE_MAX`], the least used going first.

use super::*;

const D: i64 = 1_800_000_000;

/// A stand-in key: what's kept is what's given, so a fingerprint will do.
fn key(fingerprint: &str, email: &str) -> (Vec<u8>, openpgp::KeyInfo) {
    let info = openpgp::KeyInfo { fingerprint: fingerprint.into(), emails: vec![email.into()], ..Default::default() };
    (fingerprint.as_bytes().to_vec(), info)
}

#[test]
fn a_newer_messages_key_replaces_an_older_ones_and_says_so_once() {
    let mut p = PeopleKeys::default();
    let (a, ai) = key("AAAA", "bo@studio.example");
    let (b, bi) = key("BBBB", "bo@studio.example");
    assert_eq!(p.learn_from_mail("Bo@Studio.example", &a, &ai, D, D), Learn::New);
    assert_eq!(p.take_change("bo@studio.example"), None, "a first key changes nothing");
    assert_eq!(p.learn_from_mail("bo@studio.example", &a, &ai, D, D + 5), Learn::Same);
    assert_eq!(p.learn_from_mail("bo@studio.example", &b, &bi, D + 1, D + 10), Learn::Replaced { previous: "AAAA".into() });
    let k = p.get("bo@studio.example").unwrap();
    assert_eq!((k.info.fingerprint.as_str(), k.previous.as_deref(), k.change_shown, k.from_date), ("BBBB", Some("AAAA"), false, D + 1));
    assert_eq!(p.take_change("BO@studio.example"), Some("AAAA".into()));
    assert_eq!(p.take_change("bo@studio.example"), None, "said once");
    // The same message seen again changes nothing more.
    assert_eq!(p.learn_from_mail("bo@studio.example", &b, &bi, D + 1, D + 20), Learn::Same);
    assert_eq!(p.take_change("bo@studio.example"), None);
}

#[test]
fn an_older_message_doesnt_replace_a_newer_key() {
    let mut p = PeopleKeys::default();
    let (a, ai) = key("AAAA", "bo@studio.example");
    let (b, bi) = key("BBBB", "bo@studio.example");
    assert_eq!(p.learn_from_mail("bo@studio.example", &b, &bi, D + 1, D), Learn::New);
    assert_eq!(p.learn_from_mail("bo@studio.example", &a, &ai, D, D), Learn::Kept);
    assert_eq!(p.get("bo@studio.example").unwrap().info.fingerprint, "BBBB");
    assert_eq!(p.take_change("bo@studio.example"), None);
}

#[test]
fn a_key_imported_by_hand_is_never_replaced_from_mail() {
    let mut p = PeopleKeys::default();
    let (a, ai) = key("AAAA", "bo@studio.example");
    let (b, bi) = key("BBBB", "bo@studio.example");
    assert_eq!(p.import_by_hand("bo@studio.example", &a, &ai, D), Learn::New);
    assert_eq!(p.learn_from_mail("bo@studio.example", &b, &bi, D + 100, D + 100), Learn::Kept);
    let k = p.get("bo@studio.example").unwrap();
    assert_eq!((k.info.fingerprint.as_str(), k.learnt), ("AAAA", Learnt::ByHand));
    assert_eq!(p.learn_from_mail("bo@studio.example", &a, &ai, D + 100, D + 100), Learn::Same);
    assert_eq!(p.get("bo@studio.example").unwrap().learnt, Learnt::ByHand, "still the user's choice");
    // A key from mail, imported by hand: the user's now, said by no notice (they chose it).
    assert_eq!(p.learn_from_mail("zoe@studio.example", &a, &ai, D, D), Learn::New);
    assert_eq!(p.import_by_hand("zoe@studio.example", &b, &bi, D), Learn::Replaced { previous: "AAAA".into() });
    assert_eq!(p.take_change("zoe@studio.example"), None);
    assert_eq!(p.get("zoe@studio.example").unwrap().learnt, Learnt::ByHand);
    // Forgotten, mail may bring one again.
    assert!(p.forget("bo@studio.example") && !p.forget("bo@studio.example"));
    assert_eq!(p.learn_from_mail("bo@studio.example", &b, &bi, D, D), Learn::New);
    // Nothing for no one, nor a key too big to be one.
    assert_eq!(p.learn_from_mail(" ", &b, &bi, D, D), Learn::Kept);
    assert_eq!(p.learn_from_mail("eve@studio.example", &vec![0; (1 << 20) + 1], &bi, D, D), Learn::Kept);
}

#[test]
fn the_ten_thousand_and_first_lets_go_the_least_used_learnt_from_mail() {
    assert_eq!(PEOPLE_MAX, 10_000);
    let mut p = PeopleKeys::default();
    // Ten imported by hand, used longest ago of all; the rest from mail.
    for i in 0..10 {
        let email = format!("hand{i}@studio.example");
        let (k, info) = key(&format!("H{i:04}"), &email);
        p.import_by_hand(&email, &k, &info, 0);
    }
    for i in 0..PEOPLE_MAX - 10 {
        let email = format!("p{i}@studio.example");
        let (k, info) = key(&format!("M{i:05}"), &email);
        p.learn_from_mail(&email, &k, &info, D, D + i64::try_from(i).unwrap());
    }
    // One of them used again: no longer the least used.
    p.used("p0@studio.example", D + 50_000);
    assert_eq!(p.len(), PEOPLE_MAX);
    let (k, info) = key("NEW", "new@studio.example");
    assert_eq!(p.learn_from_mail("new@studio.example", &k, &info, D, D + 60_000), Learn::New);
    assert_eq!(p.len(), PEOPLE_MAX);
    assert!(p.get("new@studio.example").is_some());
    assert!(p.get("p1@studio.example").is_none(), "the least used from mail went");
    assert!(p.get("p0@studio.example").is_some() && p.get("p2@studio.example").is_some());
    assert!((0..10).all(|i| p.get(&format!("hand{i}@studio.example")).is_some()), "every key imported by hand stays");
    // A key imported by hand is always kept, one from mail making room for it.
    let (k, info) = key("HNEW", "hnew@studio.example");
    assert_eq!(p.import_by_hand("hnew@studio.example", &k, &info, D + 70_000), Learn::New);
    assert_eq!(p.len(), PEOPLE_MAX);
}

/// What a file brings back is checked as any file: a key under another address, one too big, and
/// more than there may be are let go.
#[test]
fn a_file_read_back_is_checked() {
    let mut p = PeopleKeys::default();
    let (k, info) = key("AAAA", "bo@studio.example");
    p.learn_from_mail("bo@studio.example", &k, &info, D, D);
    let mut odd = p.get("bo@studio.example").unwrap().clone();
    odd.email = "eve@studio.example".into();
    p.keys.insert("bo2@studio.example".into(), odd);
    let mut big = p.get("bo@studio.example").unwrap().clone();
    big.email = "big@studio.example".into();
    big.public = vec![0; (1 << 20) + 1];
    p.keys.insert("big@studio.example".into(), big);
    let json = serde_json::to_string(&p).unwrap();
    assert!(json.contains("\"public\":\"41414141\""), "kept as hex: {json}");
    let back = PeopleKeys::checked(serde_json::from_str(&json).unwrap());
    assert_eq!(back.keys.keys().collect::<Vec<_>>(), ["bo@studio.example"]);
    assert_eq!(back.get("bo@studio.example").unwrap().public, k);
}
