// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Tests: an import that is counted, stopped and gone on with, from its journal.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use proptest::prelude::*;
use quatrelle_mime as mime;
use quatrelle_model::pim::{Note, PIM_DOC, Pim, StoredCard, StoredEvent, StoredNote};
use quatrelle_model::{
    EmailAddress, Flags, Folder, FolderRole, IMPORT_DOC, IMPORT_FILE_CHANGED, IMPORT_NO_JOURNAL, ImportJournal, MessageKey, Summary,
};
use quatrelle_store::{FolderState, PAGE_MESSAGES, Store};

use super::super::{Imported, Progress, Run, import, import_run};
use super::open::data_file;

/// The messages of the mailbox below.
const MAIL: usize = 24;

/// A data file of three mail folders (one inside another), three contacts, two calendar items
/// and a note, written by the export.
fn mailbox(name: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-resume-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::memory();
    let mut folders = Vec::new();
    for (path, role, n) in [("INBOX", FolderRole::Inbox, 12u32), ("INBOX/Projects", FolderRole::Other, 7), ("Sent Items", FolderRole::Sent, 5)] {
        let messages: Vec<Summary> = (1..=n)
            .map(|uid| {
                let d = mime::Draft {
                    to: vec![EmailAddress::new("Ana", "ana@studio.example")],
                    subject: format!("{path} #{uid}"),
                    body: format!("Notes for take {uid}."),
                    ..Default::default()
                };
                let from = EmailAddress::new("Riley", "riley@studio.example");
                let raw = mime::build(&d, &from, 1_704_067_200 + i64::from(uid) * 60, &format!("{}.{uid}@studio.example", path.len())).unwrap();
                let key = MessageKey { account: "a1".into(), folder: path.into(), validity: 1, uid };
                store.write_body(&key, &raw).unwrap();
                mime::summary_from_full(&raw, key, Flags { seen: uid % 3 == 0, ..Default::default() }, 10, 0)
            })
            .collect();
        store.save_folder("a1", path, &FolderState { validity: 1, uid_next: n + 1, messages, ..Default::default() }).unwrap();
        folders.push(Folder { account: "a1".into(), path: path.into(), delimiter: Some('/'), role, selectable: true, ..Default::default() });
    }
    store.save_folders("a1", &folders).unwrap();
    let mut pim = Pim::default();
    for (k, name) in ["Ana Lima", "Ben Okoro", "Cy Tran"].iter().enumerate() {
        let card = quatrelle_model::vcard::Card { uid: format!("c{k}"), name: (*name).into(), ..Default::default() };
        pim.cards.push(StoredCard { card, ..Default::default() });
    }
    for (k, what) in ["Screening", "Mix review"].iter().enumerate() {
        let start = 1_792_000_000 + k as i64 * 86_400;
        let event = quatrelle_model::ical::CalEvent { uid: format!("e{k}"), summary: (*what).into(), start, end: start + 3600, ..Default::default() };
        pim.events.push(StoredEvent { event, ..Default::default() });
    }
    let note = Note { uid: "n0".into(), text: "Ask about the poster".into(), modified: 1_700_000_500, ..Default::default() };
    pim.notes.push(StoredNote { note, ..Default::default() });
    store.save_account_doc("a1", PIM_DOC, &pim).unwrap();
    let file = dir.join("Studio.pst");
    crate::pst_export::export(&store, "a1", "Studio mail", &file, &Default::default(), &mut |_| {}).unwrap();
    (dir, file)
}

/// Import `file` into `account`, stopped once `stop_after` items were let through (`None`:
/// never), going on from the journal when `resume`.
fn run(file: &Path, store: &Store, account: &str, stop_after: Option<usize>, resume: bool) -> Result<Imported, String> {
    let asked = Cell::new(0usize);
    let stop = || {
        asked.set(asked.get() + 1);
        stop_after.is_some_and(|n| asked.get() > n)
    };
    import_run(file, store, account, Run { stop: Some(&stop), resume, allow_changed: false }, &mut |_| {})
}

/// Everything an account holds after an import: its folders with their counts, every list in
/// full, every message's source, and its contacts, calendar and notes.
type Held = (Vec<Folder>, Vec<(String, u32, Vec<Summary>, Vec<Option<Vec<u8>>>)>, String);

