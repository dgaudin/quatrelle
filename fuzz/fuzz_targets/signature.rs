// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A message's text read for its author's own part and its signature, then the signature checked
//! against the sender: the input is the sender's name, address and the text, split at the first
//! two line breaks.
#![no_main]

use std::collections::BTreeSet;

use libfuzzer_sys::fuzz_target;
use quatrelle_model::{EmailAddress, lookalike};

fuzz_target!(|data: &[u8]| {
    let all = String::from_utf8_lossy(data);
    let (name, rest) = all.split_once('\n').unwrap_or((&all, ""));
    let (email, text) = rest.split_once('\n').unwrap_or((rest, ""));
    let _ = lookalike::lookup_name(name);
    let _ = lookalike::own_text(text);
    let _ = lookalike::signature_shows(text);
    let mut people = lookalike::KnownPeople::new();
    people.add(&EmailAddress::new("Dana Whitfield", "dana@corp.example"));
    let known: BTreeSet<String> = ["studio.example".to_string()].into();
    let check = lookalike::SenderCheck::new(known.clone(), people, ["ana@studio.example".to_string()]);
    let _ = check.check_signature(&EmailAddress::new(name, email), text, data.len() % 2 == 0, &known);
});
