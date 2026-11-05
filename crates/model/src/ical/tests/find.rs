// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Issue #33: the calendar's search box, read and matched against events.

use super::*;
use crate::dates::days_from_civil;
use crate::search::fold_word;

fn board() -> CalEvent {
    CalEvent {
        uid: "b".into(),
        summary: "Board meeting".into(),
        location: "Room 2.14, Main building".into(),
        description: "Budget for 2027 and the new studio".into(),
        organizer: Some(Attendee { email: "ana@studio.example".into(), name: "Ana Lima".into(), ..Default::default() }),
        attendees: vec![Attendee { email: "jun.park@studio.example".into(), name: "Jun Park".into(), ..Default::default() }],
        categories: vec!["Red category".into(), "Clients".into()],
        ..Default::default()
    }
}

fn hit(q: &str) -> bool {
    EventQuery::parse(q).matches(&board())
}

#[test]
fn words_and_fields_find_an_event() {
    // Anywhere, as word starts.
    for q in ["board", "BOARD meet", "budg", "2.14", "jun", "park@studio", "clients", "studio.example"] {
        assert!(hit(q), "{q}");
    }
    for q in ["oard", "dinner", "board dinner"] {
        assert!(!hit(q), "{q}");
    }
    // Each field where it looks.
    for (q, yes) in [
        ("location:2.14", true),
        ("where:main", true),
        ("location:board", false),
        ("subject:board", true),
        ("title:budget", false),
        ("notes:budget", true),
        ("organizer:ana", true),
        ("from:lima", true),
        ("from:jun", false),
        ("attendee:jun", true),
        ("to:ana", false),
        ("with:park", true),
        ("category:red", true),
        ("category:blue", false),
        ("organizer: ana", true),
    ] {
        assert_eq!(hit(q), yes, "{q}");
    }
    // A list is any of them; a phrase as written; `-` turns a term round.
    assert!(hit("organizer:zoe,ana") && !hit("organizer:zoe,ben"));
    assert!(hit("\"board meeting\"") && !hit("\"meeting board\""));
    assert!(hit("-dinner") && !hit("-board"));
    assert!(hit("location:\"room 2.14\""));
}

#[test]
fn dates_and_kinds_narrow_it() {
    let q = EventQuery::parse("board after:2026-10-01 before:2026/11/01");
    assert_eq!(q.after, Some(days_from_civil(2026, 10, 1)));
    assert_eq!(q.before, Some(days_from_civil(2026, 11, 1)));
    assert!(q.matches(&board()), "the dates pick occurrences, not events");
    // Turned round: not after is before.
    let q = EventQuery::parse("-after:20261001");
    assert_eq!((q.after, q.before), (None, Some(days_from_civil(2026, 10, 1))));
    // Two of the same: the narrower.
    let q = EventQuery::parse("after:2026-01-01 after:2026-06-01 before:2027-01-01 before:2026-12-01");
    assert_eq!((q.after, q.before), (Some(days_from_civil(2026, 6, 1)), Some(days_from_civil(2026, 12, 1))));
    // A date that isn't one is looked for as words.
    assert!(!EventQuery::parse("after:soon").matches(&board()));
    assert!(EventQuery::parse("after:2026-13-45").after.is_none());
    let mut private = board();
    assert!(!EventQuery::parse("is:private").matches(&private));
    private.private = true;
    assert!(EventQuery::parse("is:private").matches(&private));
    private.private_kind = PrivateKind::Confidential;
    assert!(EventQuery::parse("is:private").matches(&private) && EventQuery::parse("is:confidential").matches(&private));
    assert!(EventQuery::parse("-is:private").matches(&board()));
    let series = CalEvent { rrule: Some("FREQ=WEEKLY".into()), ..board() };
    assert!(EventQuery::parse("is:recurring board").matches(&series) && !EventQuery::parse("is:recurring").matches(&board()));
}

#[test]
fn odd_queries_dont_break_it() {
    assert!(EventQuery::parse("").is_empty() && EventQuery::parse("   ").is_empty());
    assert!(!EventQuery::parse("after:2026-10-01").is_empty());
    for q in ["\"", "\"\"", ":", "-", "--", "location:", "location:,,,", "is:", "-is:", "after:", "@", "\u{0}", "📅", "ü:ö", "a:b:c"] {
        let parsed = EventQuery::parse(q);
        let _ = parsed.matches(&board());
        let _ = parsed.matches(&CalEvent::default());
    }
    assert!(EventQuery::parse("@").matches(&board()), "punctuation alone is looked for as written");
    // Huge: a few dozen terms at most, long notes looked through only so far.
    let long = "word ".repeat(10_000);
    assert!(EventQuery::parse(&long).terms.len() <= 64);
    let mut big = board();
    big.description = format!("{}needle", "x ".repeat(100_000));
    assert!(!EventQuery::parse("needle").matches(&big), "past what's looked through");
    big.description = "needle ".to_string() + &"x ".repeat(100_000);
    assert!(EventQuery::parse("needle").matches(&big));
}

