// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Round trips that must hold for any input, not only the examples the unit tests pick:
//! what Quatrelle writes it reads back the same.

use proptest::prelude::*;
use quatrelle_model::addr::trust::{site, url_host};
use quatrelle_model::dates::Clock;
use quatrelle_model::search::{Advanced, Doc, Index, MAX_TOKEN_CHARS, Query, fold_word, tokens};
use quatrelle_model::vcard::{self, Card, PostalAddress, Typed};
use quatrelle_model::{EmailAddress, Flag, MessageKey, MessageKind, Op, Smime, Summary, addr, ical, lookalike, utf7};

/// Words the search box reads as plain words (no `field:`, no leading `-` or quote).
fn words() -> impl Strategy<Value = String> {
    prop::collection::vec("[a-zA-Z0-9à-ÿ]{1,8}", 0..4).prop_map(|w| w.join(" "))
}

fn advanced() -> impl Strategy<Value = Advanced> {
    (words(), words(), words(), words(), words(), any::<[bool; 4]>()).prop_map(|(w, from, to, subject, body, b)| Advanced {
        words: w,
        from,
        to,
        subject,
        body,
        attachments: b[0],
        unread: b[1],
        flagged: b[2],
        important: b[3],
        rest: String::new(),
    })
}

/// Text a person types into a card's field: any characters, lines included.
fn field() -> impl Strategy<Value = String> {
    "[^\r\u{0}]{0,40}".prop_map(|s| s.trim().to_string())
}

fn typed(value: &'static str) -> impl Strategy<Value = Typed> {
    (value, prop::sample::select(vec!["", "work", "home", "cell"])).prop_map(|(value, kind)| Typed { value, kind: kind.to_string() })
}

fn card() -> impl Strategy<Value = Card> {
    (
        "[a-z0-9-]{1,20}",
        "[^\r\n\u{0}]{0,30}".prop_filter("a name", |s| !s.trim().is_empty()).prop_map(|s| s.trim().to_string()),
        (field(), field(), field(), field(), field()),
        prop::collection::vec(typed(r"[a-z0-9.+-]{1,10}@[a-z]{1,8}\.[a-z]{2,3}"), 0..3),
        prop::collection::vec(typed(r"\+?[0-9 ]{3,15}"), 0..3),
        prop::collection::vec((field(), field(), field(), field(), field()), 0..2),
        prop::sample::select(vec!["", "1990-04-01", "--12-31"]),
        // The rest of `N` as a server writes it: additional names, prefixes, suffixes.
        prop::sample::select(vec!["", "Marie;Dr.;Jr.", ";Dr.;", ";;Jr.", "Ann\\, Lee;;", "Marie;;"]),
    )
        .prop_map(|(uid, name, (given, family, org, title, note), emails, phones, addresses, birthday, name_rest)| Card {
            uid,
            name,
            given: given.replace('\n', " "),
            family: family.replace('\n', " "),
            name_rest: name_rest.to_string(),
            emails: emails.into_iter().map(|t| Typed { value: t.value.trim().to_string(), kind: t.kind }).collect(),
            phones: phones.into_iter().map(|t| Typed { value: t.value.trim().to_string(), kind: t.kind }).collect(),
            org: org.replace('\n', " "),
            title: title.replace('\n', " "),
            note,
            addresses: addresses
                .into_iter()
                .map(|(street, city, region, postcode, country)| PostalAddress {
                    kind: "home".into(),
                    street,
                    city: city.replace('\n', " "),
                    region: region.replace('\n', " "),
                    postcode: postcode.replace('\n', " "),
                    country: country.replace('\n', " "),
                })
                .filter(|a| !a.is_empty())
                .collect(),
            birthday: birthday.to_string(),
            extra: Vec::new(),
            ..Default::default()
        })
}

