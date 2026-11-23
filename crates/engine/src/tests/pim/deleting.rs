// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a task, a note or a contact deleted while the sync that makes it on the server
//! is on its way is held until the server says where it made it, then deleted there as any other,
//! and never comes back; one deleted before any sync read it never goes. Tasks and notes are
//! Exchange's, contacts Exchange's or a CardDAV server's.

use quatrelle_model::dates::Clock;
use quatrelle_model::pim::{BookInfo, DavSync, PIM_DOC, Pim, StoredCard, StoredNote, StoredTask};

use super::*;

/// What is made, and on which kind of server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Task,
    Note,
    ExchangeContact,
    CardDavContact,
}

const BOOK: &str = "https://dav.example.org/book/";

impl Kind {
    /// Where the server makes the item that was sent to it.
    fn href(self) -> String {
        match self {
            Kind::Task => "task:T1".into(),
            Kind::Note => "note:N1".into(),
            Kind::ExchangeContact => "contact:C1".into(),
            Kind::CardDavContact => format!("{BOOK}c1.vcf"),
        }
    }

    fn book(self) -> &'static str {
        if self == Kind::CardDavContact { BOOK } else { "ews:contacts" }
    }
}

/// Items as a sync carries them, of whichever kind is at hand.
#[derive(Clone, Default)]
struct Items {
    tasks: Vec<StoredTask>,
    notes: Vec<StoredNote>,
    cards: Vec<StoredCard>,
}

/// An account of the kind, its tasks, notes or contacts read once (none).
fn session(name: &str, kind: Kind) -> (Session, String) {
    let mut s = Session::open(Some(temp(name)), Clock::new(NOW, 0));
    let add = match kind {
        Kind::CardDavContact => json!({"email": "zoe@contoso.example", "password": "pw"}),
        _ => json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}),
    };
    let id = s.run("account.add", &add).unwrap()["account"].as_str().unwrap().to_string();
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(kind, "state-1", Items::default(), Items::default(), Vec::new())) });
    s.take_jobs();
    (s, id)
}

/// A sync's result: what it read of the folder or address book (up to `state`), what it made on
/// the server, and what it deleted there.
fn result(kind: Kind, state: &str, read: Items, made: Items, deleted: Vec<String>) -> DavSync {
    let mut sync = DavSync { checked: NOW, deleted, ..Default::default() };
    match kind {
        Kind::Task => (sync.tasks, sync.tasks_state, sync.pushed_tasks) = (Some(read.tasks), state.into(), made.tasks),
        Kind::Note => (sync.notes, sync.notes_state, sync.pushed_notes) = (Some(read.notes), state.into(), made.notes),
        Kind::ExchangeContact | Kind::CardDavContact => {
            let book = kind.book();
            sync.books = Some(vec![BookInfo { id: book.into(), name: "Contacts".into(), ..Default::default() }]);
            sync.synced_books = vec![book.into()];
            (sync.cards, sync.pushed_cards) = (read.cards, made.cards);
            if kind == Kind::CardDavContact {
                sync.carddav_home = Some("https://dav.example.org/".into());
            }
        }
    }
    sync
}

/// The account's calendars, tasks, notes and contacts as saved: what a sync that starts now reads.
fn as_saved(s: &Session, id: &str) -> Pim {
    s.store.as_ref().unwrap().load_account_doc(id, PIM_DOC).unwrap()
}

/// One made here: its name (uid).
fn create(s: &mut Session, id: &str, kind: Kind) -> String {
    let made = match kind {
        Kind::Task => s.run("task.create", &json!({"subject": "Send the budget", "account": id})),
        Kind::Note => s.run("note.create", &json!({"text": "Call Ana back", "account": id})),
        Kind::ExchangeContact | Kind::CardDavContact => {
            s.run("contact.create", &json!({"name": "Ana Lima", "emails": "ana@soundlab.example", "account": id}))
        }
    };
    made.unwrap()["uid"].as_str().unwrap().to_string()
}