#[test]
fn the_join_link_is_found_in_the_place() {
    let teams = CalEvent { summary: "Sync".into(), join_url: "https://teams.microsoft.com/l/meetup-join/19%3a".into(), ..Default::default() };
    let m = |q: &str| EventQuery::parse(q).matches(&teams);
    assert!(m("teams.microsoft.com") && m("location:teams") && m("where:meetup"));
    assert!(!m("notes:teams"), "not in its notes");
}

/// The conference's label and link are looked for with the place.
#[test]
fn the_search_finds_the_conference() {
    let jitsi = CalEvent {
        summary: "Point".into(),
        location: "Salle 3".into(),
        conference: Conference::new("https://jitsi.example.org/team", "Jitsi équipe"),
        ..Default::default()
    };
    let bare = CalEvent { conference: None, ..jitsi.clone() };
    for q in ["location:équipe", "jitsi.example.org"] {
        assert!(EventQuery::parse(q).matches(&jitsi), "{q}");
        assert!(!EventQuery::parse(q).matches(&bare), "{q}");
    }
}

/// A bare word looks through the fields one by one: a needle's words may each be in a different
/// field, the notes are read last, and what the field's words are doesn't change.
#[test]
fn a_bare_word_looks_through_each_field() {
    assert!(hit("board-budget"), "the subject's and the notes' words");
    assert!(hit("2027 lima") && hit("studio clients"));
    assert!(!hit("board-dinner"));
    assert!(hit("notes:2027") && !hit("subject:2027"));
    // A phrase written across fields (the separator in it) still finds them joined.
    assert!(hit("\"meeting \u{1f} room\""));
    // A word ending in a sigma is lowercased on its own (`ς`), the text as a whole may not be.
    let road = CalEvent { summary: "ΟΔΟΣ^Α".into(), ..Default::default() };
    for q in ["οδος", "ΟΔΟΣ", "οδο", "α"] {
        assert!(EventQuery::parse(q).matches(&road), "{q}");
    }
    // An address or a room's number inside a word, an accented word (typed with its accents or
    // without), a word in a long note.
    let mut notes = board();
    notes.description = format!("{}Réunion du comité, salle B.12, écrire à info@post.example", "x ".repeat(5_000));
    for q in ["réunion", "RÉUN", "comité", "b.12", "post.example", "info@post", "notes:salle", "ecrire", "comite"] {
        assert!(EventQuery::parse(q).matches(&notes), "{q}");
    }
    for q in ["eunion", "notes:board"] {
        assert!(!EventQuery::parse(q).matches(&notes), "{q}");
    }
    // Over several lines: each word on its line, a word cut by a line break isn't one.
    notes.description = "Bonjour,\r\nCode secret : x123q\n\nRejoindre\u{2028}maintenant\nΟΔΟΣ\nfin-\nale ΣΑΣ".into();
    for q in ["secret", "x123", "rejoindre", "maint", "οδος", "ale", "σας", "ΣΑΣ", "fin", "bonjour code"] {
        assert!(EventQuery::parse(q).matches(&notes), "{q}");
    }
    for q in ["finale", "ecret", "jour", "notes:x123q,secretx"] {
        assert_eq!(EventQuery::parse(q).matches(&notes), q.starts_with("notes:"), "{q}");
    }
    // A field starting with a mark that joins the space before it (a combining one that is part
    // of a letter): its words are those it had among the fields joined.
    let mark = CalEvent { location: "\u{345}".into(), description: "A".into(), ..Default::default() };
    for q in ["\u{345}a", "\u{345}", "a", "location:\u{345}"] {
        let parsed = EventQuery::parse(q);
        assert_eq!(parsed.matches(&mark), matched_before(&parsed, &mark), "{q:?}");
    }
    // The mark alone folds to nothing (accents are read away): what's left is `a`.
    assert!(EventQuery::parse("\u{345}a").matches(&mark));
}

/// The search as it read events before it looked through them field by field: every field joined,
/// folded and cut into words whole (with their parts), as the mail search reads a text.
fn matched_before(q: &EventQuery, e: &CalEvent) -> bool {
    fn people(list: &[&Attendee]) -> String {
        list.iter().map(|a| format!("{} {} ", a.name, a.email)).collect()
    }
    fn text(f: EventField, e: &CalEvent) -> String {
        match f {
            EventField::Subject => e.summary.clone(),
            EventField::Location if e.join_url.is_empty() || e.location.contains(&e.join_url) => e.location.clone(),
            EventField::Location => format!("{} \u{1f} {}", e.location, e.join_url),
            EventField::Notes => e.description.chars().take(64 * 1024).collect(),
            EventField::Organizer => people(&e.organizer.iter().collect::<Vec<_>>()),
            EventField::Attendees => people(&e.attendees.iter().collect::<Vec<_>>()),
            EventField::Categories => e.categories.join(" \u{1f} "),
            EventField::Any => {
                [EventField::Subject, EventField::Location, EventField::Organizer, EventField::Attendees, EventField::Categories, EventField::Notes]
                    .into_iter()
                    .map(|f| text(f, e))
                    .collect::<Vec<_>>()
                    .join(" \u{1f} ")
            }
        }
    }
    q.terms.iter().all(|(t, negated)| {
        let hit = match t {
            EventTerm::Private => e.private,
            EventTerm::Recurring => e.rrule.is_some() || e.recurrence_id.is_some(),
            EventTerm::Text(f, list) => {
                let text = text(*f, e);
                let folded = fold_word(&text);
                let have: Vec<String> = crate::search::words_and_parts(&text).map(std::borrow::Cow::into_owned).collect();
                list.iter().any(|n| match n {
                    Needle::Phrase(p) => folded.contains(fold_word(p).as_ref()),
                    Needle::Words(ws) => ws
                        .iter()
                        .map(|w| fold_word(w))
                        .all(|w| have.iter().any(|h| h.starts_with(w.as_ref())) || (w.contains(['@', '.']) && folded.contains(w.as_ref()))),
                })
            }
        };
        hit != *negated
    })
}