proptest! {
    /// The Advanced search form becomes a query and comes back the same.
    #[test]
    fn advanced_search_form_round_trips(a in advanced()) {
        prop_assert_eq!(Advanced::from_query(&a.to_query()), a);
    }

    /// Whatever was typed in the search box, the form it fills writes a query that fills the
    /// same form again, and asks the same search.
    #[test]
    fn advanced_search_settles_after_one_trip(q in r#"[a-z:" -]{0,40}|\PC{0,40}"#) {
        let once = Advanced::from_query(&q);
        let twice = Advanced::from_query(&once.to_query());
        prop_assert_eq!(&Advanced::from_query(&twice.to_query()), &twice);
        let _ = Query::parse(&once.to_query());
    }

    /// Folding a word twice changes nothing more than once, whatever its letters (upstream's,
    /// 41464ec1).
    #[test]
    fn folding_is_idempotent(w in r"\PC{0,30}|[a-zA-Zà-ɏͰ-ϿḀ-ỿἀ-῿！-～Ѐ-ӿ\u{300}-\u{36f}]{0,20}") {
        let once = fold_word(&w).into_owned();
        prop_assert_eq!(fold_word(&once).into_owned(), once.clone());
        for t in tokens(&w) {
            prop_assert!(!t.is_empty() && t.chars().count() <= MAX_TOKEN_CHARS);
            prop_assert_eq!(fold_word(&t).into_owned(), t.to_string());
        }
    }

    /// Whatever is typed, reading it and checking it against any message never panics; and a
    /// message holding the words typed, accents or not, is found by them through the index.
    #[test]
    fn any_query_reads_and_checks(q in r#"[a-z:"<>=./ 0-9-]{0,40}|\PC{0,60}"#, subject in r"\PC{0,30}") {
        let query = Query::parse(&q);
        let m = quatrelle_model::Summary { subject, ..Default::default() };
        for body in [None, Some(q.as_str())] {
            let _ = query.matches(&Doc { summary: &m, body, indexed: body.is_none(), awaiting: false });
        }
        let _ = (query.index_words(), query.needs_body());
        let mut ix = Index::new();
        ix.add(0, &q);
        let _ = ix.candidates(&query);
    }

    /// Whatever is typed with date fields, placed on any clock (the ends of time too) and
    /// checked against any date, never panics; a day written out holds from its start to the
    /// next one's, and no longer.
    #[test]
    fn dates_are_placed_on_any_clock(
        q in r#"(received|sent|date|after|before|reçu|avant|après):[a-zA-Zéè"'<>=./ 0-9-]{0,24}|\PC{0,40}"#,
        now in any::<i64>(),
        offset in any::<i32>(),
        t in any::<i64>(),
        (y, m, d) in (1970i64..=9999, 1u32..=12, 1u32..=28),
    ) {
        let clock = Clock::new(now, offset);
        let mut query = Query::parse(&q);
        query.resolve(&clock);
        let m_ = quatrelle_model::Summary { date: t, received: now, ..Default::default() };
        let _ = query.dates_hold(&m_);
        let _ = query.matches(&Doc { summary: &m_, body: None, indexed: false, awaiting: false });
        let mut day = Query::parse(&format!("received:{y:04}-{m:02}-{d:02}"));
        prop_assert!(day.problems.is_empty());
        day.resolve(&clock);
        let start = clock.midnight(quatrelle_model::dates::days_from_civil(y, m, d));
        let next = clock.midnight(quatrelle_model::dates::days_from_civil(y, m, d) + 1);
        let at = |t: i64| quatrelle_model::Summary { date: t, received: t, ..Default::default() };
        prop_assert!(day.dates_hold(&at(start)) && !day.dates_hold(&at(next)) && !day.dates_hold(&at(start.saturating_sub(1))));
    }

    /// Plain words, typed with or without their accents, find the text that has them.
    #[test]
    fn words_are_found_with_or_without_accents(w in "[a-zà-ÿ]{2,12}", plain in any::<bool>()) {
        let typed = if plain { fold_word(&w).into_owned() } else { w.to_uppercase() };
        let mut ix = Index::new();
        ix.add(7, &format!("Before {w} after"));
        let query = Query::parse(&typed);
        prop_assert!(ix.candidates(&query).contains(&7), "{} in {}", typed, w);
        let m = quatrelle_model::Summary { subject: format!("Before {w} after"), ..Default::default() };
        let found = query.matches(&Doc { summary: &m, body: None, indexed: false, awaiting: false });
        prop_assert!(found, "{} in {}", typed, w);
    }

    /// A contact written as a vCard reads back as the same contact.
    #[test]
    fn vcard_round_trips(c in card()) {
        let text = vcard::write(&c);
        let back = vcard::parse(&text);
        prop_assert_eq!(back.len(), 1, "{}", text);
        // The card, its lines kept as written beside it ([`Card::as_written`]); written again,
        // the same text.
        prop_assert_eq!(&Card { as_written: Default::default(), ..back[0].clone() }, &c, "{}", text);
        prop_assert_eq!(vcard::write(&back[0]), text);
    }

    /// Any folder name survives IMAP's modified UTF-7, and the wire form is ASCII.
    #[test]
    fn utf7_round_trips(name in r"\PC{0,40}") {
        let wire = utf7::encode(&name);
        prop_assert!(wire.is_ascii());
        prop_assert_eq!(utf7::decode(&wire), name);
    }

    /// iCalendar text values: escaped and unescaped, the same text.
    #[test]
    fn ical_text_round_trips(text in "[^\r]{0,60}") {
        prop_assert_eq!(ical::unescape(&ical::escape(&text)), text);
    }

    /// What the fuzz target `trust` holds as well: a host read from a URL is clean, its site is
    /// stable, and the site reads back as itself (a question about it can be answered).
    #[test]
    fn a_hosts_site_is_stable_and_reads_back(url in r"(https?://)?[a-zA-Z0-9.:_@%/\\\[\]ü-]{0,40}") {
        if let Some(host) = url_host(&url) {
            prop_assert!(!host.contains(['\\', '%', '@']), "{}", host);
            let s = site(&host);
            prop_assert_eq!(site(&s), s.clone());
            if !s.is_empty() {
                prop_assert_eq!(url_host(&s).map(|h| site(&h)), Some(s));
            }
        }
    }

    /// A person's name compares the same whichever order its words come in, "Last, First"
    /// included (issue #29), and any name and address is checked without fault.
    #[test]
    fn known_names_compare_in_any_order(first in "[a-zA-Zà-ÿ]{2,12}", last in "[a-zA-Zà-ÿ]{2,12}", junk in r"\PC{0,80}") {
        let key = lookalike::name_key(&format!("{first} {last}"));
        prop_assert_eq!(&key, &lookalike::name_key(&format!("{last}, {first}")));
        prop_assert_eq!(&key, &lookalike::name_key(&format!("  {} {}  ", last.to_uppercase(), first)));
        let mut people = lookalike::KnownPeople::new();
        people.add(&EmailAddress::new(format!("{first} {last}"), "them@work.example"));
        let from = EmailAddress::new(format!("{last}, {first}"), "them.too@gmail.com");
        prop_assert_eq!(people.check(&from).is_some(), key.is_some());
        let _ = people.check(&EmailAddress::new(junk.clone(), junk.clone()));
        let _ = lookalike::name_key(&junk);
    }

    /// Nothing after the first quotation mark changes the signature found: a quoted or forwarded
    /// message's own signature is never read as the sender's.
    #[test]
    fn signature_ignores_what_follows_a_quote(a in ".{0,400}", b in ".{0,400}") {
        let with = lookalike::signature_shows(&format!("{a}\n-----Original Message-----\n{b}"));
        prop_assert_eq!(with, lookalike::signature_shows(&format!("{a}\n-----Original Message-----\n")));
    }

    /// The name asked of the directory is never longer than 100 characters, whatever the
    /// sender's name.
    #[test]
    fn lookup_name_is_short(s in ".{0,600}") {
        prop_assert!(lookalike::lookup_name(&s).map_or(0, |n| n.chars().count()) <= 100);
    }
}

