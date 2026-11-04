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
    zoe_session_on(name, true)
}

/// [`zoe_session`], the account on Exchange or (`exchange` false) on IMAP.
fn zoe_session_on(name: &str, exchange: bool) -> Session {
    let store = temp(name);
    let config = AccountConfig {
        id: ME.into(),
        name: "Zoe Park".into(),
        email: "zoe@studio.example".into(),
        username: "zoe".into(),
        ews: exchange.then(|| "https://mail.studio.example/EWS/Exchange.asmx".into()),
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
    deliver_raw(s, n, &raw)
}

/// Put a message's source in Zoe's Inbox and open it in the reading pane.
fn deliver_raw(s: &mut Session, n: u32, raw: &[u8]) -> MessageKey {
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder(ME, "INBOX").unwrap();
    let key = MessageKey { account: ME.into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9400 + n };
    inbox.messages.insert(0, quatrelle_mime::summary_from_full(raw, key.clone(), quatrelle_model::Flags::default(), 0, NOW));
    store.save_folder(ME, "INBOX", &inbox).unwrap();
    store.write_body(&key, raw).unwrap();
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

/// The list's marks come without any message opened: with no book, the window's wish starts its
/// reading (nothing read then, nothing gathered), its slices read it, and the quiet gathering then
/// marks the warned row. A second wish, once gathered, starts nothing.
#[test]
fn the_marks_come_without_opening_a_message() {
    let mut s = zoe_session("senders-unopened");
    let d = quatrelle_mime::Draft { subject: "Quick favour".into(), body: "Are you at your desk?".into(), ..Default::default() };
    let raw = quatrelle_mime::build(&d, &EmailAddress::new("Zoe Park", "zoe.park.director@gmail.com"), NOW, "<unopened@x>").unwrap();
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder(ME, "INBOX").unwrap();
    let key = MessageKey { account: ME.into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9500 };
    inbox.messages.insert(0, quatrelle_mime::summary_from_full(&raw, key, quatrelle_model::Flags::default(), 0, NOW));
    store.save_folder(ME, "INBOX", &inbox).unwrap();
    s.apply(Event::FolderChanged { account: ME.into(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": ME, "folder": "INBOX"})).unwrap();
    assert!(s.contacts_book().is_none() && s.senders.is_none() && s.reading.is_none());
    s.want_senders();
    assert!(s.contacts_book().is_some() && s.contacts_pending(), "started, not read");
    assert!(s.senders.is_none());
    for _ in 0..100 {
        if s.contacts_step(1) {
            break;
        }
        s.keep_senders_quietly();
        assert!(s.senders.is_none(), "not while the book is read");
    }
    s.keep_senders_quietly();
    let m = s.list.iter().find(|m| m.subject == "Quick favour").cloned().unwrap();
    assert!(s.sender_marks(&m).warning, "the user's own name from another address");
    assert!(s.reading.is_none(), "nothing opened");
    let revision = s.contacts_revision();
    s.want_senders();
    assert!(!s.contacts_pending() && s.contacts_revision() == revision, "nothing started again");
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

/// Final review n° 4: a calendar saved or synced moves the PIM revision too; the people are
/// gathered again only when the cards changed (one added, renamed), not for an appointment.
#[test]
fn a_calendar_change_doesnt_gather_the_people_again() {
    let mut s = zoe_session("senders-calendar");
    let uid = s.run("contact.create", &json!({"name": "Ana Lima", "emails": ["ana@corp.example"]})).unwrap()["uid"].as_str().unwrap().to_string();
    s.keep_senders();
    let gathered = |s: &Session| s.senders.as_ref().unwrap().gathered;
    let first = gathered(&s);
    let pim = s.pim_revision;
    s.run("event.create", &json!({"summary": "Review", "start": "2026-11-05T11:00", "end": "2026-11-05T12:00"})).unwrap();
    assert_ne!(s.pim_revision, pim, "the appointment moved the revision");
    s.keep_senders();
    assert_eq!(gathered(&s), first, "an appointment: not gathered again");
    // A card added: gathered.
    card(&mut s, "Bo Chen", "bo@corp.example");
    s.keep_senders();
    assert_eq!(gathered(&s), first + 1);
    assert!(s.senders.as_ref().unwrap().check.knows("bo@corp.example"));
    // A card renamed: gathered, the warning calling her by her new name.
    s.run("contact.update", &json!({"uid": uid, "name": "Ana Costa"})).unwrap();
    s.keep_senders();
    assert_eq!(gathered(&s), first + 2);
    assert!(s.senders.as_ref().unwrap().check.people().addresses_of("Ana Costa").is_some());
    // And another appointment after it: not again.
    s.run("event.create", &json!({"summary": "Lunch", "start": "2026-11-06T12:00", "end": "2026-11-06T13:00"})).unwrap();
    s.keep_senders();
    assert_eq!(gathered(&s), first + 2);
    done(s);
}

/// Ruling 15: someone known only from the directory is worn on another address only from a mail
/// service (or a look-alike of their domain): a namesake at a customer says nothing. Known by a
/// card, the whole rule.
#[test]
fn a_directory_namesake_at_a_customer_isnt_warned() {
    let mut s = zoe_session("senders-namesake");
    directory_answers(&mut s, "jean", &[("Jean Martin", "jean.martin@corp.example")]);
    deliver(&mut s, 1, ("Jean Martin", "jean.martin@client.example"), "Order");
    assert_eq!(warning(&s), None, "a customer called Jean Martin");
    deliver(&mut s, 2, ("Jean Martin", "jean.martin.1984@gmail.com"), "Urgent");
    let warned = |real: &str| {
        Some(SenderWarning::NameOfKnownPerson { name: "Jean Martin".into(), known: "jean.martin@corp.example".into(), real: real.into() })
    };
    assert_eq!(warning(&s), warned("jean.martin.1984@gmail.com"));
    done(s);
    let mut s = zoe_session("senders-namesake-card");
    card(&mut s, "Jean Martin", "jean.martin@corp.example");
    deliver(&mut s, 1, ("Jean Martin", "jean.martin@client.example"), "Order");
    assert_eq!(warning(&s), warned("jean.martin@client.example"), "known by a card: as before");
    done(s);
}

/// The names the directory is being asked about, in order (the session's other jobs dropped).
fn asked(s: &mut Session) -> Vec<String> {
    s.take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            Job::SearchDirectory { query, .. } => Some(query),
            _ => None,
        })
        .collect()
}

/// A message's source as a server gives it: `headers` added, the body of type `kind`.
fn source(n: u32, from: &str, headers: &str, kind: &str, body: &str) -> Vec<u8> {
    format!(
        "From: {from}\r\nTo: Zoe Park <zoe@studio.example>\r\nSubject: Message {n}\r\nDate: Mon, 03 Nov 2026 11:00:00 +0000\r\n\
         Message-ID: <question{n}@x.example>\r\n{headers}MIME-Version: 1.0\r\nContent-Type: {kind}; charset=utf-8\r\n\
         Content-Transfer-Encoding: 8bit\r\n\r\n{body}\r\n"
    )
    .into_bytes()
}

const DANA: (&str, &str) = ("Dr. Dana Whitfield, CEO", "dw.ceo@gmail.com");

#[test]
fn an_unknown_persons_name_is_asked_once() {
    let mut s = zoe_session("question-once");
    let key = deliver(&mut s, 1, DANA, "Wire");
    assert_eq!(asked(&mut s), ["dana whitfield"], "the name as a person would type it");
    // Opened again, in the pane or a window: not asked again.
    deliver(&mut s, 2, ("Zoe Park", "zoe@studio.example"), "Mine");
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    s.run("message.popOut", &json!({"key": key.to_id()})).unwrap();
    assert!(asked(&mut s).is_empty());
    // Answered, nor then.
    directory_answers(&mut s, "dana whitfield", &[]);
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    assert!(asked(&mut s).is_empty());
    done(s);
}

#[test]
fn no_question_when_known_or_no_person_or_bulk_or_no_exchange() {
    let mut s = zoe_session("question-none");
    card(&mut s, "Dana Whitfield", "dana@corp.example");
    asked(&mut s);
    deliver(&mut s, 1, ("Dana Whitfield", "dana@corp.example"), "Known");
    deliver(&mut s, 2, ("IT Support", "helpdesk@gmail.com"), "A team");
    deliver(&mut s, 3, ("Dana", "dana.w@gmail.com"), "One word");
    let list = source(4, "Ravi Menon <news@corp-news.example>", "List-Id: <weekly.corp-news.example>\r\n", "text/plain", "This week.");
    deliver_raw(&mut s, 4, &list);
    assert!(asked(&mut s).is_empty());
    for name in ["dana whitfield", "it support", "dana", "ravi menon"] {
        assert!(!s.directory_searching(ME, name), "{name}");
    }
    done(s);
    // No Exchange account: no directory to ask.
    let mut s = zoe_session_on("question-imap", false);
    deliver(&mut s, 1, DANA, "Wire");
    assert!(asked(&mut s).is_empty());
    done(s);
}

#[test]
fn the_answer_brings_the_warning() {
    let mut s = zoe_session("question-answer");
    deliver(&mut s, 1, DANA, "Wire");
    assert_eq!(warning(&s), None, "nobody knows Dana yet");
    assert_eq!(asked(&mut s), ["dana whitfield"]);
    directory_answers(&mut s, "dana whitfield", &[("Dana Whitfield", "dana@corp.example")]);
    assert_eq!(
        warning(&s),
        Some(SenderWarning::NameOfKnownPerson { name: "Dana Whitfield".into(), known: "dana@corp.example".into(), real: "dw.ceo@gmail.com".into() })
    );
    assert!(asked(&mut s).is_empty(), "the answer asks nothing more");
    done(s);
    // The directory couldn't be asked: logged, and nothing shown.
    let mut s = zoe_session("question-offline");
    deliver(&mut s, 1, DANA, "Wire");
    assert_eq!(asked(&mut s), ["dana whitfield"]);
    s.apply(Event::DirectoryPeople {
        account: ME.into(),
        query: "dana whitfield".into(),
        people: Vec::new(),
        more: false,
        error: Some("offline".into()),
    });
    assert_eq!(warning(&s), None);
    assert!(asked(&mut s).is_empty());
    done(s);
}

/// Final review n° 1: the directory is asked whether or not something was found. A colleague at
/// a sister domain, the organisation's site in her signature, is warned until the directory
/// says it knows her address; a Gmail fake of a known name is asked about too, and stays warned.
#[test]
fn the_directory_is_asked_with_a_warning_shown_too() {
    let mut s = zoe_session("question-warned");
    let body = "Here's the plan for Monday.\r\n\r\nDana Whitfield\r\nwww.studio.example";
    deliver_raw(&mut s, 1, &source(1, "Dana Whitfield <dana@studio-group.example>", "", "text/plain", body));
    assert_eq!(warning(&s), Some(SenderWarning::SignatureShowsOther { shown: "studio.example".into(), real: "dana@studio-group.example".into() }));
    assert_eq!(asked(&mut s), ["dana whitfield"]);
    directory_answers(&mut s, "dana whitfield", &[("Dana Whitfield", "dana@studio-group.example")]);
    assert_eq!(warning(&s), None, "a colleague after all");
    done(s);
    let mut s = zoe_session("question-fake");
    card(&mut s, "Dana Whitfield", "dana@studio.example");
    asked(&mut s);
    deliver(&mut s, 1, ("Dana Whitfield", "dana.whitfield1984@gmail.com"), "Wire");
    let warned = Some(SenderWarning::NameOfKnownPerson {
        name: "Dana Whitfield".into(),
        known: "dana@studio.example".into(),
        real: "dana.whitfield1984@gmail.com".into(),
    });
    assert_eq!(warning(&s), warned);
    assert_eq!(asked(&mut s), ["dana whitfield"]);
    directory_answers(&mut s, "dana whitfield", &[("Dana Whitfield", "dana@studio.example")]);
    assert_eq!(warning(&s), warned, "the directory knows her real address, not this one");
    done(s);
}

/// Review Focus 2: arrowing down through 20 messages from strangers, at most one sender's
/// question waits (the last one's), and Check Names' questions aren't held behind it.
#[test]
fn strangers_in_a_row_ask_for_the_last() {
    const SURNAMES: [&str; 20] = [
        "Abbott", "Barlow", "Carver", "Dalton", "Ellery", "Fenwick", "Garrow", "Hollis", "Ingram", "Jessop", "Kenning", "Lowther", "Marlow",
        "Norris", "Oakley", "Pruitt", "Quigley", "Rowan", "Sutton", "Thorne",
    ];
    let mut s = zoe_session("question-strangers");
    // Check Names asks about one name while another waits.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "reseau; dupont"})).unwrap();
    s.run("compose.checkNames", &json!({})).unwrap();
    assert_eq!(asked(&mut s), ["reseau"]);
    let names: Vec<String> = SURNAMES.iter().map(|n| format!("Pat {n}")).collect();
    for (n, name) in (1..).zip(&names) {
        deliver(&mut s, n, (name, &format!("pat.{}@gmail.com", name.to_lowercase().replace(' ', "."))), &format!("From {name}"));
        assert!(asked(&mut s).is_empty(), "{name}: the account is being asked about another name");
    }
    for name in names.iter().take(19) {
        assert!(!s.directory_searching(ME, name), "{name}: gave its place to the next");
    }
    assert!(s.directory_searching(ME, "Pat Thorne"), "the last one opened waits");
    assert!(s.directory_searching(ME, "dupont"), "Check Names' question still waits");
    // Check Names' questions go first, even one asked after the sender's.
    directory_answers(&mut s, "reseau", &[]);
    assert_eq!(asked(&mut s), ["dupont"]);
    s.run("compose.update", &json!({"to": "garcia"})).unwrap();
    s.run("compose.checkNames", &json!({})).unwrap();
    directory_answers(&mut s, "dupont", &[]);
    assert_eq!(asked(&mut s), ["garcia"]);
    directory_answers(&mut s, "garcia", &[]);
    assert_eq!(asked(&mut s), ["pat thorne"]);
    done(s);
}

#[test]
fn the_signature_warns_in_the_pane_and_a_window() {
    let mut s = zoe_session("signature-warns");
    let body = "Please wire the deposit today, I'm boarding.\r\n\r\nDirection\r\nwww.studio.example";
    let key = deliver_raw(&mut s, 1, &source(1, "Direction <dg.studio@gmail.com>", "", "text/plain", body));
    let warned = Some(SenderWarning::SignatureShowsOther { shown: "studio.example".into(), real: "dg.studio@gmail.com".into() });
    assert_eq!(warning(&s), warned);
    s.run("message.popOut", &json!({"key": key.to_id()})).unwrap();
    assert!(s.windows.iter().any(|w| w.key == key && w.warning == warned));
    done(s);
}

/// Review Focus 1: an outsider's reply quoting the user's own message, her signature in the
/// quote, says nothing, as Outlook (a bordered block or a rule) and Gmail lay it out.
#[test]
fn an_outlook_reply_quoting_the_user_doesnt_warn() {
    let mine = "<p>Hello Bo</p><p>Zoe Park<br>www.studio.example<br>zoe@studio.example</p>";
    let headers = "<p><b>De :</b> Zoe Park &lt;zoe@studio.example&gt;<br><b>Envoyé :</b> lundi 3 novembre 2026 10:00<br><b>À :</b> Bo Chen</p>";
    let replies = [
        format!("<p>Thanks, done.</p><p>Bo</p><div style=\"border:none;border-top:solid #E1E1E1 1.0pt\">{headers}</div>{mine}"),
        format!("<p>Thanks, done.</p><p>Bo</p><hr>{headers}{mine}"),
        format!(
            "<div dir=\"ltr\"><p>Thanks, done.</p><p>Bo</p></div><div class=\"gmail_quote\"><div dir=\"ltr\" class=\"gmail_attr\">Le lun. 3 nov. 2026 \
             à 10:00, Zoe Park &lt;<a href=\"mailto:zoe@studio.example\">zoe@studio.example</a>&gt; a écrit :<br></div>\
             <blockquote class=\"gmail_quote\" style=\"margin:0px 0px 0px 0.8ex;border-left:1px solid rgb(204,204,204);padding-left:1ex\">{mine}</blockquote></div>"
        ),
    ];
    let mut s = zoe_session("signature-reply");
    for (n, html) in (1..).zip(&replies) {
        deliver_raw(&mut s, n, &source(n, "Bo Chen <bo@supplier.example>", "", "text/html", &format!("<html><body>{html}</body></html>")));
        let text = s.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.text.clone()).unwrap_or_default();
        assert_eq!(warning(&s), None, "{n}: {text:?}");
    }
    done(s);
}

/// An HTML message's links are noted at the end of its text: a supplier's link to the user's
/// portal, in a sentence, isn't their signature showing her organisation.
#[test]
fn a_link_in_the_message_isnt_the_signature() {
    let mut s = zoe_session("signature-link");
    let html = "<html><body><p>Hello Zoe, the new prices are on <a href=\"https://portal.studio.example/prices\">your portal</a>.</p>\
                <p>Bo Chen<br>Supplier Ltd</p></body></html>";
    deliver_raw(&mut s, 1, &source(1, "Bo Chen <bo@supplier.example>", "", "text/html", html));
    assert_eq!(warning(&s), None);
    done(s);
}
