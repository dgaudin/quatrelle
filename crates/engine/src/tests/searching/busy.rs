//! Engine tests: a search over a big mailbox whose folders keep changing (a busy shared mailbox
//! on Exchange, its folder list sent again every few seconds), driven a slice at a time as the
//! window drives it.

use std::collections::BTreeSet;

use quatrelle_model::{AccountConfig, Folder};
use quatrelle_store::FolderState;

use super::*;

/// What the window adds to the index per frame.
const SLICE: u64 = 500;

/// The path of a busy mailbox's `i`-th folder.
fn folder_path(i: usize) -> String {
    if i == 0 { "INBOX".to_string() } else { format!("Folder{i:03}") }
}

/// A message of a busy mailbox: every hundredth one says "needle".
fn message(folder: &str, uid: u32, date: i64) -> Summary {
    let subject = if uid % 100 == 0 { format!("needle {folder} {uid}") } else { format!("Report {folder} {uid}") };
    Summary {
        key: MessageKey { account: "a1".into(), folder: folder.to_string(), validity: 1, uid },
        subject,
        from: EmailAddress::new("Ann Person", "ann@studio.example"),
        to: vec![EmailAddress::new("Me", "me@studio.example")],
        date,
        preview: "a short body".into(),
        ..Default::default()
    }
}

