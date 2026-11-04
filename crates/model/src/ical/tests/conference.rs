// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! An event's `CONFERENCE` (RFC 7986 §5.11): read whatever the order of its parameters, the
//! others kept as they came, written with its label quoted as RFC 5545 quotes a parameter, and
//! taken by Join and the event's links.

use proptest::prelude::*;

use super::*;

/// The one event of a calendar holding these lines.
fn event(lines: &[&str]) -> CalEvent {
    let body: String = lines.iter().map(|l| format!("{l}\r\n")).collect();
    let ics = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:c1\r\nDTSTART:20261016T140000Z\r\nDTEND:20261016T150000Z\r\n{body}END:VEVENT\r\nEND:VCALENDAR\r\n"
    );
    parse(&ics, 0).events.into_iter().next().expect("one event")
}

fn conference(url: &str, label: &str, features: &[&str]) -> Conference {
    Conference { url: url.into(), label: label.into(), features: features.iter().map(|f| f.to_string()).collect() }
}

/// An event with this conference and nothing else to say.
fn with(c: Option<Conference>) -> CalEvent {
    CalEvent { uid: "c1".into(), start: 1_792_000_000, end: 1_792_003_600, conference: c, ..Default::default() }
}

/// The calendar file written for an event, its lines unfolded.
fn written(e: &CalEvent, method: Option<&str>) -> String {
    write(std::slice::from_ref(e), method, 0).replace("\r\n ", "")
}

/// The `CONFERENCE` lines of an unfolded file.
fn conference_lines(text: &str) -> Vec<&str> {
    text.split("\r\n").filter(|l| l.starts_with("CONFERENCE")).collect()
}

#[test]
fn a_conference_is_read_in_any_order() {
    let e = event(&["CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=Webex:https://acme.webex.com/meet/ana"]);
    assert_eq!(e.conference, Some(conference("https://acme.webex.com/meet/ana", "Webex", &["AUDIO", "VIDEO"])));
    assert!(!e.extra.iter().any(|l| l.starts_with("CONFERENCE")), "{:?}", e.extra);
    // Parameters in another order, `VALUE` in small letters, the label quoted with a comma in it.
    let e = event(&["CONFERENCE;LABEL=\"Salle A, 2e\";FEATURE=video;value=uri:https://meet.example.org/abc"]);
    assert_eq!(e.conference, Some(conference("https://meet.example.org/abc", "Salle A, 2e", &["VIDEO"])));
    // No parameter at all: a link, and nothing said of it.
    let e = event(&["CONFERENCE:https://jitsi.example.org/team"]);
    assert_eq!(e.conference, Some(conference("https://jitsi.example.org/team", "", &[])));
    assert!(e.extra.is_empty(), "{:?}", e.extra);
    // A phone among other features is still a conference.
    let e = event(&["CONFERENCE;VALUE=URI;FEATURE=PHONE,VIDEO:https://meet.example.org/x"]);
    assert_eq!(e.conference, Some(conference("https://meet.example.org/x", "", &["PHONE", "VIDEO"])));
    // Features that aren't tokens say nothing: as if there were none, and a phone stays a phone.
    for line in ["CONFERENCE;FEATURE=:https://x.example/", "CONFERENCE;FEATURE= a b ,x_y:https://x.example/"] {
        assert_eq!(event(&[line]).conference, Some(conference("https://x.example/", "", &[])), "{line}");
    }
    let phone = "CONFERENCE;FEATURE=PHONE,x_y:https://x.example/";
    assert_eq!((event(&[phone]).conference, event(&[phone]).extra), (None, vec![phone.to_string()]));
    // At most ten features of at most 40 bytes each are kept.
    let many: Vec<String> = (0..12).map(|i| format!("F{i}")).collect();
    let e = event(&[&format!("CONFERENCE;FEATURE={},{},VIDEO:https://x.example/", "X".repeat(41), many.join(","))]);
    assert_eq!(e.conference.map(|c| c.features), Some(many.into_iter().take(10).collect::<Vec<_>>()));
    // Judged on those kept: ten phones and a video would be written as ten phones, no conference.
    let phones = format!("CONFERENCE;FEATURE={}VIDEO:https://x.example/", "PHONE,".repeat(10));
    assert_eq!((event(&[&phones]).conference, event(&[&phones]).extra), (None, vec![phones.clone()]));
}

