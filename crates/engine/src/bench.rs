// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A big mailbox, on demand: what opening it, its biggest folder, a search and the address book
//! cost. An ignored test, run by hand on a cache a data file was imported into (see
//! `quatrelle-net`'s `pst_bench`):
//!
//! ```text
//! MC_CACHE=C:\mc-bench\cache-many cargo test -p quatrelle-engine --release --lib bench::big_mailbox -- --ignored --nocapture
//! ```
//!
//! Each step prints how long it took; memory is watched from outside. Steps are a few seconds
//! apart so the watcher sees each.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::Session;
use crate::model::dates::Clock;
use crate::store::Store;

fn step<T>(what: &str, f: impl FnOnce() -> T) -> T {
    std::thread::sleep(Duration::from_secs(3));
    let t = Instant::now();
    let r = f();
    println!("{what}: {:.2} s", t.elapsed().as_secs_f64());
    r
}

#[test]
#[ignore]
fn big_mailbox() {
    let cache = PathBuf::from(std::env::var("MC_CACHE").expect("MC_CACHE: a cache a data file was imported into"));
    let store = Store::open_packed(cache.join("root"), cache.join("files")).unwrap();
    store
        .save_accounts(&[crate::model::AccountConfig { id: "d1".into(), name: "Big".into(), email: "big@local".into(), ..Default::default() }])
        .unwrap();
    let mut s = step("open", || Session::open(Some(store), Clock::new(1_800_000_000, 0)));
    let folders: Vec<(String, u32)> = s.account("d1").map(|a| a.folders.iter().map(|f| (f.path.clone(), f.total)).collect()).unwrap_or_default();
    println!("folders: {folders:?}");
    let biggest = folders.iter().max_by_key(|(_, n)| *n).map(|(p, _)| p.clone()).unwrap_or_else(|| "INBOX".into());
    step(&format!("open {biggest}"), || s.run("folder.open", &json!({"account": "d1", "folder": biggest})).unwrap());
    println!("list: {} messages", s.list.len());
    step("refresh the list", || s.refresh_list());
    let first = s.list.first().map(|m| m.key.clone());
    if let Some(k) = first {
        let _ = s.run("list.select", &json!({"key": k.to_id()}));
        step("mark one read", || drop(s.run("mail.markRead", &json!({}))));
    }
    step("search this folder", || s.run_search("budget review", crate::SearchScope::Current));
    println!("found {}", s.list.len());
    step("search everything", || s.run_search("storyboard deadline", crate::SearchScope::All));
    println!("found {}", s.list.len());
    step("search everything again", || s.run_search("festival", crate::SearchScope::All));
    step("address book", || s.address_book("all", "zo").len());
    step("rest", || std::thread::sleep(Duration::from_secs(2)));
}

fn rss() -> String {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let get = |k: &str| {
        s.lines().find(|l| l.starts_with(k)).map(|l| l.split_whitespace().nth(1).unwrap_or("0").parse::<u64>().unwrap_or(0) / 1024).unwrap_or(0)
    };
    format!("rss {} MB, peak {} MB", get("VmRSS:"), get("VmHWM:"))
}