/// What a sync that read `saved` does: each item never sent made on the server, and read back as
/// the server keeps it (named by its item there). (What it made, what it read.)
fn made(kind: Kind, saved: &Pim) -> (Items, Items) {
    let (href, etag) = (kind.href(), "ck-1".to_string());
    let (mut made, mut read) = (Items::default(), Items::default());
    for t in saved.tasks.iter().filter(|t| t.dirty && t.href.is_empty()) {
        made.tasks.push(StoredTask { href: href.clone(), etag: etag.clone(), dirty: false, ..t.clone() });
        let mut copy = t.task.clone();
        copy.uid.clone_from(&href);
        read.tasks.push(StoredTask { href: href.clone(), etag: etag.clone(), task: copy, dirty: false });
    }
    for n in saved.notes.iter().filter(|n| n.dirty && n.href.is_empty()) {
        made.notes.push(StoredNote { href: href.clone(), etag: etag.clone(), dirty: false, ..n.clone() });
        let mut copy = n.note.clone();
        copy.uid.clone_from(&href);
        read.notes.push(StoredNote { href: href.clone(), etag: etag.clone(), note: copy, dirty: false });
    }
    for c in saved.cards.iter().filter(|c| c.dirty && c.href.is_empty()) {
        made.cards.push(StoredCard { href: href.clone(), etag: etag.clone(), dirty: false, ..c.clone() });
        // Exchange names a contact by its item, a CardDAV server keeps the name it was sent.
        let mut copy = c.card.clone();
        if kind == Kind::ExchangeContact {
            copy.uid.clone_from(&href);
        }
        read.cards.push(StoredCard { book: kind.book().into(), href: href.clone(), etag: etag.clone(), card: copy, dirty: false });
    }
    (made, read)
}

/// How many the account keeps of what's made here.
fn kept(s: &Session, id: &str) -> usize {
    let p = &s.pim[id];
    p.tasks.len() + p.notes.len() + p.cards.len()
}

/// How many syncs of the account the session asked for since it was last looked at.
fn syncs(s: &mut Session, id: &str) -> usize {
    s.take_jobs().iter().filter(|j| matches!(j, Job::SyncDav { account } if account == id)).count()
}

fn delete(s: &mut Session, kind: Kind, uid: &str) {
    let cmd = match kind {
        Kind::Task => "task.delete",
        Kind::Note => "note.delete",
        Kind::ExchangeContact | Kind::CardDavContact => "contact.delete",
    };
    s.run(cmd, &json!({"uid": uid})).unwrap();
}

/// Made here, then deleted at once: the sync that made it on the server was on its way, and its
/// result brought it back (the server's copy, read back with the folder), where it stayed on the
/// server until deleted a second time. Held until the server says where it made it, it's deleted
/// there at the next sync as any other, and never comes back; nothing is made twice.
fn deleted_while_it_went_up(kind: Kind, name: &str) {
    let (mut s, id) = session(name, kind);
    let uid = create(&mut s, &id, kind);
    assert_eq!(syncs(&mut s, &id), 1, "{kind:?}: it goes to the server");
    // The sync starts: what was saved, made on the server and read back from it.
    let (made, read) = made(kind, &as_saved(&s, &id));
    // Deleted meanwhile.
    delete(&mut s, kind, &uid);
    assert_eq!(kept(&s, &id), 0);
    // The sync's result comes in.
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(kind, "state-2", read.clone(), made.clone(), Vec::new())) });
    assert_eq!(kept(&s, &id), 0, "{kind:?}: the one deleted here doesn't come back");
    // Nor with the result of another sync that read it before it was deleted.
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(kind, "state-2", read, made, Vec::new())) });
    assert_eq!(kept(&s, &id), 0, "{kind:?}: nor with a later result");
    // Deleted on the server by the next sync, as one deleted here is.
    assert_eq!(syncs(&mut s, &id), 1, "{kind:?}: a sync deletes it on the server");
    let saved = as_saved(&s, &id);
    assert_eq!(saved.deleted.iter().map(|t| t.href.as_str()).collect::<Vec<_>>(), [kind.href().as_str()], "{kind:?}: its deletion goes");
    assert!(saved.tasks.is_empty() && saved.notes.is_empty() && saved.cards.is_empty(), "{kind:?}: nothing made again");
    // That sync's result: deleted there, the folder read without it.
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(kind, "state-3", Items::default(), Items::default(), vec![kind.href()])) });
    assert_eq!(kept(&s, &id), 0, "{kind:?}");
    assert!(s.pim[&id].deleted.is_empty(), "{kind:?}: deleted once");
    assert!(s.pim[&id].held.is_empty(), "{kind:?}: nothing held any more");
    assert_eq!(syncs(&mut s, &id), 0, "{kind:?}: nothing more to send");
    done(s);
}

