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
