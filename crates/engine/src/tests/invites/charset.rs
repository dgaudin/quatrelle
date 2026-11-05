// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: an invitation in a charset other than UTF-8 is read in it (the calendar file's
//! bytes come as they were sent, since attachments are saved that way).

use super::*;

/// Put a message carrying this calendar file (its type and bytes) in the sample Inbox and open it.
fn deliver_ics(s: &mut Session, n: u32, mime: &str, data: Vec<u8>) {
    let d = quatrelle_mime::Draft {
        to: vec![quatrelle_model::EmailAddress::new("Riley", "riley@studio.example")],
        subject: format!("REQUEST {n}"),
        body: "See the calendar.".into(),
        attachments: vec![quatrelle_mime::OutAttachment { name: "invite.ics".into(), mime: mime.into(), data }],
        ..Default::default()
    };
    let raw = quatrelle_mime::build(&d, &quatrelle_model::EmailAddress::new(ANA.0, ANA.1), NOW, &format!("<i{n}@x>")).unwrap();
    let store = s.store.clone().unwrap();
    let mut state = store.load_folder("sample", "INBOX").unwrap();
    let key = quatrelle_model::MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: state.validity, uid: 9000 + n };
    state.messages.insert(0, quatrelle_mime::summary_from_full(&raw, key.clone(), quatrelle_model::Flags::default(), 0, NOW));
    store.save_folder("sample", "INBOX", &state).unwrap();
    store.write_body(&key, &raw).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
}

#[test]
fn an_invitation_in_another_charset_is_read_in_it() {
    let mut s = sample_session("invite-charset");
    let event = CalEvent {
        uid: "latin@soundlab.example".into(),
        summary: "Caf\u{e9} review".into(),
        start: NOW + 86_400,
        end: NOW + 86_400 + 3600,
        organizer: Some(Attendee { email: ANA.1.into(), name: ANA.0.into(), ..Default::default() }),
        attendees: vec![me("NEEDS-ACTION")],
        ..Default::default()
    };
    let ics = quatrelle_model::ical::write(&[event], Some("REQUEST"), NOW);
    // In Windows-1252 the accent is the one byte 0xE9, which isn't UTF-8.
    let latin: Vec<u8> = ics.chars().map(|c| u8::try_from(u32::from(c)).unwrap()).collect();
    assert!(latin.contains(&0xe9));
    deliver_ics(&mut s, 1, "text/calendar; method=REQUEST; charset=windows-1252", latin);
    assert_eq!(s.invite().unwrap().event.summary, "Caf\u{e9} review");
    done(s);
}
