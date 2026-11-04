// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: who the sender check knows (the user's accounts, the contacts' cards, the people
//! written to, the directory's people), gathered once per change of what it's made of; the
//! message list's marks and the two Options settings that show them.

use quatrelle_model::lookalike::SenderWarning;
use quatrelle_model::{AccountConfig, Addressee, Folder};
use quatrelle_store::FolderState;

use super::*;

/// The account's id.
const ME: &str = "zoe";

/// A session with one Exchange account, Zoe Park's (`zoe@studio.example`), and an empty Inbox and
/// Sent: nobody else is known yet.
fn zoe_session(name: &str) -> Session {
    let store = temp(name);
    let config = AccountConfig {
        id: ME.into(),
        name: "Zoe Park".into(),
        email: "zoe@studio.example".into(),
        username: "zoe".into(),
        ews: Some("https://mail.studio.example/EWS/Exchange.asmx".into()),
        ..Default::default()
    };
    store.save_accounts(&[config]).unwrap();
    let folders: Vec<Folder> = [("INBOX", FolderRole::Inbox), ("Sent", FolderRole::Sent)]
        .into_iter()
        .map(|(path, role)| Folder { account: ME.into(), path: path.into(), role, selectable: true, ..Default::default() })
        .collect();
    for f in &folders {
        store.save_folder(ME, &f.path, &FolderState::<Summary> { validity: 1, uid_next: 1, ..Default::default() }).unwrap();
    }
    store.save_folders(ME, &folders).unwrap();
    let mut s = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    s
}

