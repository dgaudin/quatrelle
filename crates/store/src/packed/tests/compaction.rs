// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Compact Now, in place.

use super::*;
use crate::packed::Packs;

/// Compact Now gives the space mail deleted took back to the disk, and what's left reads back
/// whole; whatever wants the file meanwhile waits for it rather than failing.
#[test]
fn a_data_file_compacts_and_others_wait_for_it() {
    let root = temp("compact");
    let store = Store::open_packed(root.join("app"), root.join("Quatrelle Files")).unwrap();
    store.save_accounts(&[account("a1", "ana@example.com")]).unwrap();
    let key = |uid: u32| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
    let body = |uid: u32| noise(200_000, u64::from(uid));
    for uid in 1..=300 {
        store.write_body(&key(uid), &body(uid)).unwrap();
    }
    // Most of it deleted: the file keeps its size.
    for uid in 1..=300 {
        if uid % 4 != 0 {
            store.remove_body(&key(uid));
        }
    }
    let file = store.data_file("a1").unwrap();
    // What File › Info says it takes: the room on the disk.
    let full = crate::packed::on_disk(&std::fs::metadata(&file).unwrap());
    // Another thread reading while it's compacted waits, then reads.
    let reader = {
        let store = store.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(5));
            (0..20).all(|_| store.read_body(&key(4)).is_some_and(|b| b == body(4)))
        })
    };
    let (before, after) = store.compact_data_file("a1").unwrap().unwrap();
    assert_eq!(before, full);
    assert!(after < full / 2, "{full} → {after}");
    assert_eq!(crate::packed::on_disk(&std::fs::metadata(&file).unwrap()), after);
    assert!(reader.join().unwrap(), "read while it was compacted");
    for uid in (4..=300).step_by(4) {
        assert_eq!(store.read_body(&key(uid)).as_deref(), Some(body(uid).as_slice()), "{uid}");
    }
    // And written to after.
    store.write_body(&key(1), b"again").unwrap();
    assert_eq!(store.read_body(&key(1)).as_deref(), Some(&b"again"[..]));
    assert_eq!(store.compact_data_file("nobody").unwrap(), None, "no data file");
    let _ = std::fs::remove_dir_all(&root);
}

/// A data file stopped dead right after Compact Now opens without being gone through first: the
/// compaction's own commits don't keep what a repair would work out (redb skips it there), so a
/// sealed commit follows them. The writer is this test run again, which writes, deletes most of
/// it, compacts, and leaves without closing anything.
#[test]
fn a_data_file_killed_after_compacting_needs_no_repair() {
    const WRITER: &str = "QUATRELLE_TEST_COMPACTED_WRITER";
    let key = |uid: u32| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
    let body = |uid: u32| noise(100_000, u64::from(uid));
    if let Some(dir) = std::env::var_os(WRITER) {
        let dir = std::path::PathBuf::from(dir);
        let store = Store::open_packed(dir.join("app"), dir.join("Files")).unwrap();
        store.save_accounts(&[account("a1", "a1@example.org")]).unwrap();
        for uid in 1..=60 {
            store.write_body(&key(uid), &body(uid)).unwrap();
        }
        for uid in (1..=60).filter(|u| u % 3 != 0) {
            store.remove_body(&key(uid));
        }
        let (before, after) = store.compact_data_file("a1").unwrap().unwrap();
        assert!(after < before, "{before} → {after}: the test needs a compaction");
        // No destructor runs: the data file is left as a crash leaves it.
        std::process::exit(3);
    }
    let t = temp("compacted-crash");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "packed::tests::compaction::a_data_file_killed_after_compacting_needs_no_repair", "--test-threads=1"])
        .env(WRITER, &t)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(3), "the writer stopped dead after compacting");
    let file = std::fs::read_dir(t.join("Files"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().is_some_and(|x| x == super::super::EXTENSION))
        .unwrap();
    let repaired = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = repaired.clone();
    let db = redb::Database::builder().set_repair_callback(move |_| seen.set(true)).create(&file).unwrap();
    assert!(!repaired.get(), "no repair was needed");
    drop(db);
    let store = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    for uid in 1..=60 {
        let kept = store.read_body(&key(uid));
        assert_eq!(kept.as_deref(), (uid % 3 == 0).then(|| body(uid)).as_deref(), "{uid}");
    }
    drop(store);
    let _ = std::fs::remove_dir_all(&t);
}

