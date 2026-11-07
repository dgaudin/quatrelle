// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Dates in a search (adapted from upstream 41464ec1 and 427337f4): periods in the user's zone,
//! the names from today in English and French, and what isn't a date.

use super::*;
use crate::dates::Clock;
use crate::zone::Zone;

fn berlin(now: i64) -> Clock {
    Clock::new(now, 3600).in_zone(Zone::named("Europe/Berlin"))
}

fn range(q: &str, clock: &Clock) -> Option<(i64, i64)> {
    let mut query = Query::parse(q);
    query.resolve(clock);
    match query.clauses.first().map(|c| &c.term) {
        Some(Term::Date(spec)) => spec.range,
        _ => None,
    }
}

/// UTC seconds of a local Berlin wall-clock midnight.
fn local(y: i64, m: u32, d: u32, clock: &Clock) -> i64 {
    clock.midnight(crate::dates::days_from_civil(y, m, d))
}

/// `received:2024` is the year in the user's zone: from its first local midnight to the next
/// year's.
#[test]
fn received_is_a_period_in_the_local_zone() {
    let c = berlin(1_718_000_000); // June 2024
    assert_eq!(range("received:2024", &c), Some((1_704_063_600, 1_735_686_000)), "2024-01-01 00:00 and 2025-01-01 00:00, CET");
    assert_eq!(range("received:2024-03", &c), Some((local(2024, 3, 1, &c), local(2024, 4, 1, &c))));
    assert_eq!(range("sent:2024/03/15", &c), Some((local(2024, 3, 15, &c), local(2024, 3, 16, &c))));
    assert_eq!(range("date:20240315", &c), range("received:2024-03-15", &c));
    // Summer time starts on 31 March 2024: that day is 23 hours long.
    let (start, end) = range("received:2024-03-31", &c).unwrap_or_default();
    assert_eq!(end - start, 23 * 3600);
    let (start, end) = range("received:2024-10-27", &c).unwrap_or_default();
    assert_eq!(end - start, 25 * 3600, "and it ends on 27 October");
    // Ranges, comparisons, after and before.
    assert_eq!(range("received:2024-01-01..2024-03-31", &c), Some((local(2024, 1, 1, &c), local(2024, 4, 1, &c))));
    assert_eq!(range("received:2024-03..", &c), Some((local(2024, 3, 1, &c), i64::MAX)));
    assert_eq!(range("received:..2024", &c), Some((i64::MIN, local(2025, 1, 1, &c))));
    assert_eq!(range("received:>=2024-03-01", &c), Some((local(2024, 3, 1, &c), i64::MAX)));
    assert_eq!(range("received:>2024-03", &c), Some((local(2024, 4, 1, &c), i64::MAX)));
    assert_eq!(range("received:<2024-03-01", &c), Some((i64::MIN, local(2024, 3, 1, &c))));
    assert_eq!(range("received:<=2024-03", &c), Some((i64::MIN, local(2024, 4, 1, &c))));
    assert_eq!(range("after:2024-03-01", &c), Some((local(2024, 3, 1, &c), i64::MAX)));
    assert_eq!(range("before:2024-03-01", &c), Some((i64::MIN, local(2024, 3, 1, &c))));
    // Matching: inside, at the ends, turned round.
    let mut m = sample();
    let check = |s: &str, m: &Summary| {
        let mut query = Query::parse(s);
        query.resolve(&c);
        query.matches(&Doc { summary: m, body: None, indexed: false, awaiting: false })
    };
    m.date = local(2024, 1, 1, &c);
    assert!(check("received:2024", &m) && !check("received:2023", &m) && !check("-received:2024", &m));
    m.date = local(2025, 1, 1, &c) - 1;
    assert!(check("received:2024", &m) && check("before:2025", &m) && !check("after:2025", &m));
    // Not placed on a clock yet: nothing holds.
    assert!(!q("received:2024", &m));
    // A date needs no word of the text, and the index doesn't narrow by it.
    assert!(!Query::parse("received:2024").needs_body() && Query::parse("received:2024").index_words().is_empty());
}

/// `last week` is Monday to Monday, whether today is a Monday or a Sunday.
#[test]
fn weeks_months_and_days_from_today() {
    let monday = berlin(1_718_013_600); // Monday 2024-06-10 12:00 CEST
    let sunday = berlin(1_718_532_000); // Sunday 2024-06-16 12:00 CEST
    for c in [monday, sunday] {
        assert_eq!(range("received:\"last week\"", &c), Some((local(2024, 6, 3, &c), local(2024, 6, 10, &c))));
        assert_eq!(range("received:lastweek", &c), range("received:last-week", &c));
        assert_eq!(range("received:thisweek", &c), Some((local(2024, 6, 10, &c), local(2024, 6, 17, &c))));
        assert_eq!(range("received:lastmonth", &c), Some((local(2024, 5, 1, &c), local(2024, 6, 1, &c))));
        assert_eq!(range("received:thisyear", &c), range("received:2024", &c));
        assert_eq!(range("received:lastyear", &c), range("received:2023", &c));
    }
    assert_eq!(range("received:today", &monday), Some((local(2024, 6, 10, &monday), local(2024, 6, 11, &monday))));
    assert_eq!(range("received: yesterday", &sunday), Some((local(2024, 6, 15, &sunday), local(2024, 6, 16, &sunday))));
    let january = berlin(1_704_200_000); // 2024-01-02
    assert_eq!(range("received:lastmonth", &january), range("received:2023-12", &january));
}

