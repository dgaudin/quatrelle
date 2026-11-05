// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
//! What the user types in the search box (and saved searches read back): the query language,
//! the query checked against a message and its text, folding, the index, the Advanced search form, the server-side forms of a search, and IMAP's modified UTF-7. The
//! same text taken as a sender's name and address, checked for passing as someone else (a
//! look-alike domain, a known person's name on another address) and for coming from outside.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_model::search::{Advanced, Doc, Index, Query, fold_word, index_tokens, parts, tokens};
use quatrelle_model::{lookalike, utf7};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let q = Query::parse(&text);
    let _ = q.index_words();
    let _ = q.needs_body();
    // The query checked against a message whose fields and body are the same text, then the
    // text folded and indexed.
    let m = quatrelle_model::Summary { subject: text.to_string(), preview: text.to_string(), ..Default::default() };
    let _ = q.matches(&Doc { summary: &m, body: Some(&text), indexed: false, awaiting: data.len() % 2 == 0 });
    let _ = q.matches(&Doc { summary: &m, body: None, indexed: true, awaiting: false });
    let _ = tokens(&fold_word(&text)).count();
    let _ = index_tokens(&text).map(|w| parts(&w).count()).sum::<usize>();
    let mut ix = Index::new();
    ix.add(0, &text);
    let _ = ix.candidates(&q);
    let a = Advanced::from_query(&text);
    let _ = Advanced::from_query(&a.to_query());
    let _ = quatrelle_jmap::search_filter(&text);
    let _ = utf7::encode(&utf7::decode(&text));
    let (name, email) = text.split_once('\n').unwrap_or((&text, &text));
    let known = ["paypal.com".to_string(), "studio.example".to_string(), lookalike::domain_of(email)].into();
    let _ = lookalike::check_sender(&quatrelle_model::EmailAddress::new(name, email), &known);
    let _ = lookalike::skeleton(email);
    let mut people = lookalike::KnownPeople::new();
    people.add(&quatrelle_model::EmailAddress::new("Jane Director", "jane@studio.example"));
    people.add(&quatrelle_model::EmailAddress::new(email, name));
    let check = lookalike::SenderCheck::new(known, people, [name.to_string()]);
    let from = quatrelle_model::EmailAddress::new(name, email);
    let _ = check.check(&from, false);
    let _ = check.check(&from, true);
    let _ = lookalike::name_key(name);
    let _ = lookalike::is_free_mail(email);
    let org = lookalike::clean_domain(name).into_iter().collect();
    let _ = lookalike::is_external(email, &org);
});