/// Those of the issue's Review Focus 1: a phone, a chat, a phone given as a web link, the
/// conference, and a second one.
const OTHERS: [&str; 5] = [
    "CONFERENCE;VALUE=URI;FEATURE=PHONE;LABEL=Appel:tel:+33-1-23-45-67-89",
    "CONFERENCE;VALUE=URI;FEATURE=CHAT:https://chat.example.org/r",
    "CONFERENCE;VALUE=URI;FEATURE=PHONE:https://dial.example.org/",
    "CONFERENCE;VALUE=URI:https://first.example.org/",
    "CONFERENCE;VALUE=URI:https://second.example.org/",
];

#[test]
fn other_conference_lines_stay_as_they_came() {
    let e = event(&OTHERS);
    assert_eq!(e.conference, Some(conference("https://first.example.org/", "", &[])));
    assert_eq!(e.extra, [OTHERS[0], OTHERS[1], OTHERS[2], OTHERS[4]]);
}

#[test]
fn hostile_links_are_no_conference() {
    let long = format!("https://a.example/{}", "a".repeat(2040));
    assert!(long.len() > 2048);
    let mut lines: Vec<String> =
        ["javascript:alert(1)", "https://user@evil.example/", "https://acme.webex.com:443@evil.example/", "data:text/html,x", long.as_str()]
            .iter()
            .map(|v| format!("CONFERENCE;VALUE=URI:{v}"))
            .collect();
    lines.push("CONFERENCE;VALUE=TEXT:https://a.example/".into());
    for line in &lines {
        let e = event(&[line]);
        assert_eq!(e.conference, None, "{line}");
        assert_eq!(e.extra, [line.as_str()], "{line}");
    }
}

#[test]
fn a_conference_is_written() {
    let ana = "https://acme.webex.com/meet/ana";
    let out = written(&with(Conference::new(ana, "Webex")), None);
    assert_eq!(conference_lines(&out), ["CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=Webex:https://acme.webex.com/meet/ana"]);
    // An invitation carries it too.
    assert_eq!(conference_lines(&written(&with(Conference::new(ana, "Webex")), Some("REQUEST"))).len(), 1);
    // A label with what a parameter can't hold bare goes in double quotes, its own left out.
    let out = written(&with(Conference::new(ana, "Salle \"A\"; 2e")), None);
    assert_eq!(conference_lines(&out), ["CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=\"Salle A; 2e\":https://acme.webex.com/meet/ana"]);
    // No label, no `LABEL`; its own features as they are; none, no `FEATURE`.
    let out = written(&with(Conference::new("https://x.example/", "")), None);
    assert_eq!(conference_lines(&out), ["CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO:https://x.example/"]);
    let out = written(&with(Some(conference("https://x.example/", "", &["VIDEO", "SCREEN"]))), None);
    assert_eq!(conference_lines(&out), ["CONFERENCE;VALUE=URI;FEATURE=VIDEO,SCREEN:https://x.example/"]);
    let out = written(&with(Some(conference("https://x.example/", "", &[]))), None);
    assert_eq!(conference_lines(&out), ["CONFERENCE;VALUE=URI:https://x.example/"]);
    // A long link folds as every line does, and reads back the same.
    let long = format!("https://a.example/{}", "b".repeat(282));
    assert_eq!(long.len(), 300);
    let e = with(Conference::new(&long, "Webex"));
    let out = write(std::slice::from_ref(&e), None, 0);
    assert!(out.split("\r\n").all(|l| l.len() <= 75), "{out}");
    assert_eq!(parse(&out, 0).events.first().and_then(|e| e.conference.clone()), e.conference);
    // A damaged cache: a link that may not be opened isn't written; a label and features that
    // would cut the line are cleaned.
    let out = written(&with(Some(Conference { url: "javascript:x".into(), ..Default::default() })), None);
    assert!(conference_lines(&out).is_empty(), "{out}");
    let out = written(&with(Some(conference("https://x.example/", "a\r\nX-EVIL:1\"", &["VIDEO", "A;B:C"]))), None);
    assert_eq!(conference_lines(&out), ["CONFERENCE;VALUE=URI;FEATURE=VIDEO;LABEL=\"aX-EVIL:1\":https://x.example/"]);
    assert!(!out.contains("\r\nX-EVIL"), "{out}");
}

