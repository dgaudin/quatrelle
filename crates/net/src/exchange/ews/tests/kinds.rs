// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Tests of the EWS transport: what a message is, as Exchange says it (`item:ItemClass`, and
//! `PidTagIconIndex` the icon Outlook's list draws). New mail has it with its details; mail kept
//! before has it from one pass per folder, its ids and marks only, nothing downloaded.

use quatrelle_model::{MessageKind as K, Smime};

use super::*;
use crate::fakes::fake_ews::{self, Data, Server, Verb};

/// The Inbox: subject, class, icon (`PidTagIconIndex`, as Exchange writes it) and last verb.
const INBOX: [(&str, &str, Option<&str>, Option<&str>); 9] = [
    ("request", "IPM.Schedule.Meeting.Request", None, None),
    ("canceled", "IPM.Schedule.Meeting.Canceled", None, None),
    ("not delivered", "REPORT.IPM.Note.NDR", None, None),
    ("read", "REPORT.IPM.Note.IPNRN", None, None),
    ("encrypted", "IPM.Note.SMIME", None, None),
    ("away", "IPM.Note.Rules.OofTemplate.Microsoft", None, None),
    // 0x11B, an automatic reply by its icon alone.
    ("away by its icon", "IPM.Note", Some("283"), None),
    // Replied to all in Outlook: verb 103, the reply's icon (0x105).
    ("replied to all", "IPM.Note", Some("261"), Some("103")),
    ("plain", "IPM.Note", None, None),
];

/// What each of [`INBOX`] is, read with its details: its kind, what protects it, and whether it
/// was replied to all.
const READ: [(K, Smime, bool); 9] = [
    (K::MeetingRequest, Smime::None, false),
    (K::MeetingCanceled, Smime::None, false),
    (K::NotDelivered, Smime::None, false),
    (K::ReadReceipt, Smime::None, false),
    (K::Plain, Smime::Encrypted, false),
    (K::AutoReply, Smime::None, false),
    (K::AutoReply, Smime::None, false),
    (K::Plain, Smime::None, true),
    (K::Plain, Smime::None, false),
];

fn opts() -> SyncOptions {
    SyncOptions { prefetch: 0, ..Default::default() }
}

/// A mailbox holding [`INBOX`], `script`ed, its Inbox synced once (listed in full: nothing kept
/// yet): its server, its cache, the session and the numbering.
fn synced(name: &str, script: impl FnOnce(&mut Data)) -> (Server, Store, Client, State) {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    for (n, (subject, class, icon, verb)) in INBOX.iter().enumerate() {
        let raw = format!("From: a@x.example\r\nTo: riley@contoso.example\r\nSubject: {subject}\r\n\r\nx\r\n");
        let id = d.deliver(&inbox, raw.as_bytes(), true, &format!("2024-05-0{}T09:00:00Z", n + 1));
        if let Some(i) = d.items.iter_mut().find(|i| i.id == id) {
            i.class = (*class).to_string();
            let time = verb.map(|_| "2026-10-30T09:12:00Z".to_string());
            i.verb = Verb { last: verb.map(str::to_string), time, icon: icon.map(str::to_string) };
        }
    }
    script(&mut d);
    let server = fake_ews::serve(d);
    let store = crate::fake::tests_store(name);
    let user = format!("{}\\{}", fake_ews::DOMAIN, fake_ews::USER);
    let c = Client::connect(&server.url, &user, fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    list_folders(&c, &mut st, "a1").unwrap();
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts()).unwrap();
    (server, store, c, st)
}

/// The cache and the numbering as the version before left them: no kind, no protection, no reply
/// to all read, the pass not made.
fn as_before(store: &Store, st: &mut State) {
    let mut fs = store.load_folder("a1", "INBOX").unwrap();
    for m in fs.messages.iter_mut() {
        (m.kind, m.smime) = (K::Unknown, Smime::None);
        m.flags.set_keyword(quatrelle_model::REPLIED_ALL, false);
    }
    store.save_folder("a1", "INBOX", &fs).unwrap();
    st.kinds = Default::default();
}