/// Put a message in Zoe's Inbox and open it in the reading pane.
fn deliver(s: &mut Session, n: u32, from: (&str, &str), subject: &str) -> MessageKey {
    let d = quatrelle_mime::Draft {
        to: vec![EmailAddress::new("Zoe Park", "zoe@studio.example")],
        subject: subject.into(),
        body: "Can you pay this invoice today? I'm in a meeting.".into(),
        ..Default::default()
    };
    let raw = quatrelle_mime::build(&d, &EmailAddress::new(from.0, from.1), NOW, &format!("<senders{n}@x>")).unwrap();
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder(ME, "INBOX").unwrap();
    let key = MessageKey { account: ME.into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9400 + n };
    inbox.messages.insert(0, quatrelle_mime::summary_from_full(&raw, key.clone(), quatrelle_model::Flags::default(), 0, NOW));
    store.save_folder(ME, "INBOX", &inbox).unwrap();
    store.write_body(&key, &raw).unwrap();
    s.apply(Event::FolderChanged { account: ME.into(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": ME, "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    key
}

fn warning(s: &Session) -> Option<SenderWarning> {
    s.reading.as_ref().and_then(|r| r.warning.clone())
}

fn card(s: &mut Session, name: &str, email: &str) {
    s.run("contact.create", &json!({"name": name, "emails": [email]})).unwrap();
}

fn directory_answers(s: &mut Session, query: &str, people: &[(&str, &str)]) {
    let people = people.iter().map(|(n, e)| Addressee::from(EmailAddress::new(*n, *e))).collect();
    s.apply(Event::DirectoryPeople { account: ME.into(), query: query.into(), people, more: false, error: None });
}

/// The list's row for `subject`, as `list.messages` gives it.
fn row(s: &mut Session, subject: &str) -> Value {
    let r = s.run("list.messages", &json!({"limit": 500})).unwrap();
    r["messages"].as_array().unwrap().iter().find(|m| m["subject"] == subject).cloned().unwrap_or_else(|| panic!("no {subject}"))
}

const ANA: (&str, &str) = ("Ana Lima", "ana.lima.1984@gmail.com");

fn ana_warned() -> Option<SenderWarning> {
    Some(SenderWarning::NameOfKnownPerson { name: "Ana Lima".into(), known: "ana@corp.example".into(), real: "ana.lima.1984@gmail.com".into() })
}

#[test]
fn known_people_come_from_accounts_cards_written_and_directory() {
    // A card in Contacts.
    let mut s = zoe_session("senders-card");
    card(&mut s, "Ana Lima", "ana@corp.example");
    deliver(&mut s, 1, ANA, "Invoice");
    assert_eq!(warning(&s), ana_warned(), "a card");
    done(s);
    // Someone written to.
    let mut s = zoe_session("senders-written");
    s.contacts_sent(&[EmailAddress::new("Ana Lima", "ana@corp.example")]);
    deliver(&mut s, 1, ANA, "Invoice");
    assert_eq!(warning(&s), ana_warned(), "written to");
    done(s);
    // Someone the directory answered for.
    let mut s = zoe_session("senders-directory");
    directory_answers(&mut s, "ana", &[("Ana Lima", "ana@corp.example")]);
    deliver(&mut s, 1, ANA, "Invoice");
    assert_eq!(warning(&s), ana_warned(), "the directory");
    done(s);
    // Nobody knows Ana: nothing to say. The user's own name on someone else's address is.
    let mut s = zoe_session("senders-none");
    deliver(&mut s, 1, ANA, "Invoice");
    assert_eq!(warning(&s), None, "a stranger");
    deliver(&mut s, 2, ("Zoe Park", "zoe.park@outlook.com"), "Me?");
    assert_eq!(
        warning(&s),
        Some(SenderWarning::NameOfKnownPerson { name: "Zoe Park".into(), known: "zoe@studio.example".into(), real: "zoe.park@outlook.com".into() })
    );
    // Her own mail is never doubted.
    deliver(&mut s, 3, ("Zoe Park", "zoe@studio.example"), "Mine");
    assert_eq!(warning(&s), None);
    done(s);
}

#[test]
fn saving_the_sender_as_a_contact_takes_the_warning_away() {
    let mut s = zoe_session("senders-saved");
    card(&mut s, "Ana Lima", "ana@corp.example");
    let key = deliver(&mut s, 1, ANA, "Invoice");
    s.run("message.popOut", &json!({"key": key.to_id()})).unwrap();
    assert_eq!(warning(&s), ana_warned());
    assert!(s.windows.iter().any(|w| w.key == key && w.warning == ana_warned()));
    // Ana really writes from Gmail too: saved, the open message and its window say nothing.
    card(&mut s, "Ana Lima", "ana.lima.1984@gmail.com");
    s.keep_senders();
    assert_eq!(warning(&s), None);
    assert!(s.windows.iter().any(|w| w.key == key && w.warning.is_none()));
    done(s);
}

#[test]
fn marks_follow_the_options() {
    let mut s = zoe_session("senders-marks");
    card(&mut s, "Ana Lima", "ana@studio.example");
    // A colleague, known from the directory.
    directory_answers(&mut s, "sam", &[("Sam Okafor", "sam@studio.example")]);
    deliver(&mut s, 1, ("Ana Lima", "ana@studio.example"), "From a contact");
    deliver(&mut s, 2, ("Pat Doe", "pat.doe@gmail.com"), "From a stranger");
    deliver(&mut s, 3, ("Sam Okafor", "sam@studio.example"), "From a colleague");
    deliver(&mut s, 4, ("Zoe Park", "zoe@studio.example"), "From me");
    let subjects = ["From a contact", "From a stranger", "From a colleague", "From me"];
    let marks = |s: &mut Session, field: &str| -> Vec<bool> { subjects.iter().map(|m| row(s, m)[field].as_bool().unwrap()).collect() };
    // At first: names only, as Outlook, and no External.
    assert_eq!(marks(&mut s, "fromAddress"), [false, false, false, false]);
    assert_eq!(marks(&mut s, "external"), [false, false, false, false]);
    s.run("view.senderAddresses", &json!({"show": "strangers"})).unwrap();
    assert_eq!(marks(&mut s, "fromAddress"), [false, true, false, false]);
    let r = s.run("view.senderAddresses", &json!({"show": "always"})).unwrap();
    assert_eq!(r["show"], "always");
    assert_eq!(marks(&mut s, "fromAddress"), [true, true, true, false], "never on the user's own mail");
    // External: only once it's on, and only from outside the organisation.
    let r = s.run("view.externalTag", &json!({"on": true})).unwrap();
    assert_eq!(r["on"], true);
    assert_eq!(r["organisation"], json!(["studio.example"]));
    assert_eq!(marks(&mut s, "external"), [false, true, false, false]);
    let r = s.run("view.externalTag", &json!({"domains": "corp.example, studio.example"})).unwrap();
    assert_eq!(r["domains"], json!(["corp.example", "studio.example"]));
    assert_eq!(r["organisation"], json!(["corp.example", "studio.example"]));
    assert_eq!(marks(&mut s, "external"), [false, true, false, false]);
    // What isn't a setting is refused, and leaves them as they were.
    for bad in [json!({"show": "sometimes"}), json!({})] {
        assert!(matches!(s.run("view.senderAddresses", &bad), Err(CmdError::Params(_))), "{bad}");
    }
    let many: Vec<String> = (0..101).map(|i| format!("d{i}.example")).collect();
    for bad in [json!({"domains": 5}), json!({"domains": [1]}), json!({"domains": "not a domain!"}), json!({"domains": many})] {
        assert!(matches!(s.run("view.externalTag", &bad), Err(CmdError::Params(_))), "{bad}");
    }
    assert_eq!(s.prefs.sender_addresses, crate::SenderAddresses::Always);
    assert_eq!(s.prefs.org_domains, ["corp.example", "studio.example"]);
    // An empty list goes back to the accounts' domains.
    let r = s.run("view.externalTag", &json!({"domains": []})).unwrap();
    assert_eq!(r["organisation"], json!(["studio.example"]));
    done(s);
}

#[test]
fn list_and_read_say_the_marks() {
    let mut s = zoe_session("senders-commands");
    card(&mut s, "Ana Lima", "ana@corp.example");
    deliver(&mut s, 1, ANA, "Invoice");
    let r = row(&mut s, "Invoice");
    assert_eq!((&r["fromAddress"], &r["senderWarning"], &r["external"]), (&json!(false), &json!(true), &json!(false)));
    let read = s.run("message.read", &json!({})).unwrap();
    assert_eq!(
        read["warning"],
        json!({"kind": "nameOfKnownPerson", "name": "Ana Lima", "known": "ana@corp.example", "real": "ana.lima.1984@gmail.com"})
    );
    assert_eq!(read["external"], json!(false));
    s.run("view.externalTag", &json!({"on": true})).unwrap();
    s.run("view.senderAddresses", &json!({"show": "strangers"})).unwrap();
    let r = row(&mut s, "Invoice");
    assert_eq!((&r["fromAddress"], &r["senderWarning"], &r["external"]), (&json!(true), &json!(true), &json!(true)));
    assert_eq!(s.run("message.read", &json!({})).unwrap()["external"], json!(true));
    done(s);
}

/// Review Focus 5: the gathering run each frame never loads the address book, and waits while
/// it's being read.
#[test]
fn the_quiet_gathering_waits_for_the_book() {
    let mut s = zoe_session("senders-quiet");
    assert!(s.contacts_book().is_none());
    s.keep_senders_quietly();
    assert!(s.senders.is_none(), "no book: nothing gathered");
    assert!(s.contacts_book().is_none(), "and none loaded");
    s.contacts_step(usize::MAX);
    assert!(!s.contacts_pending());
    s.keep_senders_quietly();
    assert!(s.senders.is_some(), "the book read: gathered");
    // The stamp changes (a card saved) while a folder waits to be read into the book.
    card(&mut s, "Ana Lima", "ana@corp.example");
    s.contacts_folder_changed(ME, "INBOX");
    assert!(s.contacts_pending());
    s.keep_senders_quietly();
    assert!(!s.senders.as_ref().unwrap().check.people().knows_address("ana@corp.example"), "not while the book is read");
    s.contacts_step(usize::MAX);
    assert!(!s.contacts_pending());
    s.keep_senders_quietly();
    assert!(s.senders.as_ref().unwrap().check.people().knows_address("ana@corp.example"), "once it's read");
    done(s);
}

/// A name made of letters only, one per `i` (names with digits aren't people's).
fn letters(mut i: usize) -> String {
    let mut w = String::new();
    for _ in 0..4 {
        w.push(char::from(b'a' + u8::try_from(i % 26).unwrap()));
        i /= 26;
    }
    w
}

#[test]
fn gathering_is_quick() {
    let mut s = zoe_session("senders-quick");
    let written: Vec<EmailAddress> =
        (0..20_000).map(|i| EmailAddress::new(format!("Wren {}", letters(i)), format!("w{i}@written.example"))).collect();
    s.contacts_sent(&written);
    let people: Vec<(String, String)> = (0..30_000).map(|i| (format!("Dale {}", letters(i)), format!("d{i}@corp.example"))).collect();
    let people: Vec<(&str, &str)> = people.iter().map(|(n, e)| (n.as_str(), e.as_str())).collect();
    directory_answers(&mut s, "everyone", &people);
    s.keep_senders();
    assert!(s.senders.as_ref().unwrap().check.people().len() >= 49_000);
    // The best of three, as the calendar search's: a busy machine slows a run down now and then.
    let mut took = std::time::Duration::MAX;
    for n in 0..3 {
        card(&mut s, &format!("Card {}", letters(n)), &format!("card{n}@cards.example"));
        let started = std::time::Instant::now();
        s.keep_senders();
        took = took.min(started.elapsed());
        assert!(s.senders.as_ref().unwrap().check.knows(&format!("card{n}@cards.example")), "gathered again");
    }
    eprintln!("the sender check over 20,000 people written to and 30,000 of the directory: {took:?} (the best of three)");
    // The spec's 50 ms is for a release build; a debug one gets 500.
    assert!(took < std::time::Duration::from_millis(500), "{took:?}");
    // The organisation's domains typed in Options, a key at a time: the people aren't gathered
    // again for them.
    let started = std::time::Instant::now();
    for typed in ["c", "co", "corp.example", "corp.example, studio.example"] {
        s.run("view.externalTag", &json!({"domains": typed.split(", ").filter(|d| d.contains('.')).collect::<Vec<_>>()})).unwrap();
    }
    let took = started.elapsed();
    assert_eq!(s.organisation(), Some(&["corp.example", "studio.example"].map(String::from).into()));
    assert!(took < std::time::Duration::from_millis(100), "the domains alone: {took:?}");
    done(s);
}