/// Review Focus 1: an event from another app, written here again, has each of its lines once,
/// its conference with its own label and features.
#[test]
fn kept_lines_are_written_once() {
    let e = event(&OTHERS);
    let out = write(std::slice::from_ref(&e), None, 0);
    let back = parse(&out, 0).events.into_iter().next().expect("one event");
    assert_eq!((&back.conference, &back.extra), (&e.conference, &e.extra));
    let out = out.replace("\r\n ", "");
    for value in [
        "tel:+33-1-23-45-67-89",
        "https://chat.example.org/r",
        "https://dial.example.org/",
        "https://first.example.org/",
        "https://second.example.org/",
    ] {
        assert_eq!(out.matches(value).count(), 1, "{value} in {out}");
    }
}

/// An event cached before Quatrelle read `CONFERENCE` has the line among its kept ones: it's
/// taken as the file's reading would take it, and taken out of them.
#[test]
fn a_kept_conference_is_adopted() {
    let tel = "CONFERENCE;VALUE=URI;FEATURE=PHONE;LABEL=Appel:tel:+33-1-23-45-67-89";
    let second = "CONFERENCE;VALUE=URI:https://second.example.org/";
    let kept = |lines: &[&str]| lines.iter().map(|l| l.to_string()).collect::<Vec<_>>();
    let mut e = CalEvent { extra: kept(&[tel, "CONFERENCE;VALUE=URI;LABEL=Old:https://old.example/", second]), ..with(None) };
    e.adopt_kept_conference();
    assert_eq!(e.conference, Some(conference("https://old.example/", "Old", &[])));
    assert_eq!(e.extra, [tel, second]);
    let once = e.clone();
    e.adopt_kept_conference();
    assert_eq!(e, once, "a second call changes nothing");
    // An event that has its conference keeps its lines; one inside a component kept whole
    // stays there.
    let mut set = CalEvent { extra: kept(&[second]), ..with(Conference::new("https://x.example/", "")) };
    let before = set.clone();
    set.adopt_kept_conference();
    assert_eq!(set, before);
    let nested = kept(&["BEGIN:X-PART", "CONFERENCE:https://inner.example/", "END:X-PART"]);
    let mut e = CalEvent { extra: nested.clone(), ..with(None) };
    e.adopt_kept_conference();
    assert_eq!((e.conference, e.extra), (None, nested));
}

/// A conference whose link is also among the kept lines is written once.
#[test]
fn a_kept_line_never_repeats_the_conference() {
    let other = "CONFERENCE;VALUE=URI;FEATURE=CHAT:https://chat.example.org/r";
    let e = CalEvent {
        extra: vec!["CONFERENCE;VALUE=URI;FEATURE=VIDEO:HTTPS://OLD.EXAMPLE/ ".into(), other.into()],
        ..with(Conference::new("https://old.example/", "Old"))
    };
    let out = written(&e, None);
    let lines = conference_lines(&out);
    assert_eq!(lines.iter().filter(|l| l.to_ascii_lowercase().contains("old.example")).count(), 1, "{out}");
    assert_eq!(lines, ["CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=Old:https://old.example/", other]);
}

#[test]
fn a_new_conference_is_cleaned() {
    assert_eq!(Conference::new("https://x.example/", "Webex"), Some(conference("https://x.example/", "Webex", &["AUDIO", "VIDEO"])));
    let c = Conference::new("https://x.example/", &"é".repeat(100)).expect("a web link");
    assert_eq!(c.label.chars().count(), CONFERENCE_LABEL_MAX);
    assert_eq!(Conference::new("https://x.example/", "Web\u{7}ex\n").map(|c| c.label).as_deref(), Some("Webex"));
    // Cut, then trimmed: no space left at the end for the reader to take off.
    let cut = Conference::new("https://x.example/", &format!("{} b", "a".repeat(79))).map(|c| c.label);
    assert_eq!(cut, Some("a".repeat(79)));
    assert_eq!(Conference::new(" https://x.example/ ", "").map(|c| c.url).as_deref(), Some("https://x.example/"));
    assert_eq!(Conference::new("ftp://x.example", "x"), None);
    assert_eq!(Conference::new("", "x"), None);
    // A stranger's label loses the characters that turn text round or hide in it.
    let hidden = "Sal\u{202e}le\u{200b} A\u{2028}";
    let read = event(&[&format!("CONFERENCE;LABEL={hidden}:https://x.example/")]).conference.map(|c| c.label);
    assert_eq!(read.as_deref(), Some("Salle A"));
    assert_eq!(Conference::new("https://x.example/", hidden).map(|c| c.label).as_deref(), Some("Salle A"));
    let every = "a\u{202a}\u{202b}\u{202c}\u{202d}\u{202e}\u{2066}\u{2067}\u{2068}\u{2069}\u{200b}\u{200c}\u{200d}\u{200e}\u{200f}\u{2028}\u{2029}\u{feff}\u{61c}b";
    assert_eq!(Conference::new("https://x.example/", every).map(|c| c.label).as_deref(), Some("ab"));
}