/// The owner's mailbox's size, seeded: what each step leaves in memory.
#[test]
#[ignore]
fn seeded_big_mailbox_memory() {
    let folders: usize = std::env::var("MC_FOLDERS").ok().and_then(|v| v.parse().ok()).unwrap_or(110);
    let per: u32 = std::env::var("MC_PER").ok().and_then(|v| v.parse().ok()).unwrap_or(5000);
    let dir = std::env::temp_dir().join(format!("mc-mem-{}", std::process::id()));
    let store = Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
    let t = Instant::now();
    crate::sample::seed_big_mailbox(&store, folders, per, 1_800_000_000).unwrap();
    println!("seeded {} in {:.0} s; {}", folders * per as usize, t.elapsed().as_secs_f64(), rss());
    drop(store);
    let store = Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
    let mut s = Session::open(Some(store), Clock::new(1_800_000_000, 0));
    println!("open: {}", rss());
    s.run("folder.open", &json!({"account": "big", "folder": "INBOX"})).unwrap();
    println!("inbox: {} messages; {}", s.list.len(), rss());
    s.run("folder.open", &json!({"account": "big", "folder": "Folder50"})).unwrap();
    println!("another folder: {}", rss());
    let n = s.suggest_contacts_now("ann12", 8).len();
    println!("suggestions ({n}): {}", rss());
    let n = s.address_book_now("all", "zo").len();
    println!("address book ({n}): {}", rss());
    let n = s.run_search("Message 4242", crate::SearchScope::All);
    println!("search everything ({n}): {}", rss());
    s.run("folder.open", &json!({"account": "big", "folder": "INBOX"})).unwrap();
    println!("back to the inbox: {}", rss());
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A copy of a real cache (`MC_CACHE` holding `root/` and `files/`): what each step leaves in
/// memory. Nothing is sent: the session's jobs are never run.
#[test]
#[ignore]
fn real_cache_memory() {
    let cache = PathBuf::from(std::env::var("MC_CACHE").expect("MC_CACHE"));
    let store = Store::open_packed(cache.join("root"), cache.join("files")).unwrap();
    let t = Instant::now();
    let mut s = Session::open(Some(store), Clock::new(1_800_000_000, 0));
    let total: u64 = s.accounts.iter().flat_map(|a| a.folders.iter()).map(|f| u64::from(f.total)).sum();
    println!(
        "open ({} folders, {total} messages) in {:.1} s: {}",
        s.accounts.iter().map(|a| a.folders.len()).sum::<usize>(),
        t.elapsed().as_secs_f64(),
        rss()
    );
    let acc = s.accounts.first().map(|a| a.config.id.clone()).unwrap_or_default();
    let t = Instant::now();
    s.run("folder.open", &json!({"account": acc, "folder": "INBOX"})).unwrap();
    println!("inbox: {} messages in {:.1} s; {}", s.list.len(), t.elapsed().as_secs_f64(), rss());
    let t = Instant::now();
    let n = s.suggest_contacts_now("ma", 8).len();
    println!("suggestions ({n}) in {:.1} s: {}", t.elapsed().as_secs_f64(), rss());
    let t = Instant::now();
    let n = s.run_search("facture", crate::SearchScope::Current);
    println!("search the inbox ({n}) in {:.1} s: {}", t.elapsed().as_secs_f64(), rss());
    let t = Instant::now();
    let n = s.run_search("facture", crate::SearchScope::All);
    println!("search everything ({n}) in {:.1} s: {}", t.elapsed().as_secs_f64(), rss());
    s.run("folder.open", &json!({"account": acc, "folder": "INBOX"})).unwrap();
    println!("back to the inbox: {}", rss());
}

/// Down ×20 in a generated folder of 50,000 messages (`MC_PER` for another size), on a server
/// account as the owner's, every source kept here: what the window spends on each Down (the
/// list's share and the read mark's), on the echo of the mark the writer sends back
/// (`FolderChanged`), and what the writer spends keeping the mark (`Store::apply_op`, on a
/// thread of its own), online (the server takes each change at once) and offline (they wait in
/// the journal). No copy of anyone's mail: the folder is made up.
///
/// ```text
/// cargo test -p quatrelle-engine --release --lib bench::down_twenty -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn down_twenty() {
    use crate::model::{Event, Job, MessageKey};
    const NOW: i64 = 1_800_000_000;
    let per: u32 = std::env::var("MC_PER").ok().and_then(|v| v.parse().ok()).unwrap_or(50_000);
    let dir = std::env::temp_dir().join(format!("mc-down-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
    crate::sample::seed_big_mailbox(&store, 1, per, NOW).unwrap();
    // A server account: marks go to the writer as jobs and come back as `FolderChanged`.
    let mut config = store.load_accounts().unwrap().remove(0);
    config.imap.host = "imap.big.example".into();
    config.smtp.host = "smtp.big.example".into();
    let account = config.id.clone();
    store.save_accounts(&[config]).unwrap();
    let keys: Vec<MessageKey> = store.load_folder(&account, "INBOX").unwrap().messages.into_iter().map(|m| m.key).collect();
    for chunk in keys.chunks(1000) {
        let bodies: Vec<(MessageKey, Vec<u8>)> = chunk
            .iter()
            .map(|k| (k.clone(), format!("From: p{0}@big.example\r\nSubject: Message {0}\r\n\r\nHello there.\r\n", k.uid).into_bytes()))
            .collect();
        store.write_bodies(&bodies).unwrap();
    }
    let mut s = Session::open(Some(store.clone()), Clock::new(NOW, 0));
    s.run("folder.open", &json!({"account": account, "folder": "INBOX"})).unwrap();
    println!("list: {} messages", s.list.len());
    let _ = s.take_jobs();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    // What the writer does with a job (`net`'s `make_edit`): journaled, kept, its echo told.
    let write = |s: &mut Session, online: bool, writer: &mut Duration, echo: &mut Duration| {
        for job in s.take_jobs() {
            let Job::Apply { account, op } = job else { continue };
            let t = Instant::now();
            store.add_op(&account, op.clone()).unwrap();
            let changed = store.apply_op(&account, &op).unwrap();
            if online {
                store.remove_op(&account, &op).unwrap();
            }
            *writer += t.elapsed();
            let t = Instant::now();
            for folder in changed {
                s.apply(Event::FolderChanged { account: account.clone(), folder });
            }
            *echo += t.elapsed();
        }
    };
    for (conversations, by) in [(false, "date"), (true, "from")] {
        let _ = s.run("view.conversations", &json!({"on": conversations}));
        let _ = s.run("view.arrange", &json!({"by": by}));
        let Some(first) = s.list.first().map(|m| m.key.to_id()) else { continue };
        let _ = s.run("list.open", &json!({"key": first}));
        let (mut down, mut echo, mut writer) = (Duration::ZERO, Duration::ZERO, Duration::ZERO);
        write(&mut s, true, &mut writer, &mut echo);
        let (mut echo, mut writer) = (Duration::ZERO, Duration::ZERO);
        for _ in 0..20 {
            let t = Instant::now();
            let _ = s.run("list.next", &json!({}));
            down += t.elapsed();
            write(&mut s, true, &mut writer, &mut echo);
        }
        println!(
            "Down x20 (conversations {conversations}, by {by}): {:.1} ms a Down, {:.1} ms its echo; the writer {:.1} ms a mark",
            ms(down) / 20.0,
            ms(echo) / 20.0,
            ms(writer) / 20.0
        );
    }
    // A change the window didn't make (a sync's): the folder read from disk again and the list
    // built, what a body downloaded on the way cost before it came back as its summary filled.
    let mut reread = Duration::ZERO;
    for _ in 0..20 {
        let t = Instant::now();
        s.apply(Event::FolderChanged { account: account.clone(), folder: "INBOX".into() });
        reread += t.elapsed();
    }
    println!("a change from the server (FolderChanged): {:.1} ms, the folder read again", ms(reread) / 20.0);
    // A body downloaded now: its summary filled from it, taken into its row.
    let filled: Vec<crate::model::Summary> = s
        .list
        .iter()
        .take(20)
        .map(|m| crate::model::Summary { preview: format!("First take of the mix, number {}", m.key.uid), ..crate::model::Summary::clone(m) })
        .collect();
    let t = Instant::now();
    for m in filled {
        s.apply(Event::Filled { summary: Box::new(m) });
    }
    println!("a body downloaded (Filled): {:.2} ms, its row filled", ms(t.elapsed()) / 20.0);
    // Offline: the marks wait in the journal, and each one kept makes them all again.
    let _ = s.run("view.conversations", &json!({"on": false}));
    let _ = s.run("view.arrange", &json!({"by": "date"}));
    let mut costs = Vec::new();
    for _ in 0..100 {
        let _ = s.run("list.next", &json!({}));
        let (mut echo, mut writer) = (Duration::ZERO, Duration::ZERO);
        write(&mut s, false, &mut writer, &mut echo);
        costs.push(ms(writer));
    }
    let avg = |c: &[f64]| c.iter().sum::<f64>() / c.len().max(1) as f64;
    println!(
        "offline, 100 marks: the writer {:.1} ms a mark for the first 10, {:.1} ms for the last 10",
        avg(costs.get(..10).unwrap_or_default()),
        avg(costs.get(90..).unwrap_or_default())
    );
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}
