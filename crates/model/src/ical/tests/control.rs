// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A uid or a text with a control character: RFC 5545 allows none but HTAB in a content line.

use super::*;

/// A uid or a text with a control character was written raw, which RFC 5545 doesn't allow; a
/// server (Radicale) stored the uid without it, and the one kept here was never found there.
/// What's written leaves them out (HTAB stays), a server's copy has the uid without them, and
/// it's looked for there so ([`stored_uid`]). An event's uid is kept as it came all the same,
/// and a file or a message has it so: an organizer who looks for their meeting by the uid they
/// wrote finds it in the answer.
#[test]
fn a_control_character_is_not_written_and_a_uid_is_echoed_as_it_came() {
    let e = CalEvent {
        uid: "probe\u{1}two".into(),
        summary: "Hi\u{1}\tthere\u{7f}".into(),
        start: 1_800_000_000,
        end: 1_800_003_600,
        ..Default::default()
    };
    let text = write_for_server(std::slice::from_ref(&e), 0);
    assert!(!text.contains(|c: char| c.is_ascii_control() && !matches!(c, '\t' | '\r' | '\n')), "{text:?}");
    let back = parse(&text, 0).events;
    assert_eq!((back[0].uid.as_str(), back[0].summary.as_str()), ("probetwo", "Hi\tthere"));
    // An invitation that brings one: kept as it came, and the answer has it byte for byte (a
    // long one too, folded); the server's copy has it as a line may hold it, and is found by that.
    for odd in ["probe\u{1}two\u{1b}".to_string(), format!("{}\u{1}\u{7f}{}", "a".repeat(80), "é".repeat(40))] {
        let invitation = text.replace("UID:probetwo", &format!("UID:{odd}"));
        let kept = parse(&invitation, 0).events.remove(0);
        assert_eq!(kept.uid, odd);
        let reply = write(std::slice::from_ref(&kept), Some("REPLY"), 0);
        assert_eq!(parse(&reply, 0).events[0].uid, odd, "{reply:?}");
        assert!(reply.replace("\r\n ", "").contains(&format!("\r\nUID:{odd}\r\n")), "{reply:?}");
        let stored = write_for_server(std::slice::from_ref(&kept), 0);
        assert!(!stored.contains(|c: char| c.is_ascii_control() && !matches!(c, '\t' | '\r' | '\n')), "{stored:?}");
        assert!(parse(&stored, 0).events.iter().any(|x| x.uid == stored_uid(&kept.uid)), "{stored}");
    }
    assert_eq!((stored_uid(" a\u{0}b \u{7f}"), stored_uid("x\ty")), ("ab".to_string(), "x\ty".to_string()));
    // A line kept as written that would end the event once its control is left out: left out.
    let odd = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:o\r\nDTSTART:20261102T090000Z\r\nEND:VEVENT\u{1}\r\nX-AFTER:kept\r\nSUMMARY:Odd\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let o = parse(odd, 0).events.remove(0);
    let back = parse(&write_for_server(std::slice::from_ref(&o), 0), 0).events;
    assert_eq!(back.len(), 1);
    assert_eq!((back[0].summary.as_str(), back[0].extra.contains(&"X-AFTER:kept".to_string())), ("Odd", true));
}