/// One already on the server, deleted while a sync that read the folder before was on its way:
/// that sync's result doesn't bring it back, and the next one deletes it there.
fn deleted_while_a_sync_read_it(kind: Kind, name: &str) {
    let (mut s, id) = session(name, kind);
    create(&mut s, &id, kind);
    let (made, read) = made(kind, &as_saved(&s, &id));
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(kind, "state-2", read.clone(), made, Vec::new())) });
    assert_eq!(kept(&s, &id), 1, "{kind:?}: made");
    let uid = {
        let p = &s.pim[&id];
        p.tasks
            .first()
            .map(|t| t.task.uid.clone())
            .or_else(|| p.notes.first().map(|n| n.note.uid.clone()))
            .or_else(|| p.cards.first().map(|c| c.card.uid.clone()))
            .unwrap()
    };
    syncs(&mut s, &id);
    delete(&mut s, kind, &uid);
    assert_eq!(syncs(&mut s, &id), 1, "{kind:?}: its deletion goes");
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(kind, "state-3", read, Items::default(), Vec::new())) });
    assert_eq!(kept(&s, &id), 0, "{kind:?}: not brought back by what the sync read before");
    assert_eq!(as_saved(&s, &id).deleted.iter().map(|t| t.href.as_str()).collect::<Vec<_>>(), [kind.href().as_str()], "{kind:?}: still to delete");
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(kind, "state-4", Items::default(), Items::default(), vec![kind.href()])) });
    assert!(kept(&s, &id) == 0 && s.pim[&id].deleted.is_empty(), "{kind:?}");
    done(s);
}

/// Deleted before any sync read it: it never goes, nothing comes back from the syncs that began
/// before or after, and nothing is deleted on the server.
fn deleted_before_it_went(kind: Kind, name: &str) {
    let (mut s, id) = session(name, kind);
    let uid = create(&mut s, &id, kind);
    delete(&mut s, kind, &uid);
    assert_eq!(syncs(&mut s, &id), 1, "{kind:?}: the sync asked for by what was made, still to start");
    let saved = as_saved(&s, &id);
    assert!(
        saved.tasks.is_empty() && saved.notes.is_empty() && saved.cards.is_empty() && saved.deleted.is_empty(),
        "{kind:?}: nothing for that sync"
    );
    for state in ["state-2", "state-3"] {
        s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(kind, state, Items::default(), Items::default(), Vec::new())) });
        assert!(kept(&s, &id) == 0 && s.pim[&id].deleted.is_empty(), "{kind:?}");
        assert_eq!(syncs(&mut s, &id), 0, "{kind:?}: nothing to send");
    }
    done(s);
}

#[test]
fn a_task_deleted_while_exchange_made_it_stays_deleted_and_goes_from_the_server() {
    deleted_while_it_went_up(Kind::Task, "deleting-flight-task");
}

#[test]
fn a_note_deleted_while_exchange_made_it_stays_deleted_and_goes_from_the_server() {
    deleted_while_it_went_up(Kind::Note, "deleting-flight-note");
}

#[test]
fn a_contact_deleted_while_exchange_made_it_stays_deleted_and_goes_from_the_server() {
    deleted_while_it_went_up(Kind::ExchangeContact, "deleting-flight-contact-ews");
}

#[test]
fn a_contact_deleted_while_carddav_made_it_stays_deleted_and_goes_from_the_server() {
    deleted_while_it_went_up(Kind::CardDavContact, "deleting-flight-contact-dav");
}

#[test]
fn a_task_exchange_has_deleted_while_a_sync_read_it_stays_deleted() {
    deleted_while_a_sync_read_it(Kind::Task, "deleting-read-task");
}

#[test]
fn a_note_exchange_has_deleted_while_a_sync_read_it_stays_deleted() {
    deleted_while_a_sync_read_it(Kind::Note, "deleting-read-note");
}

#[test]
fn a_contact_exchange_has_deleted_while_a_sync_read_it_stays_deleted() {
    deleted_while_a_sync_read_it(Kind::ExchangeContact, "deleting-read-contact-ews");
}

#[test]
fn a_contact_carddav_has_deleted_while_a_sync_read_it_stays_deleted() {
    deleted_while_a_sync_read_it(Kind::CardDavContact, "deleting-read-contact-dav");
}

#[test]
fn a_task_deleted_before_exchange_read_it_never_goes() {
    deleted_before_it_went(Kind::Task, "deleting-before-task");
}

#[test]
fn a_note_deleted_before_exchange_read_it_never_goes() {
    deleted_before_it_went(Kind::Note, "deleting-before-note");
}

#[test]
fn a_contact_deleted_before_exchange_read_it_never_goes() {
    deleted_before_it_went(Kind::ExchangeContact, "deleting-before-contact-ews");
}

#[test]
fn a_contact_deleted_before_carddav_read_it_never_goes() {
    deleted_before_it_went(Kind::CardDavContact, "deleting-before-contact-dav");
}