proptest! {
    /// A model's answer is hostile text: reading it never panics, and what it gives is within bounds.
    #[test]
    fn reading_any_answer_never_panics(text in "(?s).{0,65536}") {
        if let Some((score, reasons)) = quatrelle_model::scam::read_answer(&text) {
            prop_assert!(score <= 100 && reasons.len() <= 3 && reasons.iter().all(|r| r.chars().count() <= 120));
        }
    }
}

/// An entry of a recipient field as people write it: bare, named, named with a comma, with a
/// quote left open or closed wrong, with another entry's separator or none after it.
fn typed_entry() -> impl Strategy<Value = String> {
    let name = prop::sample::select(vec!["", "Jo ", "\"Park, Zoe\" ", "Jo 6\" Doe ", "\"Jo Doe ", "Jo\" ", "\"Lee\" ", "Bob, ", "\""]);
    let local = "[a-z]{1,4}";
    let sep = prop::sample::select(vec!["; ", ", ", " ", "\n", ";", "", " ; "]);
    (name, local, any::<bool>(), sep).prop_map(|(name, local, chevrons, sep)| {
        let a = format!("{local}@x.example");
        if chevrons { format!("{name}<{a}>{sep}") } else { format!("{name}{a}{sep}") }
    })
}

/// Every `<x@y>` of the text, with whether it is in a name: a pair of quotes that opens where an
/// entry starts (the text's start, or after `;`, `,` or `>`) and closes. A quote anywhere else is
/// an inch mark, and an address after it is no one's name.
fn chevron_addresses(text: &str) -> Vec<(String, bool)> {
    let mut found = Vec::new();
    // The span now open: where its quote began an entry, and the addresses seen in it.
    let mut span: Option<(bool, Vec<usize>)> = None;
    let mut last = None;
    let mut from: Option<usize> = None;
    for (i, c) in text.char_indices() {
        match c {
            '"' => match span.take() {
                Some((starts_entry, held)) => {
                    for at in held {
                        if let Some((_, excused)) = found.get_mut(at) {
                            *excused = starts_entry;
                        }
                    }
                }
                None => span = Some((last.is_none_or(|l: char| matches!(l, ';' | ',' | '>')), Vec::new())),
            },
            '<' => from = Some(i),
            '>' => {
                if let Some(at) = from.take() {
                    let inside = text.get(at + 1..i).unwrap_or_default();
                    if inside.contains('@') {
                        if let Some((_, held)) = span.as_mut() {
                            held.push(found.len());
                        }
                        found.push((inside.to_string(), false));
                    }
                }
            }
            _ => {}
        }
        if !c.is_whitespace() {
            last = Some(c);
        }
    }
    found
}