/// A local account `a1` of `folders` folders of `per` messages each (newest first).
fn busy_store(name: &str, folders: usize, per: u32) -> Store {
    let store = temp(name);
    store.save_accounts(&[AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
    let mut list = Vec::new();
    for i in 0..folders {
        let path = folder_path(i);
        let role = if i == 0 { FolderRole::Inbox } else { FolderRole::Other };
        list.push(Folder { account: "a1".into(), path: path.clone(), role, selectable: true, total: per, ..Default::default() });
        let base = i64::try_from(i).unwrap() * i64::from(per);
        let messages = (1..=per).rev().map(|uid| message(&path, uid, NOW - 86_400 * 400 + (base + i64::from(uid)) * 60)).collect();
        store.save_folder("a1", &path, &FolderState { validity: 1, messages, ..Default::default() }).unwrap();
    }
    store.save_folders("a1", &list).unwrap();
    store
}

fn open(store: &Store) -> Session {
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    s
}

/// Mail that arrives in a folder (on the server, then synced): written to the cache, then the
/// worker says so.
fn arrive(s: &mut Session, folder: &str, subject: &str) -> MessageKey {
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder("a1", folder).unwrap();
    let uid = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1;
    let mut m = message(folder, uid, NOW - 60);
    m.subject = subject.to_string();
    let key = m.key.clone();
    st.messages.insert(0, m);
    store.save_folder("a1", folder, &st).unwrap();
    s.apply(Event::FolderChanged { account: "a1".into(), folder: folder.to_string() });
    key
}

/// The folder list as the server sends it again: unchanged, as it comes every few seconds.
fn folders_again(s: &mut Session) {
    let folders = s.account("a1").unwrap().folders.clone();
    s.apply(Event::Folders { account: "a1".into(), folders });
}

/// What the search must find, read from the cache itself: every message of the account's
/// folders whose subject says "needle".
fn needles(s: &Session) -> BTreeSet<MessageKey> {
    let store = s.store.as_ref().unwrap();
    let mut out = BTreeSet::new();
    for f in &s.account("a1").unwrap().folders {
        for m in store.load_folder("a1", &f.path).unwrap().messages {
            if m.subject.to_lowercase().contains("needle") {
                out.insert(m.key);
            }
        }
    }
    out
}

/// The keys in the list, every one once.
fn listed(s: &Session) -> BTreeSet<MessageKey> {
    let keys: Vec<MessageKey> = s.list.iter().map(|m| m.key.clone()).collect();
    let set: BTreeSet<MessageKey> = keys.iter().cloned().collect();
    assert_eq!(keys.len(), set.len(), "a message is listed twice");
    set
}

/// Search the account as the window does, a slice a call, `between` run after each call while
/// more is left (sync events coming in meanwhile). How many calls it took, or `None` when it
/// hadn't finished after `max`.
fn drive(s: &mut Session, q: &str, max: usize, mut between: impl FnMut(&mut Session, usize)) -> Option<usize> {
    for step in 0..max {
        let v = s.run("search.run", &json!({"query": q, "scope": "mailbox", "index": SLICE})).unwrap();
        if v["pending"] == false {
            return Some(step + 1);
        }
        between(s, step);
    }
    None
}

/// The folder list sent again after every slice, and an already-read folder told of changes
/// every few: the search still finishes, in about the calls the mailbox needs.
#[test]
fn a_search_finishes_while_the_folders_keep_changing() {
    let (folders, per) = (24, 1200);
    let store = busy_store("busy-finish", folders, per);
    let mut s = open(&store);
    // A list cache smaller than the mailbox, as the owner's is.
    s.set_cache_limit(5_000);
    let slices = folders * per as usize / SLICE as usize;
    let calls = drive(&mut s, "needle", slices * 3, |s, step| {
        folders_again(s);
        if step % 4 == 3 {
            s.apply(Event::FolderChanged { account: "a1".into(), folder: "INBOX".into() });
        }
    });
    let calls = calls.unwrap_or_else(|| panic!("the search never finished in {} calls", slices * 3));
    assert!(calls <= slices * 2, "{calls} calls for {slices} slices of the mailbox");
    assert_eq!(listed(&s), needles(&s));
    assert_eq!(s.list.len(), folders * per as usize / 100);
    // Once done, the folder list sent again keeps the index: nothing is left to do.
    folders_again(&mut s);
    let v = s.run("search.run", &json!({"query": "needle", "index": SLICE})).unwrap();
    assert_eq!(v["pending"], false, "the index is kept: {v}");
    done(s);
}

/// Mail that arrives while the scan goes on, in a folder already read, the one being read and
/// one not read yet, is found once the scan is done; the folders aren't read again whole.
#[test]
fn mail_arriving_during_a_search_is_found() {
    let (folders, per) = (12, 1200);
    let store = busy_store("busy-arrive", folders, per);
    let mut s = open(&store);
    let mut arrived = Vec::new();
    let calls = drive(&mut s, "needle", 200, |s, step| {
        folders_again(s);
        // Six slices in: the first two folders are read, the third is being read.
        if step == 5 || step == 9 {
            for i in [0, 2, 3, 10] {
                arrived.push(arrive(s, &folder_path(i), &format!("needle arrived {step}")));
            }
        }
    });
    assert!(calls.is_some_and(|n| n <= 2 * folders * per as usize / SLICE as usize), "the search finishes: {calls:?}");
    let found = listed(&s);
    for k in &arrived {
        assert!(found.contains(k), "{k:?} arrived during the search and is found");
    }
    assert_eq!(found, needles(&s));
    // Arriving after the scan is done, it's found as well.
    let late = arrive(&mut s, "Folder005", "needle late");
    assert!(drive(&mut s, "needle", 10, |_, _| {}).is_some());
    assert!(listed(&s).contains(&late), "mail arriving after the scan is found");
    assert_eq!(listed(&s), needles(&s));
    done(s);
}

/// Messages moved or deleted while the scan goes on (here, and on the server) never show where
/// they were; a moved one shows once, where it went.
#[test]
fn mail_moved_or_deleted_during_a_search_is_not_found_where_it_was() {
    let (folders, per) = (12, 1200);
    let store = busy_store("busy-move", folders, per);
    let mut s = open(&store);
    let key = |f: usize, uid: u32| MessageKey { account: "a1".into(), folder: folder_path(f), validity: 1, uid };
    let gone = [key(0, 1100), key(1, 500), key(4, 300), key(9, 900)];
    let calls = drive(&mut s, "needle", 200, |s, step| {
        folders_again(s);
        if step == 6 {
            // Here: one read already goes to a folder read too, one to a folder still to read;
            // one is deleted.
            s.apply_op("a1", Op::Move { keys: vec![gone[0].clone()], to: folder_path(1) });
            s.apply_op("a1", Op::Move { keys: vec![gone[1].clone()], to: folder_path(8) });
            s.apply_op("a1", Op::Expunge { keys: vec![gone[2].clone()] });
            // On the server: one deleted in a folder still to read, as a sync brings it.
            let store = s.store.clone().unwrap();
            let mut st = store.load_folder("a1", &gone[3].folder).unwrap();
            st.messages.retain(|m| m.key != gone[3]);
            store.save_folder("a1", &gone[3].folder, &st).unwrap();
            s.apply(Event::FolderChanged { account: "a1".into(), folder: gone[3].folder.clone() });
        }
    });
    assert!(calls.is_some(), "the search finishes");
    let found = listed(&s);
    for k in &gone {
        assert!(!found.contains(k), "{k:?} is no longer there");
    }
    assert_eq!(found, needles(&s));
    assert_eq!(found.len(), folders * per as usize / 100 - 2, "the two moved are found where they went, once");
    done(s);
}

/// A folder made during a search of the mailbox is searched too; one deleted or renamed no
/// longer answers under its old name.
#[test]
fn folders_added_or_removed_during_a_search_follow() {
    let (folders, per) = (12, 1200);
    let store = busy_store("busy-folders", folders, per);
    let mut s = open(&store);
    let calls = drive(&mut s, "needle", 200, |s, step| {
        folders_again(s);
        if step == 6 {
            // Made on the server: the folder list says so.
            let store = s.store.clone().unwrap();
            let messages = (1..=300u32).rev().map(|uid| message("Fresh", uid, NOW - 3_600 + i64::from(uid))).collect();
            store.save_folder("a1", "Fresh", &FolderState { validity: 1, messages, ..Default::default() }).unwrap();
            let mut list = s.account("a1").unwrap().folders.clone();
            list.push(Folder { account: "a1".into(), path: "Fresh".into(), selectable: true, ..Default::default() });
            // Gone on the server: one read already, one still to read.
            list.retain(|f| f.path != folder_path(1) && f.path != folder_path(9));
            store.save_folders("a1", &list).unwrap();
            s.apply(Event::Folders { account: "a1".into(), folders: list });
            // Renamed here: one read already.
            s.run("folder.rename", &json!({"account": "a1", "folder": folder_path(2), "name": "Renamed"})).unwrap();
        }
    });
    assert!(calls.is_some(), "the search finishes");
    let found = listed(&s);
    assert!(found.iter().any(|k| k.folder == "Fresh"), "the new folder is searched");
    assert!(found.iter().any(|k| k.folder == "Renamed"), "the renamed folder is searched under its name");
    for gone in [folder_path(1), folder_path(2), folder_path(9)] {
        assert!(!found.iter().any(|k| k.folder == gone), "{gone} no longer answers");
    }
    assert_eq!(found, needles(&s));
    done(s);
}

/// A search a slice at a time, with sync events in between, answers what a whole search at once
/// answers over the same mailbox.
#[test]
fn a_search_in_slices_answers_what_a_whole_one_does() {
    let (folders, per) = (12, 1200);
    let store = busy_store("busy-same", folders, per);
    let mut s = open(&store);
    for q in ["needle", "report folder003", "ann"] {
        let calls = drive(&mut s, q, 200, |s, step| {
            folders_again(s);
            if step == 3 {
                arrive(s, "INBOX", &format!("needle report folder003 arrived for {q}"));
            }
        });
        assert!(calls.is_some(), "{q}: the search finishes");
        let sliced = listed(&s);
        let mut whole = open(&store);
        whole.run_search(q, SearchScope::Mailbox);
        assert_eq!(sliced, listed(&whole), "{q}: the same answer");
        assert!(!sliced.is_empty(), "{q}: something is found");
    }
    done(s);
}

/// What each slice costs from the first to the last over a big mailbox: it must not grow with
/// what's indexed already. Run by hand (release build):
///
/// ```text
/// cargo test -p quatrelle-engine --release --lib slices_cost_the_same -- --ignored --nocapture
/// ```
///
/// `QUATRELLE_SLICE_FOLDERS` and `QUATRELLE_SLICE_PER` set the mailbox's size (100 × 2000),
/// `QUATRELLE_SLICE_QUERY` the query (`needle`: one message in a hundred).
#[test]
#[ignore]
fn slices_cost_the_same_from_first_to_last() {
    let env = |k: &str, d: usize| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let (folders, per) = (env("QUATRELLE_SLICE_FOLDERS", 100), env("QUATRELLE_SLICE_PER", 2000));
    let q = std::env::var("QUATRELLE_SLICE_QUERY").unwrap_or_else(|_| "needle".into());
    let store = busy_store("busy-cost", folders, u32::try_from(per).unwrap());
    let mut s = open(&store);
    let mut times = Vec::new();
    let started = std::time::Instant::now();
    loop {
        let t = std::time::Instant::now();
        let v = s.run("search.run", &json!({"query": q, "scope": "mailbox", "index": SLICE})).unwrap();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        if v["pending"] == false {
            break;
        }
        assert!(times.len() < 100 * folders * per / SLICE as usize, "the search must finish");
    }
    let tenth = (times.len() / 10).max(1);
    let avg = |s: &[f64]| s.iter().sum::<f64>() / s.len() as f64;
    let (first, last) = (avg(&times[..tenth]), avg(&times[times.len() - tenth..]));
    println!(
        "{} messages, {} slices in {:.1} s: first tenth {first:.2} ms a slice, last tenth {last:.2} ms, slowest {:.2} ms; {} found",
        folders * per,
        times.len(),
        started.elapsed().as_secs_f64(),
        times.iter().cloned().fold(0.0, f64::max),
        s.list.len()
    );
    if q == "needle" {
        assert_eq!(s.list.len(), folders * per / 100);
    }
    assert!(last <= 2.0 * first.max(0.5), "slices get slower: {first:.2} ms then {last:.2} ms");
    done(s);
}
