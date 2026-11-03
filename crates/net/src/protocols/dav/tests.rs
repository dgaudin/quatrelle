// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
use quatrelle_model::ical::CalEvent;
use quatrelle_model::pim::{StoredCard, StoredEvent, Tombstone};
use quatrelle_model::vcard::Card;
use quatrelle_model::zone::Zone;

use super::*;
use crate::fakes::fake_dav::{self, BOOK, CAL, PASSWORD, USER};

/// What an account at the fake server's domain trusts, with these (URLs or hosts) besides.
fn trusting(extra: &[&str]) -> Trust {
    Trust::of(&AccountConfig { email: USER.into(), ..Default::default() }, extra)
}

/// The same server under its other name: `127.0.0.1` and `localhost` are two sites.
fn by_name(url: &str) -> String {
    url.replace("127.0.0.1", "localhost")
}

/// The site an `Untrusted` names, or what came instead.
fn untrusted<T: std::fmt::Debug>(r: Result<T>) -> String {
    match r {
        Err(NetError::Untrusted { site }) => site,
        other => format!("not Untrusted: {other:?}"),
    }
}

#[test]
fn hrefs_are_made_absolute() {
    let dav = Dav::new("u", "p", trusting(&["h.example"]));
    assert_eq!(dav.absolute("https://h.example/dav/cal/", "/dav/cal/1.ics").unwrap(), "https://h.example/dav/cal/1.ics");
    assert_eq!(dav.absolute("https://h.example/dav/cal/", "//h.example/x").unwrap(), "https://h.example/x", "the scheme is kept");
}

/// Whether a request as the server saw it carried the password.
fn password_seen(request: &str) -> bool {
    request.to_ascii_lowercase().contains("authorization: basic dtpw")
}