proptest! {
    /// A list that reads gives no address that isn't in the text and loses none that is written
    /// between `<` and `>` outside a name in closed quotes (whatever the quotes around are, and
    /// whatever ends the entries): either it says what it couldn't read, or it has everyone.
    #[test]
    fn parse_list_never_invents_nor_silently_drops(entries in prop::collection::vec(typed_entry(), 1..6)) {
        let text = entries.concat();
        if let Ok(read) = addr::parse_list(&text) {
            for a in &read {
                prop_assert!(text.contains(a.email.as_str()), "{} is not in {:?}", a.email, text);
            }
            // One in a name (a closed pair of quotes from an entry's start) may be left out of the
            // reading; any other is in it, or the list wouldn't have read.
            for (email, in_a_name) in chevron_addresses(&text) {
                if in_a_name {
                    continue;
                }
                prop_assert!(read.iter().any(|a| a.email == email), "{} was lost from {:?}: {:?}", email, text, read);
            }
        }
        // What a draft keeps is read from the text too.
        for a in addr::parse_valid(&text) {
            prop_assert!(text.contains(a.email.as_str()));
        }
    }
}

/// Every kind of message.
const KINDS: [MessageKind; 16] = [
    MessageKind::Unknown,
    MessageKind::Plain,
    MessageKind::AutoReply,
    MessageKind::NotDelivered,
    MessageKind::Delayed,
    MessageKind::Delivered,
    MessageKind::ReadReceipt,
    MessageKind::NotRead,
    MessageKind::Recall,
    MessageKind::RecallDone,
    MessageKind::RecallFailed,
    MessageKind::MeetingRequest,
    MessageKind::MeetingCanceled,
    MessageKind::MeetingAccepted,
    MessageKind::MeetingTentative,
    MessageKind::MeetingDeclined,
];