#[test]
fn join_takes_the_server_then_the_conference_then_the_notes() {
    let teams = "https://teams.microsoft.com/l/meetup-join/19%3a";
    let jitsi = "https://jitsi.example.org/team";
    let zoom = "https://us02web.zoom.us/j/1";
    let e = CalEvent { join_url: teams.into(), conference: Conference::new(jitsi, ""), description: format!("Join: {zoom}"), ..Default::default() };
    assert_eq!(e.join_link().as_deref(), Some(teams));
    let no_server = CalEvent { join_url: String::new(), ..e.clone() };
    assert_eq!(no_server.join_link().as_deref(), Some(jitsi), "not a known service: the calendar says it joins");
    let no_conference = CalEvent { conference: None, ..no_server.clone() };
    assert_eq!(no_conference.join_link().as_deref(), Some(zoom));
    let not_a_meeting = CalEvent { join_url: "https://example.com/".into(), ..e };
    assert_eq!(not_a_meeting.join_link().as_deref(), Some(jitsi));
    // A damaged cache's conference that may not be opened: the notes, as before.
    let damaged = CalEvent { conference: Some(Conference { url: "javascript:x".into(), ..Default::default() }), ..no_server };
    assert_eq!(damaged.join_link().as_deref(), Some(zoom));
}

#[test]
fn links_list_the_conference() {
    let jitsi = "https://jitsi.example.org/team";
    let docs = "https://docs.example.org/a";
    let e = CalEvent { conference: Conference::new(jitsi, "Jitsi"), description: format!("Agenda: {docs}"), ..Default::default() };
    assert_eq!(e.links(), [jitsi, docs]);
    // Once, also when the notes hold it.
    let both = CalEvent { description: format!("Agenda: {docs}, then {jitsi}"), ..e.clone() };
    assert_eq!(both.links().iter().filter(|l| *l == jitsi).count(), 1);
    // After the join address.
    let teams = "https://teams.microsoft.com/l/meetup-join/x";
    assert_eq!(CalEvent { join_url: teams.into(), ..e }.links(), [teams, jitsi, docs]);
}

proptest! {
    /// A conference made here, written then read, is the one made.
    #[test]
    fn a_conference_written_is_read_back(url in "https://[a-z]{1,12}\\.example(/[a-zA-Z0-9._~-]{0,20}){0,3}", label in "\\PC{0,100}") {
        let made = Conference::new(&url, &label);
        prop_assert!(made.is_some(), "{}", url);
        let e = with(made.clone());
        let back = parse(&write(std::slice::from_ref(&e), None, 0), 0);
        prop_assert_eq!(back.events.first().and_then(|e| e.conference.clone()), made);
    }

    /// The features a line names are read upper-cased, the first ten kept; none named is a
    /// conference that says nothing of them, and one whose first ten are only a phone, a chat or
    /// a feed is none. Written again, they read back the same.
    #[test]
    fn the_features_named_are_read(
        url in "https://[a-z]{1,12}\\.example(/[a-zA-Z0-9._~-]{0,20}){0,3}",
        named in prop::collection::vec(prop_oneof![Just("PHONE".to_string()), Just("chat".to_string()), Just("Feed".to_string()), "[A-Za-z0-9-]{1,40}"], 0..=12),
    ) {
        let e = event(&[&format!("CONFERENCE;VALUE=URI;FEATURE={}:{url}", named.join(","))]);
        let upper: Vec<String> = named.iter().map(|f| f.to_ascii_uppercase()).collect();
        let first: Vec<&str> = upper.iter().take(10).map(String::as_str).collect();
        let phone_only = !first.is_empty() && first.iter().all(|f| matches!(*f, "PHONE" | "CHAT" | "FEED"));
        let expected = (!phone_only).then(|| conference(&url, "", &first));
        prop_assert_eq!(&e.conference, &expected);
        let back = parse(&write(std::slice::from_ref(&e), None, 0), 0);
        prop_assert_eq!(back.events.first().and_then(|e| e.conference.clone()), expected);
    }
}
