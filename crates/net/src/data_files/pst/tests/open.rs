// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! How an import holds the data file: read only, through a cache, and with the handle that reads
//! items opened again before its own caches grow too large.

use std::path::PathBuf;

use outlook_pst::{UnicodePstBatchMessage, UnicodePstFile as Writer, UnicodePstMessage, UnicodePstRecipient, UnicodePstRecipientType};
use quatrelle_store::Store;

use super::super::open::{HANDLE_CACHE, Recycle};
use super::super::{import, import_recycled};

/// A data file of `n` messages in its Inbox, in a folder of its own.
fn data_file(name: &str, n: usize) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pst = dir.join("in.pst");
    let owned: Vec<(String, String)> = (0..n).map(|k| (format!("Report #{k}"), format!("<r{k}@run.example>"))).collect();
    let body = "Here is the week's plan, as agreed. ".repeat(40);
    let recipients =
        [UnicodePstRecipient { name: "Zoe", email: "zoe@studio.example", recipient_type: UnicodePstRecipientType::To, ..Default::default() }];
    let batch: Vec<UnicodePstBatchMessage<'_>> = owned
        .iter()
        .enumerate()
        .map(|(k, o)| UnicodePstBatchMessage {
            message: UnicodePstMessage {
                subject: &o.0,
                sender_name: "Ana",
                sender_email: "ana@soundlab.example",
                recipients: &recipients,
                body: &body,
                html_body: None,
                message_id: &o.1,
                delivery_time: (1_700_000_000 + k as i64 * 60 + 11_644_473_600) * 10_000_000,
                message_class: "IPM.Note",
                properties: &[],
            },
            attachments: &[],
        })
        .collect();
    Writer::create_in_folder_with_attachments(&pst, &["Inbox"], &batch[0].message, batch[0].attachments).unwrap();
    Writer::append_many_in_folder_with_attachments(&pst, &["Inbox"], &batch[1..]).unwrap();
    (dir, pst)
}

/// What every folder holds, by subject and date: the same whichever way the file was read.
fn contents(store: &Store, account: &str) -> Vec<(String, Vec<(String, i64)>)> {
    store
        .load_folders(account)
        .unwrap()
        .iter()
        .map(|f| {
            let mut list: Vec<(String, i64)> =
                store.load_folder(account, &f.path).unwrap().messages.iter().map(|m| (m.subject.clone(), m.date)).collect();
            list.sort();
            (f.path.clone(), list)
        })
        .collect()
}