/// Any JSON value: what another version or a damaged disk may have written for a message's kind,
/// the names of the kinds among the strings.
fn any_json() -> impl Strategy<Value = serde_json::Value> {
    use serde_json::Value;
    let names = prop::sample::select(KINDS.to_vec()).prop_map(|k| serde_json::to_value(k).unwrap_or_default());
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        (-1e300f64..1e300).prop_map(Value::from),
        r"\PC{0,20}".prop_map(Value::String),
        names,
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            prop::collection::vec(("[a-z]{0,6}", inner), 0..4).prop_map(|kv| Value::Object(kv.into_iter().collect())),
        ]
    })
}

proptest! {
    /// A message keeps its kind through the cache, whichever it is, and one not said yet isn't
    /// written (a cache from before, and 1.5.0's, stay as they were).
    #[test]
    fn summaries_round_trip_with_their_kind(kind in prop::sample::select(KINDS.to_vec()), subject in r"\PC{0,30}", uid in any::<u32>()) {
        let m = Summary {
            key: MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid },
            subject,
            kind,
            smime: Smime::Signed,
            ..Default::default()
        };
        let json = serde_json::to_string(&m).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(json.contains(r#""kind":"#), kind != MessageKind::Unknown, "{}", json);
        let back: Summary = serde_json::from_str(&json).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(back, m);
    }

    /// Whatever stands for a message's kind in a page of the cache, the message reads, its kind
    /// the one named or not said yet.
    #[test]
    fn any_json_kind_reads(kind in any_json()) {
        let named = serde_json::from_value::<MessageKind>(kind.clone()).unwrap_or_default();
        let page = serde_json::json!({"key": {"account": "a", "folder": "INBOX", "validity": 1, "uid": 7}, "subject": "s", "kind": kind});
        let text = page.to_string();
        let read = serde_json::from_str::<Summary>(&text);
        prop_assert!(read.is_ok(), "{}", text);
        if let Ok(m) = read {
            prop_assert_eq!((m.kind, m.key.uid, m.subject.as_str()), (named, 7, "s"));
        }
    }
}

/// A flag change as 1.5.0 knows it: what it writes, and all it reads of a change kept for the
/// server.
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
enum Before {
    SetFlag { keys: Vec<MessageKey>, flag: Flag, on: bool },
}

proptest! {
    /// A flag change comes back from the changes kept for the server as it was made; one that
    /// isn't to all is written byte for byte as 1.5.0 writes it, and 1.5.0 reads any of them.
    #[test]
    fn flag_changes_round_trip(
        flag in prop::sample::select(vec![Flag::Seen, Flag::Flagged, Flag::Answered, Flag::Forwarded, Flag::Deleted]),
        on in any::<bool>(),
        all in any::<bool>(),
        uid in any::<u32>(),
    ) {
        let keys = vec![MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid }];
        let op = Op::SetFlag { keys: keys.clone(), flag, on, all };
        let json = serde_json::to_string(&op).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let back: Op = serde_json::from_str(&json).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(back, op);
        let before = Before::SetFlag { keys, flag, on };
        if !all {
            prop_assert_eq!(&json, &serde_json::to_string(&before).map_err(|e| TestCaseError::fail(e.to_string()))?);
        }
        let read: Before = serde_json::from_str(&json).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(read, before);
    }
}