/// What a message kept is: its kind, what protects it, and whether it was replied to all.
fn kept(store: &Store, subject: &str) -> Option<(K, Smime, bool)> {
    let fs = store.load_folder("a1", "INBOX").unwrap();
    fs.messages.iter().find(|m| m.subject == subject).map(|m| (m.kind, m.smime, m.flags.replied_all()))
}

/// A message comes in with what Exchange says it is: its class first, its icon for a note, the
/// protection its class says; one replied to all in Outlook, so.
#[test]
fn a_new_message_takes_its_kind_from_its_class_and_icon() {
    let (_server, store, _c, _st) = synced("ews-kinds-new", |_| {});
    for ((subject, ..), want) in INBOX.iter().zip(READ) {
        assert_eq!(kept(&store, subject), Some(want), "{subject}");
    }
    let _ = std::fs::remove_dir_all(store.root());
}

/// Mail kept by the version before gets what it is from one pass over its folder: the messages
/// that may be more than mail, or were replied to all, asked for alone, their ids and marks only,
/// a page at a time; nothing downloaded. Those Exchange says nothing special of wait for their
/// source. Done, the pass isn't made again, and an unchanged folder costs one look, as before.
#[test]
fn mail_kept_before_takes_its_kind_in_one_pass_without_its_source() {
    let (server, store, c, mut st) = synced("ews-kinds-kept", |_| {});
    as_before(&store, &mut st);
    let shapes = server.data.lock().unwrap().item_shapes.len();
    server.data.lock().unwrap().calls.clear();
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts()).unwrap();
    for (n, ((subject, ..), read)) in INBOX.iter().zip(READ).enumerate() {
        let want = match n {
            4 => (K::Unknown, Smime::Encrypted, false),
            6 | 8 => (K::Unknown, Smime::None, false),
            7 => (K::Unknown, Smime::None, true),
            _ => read,
        };
        assert_eq!(kept(&store, subject), Some(want), "{subject}");
    }
    let calls = server.data.lock().unwrap().calls.clone();
    assert_eq!(calls.iter().filter(|c| c.as_str() == "FindItem kinds").count(), 7_usize.div_ceil(2), "seven found, two a page: {calls:?}");
    assert!(calls.iter().all(|c| matches!(c.as_str(), "GetFolder" | "FindItem" | "FindItem kinds")), "nothing else asked: {calls:?}");
    assert!(!server.data.lock().unwrap().item_shapes[shapes..].iter().any(|s| s == "MimeContent"), "nothing downloaded");
    assert!(!st.kinds_wanted("INBOX"), "the pass done in this folder");

    server.data.lock().unwrap().calls.clear();
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts()).unwrap();
    assert_eq!(server.data.lock().unwrap().calls, ["GetFolder"], "not made again: an unchanged folder, one look");
    let _ = std::fs::remove_dir_all(store.root());
}

/// A server that won't search so has the pass noted done (the sources will tell), asked once.
#[test]
fn a_server_that_refuses_the_kinds_search_is_asked_once() {
    let (server, store, c, mut st) = synced("ews-kinds-refused", |d| d.kinds_refused = true);
    as_before(&store, &mut st);
    for _ in 0..2 {
        let r = sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts());
        assert!(r.is_ok(), "{r:?}");
    }
    let asked = server.data.lock().unwrap().calls.iter().filter(|c| c.as_str() == "FindItem kinds").count();
    assert_eq!(asked, 1, "asked once");
    assert!(!st.kinds_wanted("INBOX"), "noted done");
    let _ = std::fs::remove_dir_all(store.root());
}

/// The pass comes after the folder's own read: its new mail is in first, not kept waiting while
/// the folder is searched for what its mail kept before is.
#[test]
fn a_folder_is_read_before_it_is_asked_what_its_mail_is() {
    let (server, store, c, mut st) = synced("ews-kinds-after", |_| {});
    as_before(&store, &mut st);
    {
        let mut d = server.data.lock().unwrap();
        let inbox = d.by_name("Inbox").map(|f| f.id.clone()).unwrap();
        d.deliver(&inbox, b"From: b@y.example\r\nSubject: new\r\n\r\nx\r\n", false, "2024-06-01T09:00:00Z");
        d.calls.clear();
    }
    let r = sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts());
    assert!(r.as_ref().is_ok_and(|r| r.new == 1), "{r:?}");
    let calls = server.data.lock().unwrap().calls.clone();
    let at = |name: &str| calls.iter().position(|c| c == name);
    assert!(matches!((at("SyncFolderItems"), at("FindItem kinds")), (Some(read), Some(pass)) if read < pass), "its changes read first: {calls:?}");
    let _ = std::fs::remove_dir_all(store.root());
}