fn held(store: &Store, account: &str) -> Held {
    let folders = store.load_folders(account).unwrap();
    let lists = folders
        .iter()
        .map(|f| {
            let st = store.load_folder(account, &f.path).unwrap();
            let sources = st.messages.iter().map(|m| store.read_body(&m.key)).collect();
            (f.path.clone(), st.uid_next, st.messages, sources)
        })
        .collect();
    let pim: Pim = store.load_account_doc(account, PIM_DOC).unwrap();
    (folders, lists, serde_json::to_string(&pim).unwrap())
}

fn journal(store: &Store, account: &str) -> ImportJournal {
    store.load_account_doc(account, IMPORT_DOC).unwrap()
}

/// The mailbox, and what importing it in one go gives.
static WHOLE: LazyLock<(PathBuf, Imported, Held)> = LazyLock::new(|| {
    let (_, file) = mailbox("whole");
    let store = Store::memory();
    let done = run(&file, &store, "b1", None, false).unwrap();
    let all = held(&store, "b1");
    (file, done, all)
});

/// The messages of the folders that are imported are counted from the file's folders before the
/// first comes in, and told with the folder being read.
#[test]
fn the_messages_are_counted_before_the_first() {
    let (file, done, _) = &*WHOLE;
    assert_eq!((done.messages, done.total, done.stopped), (MAIL, Some(MAIL), false), "{done:?}");
    let mut told: Vec<(usize, Option<usize>, String)> = Vec::new();
    let store = Store::memory();
    import_run(file, &store, "b1", Run::default(), &mut |p: &Progress<'_>| told.push((p.messages, p.total, p.folder.to_string()))).unwrap();
    assert_eq!(told.first(), Some(&(0, Some(MAIL), String::new())), "the total, before any folder");
    let folders: Vec<&str> = told.iter().skip(1).map(|t| t.2.as_str()).collect();
    assert!(["INBOX", "INBOX/Projects", "Sent Items"].iter().all(|f| folders.contains(f)), "{told:?}");
    assert!(told.iter().all(|t| t.1 == Some(MAIL)) && told.windows(2).all(|w| w[0].0 <= w[1].0), "{told:?}");
    let j = journal(&store, "b1");
    assert!(j.finished && !j.unfinished() && j.total == Some(MAIL) && j.messages == MAIL, "{j:?}");
    assert_eq!(j.path, file.to_string_lossy());
    assert_eq!(j.size, std::fs::metadata(file).unwrap().len());
    assert!(j.folders.iter().all(|f| f.ids.windows(2).all(|w| w[0] < w[1])), "sorted: {j:?}");
    // The old way in is told the count alone, and writes the same journal.
    let plain = Store::memory();
    assert_eq!(import(file, &plain, "b1", &mut |_| {}).unwrap().total, Some(MAIL));
    assert_eq!(journal(&plain, "b1"), j);
}

/// An import stopped after N items keeps those N: its messages in their folders' lists, its
/// folders, its contacts and calendar, and a journal that says it isn't finished.
#[test]
fn a_stopped_import_keeps_what_came_in() {
    let (file, whole, _) = &*WHOLE;
    for n in [0usize, 1, 9, 20, 29] {
        let store = Store::memory();
        let done = run(file, &store, "b1", Some(n), false).unwrap();
        assert!(done.stopped && done.last_save.is_none(), "{done:?}");
        assert_eq!(done.messages + done.contacts + done.events + done.notes + done.skipped, n, "{done:?}");
        assert_eq!(done.total, Some(MAIL));
        let folders = store.load_folders("b1").unwrap();
        assert_eq!(folders, done.folders);
        assert!(folders.iter().any(|f| f.role == FolderRole::Inbox));
        let listed: usize = folders.iter().map(|f| store.load_folder("b1", &f.path).unwrap().messages.len()).sum();
        assert_eq!((listed, folders.iter().map(|f| f.total as usize).sum::<usize>()), (done.messages, done.messages));
        let pim: Pim = store.load_account_doc("b1", PIM_DOC).unwrap();
        assert_eq!((pim.cards.len(), pim.events.len(), pim.notes.len()), (done.contacts, done.events, done.notes));
        let j = journal(&store, "b1");
        assert!(j.unfinished() && j.ids() == n && j.messages == done.messages && j.total == Some(MAIL), "{j:?}");
    }
    assert!(whole.messages + whole.contacts + whole.events + whole.notes > 29, "29 stops short of the end");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Stopped anywhere (twice), then gone on with to the end, an import is the import in one
    /// go: the same folders, lists, sources, contacts and calendar, nothing twice and nothing
    /// missing. So is one whose journal is behind what was written (`late`: the journal of the
    /// first stop put back after the second run, as a crash before the journal's write leaves it).
    #[test]
    fn stopped_anywhere_and_gone_on_with_is_the_import_in_one_go(first in 0usize..34, second in 0usize..34, late: bool) {
        let (file, whole, all) = &*WHOLE;
        let store = Store::memory();
        let one = run(file, &store, "b1", Some(first), false).unwrap();
        let early = journal(&store, "b1");
        let two = run(file, &store, "b1", Some(second), true).unwrap();
        prop_assert!(two.messages >= one.messages && two.messages <= whole.messages);
        if late {
            store.save_account_doc("b1", IMPORT_DOC, &early).unwrap();
        }
        let done = run(file, &store, "b1", None, true).unwrap();
        prop_assert!(!done.stopped);
        prop_assert_eq!((done.messages, done.skipped, done.total), (whole.messages, whole.skipped, whole.total));
        prop_assert_eq!((done.contacts, done.events, done.notes, done.tasks), (whole.contacts, whole.events, whole.notes, whole.tasks));
        prop_assert_eq!(&done.folders, &whole.folders);
        prop_assert_eq!(&held(&store, "b1"), all);
        let j = journal(&store, "b1");
        prop_assert!(j.finished && j.messages == whole.messages);
        // Gone on with once more, a finished import reads nothing and changes nothing.
        let again = run(file, &store, "b1", None, true).unwrap();
        prop_assert_eq!(again.messages, whole.messages);
        prop_assert_eq!(&held(&store, "b1"), all);
    }
}

/// A journal a page behind what was written (a crash between a page of mail and the journal),
/// in a folder kept in pages: going on writes the unnoted messages over themselves, and the
/// folder ends as the one of an import in one go.
#[test]
fn a_journal_a_page_behind_brings_nothing_in_twice() {
    let n = PAGE_MESSAGES + 300;
    let (dir, pst) = data_file("resume-pages", n);
    let whole = Store::memory();
    assert_eq!(run(&pst, &whole, "b1", None, false).unwrap().messages, n);
    let store = Store::memory();
    assert_eq!(run(&pst, &store, "b1", Some(100), false).unwrap().messages, 100);
    let early = journal(&store, "b1");
    // Past the first page: the journal was written with it, then at the stop.
    let two = run(&pst, &store, "b1", Some(PAGE_MESSAGES + 50), true).unwrap();
    assert_eq!((two.messages, two.stopped), (PAGE_MESSAGES + 50, true));
    assert_eq!(journal(&store, "b1").ids(), PAGE_MESSAGES + 50);
    assert_eq!(store.load_folder("b1", "INBOX").unwrap().messages.len(), PAGE_MESSAGES + 50);
    store.save_account_doc("b1", IMPORT_DOC, &early).unwrap();
    let done = run(&pst, &store, "b1", None, true).unwrap();
    assert_eq!((done.messages, done.skipped, done.stopped), (n, 0, false));
    let list = store.load_folder("b1", "INBOX").unwrap();
    let mut uids: Vec<u32> = list.messages.iter().map(|m| m.key.uid).collect();
    uids.sort_unstable();
    assert_eq!(uids, (1..=n as u32).collect::<Vec<_>>(), "each number once");
    assert_eq!(held(&store, "b1"), held(&whole, "b1"));
    // Stopped in the second page with the journal as the page's write left it (a crash right
    // after): the same.
    let store = Store::memory();
    run(&pst, &store, "b1", Some(PAGE_MESSAGES), false).unwrap();
    let at_page = journal(&store, "b1");
    assert_eq!(at_page.ids(), PAGE_MESSAGES);
    run(&pst, &store, "b1", Some(PAGE_MESSAGES + 200), true).unwrap();
    store.save_account_doc("b1", IMPORT_DOC, &at_page).unwrap();
    assert_eq!(run(&pst, &store, "b1", None, true).unwrap().messages, n);
    assert_eq!(held(&store, "b1"), held(&whole, "b1"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The journal is written with each page of mail while the import runs, not only at its end:
/// read as the import goes, it never notes more than the folder's list holds.
#[test]
fn the_journal_follows_the_pages_as_they_are_written() {
    let n = PAGE_MESSAGES + 300;
    let (dir, pst) = data_file("resume-follow", n);
    let store = Store::memory();
    let mut seen = Vec::new();
    let mut progress = |p: &Progress<'_>| {
        let j = journal(&store, "b1");
        let listed = store.load_folder("b1", "INBOX").unwrap().messages.len();
        assert!(j.is_some() && j.ids() <= listed, "{} noted, {listed} listed, at {}", j.ids(), p.messages);
        seen.push(j.ids());
    };
    import_run(&pst, &store, "b1", Run::default(), &mut progress).unwrap();
    assert!(seen.contains(&PAGE_MESSAGES), "written with the first page: {seen:?}");
    assert!(store.load_folders("b1").unwrap().iter().any(|f| f.path == "INBOX" && f.total as usize == n));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A data file that changed since the import began is said, and nothing comes in; allowed, what
/// the journal doesn't have comes in, and nothing twice.
#[test]
fn a_changed_file_is_said_and_brings_nothing_in_unasked() {
    use outlook_pst::{UnicodePstFile as Writer, UnicodePstMessage};
    let (dir, file) = mailbox("changed");
    let store = Store::memory();
    let one = run(&file, &store, "b1", Some(15), false).unwrap();
    let before = held(&store, "b1");
    let more = UnicodePstMessage {
        subject: "Came later",
        sender_name: "Ana",
        sender_email: "ana@studio.example",
        recipients: &[],
        body: "One more.",
        html_body: None,
        message_id: "<later@studio.example>",
        delivery_time: (1_704_100_000 + 11_644_473_600) * 10_000_000,
        message_class: "IPM.Note",
        properties: &[],
    };
    drop(Writer::append_in_folder(&file, &["Inbox"], &more).unwrap());
    assert_eq!(run(&file, &store, "b1", None, true).unwrap_err(), IMPORT_FILE_CHANGED);
    assert_eq!(held(&store, "b1"), before, "nothing came in");
    assert!(journal(&store, "b1").unfinished());
    let asked = || false;
    let done = import_run(&file, &store, "b1", Run { stop: Some(&asked), resume: true, allow_changed: true }, &mut |_| {}).unwrap();
    assert!(done.messages > one.messages);
    let fresh = Store::memory();
    let whole = run(&file, &fresh, "b1", None, false).unwrap();
    assert_eq!((done.messages, done.contacts, done.events, done.notes), (whole.messages, whole.contacts, whole.events, whole.notes));
    let subjects = |s: &Store| {
        let mut all: Vec<(String, String)> = Vec::new();
        for f in s.load_folders("b1").unwrap() {
            all.extend(s.load_folder("b1", &f.path).unwrap().messages.into_iter().map(|m| (f.path.clone(), m.subject)));
        }
        all.sort();
        all
    };
    assert_eq!(subjects(&store), subjects(&fresh));
    assert!(subjects(&store).iter().any(|(_, s)| s == "Came later"));
    let j = journal(&store, "b1");
    assert!(j.finished && j.size == std::fs::metadata(&file).unwrap().len());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A damaged journal is set aside, never fatal: the import can't go on (said so), and what came
/// in stays. An account that never had a journal is told the same.
#[test]
fn a_damaged_journal_is_set_aside_and_said() {
    fn find(dir: &Path, name: &str) -> bool {
        std::fs::read_dir(dir).is_ok_and(|rd| rd.flatten().any(|e| if e.path().is_dir() { find(&e.path(), name) } else { e.file_name() == name }))
    }
    let (dir, file) = mailbox("damaged");
    let store = Store::open(dir.join("cache")).unwrap();
    let one = run(&file, &store, "b1", Some(12), false).unwrap();
    let before = held(&store, "b1");
    assert!(find(&dir.join("cache"), IMPORT_DOC));
    for bad in [serde_json::json!("not a journal"), serde_json::json!({"path": "x.pst", "folders": [{"ids": [1, "two"]}]})] {
        store.save_account_doc("b1", IMPORT_DOC, &bad).unwrap();
        assert_eq!(run(&file, &store, "b1", None, true).unwrap_err(), IMPORT_NO_JOURNAL);
        assert!(find(&dir.join("cache"), "import.json.bad") && !find(&dir.join("cache"), IMPORT_DOC), "set aside");
        assert_eq!(held(&store, "b1"), before, "{} messages stay", one.messages);
    }
    assert_eq!(run(&file, &store, "never", None, true).unwrap_err(), IMPORT_NO_JOURNAL);
    let _ = std::fs::remove_dir_all(&dir);
}
