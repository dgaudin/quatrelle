// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The link that starts a Teams call: one address is one participant, nothing else gets in.

use proptest::prelude::*;

use super::*;

#[test]
fn a_call_link_names_the_people_and_nothing_else() {
    assert_eq!(
        teams_call_link(&["ana@contoso.example", "bruno+x@contoso.example"], false).as_deref(),
        Some("https://teams.microsoft.com/l/call/0/0?users=ana%40contoso.example,bruno%2Bx%40contoso.example")
    );
    assert_eq!(
        teams_call_link(&["ana@contoso.example"], true).as_deref(),
        Some("https://teams.microsoft.com/l/call/0/0?users=ana%40contoso.example&withVideo=true")
    );
    assert_eq!(
        teams_call_link(&[" Ana@contoso.example ", "ana@CONTOSO.example"], false).as_deref(),
        Some("https://teams.microsoft.com/l/call/0/0?users=Ana%40contoso.example")
    );
}

#[test]
fn an_address_cant_add_someone_or_an_option_to_the_call() {
    let long = format!("{}@contoso.example", "a".repeat(320));
    for bad in [
        "ana@contoso.example,eve@evil.example",
        "4:5550100",
        "ana@contoso.example&withVideo=true",
        "ana @contoso.example",
        "ana@contoso.example#x",
        "a\u{202E}na@contoso.example",
        "ana%40contoso.example",
        "",
        "ana",
        "ana@",
        "@contoso.example",
        "a@b@c",
        long.as_str(),
    ] {
        assert_eq!(teams_call_link(&[bad], false), None, "{bad:?}");
        assert_eq!(teams_call_link(&["ana@contoso.example", bad], false), None, "one bad address refuses the call: {bad:?}");
    }
    let many: Vec<String> = (0..21).map(|i| format!("p{i}@contoso.example")).collect();
    assert_eq!(teams_call_link(&many.iter().map(String::as_str).collect::<Vec<_>>(), false), None);
    assert_eq!(teams_call_link(&[], false), None);
}

#[test]
fn only_a_call_link_is_one() {
    let call = "https://teams.microsoft.com/l/call/0/0?users=ana%40contoso.example";
    for yes in [
        call,
        "https://teams.microsoft.com/l/call/0/0?users=ana%40contoso.example,b%40contoso.example&withVideo=true",
        "HTTPS://TEAMS.MICROSOFT.COM/L/CALL/0/0?users=a%40b.example",
    ] {
        assert!(is_teams_call_link(yes), "{yes}");
    }
    for no in [
        "http://teams.microsoft.com/l/call/0/0?users=a%40b.example",
        "https://teams.microsoft.com:443/l/call/0/0?users=a%40b.example",
        "https://x.teams.microsoft.com/l/call/0/0?users=a%40b.example",
        "https://teams.microsoft.com.evil.example/l/call/0/0?users=a%40b.example",
        "https://evil.example/teams.microsoft.com/l/call/0/0?users=a%40b.example",
        "https://teams.live.com/l/call/0/0?users=a%40b.example",
        "https://teams.microsoft.com/l/call/0/0",
        "https://teams.microsoft.com/l/call/0/0?users=",
        "https://teams.microsoft.com/l/call/0/1?users=a%40b.example",
        "https://teams.microsoft.com/l/call/0/0/x?users=a%40b.example",
        "https://teams.microsoft.com/l/call/0/0?users=a%40b.example&source=x",
        "https://teams.microsoft.com/l/call/0/0?users=a%40b.example#x",
        "https://teams.microsoft.com/l/call/0/0?users=a@b.example",
        "https://teams.microsoft.com/l/call/0/0?users=a%40b.example&withVideo=false",
        "https://teams.microsoft.com/l/meetup-join/x",
        "https://teams.microsoft.com/l/call/0/0?users=a b",
    ] {
        assert!(!is_teams_call_link(no), "{no}");
    }
    assert!(!is_meeting_link(call) && !is_teams_link(call) && !is_teams_chat_link(call), "a call link isn't a meeting nor a chat");
}

/// The bytes a `%XX`-encoded value stands for, as text.
fn decoded(v: &str) -> String {
    let b = v.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            out.push(u8::from_str_radix(&v[i + 1..i + 3], 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

proptest! {
    #[test]
    fn a_call_link_built_is_one_and_says_its_people(
        people in proptest::collection::btree_set(
            "[a-z0-9_'+-][a-z0-9.+_'-]{0,10}[a-z0-9_'+-]@[a-z0-9]{1,6}-?[a-z0-9]{1,5}\\.example", 1..=5)
    ) {
        let people: Vec<String> = people.into_iter().collect();
        let refs: Vec<&str> = people.iter().map(String::as_str).collect();
        let link = teams_call_link(&refs, false).expect("plain addresses");
        prop_assert!(is_teams_call_link(&link));
        let users = link.split_once("users=").unwrap().1;
        let back: Vec<String> = users.split(',').map(decoded).collect();
        prop_assert_eq!(back, people);
    }
}

#[test]
fn each_forbidden_character_alone_refuses_the_call() {
    for c in [':', '%', '&', '=', '/', '\\', '?', '"', '<', '>', ',', '#', ' ', '\u{7}', '\u{202E}', '\u{200F}'] {
        let bad = format!("an{c}a@contoso.example");
        assert_eq!(teams_call_link(&[bad.as_str()], false), None, "{c:?}");
    }
}

#[test]
fn the_call_link_bounds_are_exact() {
    let of_len = |n: usize| format!("{}@contoso.example", "a".repeat(n - "@contoso.example".len()));
    assert!(teams_call_link(&[of_len(320).as_str()], false).is_some());
    assert_eq!(teams_call_link(&[of_len(321).as_str()], false), None);
    let people = |n: usize| (0..n).map(|i| format!("p{i}@contoso.example")).collect::<Vec<_>>();
    let twenty = people(20);
    assert!(teams_call_link(&twenty.iter().map(String::as_str).collect::<Vec<_>>(), false).is_some());
    let mut twenty_one = people(21);
    assert_eq!(teams_call_link(&twenty_one.iter().map(String::as_str).collect::<Vec<_>>(), false), None);
    twenty_one[20] = "P0@CONTOSO.example".into();
    assert!(teams_call_link(&twenty_one.iter().map(String::as_str).collect::<Vec<_>>(), false).is_some(), "the cap counts people, not addresses");
}