mod same_as_before {
    use proptest::prelude::*;

    use super::*;

    /// Letters whose small form is odd (a sigma, a dotted capital I, the Kelvin sign), marks
    /// written apart (accents, one that is a letter's part, a joiner, a soft hyphen), line breaks, an address's and a room's punctuation, the fields' separator.
    const CHARS: &[char] = &[
        'a', 'b', 'A', 'B', 'Σ', 'σ', 'ς', 'İ', 'i', 'K', '\u{212a}', 'k', 'é', 'e', '\u{301}', '\u{345}', '\u{200d}', '\u{ad}', ' ', '\n', '\r',
        '@', '.', '-', ':', '^', '\u{1f}', '\'', '\u{2019}', '_', 'ß', 'Ł',
    ];
    const PIECES: &[&str] = &[
        "a",
        "ab",
        "b",
        "Σ",
        "σας",
        "ΑΣ",
        "i",
        "İ",
        "k",
        "é",
        "e\u{301}",
        "\u{345}a",
        "@",
        "a@b",
        "b.a",
        "ab'ba",
        "ab\u{2019}ba",
        "ss",
        "l",
        ".",
        "\"a b\"",
        "\"b \u{1f} a\"",
        "\"\u{1f}\"",
        "subject:",
        "notes:",
        "location:",
        "organizer:",
        "attendee:",
        "category:",
        "is:private",
        "-",
        " ",
        " ",
        ",",
        "\"",
    ];

    fn text(max: usize) -> impl Strategy<Value = String> {
        prop::collection::vec(prop::sample::select(CHARS), 0..max).prop_map(|v| v.into_iter().collect())
    }

    fn person() -> impl Strategy<Value = Attendee> {
        (text(6), text(6)).prop_map(|(name, email)| Attendee { name, email, ..Default::default() })
    }

    fn event() -> impl Strategy<Value = CalEvent> {
        (
            text(10),
            text(10),
            text(8),
            text(40),
            prop::option::of(person()),
            prop::collection::vec(person(), 0..3),
            prop::collection::vec(text(5), 0..3),
            any::<bool>(),
        )
            .prop_map(|(summary, location, join_url, description, organizer, attendees, categories, private)| CalEvent {
                summary,
                location,
                join_url,
                description,
                organizer,
                attendees,
                categories,
                private,
                ..Default::default()
            })
    }

    fn query() -> impl Strategy<Value = String> {
        prop::collection::vec(prop::sample::select(PIECES), 1..6).prop_map(|v| v.concat())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(3000))]
        /// What a query finds is what it found before.
        #[test]
        fn a_query_finds_what_it_found_before(q in query(), e in event()) {
            let parsed = EventQuery::parse(&q);
            prop_assert_eq!(parsed.matches(&e), matched_before(&parsed, &e), "{:?} in {:?}", q, e);
        }
    }
}

/// The owner's cases (04/11), in the calendar too: accents are read away both ways round, and the
/// parts of words joined by an apostrophe, straight or typographic, are words.
#[test]
fn accents_and_apostrophes_in_the_calendar() {
    let e = CalEvent {
        summary: "Réunion d'équipe".into(),
        location: "Salle Hélène".into(),
        description: "Résumé de l\u{2019}ordre du jour".into(),
        ..Default::default()
    };
    for q in [
        "equipe",
        "ÉQUIPE",
        "reunion",
        "subject:equipe",
        "helene",
        "where:helene",
        "resume",
        "ordre",
        "l'ordre",
        "\"reunion d'equipe\"",
        "\"l'ordre du\"",
    ] {
        assert!(EventQuery::parse(q).matches(&e), "{q}");
    }
    assert!(!EventQuery::parse("-equipe").matches(&e));
    let plain = CalEvent { summary: "Reunion equipe".into(), ..Default::default() };
    for q in ["équipe", "réunion", "\"réunion équipe\""] {
        assert!(EventQuery::parse(q).matches(&plain), "{q}");
    }
}
