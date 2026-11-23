// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A contact or a note changed here is made again on Exchange and its old copy deleted: only an
//! old copy that "isn't there" lets it be made; one refused, or not deleted for now, doesn't (no
//! second copy). A deletion asked for waits when the server can't do it now, and a reminder
//! answer the server refuses isn't taken for given.

use quatrelle_model::pim::{Note, Pim, ReminderAnswer, StoredCard, StoredNote, Tombstone};
use quatrelle_model::vcard;

use super::*;

const NOW: i64 = 1_792_000_000;

/// An account with its folders and, when `held`, a contact `C1` and a note `N1` on the server.
fn account(held: bool) -> Data {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    let contacts = d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let notes = d.folder("Notes", None, Some("notes"), "IPF.StickyNote");
    if held {
        let card = parse("<t:Contact><t:DisplayName>Zoe Park</t:DisplayName><t:GivenName>Zoe</t:GivenName><t:Surname>Park</t:Surname></t:Contact>");
        d.pim.push(("C1".into(), contacts, card.kids[0].clone()));
        let note = parse(
            "<t:Message><t:ItemClass>IPM.StickyNote</t:ItemClass><t:Subject>Call Ana</t:Subject><t:Body BodyType=\"Text\">Call Ana</t:Body></t:Message>",
        );
        d.pim.push(("N1".into(), notes, note.kids[0].clone()));
    }
    d
}

fn connect(server: &super::super::server::Server) -> crate::ews::Client {
    crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "zoe@contoso.example").unwrap()
}

/// A contact changed here, whose item on the server is `C1`.
fn changed_contact() -> Pim {
    let card =
        vcard::parse("BEGIN:VCARD\r\nVERSION:4.0\r\nUID:zoe\r\nFN:Zoe Park\r\nN:Park;Zoe;;;\r\nTEL;TYPE=work:+1 555 0199\r\nEND:VCARD\r\n").remove(0);
    let mut pim = Pim::default();
    pim.cards.push(StoredCard { book: crate::ews_pim::CONTACTS.into(), href: "contact:C1".into(), etag: "k".into(), card, dirty: true });
    pim
}

/// A note changed here, whose item on the server is `N1`.
fn changed_note() -> Pim {
    let mut pim = Pim::default();
    pim.notes.push(StoredNote {
        note: Note { uid: "n1".into(), text: "Call Ana, tonight".into(), color: 1, modified: 5, ..Default::default() },
        href: "note:N1".into(),
        dirty: true,
        ..Default::default()
    });
    pim
}

/// How many contacts (`Contact`) or notes (`Note`) the server holds.
fn count(server: &super::super::server::Server, class: &str) -> usize {
    let d = server.data.lock().unwrap();
    d.pim.iter().filter(|(_, _, el)| if class == "Note" { el.text_of("ItemClass") == Some("IPM.StickyNote") } else { el.name == class }).count()
}

/// The old copy of a changed contact can't be deleted (refused): the contact isn't made again,
/// the refusal is said, and the change stays for the next sync, which doesn't make a second one
/// either.
#[test]
fn a_changed_contact_whose_old_copy_is_refused_is_made_once() {
    let server = serve(account(true));
    server.data.lock().unwrap().undeletable.push("C1".into());
    let c = connect(&server);
    let pim = changed_contact();
    for at in [NOW, NOW + 60] {
        let out = crate::ews_pim::sync(&c, &pim, at).unwrap();
        assert_eq!(out.errors.len(), 1, "the refusal is said: {:?}", out.errors);
        assert!(out.pushed_cards.is_empty(), "not made again: {:?}", out.pushed_cards);
        assert_eq!(count(&server, "Contact"), 1, "one on the server, not two");
    }
}

/// The same for a note.
#[test]
fn a_changed_note_whose_old_copy_is_refused_is_made_once() {
    let server = serve(account(true));
    server.data.lock().unwrap().undeletable.push("N1".into());
    let c = connect(&server);
    let pim = changed_note();
    for at in [NOW, NOW + 60] {
        let out = crate::ews_pim::sync(&c, &pim, at).unwrap();
        assert_eq!(out.errors.len(), 1, "the refusal is said: {:?}", out.errors);
        assert!(out.pushed_notes.is_empty(), "not made again: {:?}", out.pushed_notes);
        assert_eq!(count(&server, "Note"), 1, "one on the server, not two");
    }
}