/// The file is read through the cache: every byte read at most once, in far fewer reads than
/// pages.
#[test]
fn the_file_is_read_once_in_few_reads() {
    let (dir, pst) = data_file("reads", 600);
    let done = import(&pst, &Store::memory(), "c1", &mut |_| {}).unwrap();
    let len = std::fs::metadata(&pst).unwrap().len();
    let r = done.reading;
    assert_eq!(done.messages, 600);
    assert!(r.bytes > 0 && r.bytes <= len, "{r:?}");
    assert!(r.reads > 0 && r.reads < len / 512, "{r:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The handle that reads items opened again every 3 items (and every one) loses nothing and
/// brings nothing in twice; and what was read of the file stays read, so a new handle doesn't
/// read it again: every byte of this small file is read once, however often it's reopened.
#[test]
fn reopening_the_reader_loses_nothing_and_rereads_nothing() {
    let (dir, pst) = data_file("recycle", 250);
    let store = Store::memory();
    let plain = import(&pst, &store, "r0", &mut |_| {}).unwrap();
    assert_eq!((plain.messages, plain.reading.reopens), (250, 0), "a small file needs no reopening");
    let len = std::fs::metadata(&pst).unwrap().len();
    assert!(plain.reading.bytes > 0 && plain.reading.bytes <= len, "{:?}", plain.reading);
    for (account, every) in [("r3", 3), ("r1", 1)] {
        let done = import_recycled(&pst, &store, account, Recycle { every, limit: HANDLE_CACHE }, &mut |_| {}).unwrap();
        assert_eq!((done.messages, done.skipped), (250, 0));
        assert_eq!(done.reading.reopens as usize, 250usize.div_ceil(every) - 1, "{:?}", done.reading);
        assert_eq!(contents(&store, account), contents(&store, "r0"));
        assert!(done.reading.bytes <= len, "read again after reopening: {:?}", done.reading);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// What sets the reopening off in the app is the pages a handle has read, not a count of items:
/// let hold only a few KiB, the handle that reads items is opened again as it reads, and the
/// import is the same as with no reopening, every byte of the file still read at most once.
#[test]
fn the_reader_is_reopened_once_its_pages_hold_too_much() {
    let (dir, pst) = data_file("page-limit", 250);
    let store = Store::memory();
    let plain = import(&pst, &store, "p0", &mut |_| {}).unwrap();
    assert_eq!(plain.reading.reopens, 0);
    let len = std::fs::metadata(&pst).unwrap().len();
    let done = import_recycled(&pst, &store, "p1", Recycle { every: 0, limit: 8 * 1024 }, &mut |_| {}).unwrap();
    assert_eq!((done.messages, done.skipped), (250, 0));
    assert!(done.reading.reopens > 0, "the pages read were counted: {:?}", done.reading);
    assert!(done.reading.reopens < 250, "and not every item set it off: {:?}", done.reading);
    assert_eq!(contents(&store, "p1"), contents(&store, "p0"));
    assert!(done.reading.bytes <= len, "{:?}", done.reading);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A data file that can't be written to imports: nothing asks to write it.
#[test]
fn a_read_only_data_file_imports() {
    let (dir, pst) = data_file("read-only", 5);
    let mut perms = std::fs::metadata(&pst).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&pst, perms.clone()).unwrap();
    let done = import(&pst, &Store::memory(), "o1", &mut |_| {});
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&pst, perms).unwrap();
    assert_eq!(done.map(|d| d.messages), Ok(5));
    let _ = std::fs::remove_dir_all(&dir);
}

/// While it's read, no other program may open the data file to write (Windows' sharing
/// violation, error 32), and the import leaves it as it was.
#[cfg(windows)]
#[test]
fn nothing_writes_the_data_file_while_it_is_read() {
    let (dir, pst) = data_file("shared", 450);
    let before = std::fs::metadata(&pst).unwrap().modified().unwrap();
    let mut tried = Vec::new();
    let done = import(&pst, &Store::memory(), "w1", &mut |_| {
        tried.push(std::fs::OpenOptions::new().write(true).open(&pst).map_err(|e| e.raw_os_error()).err());
        // Reading it is fine.
        assert!(std::fs::File::open(&pst).is_ok());
    })
    .unwrap();
    assert_eq!(done.messages, 450);
    assert!(!tried.is_empty() && tried.iter().all(|e| *e == Some(Some(32))), "{tried:?}");
    assert_eq!(std::fs::metadata(&pst).unwrap().modified().unwrap(), before);
    // Once it's done, the file is let go.
    assert!(std::fs::OpenOptions::new().write(true).open(&pst).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Reads made of the data file and peak memory over an import (Linux; alone in its process:
/// `cargo test -p quatrelle-net --lib -- --ignored --test-threads=1 open::measure`), of
/// `QUATRELLE_PST_MESSAGES` messages (2 000), the item reader opened again as in the app or,
/// with `QUATRELLE_PST_NEVER_REOPEN`, never.
#[cfg(target_os = "linux")]
#[test]
#[ignore]
fn measure_reads_and_memory() {
    fn proc(file: &str, field: &str) -> u64 {
        let s = std::fs::read_to_string(file).unwrap();
        let line = s.lines().find(|l| l.starts_with(field)).unwrap();
        line.split_whitespace().nth(1).unwrap().parse::<u64>().unwrap()
    }
    let n = std::env::var("QUATRELLE_PST_MESSAGES").ok().and_then(|n| n.parse().ok()).unwrap_or(2000);
    let recycle = if std::env::var_os("QUATRELLE_PST_NEVER_REOPEN").is_some() { Recycle { every: 0, limit: u64::MAX } } else { Recycle::default() };
    let (dir, pst) = data_file("measure", n);
    let len = std::fs::metadata(&pst).unwrap().len();
    let store = Store::open(dir.join("cache")).unwrap();
    std::fs::write("/proc/self/clear_refs", "5").unwrap();
    let (rss, calls) = (proc("/proc/self/status", "VmRSS:"), proc("/proc/self/io", "syscr:"));
    let t = std::time::Instant::now();
    let done = import_recycled(&pst, &store, "m1", recycle, &mut |_| {}).unwrap();
    let took = t.elapsed();
    let calls = proc("/proc/self/io", "syscr:") - calls;
    let peak = proc("/proc/self/status", "VmHWM:").saturating_sub(rss);
    eprintln!("{} messages from {len} bytes in {took:?}: {calls} read calls, peak {peak} KiB above before, {:?}", done.messages, done.reading);
    let _ = std::fs::remove_dir_all(&dir);
}