/// The names and the fields in French read as the English ones, accents and apostrophes or not.
#[test]
fn dates_in_french() {
    let c = berlin(1_718_013_600);
    for (french, english) in [
        ("reçu:hier", "received:yesterday"),
        ("recu:hier", "received:yesterday"),
        ("envoyé:aujourd'hui", "sent:today"),
        ("envoye:aujourd’hui", "sent:today"),
        ("reçu:\"la semaine dernière\"", "received:lastweek"),
        ("reçu:semaine-dernière", "received:lastweek"),
        ("reçu:\"cette semaine\"", "received:thisweek"),
        ("date:\"le mois dernier\"", "date:lastmonth"),
        ("date:ce-mois", "date:thismonth"),
        ("date:\"l'année dernière\"", "date:lastyear"),
        ("date:\"cette année\"", "date:thisyear"),
        ("avant:hier", "before:yesterday"),
        ("après:2024-03-01", "after:2024-03-01"),
        ("apres:2024-03-01", "after:2024-03-01"),
        ("avant:2024-03-01", "before:2024-03-01"),
    ] {
        assert!(range(english, &c).is_some(), "{english}");
        assert_eq!(range(french, &c), range(english, &c), "{french}");
    }
    // A bare field waits for its value in French too.
    assert_eq!(Query::parse("reçu: hier").clauses, Query::parse("reçu:hier").clauses);
    assert!(Query::parse("reçu: hier").problems.is_empty());
}

/// A date that isn't one is left out and said, never searched as words.
#[test]
fn junk_dates_are_problems() {
    for junk in [
        "received:2024-13",
        "received:2024-02-30",
        "received:99999999999",
        "received:..",
        "received:>",
        "received:1969",
        "received:yesterdays",
        "after:2024-00-10",
        "before:>=2024",
        "received:2024-01-01..2024-99",
        "received:24",
        "reçu:hiers",
    ] {
        let query = Query::parse(junk);
        assert!(query.clauses.is_empty(), "{junk}: {:?}", query.clauses);
        assert_eq!(query.problems, [Problem::Date(junk.to_string())], "{junk}");
    }
    let query = Query::parse("budget -received:2024-13 from:ana");
    assert_eq!(
        (query.clauses.len(), query.problems, Query::parse("received:2024").problems),
        (2, vec![Problem::Date("received:2024-13".into())], vec![])
    );
    // A value not meant as a date is still the words typed (the problem said all the same):
    // these weren't fields before, and prose after them was searched whole.
    for (typed, words) in [
        ("budget before: launch", vec!["budget", "before", "launch"]),
        ("meeting date: friday", vec!["meeting", "date", "friday"]),
        ("sent: from the phone", vec!["sent", "from", "the", "phone"]),
        ("after:party", vec!["after", "party"]),
        ("received:soon", vec!["received", "soon"]),
    ] {
        let query = Query::parse(typed);
        let found: Vec<String> = query
            .clauses
            .iter()
            .flat_map(|c| match &c.term {
                Term::Words(Field::Any, ws) => ws.clone(),
                _ => Vec::new(),
            })
            .collect();
        assert_eq!((found, query.problems.len()), (words.iter().map(|w| w.to_string()).collect(), 1), "{typed}");
    }
    let mut m = sample();
    m.subject = "After party photos".into();
    assert!(q("after:party", &m));
    // Leap days are days.
    assert!(Query::parse("received:2024-02-29").problems.is_empty() && !Query::parse("received:2023-02-29").problems.is_empty());
    assert!(
        super::dates::is_day("2024-03-15") && super::dates::is_day("2024/3/5") && !super::dates::is_day("2024-03") && !super::dates::is_day("today")
    );
    // Never a crash, whatever the clock.
    for now in [i64::MIN, -1, 0, i64::MAX] {
        for typed in ["received:today", "received:lastweek", "received:9999", "before:1970", "received:..9999-12-31", "after:yesterday"] {
            let mut query = Query::parse(typed);
            query.resolve(&Clock::new(now, i32::MAX));
            let _ = query.matches(&Doc { summary: &sample(), body: None, indexed: false, awaiting: false });
        }
    }
}

/// Sent late in 2023, it came in 2024: `received:` (and `after:`, `before:`) go by when it
/// came, `sent:` and `date:` by its `Date:` (adapted from upstream 62a459d3).
#[test]
fn received_goes_by_when_it_came() {
    let c = berlin(1_718_000_000);
    let mut m = sample();
    m.date = local(2024, 1, 1, &c) - 3600;
    m.received = local(2024, 1, 1, &c) + 3600;
    let check = |s: &str| {
        let mut query = Query::parse(s);
        query.resolve(&c);
        (query.matches(&Doc { summary: &m, body: None, indexed: false, awaiting: false }), query.dates_hold(&m))
    };
    for (typed, holds) in [
        ("received:2024", true),
        ("after:2024", true),
        ("before:2024", false),
        ("sent:2024", false),
        ("sent:2023", true),
        ("date:2023", true),
        ("-sent:2023", false),
    ] {
        assert_eq!(check(typed), (holds, holds), "{typed}");
    }
}