/// An old copy the server no longer has (deleted in Outlook meanwhile) lets the change be made:
/// "not there" is the one answer that counts as done.
#[test]
fn a_changed_contact_or_note_whose_old_copy_is_gone_is_made_once() {
    let server = serve(account(false));
    let c = connect(&server);
    let mut pim = changed_contact();
    pim.notes = changed_note().notes;
    let out = crate::ews_pim::sync(&c, &pim, NOW).unwrap();
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert_eq!((out.pushed_cards.len(), out.pushed_notes.len()), (1, 1));
    assert_eq!((count(&server, "Contact"), count(&server, "Note")), (1, 1));
}

/// A deletion asked here that the server can't do now (busy) isn't taken for done: it is said,
/// stays asked, and goes through at the next sync.
#[test]
fn a_deletion_tried_offline_is_tried_again_and_stays_deleted_here() {
    let server = serve(account(true));
    server.data.lock().unwrap().delete_busy.push("C1".into());
    let c = connect(&server);
    let mut pim = Pim::default();
    pim.deleted.push(Tombstone { href: "contact:C1".into(), ..Default::default() });
    let out = crate::ews_pim::sync(&c, &pim, NOW).unwrap();
    assert!(!out.deleted.iter().any(|h| h == "contact:C1"), "still to delete: {:?}", out.deleted);
    assert!(out.errors.iter().any(|e| !e.starts_with("not deleted on the server")), "{:?}", out.errors);
    assert_eq!(count(&server, "Contact"), 1, "still there");
    server.data.lock().unwrap().delete_busy.clear();
    let out = crate::ews_pim::sync(&c, &pim, NOW + 60).unwrap();
    assert!(out.deleted.iter().any(|h| h == "contact:C1"), "{:?}", out.deleted);
    assert_eq!(count(&server, "Contact"), 0);
}

/// A deletion of what the server doesn't have is done.
#[test]
fn a_deletion_of_what_is_not_there_is_done() {
    let server = serve(account(false));
    let c = connect(&server);
    let mut pim = Pim::default();
    pim.deleted.push(Tombstone { href: "contact:C1".into(), ..Default::default() });
    let out = crate::ews_pim::sync(&c, &pim, NOW).unwrap();
    assert!(out.deleted.iter().any(|h| h == "contact:C1") && out.errors.is_empty(), "{:?} {:?}", out.deleted, out.errors);
}

/// A reminder answer the server refuses (access denied) isn't taken for given: it is said and
/// kept for the next sync; one for a reminder the server no longer lists is let go.
#[test]
fn a_reminder_answer_the_server_refuses_is_kept() {
    let mut d = account(false);
    d.reminders.listed = vec![("E-one".into(), String::new(), "2026-11-04T09:00:00Z".into())];
    d.reminders.refuse = true;
    let server = serve(d);
    let c = connect(&server);
    let at = 1_793_782_800;
    let answer = |href: &str| ReminderAnswer {
        calendar: "calendar".into(),
        href: href.into(),
        uid: href.into(),
        occurrence: at,
        series: false,
        snooze_until: None,
    };
    let kept = Pim { reminder_answers: vec![answer("E-one")], ..Default::default() };
    let out = crate::ews_pim::sync(&c, &kept, at - 900).unwrap();
    assert!(out.answered.is_empty(), "kept: {:?}", out.answered);
    assert!(out.errors.iter().any(|e| e.starts_with("a reminder")), "{:?}", out.errors);
    let gone = Pim { reminder_answers: vec![answer("E-gone")], ..Default::default() };
    let out = crate::ews_pim::sync(&c, &gone, at - 900).unwrap();
    assert_eq!(out.answered, gone.reminder_answers, "not listed any more: let go");
}