/// A redirect is followed, with the password, only to a site the account trusts: to any other,
/// nothing at all is sent (with the password or without: a request without one gets a 401,
/// read as a failed sign-in), and the request is `Untrusted`.
#[test]
fn a_redirect_is_followed_only_to_a_trusted_site() {
    use crate::http::tests::{bind, serve, serve_on};
    let multi = || b"HTTP/1.1 207 Multi-Status\r\nContent-Length: 0\r\n\r\n".to_vec();
    for (method, depth) in [("PROPFIND", "0"), ("REPORT", "1")] {
        let go = |dav: &Dav, url: &str| dav.request(method, url, Some(depth), "<x/>", &[]).map(|r| r.0.status);
        let dav = Dav::new("u", "p", trusting(&["127.0.0.1"]));
        // Same server: both legs carry it.
        let (a, ha) = serve(vec![b"HTTP/1.1 301 Moved\r\nLocation: /dav/\r\n\r\n".to_vec(), multi()]);
        assert_eq!(go(&dav, &format!("{a}/")).unwrap(), 207);
        let seen = ha.join().unwrap();
        assert!(seen.len() == 2 && seen.iter().all(|r| password_seen(r)), "{method}: {seen:?}");
        assert!(seen[1].starts_with(&format!("{method} /dav/ ")), "{}", seen[1]);
        // Another site: nothing goes there.
        let (b, hb) = serve(Vec::new());
        let (a, ha) = serve(vec![format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {}/dav/\r\n\r\n", by_name(&b)).into_bytes()]);
        assert_eq!(untrusted(go(&dav, &format!("{a}/"))), "localhost", "{method}");
        assert!(password_seen(&ha.join().unwrap()[0]));
        assert!(hb.join().unwrap().is_empty(), "{method}: nothing reached the other site");
        // Both trusted: A -> B -> A, the password on every leg.
        let dav = Dav::new("u", "p", trusting(&["127.0.0.1", "localhost"]));
        let ((la, a), (lb, b)) = (bind(), bind());
        let b = by_name(&b);
        let ha = serve_on(la, vec![format!("HTTP/1.1 302 Found\r\nLocation: {b}/two\r\n\r\n").into_bytes(), multi()]);
        let hb = serve_on(lb, vec![format!("HTTP/1.1 302 Found\r\nLocation: {a}/three\r\n\r\n").into_bytes()]);
        assert_eq!(go(&dav, &format!("{a}/one")).unwrap(), 207);
        let (seen_a, seen_b) = (ha.join().unwrap(), hb.join().unwrap());
        assert!(password_seen(&seen_a[0]) && password_seen(&seen_b[0]) && password_seen(&seen_a[1]), "{method}");
        assert!(seen_a[1].starts_with(&format!("{method} /three ")), "{}", seen_a[1]);
    }
}

/// A PUT is never redirected (the redirect is the answer, nothing is sent again): the
/// password stays with the server it was given for.
#[test]
fn a_put_is_not_sent_on_after_a_redirect() {
    use crate::http::tests::serve;
    let (b, hb) = serve(Vec::new());
    let (a, ha) = serve(vec![format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {b}/x.ics\r\nContent-Length: 0\r\n\r\n").into_bytes()]);
    let r = Dav::new("u", "p", trusting(&["127.0.0.1"])).put(&format!("{a}/x.ics"), "text/calendar", "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n", None);
    assert!(r.is_err(), "{r:?}");
    let seen = ha.join().unwrap();
    assert!(seen.len() == 1 && seen[0].starts_with("PUT /x.ics ") && password_seen(&seen[0]), "{seen:?}");
    assert!(hb.join().unwrap().is_empty(), "nothing reached the other server");
}

/// A PUT or a DELETE to a site the account doesn't trust isn't sent at all.
#[test]
fn a_put_or_delete_elsewhere_is_never_sent() {
    let s = fake_dav::serve();
    let dav = Dav::new(USER, PASSWORD, trusting(&[&s.base]));
    let there = format!("{}{CAL}x.ics", by_name(&s.base));
    assert_eq!(untrusted(dav.put(&there, "text/calendar", "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n", None)), "localhost");
    assert_eq!(untrusted(dav.delete(&there, "\"1\"")), "localhost");
    assert!(s.data.lock().unwrap().log.is_empty(), "{:?}", s.data.lock().unwrap().log);
}

#[test]
fn a_well_known_redirect_within_the_site_signs_in() {
    let s = fake_dav::serve();
    let dav = Dav::new(USER, PASSWORD, trusting(&[&s.base]));
    let home = dav.home_from(Kind::Calendars, &format!("{}/.well-known/caldav", s.base)).unwrap();
    assert_eq!(home, Some(format!("{}/dav/cal/", s.base)));
    // The fake answers 401 to a request without the password: each one carried it.
    assert_eq!(s.data.lock().unwrap().log, ["PROPFIND /.well-known/caldav", "PROPFIND /dav/", "PROPFIND /dav/principals/me/"]);
}

#[test]
fn a_well_known_redirect_to_another_site_is_untrusted_and_sends_nothing_there() {
    let (a, b) = (fake_dav::serve(), fake_dav::serve());
    a.data.lock().unwrap().well_known = Some(format!("{}/dav/", by_name(&b.base)));
    let dav = Dav::new(USER, PASSWORD, trusting(&[&a.base]));
    assert_eq!(untrusted(dav.home_from(Kind::Calendars, &format!("{}/.well-known/caldav", a.base))), "localhost");
    assert_eq!(a.data.lock().unwrap().log, ["PROPFIND /.well-known/caldav"]);
    assert!(b.data.lock().unwrap().log.is_empty(), "nothing at all reached the other site: {:?}", b.data.lock().unwrap().log);
}

#[test]
fn once_trusted_it_works() {
    let (a, b) = (fake_dav::serve(), fake_dav::serve());
    a.data.lock().unwrap().well_known = Some(format!("{}/dav/", by_name(&b.base)));
    // The user allowed `localhost` (an entry in `trusted_domains` reads the same).
    let dav = Dav::new(USER, PASSWORD, trusting(&[&a.base, "localhost"]));
    let home = dav.home_from(Kind::Calendars, &format!("{}/.well-known/caldav", a.base)).unwrap();
    assert_eq!(home, Some(format!("{}/dav/cal/", by_name(&b.base))));
    assert_eq!(b.data.lock().unwrap().log, ["PROPFIND /dav/", "PROPFIND /dav/principals/me/"], "signed in there");
}

/// The review's case: a start on the account's own site whose answer names another site (the
/// principal, or the home). The next request would carry the password there: it isn't sent.
#[test]
fn an_href_on_another_site_is_never_requested() {
    for principal in [true, false] {
        let s = fake_dav::serve();
        {
            let mut d = s.data.lock().unwrap();
            if principal {
                d.principal_at = by_name(&s.base);
            } else {
                d.homes_at = by_name(&s.base);
            }
        }
        let dav = Dav::new(USER, PASSWORD, trusting(&[&s.base]));
        assert_eq!(untrusted(dav.home_from(Kind::Calendars, &format!("{}/.well-known/caldav", s.base))), "localhost", "principal: {principal}");
        // `localhost` reaches this very server: a request sent there would be in its log.
        let log = s.data.lock().unwrap().log.clone();
        let want: &[&str] = if principal {
            &["PROPFIND /.well-known/caldav", "PROPFIND /dav/"]
        } else {
            &["PROPFIND /.well-known/caldav", "PROPFIND /dav/", "PROPFIND /dav/principals/me/"]
        };
        assert_eq!(log, want, "principal: {principal}");
    }
}

/// Review Focus 2: the address's website redirects everything to a web host; another start
/// finds the home, and nothing is asked. Only when no start finds one is the first site noted
/// asked about. A start on another site in itself (an SRV target) is no different.
#[test]
fn a_start_redirected_elsewhere_is_skipped_when_another_finds_the_home() {
    let (web, host, cal) = (fake_dav::serve(), fake_dav::serve(), fake_dav::serve());
    web.data.lock().unwrap().well_known = Some(format!("{}/dav/", by_name(&host.base)));
    let dav = Dav::new(USER, PASSWORD, trusting(&["127.0.0.1"]));
    let start = |base: &str| format!("{base}/.well-known/caldav");
    let starts = [start(&web.base), start(&by_name(&host.base)), start(&cal.base)];
    assert_eq!(dav.discover_from(Kind::Calendars, &starts).unwrap(), Some(format!("{}/dav/cal/", cal.base)));
    assert!(host.data.lock().unwrap().log.is_empty(), "nothing reached the other site");
    // No start finds a home: the first site noted is the question.
    assert_eq!(untrusted(dav.discover_from(Kind::Calendars, &starts[..2])), "localhost");
    // Nothing noted, nothing found: no question.
    let none = fake_dav::serve();
    none.data.lock().unwrap().well_known = Some("/nowhere/".into());
    assert_eq!(dav.discover_from(Kind::Calendars, &[start(&none.base)]).unwrap(), None);
    assert!(host.data.lock().unwrap().log.is_empty());
}

#[test]
fn an_http_href_under_https_is_refused() {
    let dav = Dav::new(USER, PASSWORD, trusting(&["h.example"]));
    assert!(matches!(dav.absolute("https://h.example/dav/", "http://h.example/dav/cal/"), Err(NetError::Tls(_))));
    assert!(matches!(dav.absolute("https://h.example/dav/", "HTTP://h.example:8080/x"), Err(NetError::Tls(_))));
    // Plain http under http is what the server was given as.
    assert_eq!(dav.absolute("http://h.example/dav/", "http://h.example/x").unwrap(), "http://h.example/x");
    // And another site, under either.
    assert_eq!(untrusted(dav.absolute("https://h.example/dav/", "https://evil.example/x")), "evil.example");
    assert_eq!(untrusted(dav.absolute("https://h.example/dav/", "//evil.example/x")), "evil.example");
}

/// `http::Url` keeps an IPv6 host without its brackets; the trust writes it with them. Both
/// read the same here.
#[test]
fn an_ipv6_host_is_compared_as_the_trust_writes_it() {
    let dav = Dav::new(USER, PASSWORD, trusting(&["http://[::1]:8080/dav/"]));
    assert_eq!(dav.absolute("http://[::1]:8080/dav/", "/dav/cal/").unwrap(), "http://[::1]:8080/dav/cal/");
    assert!(dav.reachable(&Url::parse("http://[::1]:9/").unwrap()).is_ok());
    assert_eq!(untrusted(dav.absolute("http://[::1]:8080/dav/", "http://[::2]/x")), "[::2]");
    assert_eq!(untrusted(dav.reachable(&Url::parse("https://[fe80::1]/").unwrap())), "[fe80::1]");
}

/// A host two URL parsers would read differently is never reached, and isn't a domain to ask
/// about: its site would read as a trusted one.
#[test]
fn a_host_that_reads_two_ways_goes_nowhere() {
    let dav = Dav::new(USER, PASSWORD, trusting(&["good.example"]));
    for href in ["https://evil.example\\.good.example/x", "https://evil%2eexample.good.example/x"] {
        let r = dav.absolute("https://dav.good.example/dav/", href);
        assert!(matches!(r, Err(NetError::Protocol(_))), "{href}: {r:?}");
    }
    assert!(dav.absolute("https://dav.good.example/dav/", "https://Mail.Good.Example./x").is_ok(), "case and a final dot are one host");
}

/// `Untrusted` stops the sync like a refused sign-in: it isn't kept with the item's errors
/// (where a tombstone would stay for good and the error come back at each sync).
#[test]
fn untrusted_in_a_put_loop_is_propagated_not_collected() {
    let s = fake_dav::serve();
    let dav = Dav::new(USER, PASSWORD, trusting(&[&s.base]));
    let there = by_name(&s.base);
    let base = Pim {
        caldav_home: Some(format!("{}/dav/cal/", s.base)),
        carddav_home: Some(format!("{}/dav/card/", s.base)),
        dav_checked: Some(100),
        ..Default::default()
    };
    let event = CalEvent { uid: "e".into(), summary: "E".into(), start: 1_800_000_000, end: 1_800_003_600, ..Default::default() };
    let card = Card { uid: "c".into(), name: "C".into(), ..Default::default() };
    let cases = [
        (
            "a deletion",
            Pim { deleted: vec![Tombstone { href: format!("{there}{CAL}x.ics"), etag: "\"1\"".into(), ..Default::default() }], ..base.clone() },
        ),
        (
            "an event",
            Pim { events: vec![StoredEvent { calendar: format!("{there}{CAL}"), event, dirty: true, ..Default::default() }], ..base.clone() },
        ),
        ("a card", Pim { cards: vec![StoredCard { book: format!("{there}{BOOK}"), card, dirty: true, ..Default::default() }], ..base.clone() }),
    ];
    for (what, pim) in cases {
        assert_eq!(untrusted(sync(&dav, USER, "imap.studio.example", &pim, 100, Zone::UTC)), "localhost", "{what}");
    }
    let log = s.data.lock().unwrap().log.clone();
    assert!(!log.iter().any(|l| l.starts_with("PUT") || l.starts_with("DELETE")), "{log:?}");
}

/// The homes found and saved before are trusted like the account's servers (the password has
/// gone there for months): a saved home on another site still syncs, with no question.
#[test]
fn a_saved_home_on_another_site_still_syncs_unasked() {
    let s = fake_dav::serve();
    s.data.lock().unwrap().put(
        &format!("{CAL}a.ics"),
        &quatrelle_model::ical::write(
            &[CalEvent { uid: "a".into(), summary: "Review".into(), start: 1_800_000_000, end: 1_800_003_600, ..Default::default() }],
            None,
            0,
        ),
    );
    let there = by_name(&s.base);
    let acc = AccountConfig {
        email: USER.into(),
        imap: quatrelle_model::ServerConfig { host: "127.0.0.1".into(), ..Default::default() },
        ..Default::default()
    };
    let pim = Pim { caldav_home: Some(format!("{there}/dav/cal/")), carddav_home: Some(format!("{there}/dav/card/")), ..Default::default() };
    // As the worker builds it, from the saved PIM doc.
    let dav = Dav::new(USER, PASSWORD, trust_of(&acc, &pim));
    let out = sync(&dav, USER, "127.0.0.1", &pim, 100, Zone::UTC).unwrap();
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert_eq!(out.events.iter().map(|e| e.event.summary.as_str()).collect::<Vec<_>>(), ["Review"]);
    assert_eq!(out.calendars.map(|c| c.iter().map(|c| c.id.clone()).collect::<Vec<_>>()), Some(vec![format!("{there}{CAL}")]));
    // Without the saved homes, the same server under that name is another site.
    let bare = Dav::new(USER, PASSWORD, trust_of(&acc, &Pim::default()));
    assert_eq!(untrusted(sync(&bare, USER, "127.0.0.1", &pim, 100, Zone::UTC)), "localhost");
}

/// A home that came back on a site the user withdrew (saved by a sync already on its way) doesn't
/// put the site back: nothing goes there, and the question comes back.
#[test]
fn a_home_saved_on_a_withdrawn_site_is_no_trust() {
    let s = fake_dav::serve();
    let there = by_name(&s.base);
    let acc = AccountConfig { email: USER.into(), withdrawn_domains: vec!["localhost".into()], ..Default::default() };
    let pim = Pim { caldav_home: Some(format!("{there}/dav/cal/")), carddav_home: Some(String::new()), ..Default::default() };
    let dav = Dav::new(USER, PASSWORD, trust_of(&acc, &pim));
    assert_eq!(untrusted(sync(&dav, USER, "127.0.0.1", &pim, 100, Zone::UTC)), "localhost");
    assert_eq!(s.data.lock().unwrap().log, Vec::<String>::new(), "nothing went there");
}

/// Allowed, found, removed: with what `account.distrustDomain` leaves (no home, the look to do
/// again, the site withdrawn) the look stops at that site, nothing is sent there, and it is asked
/// about again. The saved home is what kept it trusted: gone, so is the password's way there.
#[test]
fn a_site_removed_is_looked_at_again_and_asked_about() {
    let s = fake_dav::serve();
    let there = by_name(&s.base);
    s.data.lock().unwrap().well_known = Some(format!("{there}/dav/"));
    let start = format!("{}/.well-known/caldav", s.base);
    let starts = |kind: Kind| if kind == Kind::Calendars { vec![start.clone()] } else { Vec::new() };
    let acc = AccountConfig {
        email: USER.into(),
        imap: quatrelle_model::ServerConfig { host: "127.0.0.1".into(), ..Default::default() },
        ..Default::default()
    };

    // Allowed: the home is found on the other site.
    let allowed = AccountConfig { trusted_domains: vec!["localhost".into()], ..acc.clone() };
    let found = syncing::sync_from(&Dav::new(USER, PASSWORD, trust_of(&allowed, &Pim::default())), &starts, &Pim::default(), 100, Zone::UTC).unwrap();
    assert_eq!(found.caldav_home, Some(format!("{there}/dav/cal/")));

    // Removed.
    s.data.lock().unwrap().log.clear();
    let removed = AccountConfig { withdrawn_domains: vec!["localhost".into()], ..acc };
    let forgotten = Pim { caldav_home: Some(String::new()), carddav_home: Some(String::new()), ..Default::default() };
    let dav = Dav::new(USER, PASSWORD, trust_of(&removed, &forgotten));
    let again = syncing::sync_from(&dav, &starts, &forgotten, 200, Zone::UTC).unwrap();
    assert_eq!(again.untrusted.as_deref(), Some("localhost"), "the question comes back");
    assert_eq!(again.caldav_home, None);
    assert_eq!(s.data.lock().unwrap().log, ["PROPFIND /.well-known/caldav"], "only the start, on the account's own server");
}

/// A collection a home's listing names on another site is never requested.
#[test]
fn a_collection_on_another_site_is_never_requested() {
    let s = fake_dav::serve();
    s.data.lock().unwrap().collections_at = by_name(&s.base);
    let dav = Dav::new(USER, PASSWORD, trusting(&[&s.base]));
    assert_eq!(untrusted(dav.collections(Kind::Calendars, &format!("{}/dav/cal/", s.base))), "localhost");
    assert_eq!(s.data.lock().unwrap().log, ["PROPFIND /dav/cal/"], "nothing else, under either name");
}

/// A deletion saved with an address that can't be read safely can never be sent: it is
/// dropped, not kept with the errors (where it would come back at each sync).
#[test]
fn a_deletion_saved_with_an_unreadable_address_is_dropped() {
    let dav = Dav::new(USER, PASSWORD, trusting(&[]));
    let bad = "https://evil.example\\.studio.example/x.ics".to_string();
    let pim = Pim {
        caldav_home: Some(String::new()),
        carddav_home: Some(String::new()),
        dav_checked: Some(100),
        deleted: vec![Tombstone { href: bad.clone(), etag: "\"1\"".into(), ..Default::default() }],
        ..Default::default()
    };
    let out = sync(&dav, USER, "imap.studio.example", &pim, 100, Zone::UTC).unwrap();
    assert_eq!(out.deleted, [bad]);
    assert!(out.errors.is_empty(), "{:?}", out.errors);
}

/// The fix round's Important case: the calendars' home is saved, and the address books' look
/// stops at another site. The calendars sync all the same, and the site comes back with the
/// result, to ask about once; the same day nothing is looked for or asked again, a day later
/// it is, and once the site is allowed it's looked for at once.
#[test]
fn a_contacts_look_stopped_elsewhere_leaves_the_calendars_syncing() {
    let (s, web) = (fake_dav::serve(), fake_dav::serve());
    let review = CalEvent { uid: "a".into(), summary: "Review".into(), start: 1_800_000_000, end: 1_800_003_600, ..Default::default() };
    s.data.lock().unwrap().put(&format!("{CAL}a.ics"), &quatrelle_model::ical::write(&[review], None, 0));
    web.data.lock().unwrap().well_known = Some(format!("{}/dav/", by_name(&web.base)));
    let starts = |_: Kind| vec![format!("{}/.well-known/carddav", web.base)];
    let dav = Dav::new(USER, PASSWORD, trusting(&["127.0.0.1"]));
    let pim = Pim { caldav_home: Some(format!("{}/dav/cal/", s.base)), carddav_home: Some(String::new()), ..Default::default() };

    let first = syncing::sync_from(&dav, &starts, &pim, 100, Zone::UTC).unwrap();
    assert_eq!((first.looked, first.untrusted.as_deref(), first.carddav_home.as_deref()), (true, Some("localhost"), None));
    assert_eq!(first.events.iter().map(|e| e.event.summary.as_str()).collect::<Vec<_>>(), ["Review"], "the calendars synced");
    assert!(first.errors.is_empty(), "{:?}", first.errors);

    // As the engine merges it.
    let merged = Pim { dav_checked: Some(first.checked), dav_untrusted: first.untrusted.clone(), ..pim.clone() };
    let looks = web.data.lock().unwrap().log.len();
    let same_day = syncing::sync_from(&dav, &starts, &merged, 160, Zone::UTC).unwrap();
    assert_eq!((same_day.looked, same_day.untrusted.as_deref()), (false, None), "nothing looked for, nothing to ask");
    assert_eq!(web.data.lock().unwrap().log.len(), looks);
    assert_eq!(same_day.calendars.map(|c| c.len()), Some(1));

    let next_day = syncing::sync_from(&dav, &starts, &merged, 100 + 86_401, Zone::UTC).unwrap();
    assert_eq!((next_day.looked, next_day.untrusted.as_deref()), (true, Some("localhost")));

    let allowed = Dav::new(USER, PASSWORD, trusting(&["127.0.0.1", "localhost"]));
    let found = syncing::sync_from(&allowed, &starts, &merged, 160, Zone::UTC).unwrap();
    assert_eq!((found.looked, found.untrusted.as_deref()), (true, None));
    assert_eq!(found.carddav_home, Some(format!("{}/dav/card/", by_name(&web.base))));
}