/// A read from the window's thread while Compact Now has the file is refused at once ("being
/// compacted"), rather than freezing the window until it's done (up to 30 minutes); a read from
/// another thread, and a write from any, still wait and then go through. Here the compaction
/// waits 1.5 s for a sync holding the file before it can start.
#[test]
fn a_read_from_the_window_never_waits_for_a_compaction() {
    let root = temp("compact-window");
    let store = Store::open_packed(root.join("app"), root.join("Quatrelle Files")).unwrap();
    store.save_accounts(&[account("a1", "ana@example.com")]).unwrap();
    let key = |uid: u32| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
    for uid in 1..=40 {
        store.write_body(&key(uid), &noise(50_000, u64::from(uid))).unwrap();
    }
    let crate::Files::Packed(packs) = &store.files.main else { panic!("packed") };
    // A sync holding the file: the compaction waits for it to let go.
    let held = packs.db("a1", false).unwrap().unwrap();
    let holder = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        drop(held);
    });
    let compacting = {
        let store = store.clone();
        std::thread::spawn(move || store.compact_data_file("a1").unwrap().unwrap())
    };
    while !Packs::lock(&packs.compacting).contains("a1") {
        std::thread::yield_now();
    }
    let window = {
        let store = store.clone();
        std::thread::spawn(move || {
            crate::no_waiting();
            let began = std::time::Instant::now();
            let read = store.read_body(&key(3));
            (read.is_some(), began.elapsed())
        })
    };
    let (read, waited) = window.join().unwrap();
    eprintln!("a read from the window's thread during a compaction: {waited:.2?} (read: {read})");
    assert!(waited < std::time::Duration::from_millis(100), "the window waited {waited:.2?}");
    assert!(!read, "refused: being compacted");
    // Another thread waits, then reads; a write from the window's thread waits, then writes.
    let other = {
        let store = store.clone();
        std::thread::spawn(move || store.read_body(&key(4)))
    };
    let writer = {
        let store = store.clone();
        std::thread::spawn(move || {
            crate::no_waiting();
            store.write_body(&key(1), b"Subject: x\r\n\r\nwritten while compacted")
        })
    };
    holder.join().unwrap();
    compacting.join().unwrap();
    assert_eq!(other.join().unwrap(), Some(noise(50_000, 4)), "another thread waited, then read");
    assert!(writer.join().unwrap().is_ok(), "the write waited, then wrote");
    assert_eq!(store.read_body(&key(1)).as_deref(), Some(&b"Subject: x\r\n\r\nwritten while compacted"[..]));
    assert_eq!(store.read_body(&key(3)), Some(noise(50_000, 3)), "read once it's done");
    drop(store);
    let _ = std::fs::remove_dir_all(&root);
}

/// Compact Now ends with the file as small as what it keeps, even with nothing to give back:
/// what seals it (so a crash after it needs no repair) doesn't grow it again. A commit on a file
/// compacted tight grew it, and redb doubles a file under 4 GiB as it grows: here 306 MB kept in
/// a file of 539 MB ended in one of 623 MB.
#[test]
fn a_compacted_data_file_stays_as_small_as_what_it_keeps() {
    // Noise, eight bytes at a time (quick enough for 300 MB in a debug build).
    let body = |uid: u32| -> Vec<u8> {
        let n = [5_000usize, 40_000, 300_000, 1_200_000, 70_000][uid as usize % 5] + uid as usize * 37;
        let mut x = u64::from(uid).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut out = Vec::with_capacity(n + 8);
        while out.len() < n {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            out.extend_from_slice(&x.to_le_bytes());
        }
        out.truncate(n);
        out
    };
    let root = temp("compact-tight");
    let store = Store::open_packed(root.join("app"), root.join("Quatrelle Files")).unwrap();
    store.save_accounts(&[account("a1", "ana@example.com")]).unwrap();
    let key = |uid: u32| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
    let mut kept = 0u64;
    for uid in 1..=900 {
        let b = body(uid);
        kept += b.len() as u64;
        store.write_body(&key(uid), &b).unwrap();
    }
    let (before, after) = store.compact_data_file("a1").unwrap().unwrap();
    eprintln!("{before} → {after} bytes for {kept} kept ({:.2} of it)", after as f64 / kept as f64);
    assert!(after < kept + kept / 5, "{before} → {after} bytes for {kept} kept");
    for uid in (1..=900).step_by(97) {
        assert_eq!(store.read_body(&key(uid)), Some(body(uid)), "{uid}");
    }
    drop(store);
    let _ = std::fs::remove_dir_all(&root);
}

/// What an account's data file takes on this computer (File › Info, the folder sizes, Compact
/// Now's before and after) is the room it takes on the disk, not its length: redb doubles a file
/// under 4 GiB as it grows, and on Linux the new half is a hole until it's written. The first
/// message written after Compact Now showed the cache twice as big as it was.
#[cfg(unix)]
#[test]
fn an_accounts_size_is_the_room_its_file_takes() {
    // Noise, eight bytes at a time: kept as it is, never compressed smaller.
    let body = |uid: u32| -> Vec<u8> {
        let mut x = u64::from(uid).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut out = Vec::with_capacity(200_008);
        while out.len() < 200_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            out.extend_from_slice(&x.to_le_bytes());
        }
        out
    };
    let root = temp("size-on-disk");
    let store = Store::open_packed(root.join("app"), root.join("Quatrelle Files")).unwrap();
    store.save_accounts(&[account("a1", "ana@example.com")]).unwrap();
    let key = |uid: u32| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
    let mut kept = 0u64;
    for uid in 1..=200 {
        let b = body(uid);
        kept += b.len() as u64;
        store.write_body(&key(uid), &b).unwrap();
    }
    store.compact_data_file("a1").unwrap();
    // The first message after it grows the file.
    store.write_body(&key(201), &body(201)).unwrap();
    kept += 200_000;
    let file = store.data_file("a1").unwrap();
    let length = std::fs::metadata(&file).unwrap().len();
    let shown = store.account_size("a1");
    eprintln!("{kept} kept, the file {length} long, shown {shown}");
    assert!(shown <= length);
    assert!(shown < kept + kept / 4, "{kept} kept, the file {length} long, shown {shown}");
    drop(store);
    let _ = std::fs::remove_dir_all(&root);
}