/// A big mailbox's folder whose pass is cut short (a timeout, a connection closed) is read all
/// the same, its new mail in; the pass, still to do, isn't asked again before Quatrelle next
/// starts, whether the server is well again or not, and is made then.
#[test]
fn a_kinds_pass_cut_short_doesnt_stop_the_folder_being_read() {
    let (server, store, c, mut st) = synced("ews-kinds-cut", |_| {});
    as_before(&store, &mut st);
    {
        let mut d = server.data.lock().unwrap();
        let inbox = d.by_name("Inbox").map(|f| f.id.clone()).unwrap();
        d.kinds_cut = true;
        d.deliver(&inbox, b"From: b@y.example\r\nSubject: new\r\n\r\nx\r\n", false, "2024-06-01T09:00:00Z");
    }
    let tried = || server.data.lock().unwrap().calls.iter().filter(|c| c.as_str() == "FindItem kinds cut").count();

    let r = sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts());
    assert!(r.as_ref().is_ok_and(|r| r.new == 1), "the folder read all the same: {r:?}");
    assert_eq!(kept(&store, "new"), Some((K::Plain, Smime::None, false)), "its new message in");
    assert!(st.kinds_wanted("INBOX"), "the pass still to do");
    let cut = tried();
    assert!(cut >= 1, "the pass was tried");

    // Not again in this run, the server well again or not.
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts()).unwrap();
    server.data.lock().unwrap().kinds_cut = false;
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts()).unwrap();
    let asked = server.data.lock().unwrap().calls.iter().filter(|c| c.as_str() == "FindItem kinds").count();
    assert_eq!((tried(), asked), (cut, 0), "asked once a run");
    assert_eq!(kept(&store, "request"), Some((K::Unknown, Smime::None, false)));

    // Quatrelle started again: the pass is made.
    st.save(&store, "a1").unwrap();
    let mut st = State::load(&store, "a1").unwrap();
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts()).unwrap();
    assert_eq!(kept(&store, "request"), Some((K::MeetingRequest, Smime::None, false)), "made at the next start");
    assert!(!st.kinds_wanted("INBOX"));
    let _ = std::fs::remove_dir_all(store.root());
}

/// A folder whose pass fails at every start (a big public folder whose search always times out)
/// has it tried at three starts, then given up there: never asked again, its mail kept before told
/// by its sources alone. The folder is read every time, its new mail in.
#[test]
fn a_kinds_pass_failing_at_every_start_is_given_up_at_the_third() {
    let (server, store, c, mut st) = synced("ews-kinds-given-up", |d| d.kinds_cut = true);
    as_before(&store, &mut st);
    let tried = || server.data.lock().unwrap().calls.iter().filter(|c| c.as_str() == "FindItem kinds cut").count();
    let mut at_start = Vec::new();
    for n in 1..=4 {
        // Quatrelle started again: the numbering read back from the cache.
        st.save(&store, "a1").unwrap();
        st = State::load(&store, "a1").unwrap();
        {
            let mut d = server.data.lock().unwrap();
            let inbox = d.by_name("Inbox").map(|f| f.id.clone()).unwrap();
            let raw = format!("From: b@y.example\r\nSubject: new {n}\r\n\r\nx\r\n");
            d.deliver(&inbox, raw.as_bytes(), false, &format!("2024-06-0{n}T09:00:00Z"));
        }
        let before = tried();
        let r = sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts());
        assert!(r.as_ref().is_ok_and(|r| r.new == 1), "read at start {n}: {r:?}");
        at_start.push(tried() > before);
    }
    assert_eq!(at_start, [true, true, true, false], "tried at three starts, then given up");
    assert!(!st.kinds_wanted("INBOX"), "noted done");
    assert_eq!(kept(&store, "request"), Some((K::Unknown, Smime::None, false)), "left to its source");
    let _ = std::fs::remove_dir_all(store.root());
}
