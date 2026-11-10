// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Outlook's Notes over EWS: their text, their colour and their window's size.

use quatrelle_model::pim::{Note, Pim, StoredNote};

use super::*;

#[test]
fn sticky_notes_sync_with_the_notes_folder() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let notes = d.folder("Notes", None, Some("notes"), "IPF.StickyNote");
    let old = parse(
        "<t:Message><t:ItemClass>IPM.StickyNote</t:ItemClass><t:Subject>Call Ana</t:Subject><t:Body BodyType=\"Text\">Call Ana\r\nabout the mix</t:Body>\
         <t:ExtendedProperty><t:ExtendedFieldURI PropertySetId=\"0006200E-0000-0000-C000-000000000046\" PropertyId=\"35584\" PropertyType=\"Integer\"/><t:Value>2</t:Value></t:ExtendedProperty></t:Message>",
    );
    d.pim.push(("N1".into(), notes, old.kids[0].clone()));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let mut pim = Pim::default();
    pim.notes.push(StoredNote {
        note: Note { uid: "n-local".into(), text: "Buy paper\n170 gsm".into(), color: 1, modified: 5, ..Default::default() },
        dirty: true,
        ..Default::default()
    });
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    let list = sync.notes.unwrap();
    let ana = list.iter().find(|n| n.href == "note:N1").unwrap();
    assert_eq!((ana.note.text.as_str(), ana.note.color), ("Call Ana\nabout the mix", 2));
    assert!(list.iter().any(|n| n.note.text == "Buy paper\n170 gsm" && n.note.color == 1), "made in the Notes folder, its colour kept");
    assert_eq!(sync.pushed_notes.len(), 1);
    assert!(sync.pushed_notes[0].href.starts_with("note:"));
}

/// A note's property in PSETID_Note, as Outlook writes it.
fn named(id: u32, value: u32) -> String {
    format!(
        "<t:ExtendedProperty><t:ExtendedFieldURI PropertySetId=\"0006200E-0000-0000-C000-000000000046\" PropertyId=\"{id}\" PropertyType=\"Integer\"/><t:Value>{value}</t:Value></t:ExtendedProperty>"
    )
}

/// An account with a Notes folder holding one note resized in Outlook (640 by 90), and one made
/// here at 300 by 250, to send.
fn sized(refuse: bool) -> (super::super::server::Server, Pim) {
    let mut d = Data::default();
    d.refuse_note_size = refuse;
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let notes = d.folder("Notes", None, Some("notes"), "IPF.StickyNote");
    let wide = parse(&format!(
        "<t:Message><t:ItemClass>IPM.StickyNote</t:ItemClass><t:Subject>Wide</t:Subject><t:Body BodyType=\"Text\">Wide</t:Body>{}{}{}</t:Message>",
        named(35584, 3),
        named(35586, 640),
        named(35587, 90)
    ));
    d.pim.push(("N1".into(), notes, wide.kids[0].clone()));
    let mut pim = Pim::default();
    let note = Note { uid: "n-local".into(), text: "Made here".into(), color: 1, modified: 5, width: 300, height: 250 };
    pim.notes.push(StoredNote { note, dirty: true, ..Default::default() });
    (serve(d), pim)
}

/// A note's window size (1.4) is read from Exchange and written to it with the note.
#[test]
fn a_notes_size_is_read_and_written() {
    let (server, pim) = sized(false);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    let list = sync.notes.unwrap();
    let wide = list.iter().find(|n| n.href == "note:N1").unwrap();
    assert_eq!(wide.note.size(), Some((640, 120)), "read, held to the smallest height");
    let made = list.iter().find(|n| n.note.text == "Made here").unwrap();
    assert_eq!((made.note.size(), made.note.color), (Some((300, 250)), 1), "written with the note, and read back");
    assert_eq!(sync.pushed_notes.iter().map(|n| n.note.size()).collect::<Vec<_>>(), [Some((300, 250))]);
    assert!(sync.notes_size_refused.is_empty());
}

/// A server that won't take a note's size: the note is made without it, its size goes back to
/// the server's (none), and the sync says so.
#[test]
fn a_notes_size_refused_goes_back_and_is_said() {
    let (server, pim) = sized(true);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert_eq!(sync.notes_size_refused, ["n-local"]);
    assert!(sync.errors.iter().any(|e| e.starts_with("Made here: the server didn't keep its size")), "{:?}", sync.errors);
    let pushed: Vec<(&str, Option<(u16, u16)>)> = sync.pushed_notes.iter().map(|n| (n.note.text.as_str(), n.note.size())).collect();
    assert_eq!(pushed, [("Made here", None)], "the note made, without its size");
    let list = sync.notes.unwrap();
    assert!(list.iter().any(|n| n.note.text == "Made here" && n.note.size().is_none() && n.note.color == 1), "{list:?}");
}
