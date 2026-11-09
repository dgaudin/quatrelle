// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Compact Now on a data file from before R3, whose attachments were kept raw: they're compressed
//! again first, and what they took goes back to the disk. Everything reads back the same, a file
//! written meanwhile is left as written, a run stopped or killed goes on from where it got to,
//! and a hostile `META` doesn't stop it.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use redb::ReadableDatabase;

use super::*;
use crate::packed::recompress::{Account, Target};
use crate::packed::{Packs, Tables};
use crate::{CompactStep, Recompressing};

/// What a data file from before holds: path → bytes as kept.
type Kept = Vec<(String, Vec<u8>)>;

/// A data store at `t` whose account `a1` has a data file as a build before R3 left it: no
/// `META`, and `kept` as it was kept (raw or compressed, whole or in pieces), compacted.
fn old_store(t: &std::path::Path, kept: &Kept) -> Store {
    old_store_from(t, kept.iter().cloned())
}

/// [`old_store`], its files made as they're written (64 MB a transaction at most).
fn old_store_from(t: &std::path::Path, mut kept: impl Iterator<Item = (String, Vec<u8>)>) -> Store {
    let (app, files) = (t.join("app"), t.join("Files"));
    std::fs::create_dir_all(&app).unwrap();
    std::fs::create_dir_all(&files).unwrap();
    std::fs::write(app.join("datafiles.json"), br#"{"a1": "old.quatrelle"}"#).unwrap();
    {
        let mut db = redb::Database::create(files.join("old.quatrelle")).unwrap();
        let mut more = true;
        while more {
            let tx = db.begin_write().unwrap();
            {
                let mut tables = Tables { files: tx.open_table(super::super::FILES).unwrap(), pieces: tx.open_table(super::super::PIECES).unwrap() };
                let mut n = 0usize;
                more = false;
                for (k, v) in kept.by_ref() {
                    tables.put_kept(&k, &v).unwrap();
                    n += v.len();
                    if n >= 64 << 20 {
                        more = true;
                        break;
                    }
                }
            }
            tx.commit().unwrap();
        }
        // Compacted as it was left: what Compact Now gives back is then only what it saves.
        while db.compact().unwrap() {}
    }
    let s = Store::open_packed(&app, &files).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    s
}

fn packs(s: &Store) -> &Packs {
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    packs
}

/// Every file of `a1` as it reads back (`None`: it doesn't).
fn read_back(s: &Store) -> BTreeMap<String, Option<Vec<u8>>> {
    let p = packs(s);
    p.keys("a1", "").unwrap().into_iter().map(|(k, _)| (k.clone(), p.get("a1", &k).ok().flatten())).collect()
}

/// What `a1`'s `META` says.
fn meta(s: &Store) -> super::super::Meta {
    let db = packs(s).db("a1", false).unwrap().unwrap();
    super::super::read_meta(&db.begin_read().unwrap()).unwrap()
}

/// Whether the file at `key` is kept compressed (its first bytes, or its first piece's, say so).
fn kept_compressed(s: &Store, key: &str) -> bool {
    let db = packs(s).db("a1", false).unwrap().unwrap();
    let tx = db.begin_read().unwrap();
    let files = tx.open_table(super::super::FILES).unwrap();
    let v = files.get(key).unwrap().unwrap().value().to_vec();
    if super::super::pieced_len(&v).is_none() {
        return v.starts_with(super::super::DEFLATED);
    }
    let pieces = tx.open_table(super::super::PIECES).unwrap();
    pieces.get((key, 0)).unwrap().unwrap().value().starts_with(super::super::DEFLATED)
}

/// Mail text of about `n` bytes (compresses well).
fn text(n: usize, seed: u64) -> Vec<u8> {
    format!("Subject: note {seed}\r\n\r\n").bytes().chain(b"the figures for the quarter are in ".iter().copied().cycle().take(n)).collect()
}

/// Messages with a base64 attachment each, kept raw as before R3, `total` bytes in all, made as
/// they're asked for.
fn raw_attachments(total: usize) -> impl Iterator<Item = (String, Vec<u8>)> {
    let (mut n, mut i) = (0usize, 0u64);
    std::iter::from_fn(move || {
        if n >= total {
            return None;
        }
        let size = [40_000usize, 300_000, 1_200_000, 5_000][i as usize % 4];
        let mut mail = format!("Subject: report {i}\r\nContent-Type: application/pdf\r\nContent-Transfer-Encoding: base64\r\n\r\n").into_bytes();
        mail.extend(base64(&noise(size, i + 1)));
        n += mail.len();
        i += 1;
        Some((format!("INBOX/1/{}.eml", i - 1), mail))
    })
}

/// Every file of `a1` as it reads back, by its length and a hash (`None`: it doesn't read).
fn digests(s: &Store) -> BTreeMap<String, Option<(usize, u64)>> {
    use std::hash::{Hash, Hasher};
    let p = packs(s);
    p.keys("a1", "")
        .unwrap()
        .into_iter()
        .map(|(k, _)| {
            let d = p.get("a1", &k).ok().flatten().map(|v| {
                let mut h = std::hash::DefaultHasher::new();
                v.hash(&mut h);
                (v.len(), h.finish())
            });
            (k, d)
        })
        .collect()
}

/// The probe: Compact Now on a data file from before R3 (its attachments' base64 kept raw) gives
/// back what compressing them saves, about a quarter, and every file reads back the same.
fn compact_now_gives_back_raw_attachments(total: usize) {
    let t = temp(&format!("raw-{total}"));
    let s = old_store_from(&t, raw_attachments(total));
    let before_files = digests(&s);
    let file = s.data_file("a1").unwrap();
    // What File › Info says it takes: the room on the disk.
    let size = crate::packed::on_disk(&std::fs::metadata(&file).unwrap());
    let began = std::time::Instant::now();
    // The file's size, and what it takes on the disk (a file grown by redb is sparse until written).
    let on_disk = |f: &std::path::Path| {
        #[cfg(unix)]
        return std::os::unix::fs::MetadataExt::blocks(&std::fs::metadata(f).unwrap()) * 512;
        #[cfg(not(unix))]
        return std::fs::metadata(f).unwrap().len();
    };
    let (mut recompressing, mut compacting, mut peak, mut peak_disk) = (None, None, size, on_disk(&file));
    let done = s
        .compact_now("a1", true, &AtomicBool::new(false), &mut |step| {
            peak = peak.max(std::fs::metadata(&file).map_or(0, |m| m.len()));
            peak_disk = peak_disk.max(on_disk(&file));
            match step {
                CompactStep::Recompressing(_) => recompressing = Some(began.elapsed()),
                CompactStep::Compacting => compacting = Some(began.elapsed()),
            }
        })
        .unwrap()
        .unwrap();
    let took = began.elapsed();
    let (before, after) = (done.before, done.after);
    let r = &done.recompressed;
    let phase1 = compacting.unwrap_or(took);
    eprintln!(
        "{} MB of raw attachments: data file {before} → {after} bytes ({:.3} of it) in {took:.2?}: compressed again {} files, \
         {} → {} bytes as kept, in {phase1:.2?} ({:.0} MB/s), the file at most {peak} bytes meanwhile ({peak_disk} on the disk); compacted in {:.2?}",
        total >> 20,
        after as f64 / before as f64,
        r.files,
        r.before,
        r.after,
        r.before as f64 / 1e6 / phase1.as_secs_f64(),
        took.saturating_sub(phase1),
    );
    assert!(recompressing.is_some() && compacting.is_some(), "both steps heard");
    assert_eq!(before, size);
    if room_told(before) {
        assert!(after * 100 <= before * 82, "Compact Now gave back {} of {before} bytes", before - after);
    }
    assert_eq!(digests(&s), before_files, "everything reads back the same");
    assert_eq!(meta(&s).deflate, super::super::DEFLATE_NOW, "the next one goes straight to compacting");
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

#[test]
fn compact_now_gives_back_what_raw_attachments_took() {
    compact_now_gives_back_raw_attachments(12 << 20);
}

/// The same on 200 MB: a measurement, run with `--release --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run alone with --release --ignored --nocapture"]
fn compact_now_on_200_mb_of_raw_attachments() {
    compact_now_gives_back_raw_attachments(200 << 20);
}

/// The same on about 1 GB (a 2.3 GB cache, scaled): a measurement, run with `--release
/// --ignored --nocapture` on a drive with room for it.
#[test]
#[ignore = "a measurement: run alone with --release --ignored --nocapture"]
fn compact_now_on_1_gb_of_raw_attachments() {
    compact_now_gives_back_raw_attachments(1 << 30);
}

/// The window beside Compact Now on 200 MB of raw attachments: a write from it (a message marked
/// read, on a local account) while attachments are made smaller waits at most for one batch's
/// commit; a read from it while the file is compacted is refused at once. A measurement of each
/// write and read, run with `--release --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run alone with --release --ignored --nocapture"]
fn the_window_beside_compact_now() {
    let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
    let at = |v: &[f64], q: f64| v.get(((v.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0.0);
    let t = temp("window-beside");
    let s = old_store_from(&t, raw_attachments(200 << 20));
    let key = MessageKey { account: "a1".into(), folder: "Drafts".into(), validity: 1, uid: 1 };
    s.write_body(&key, b"Subject: x\r\n\r\nx").unwrap();
    let phase = std::sync::Arc::new(AtomicU32::new(0));
    let running = {
        let (s, phase) = (s.clone(), phase.clone());
        std::thread::spawn(move || {
            s.compact_now("a1", true, &AtomicBool::new(false), &mut |step| {
                phase.store(if step == CompactStep::Compacting { 2 } else { 1 }, Ordering::SeqCst);
            })
            .unwrap()
            .unwrap()
        })
    };
    crate::no_waiting();
    let (mut writes, mut reads, mut refused) = (Vec::new(), Vec::new(), 0u32);
    while !running.is_finished() {
        match phase.load(Ordering::SeqCst) {
            1 => {
                let t0 = std::time::Instant::now();
                s.write_body(&key, format!("Subject: x\r\n\r\nread {}", writes.len()).as_bytes()).unwrap();
                writes.push(ms(t0.elapsed()));
            }
            2 => {
                let t0 = std::time::Instant::now();
                refused += u32::from(s.read_body(&key).is_none());
                reads.push(ms(t0.elapsed()));
            }
            _ => {}
        }
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
    let done = running.join().unwrap();
    writes.sort_by(f64::total_cmp);
    reads.sort_by(f64::total_cmp);
    eprintln!(
        "{} → {} bytes; {} writes from the window while attachments were made smaller: p50 {:.1} ms, p99 {:.1} ms, at most {:.1} ms; \
         {} reads from it while compacted ({refused} refused): at most {:.3} ms",
        done.before,
        done.after,
        writes.len(),
        at(&writes, 0.5),
        at(&writes, 0.99),
        writes.last().copied().unwrap_or(0.0),
        reads.len(),
        reads.last().copied().unwrap_or(0.0),
    );
    assert!(at(&writes, 0.99) < 500.0);
    assert!(reads.last().copied().unwrap_or(0.0) < 50.0);
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// A data file from before: base64 attachments kept raw (whole and in pieces, one of them bigger
/// than a batch), pictures kept raw (they don't compress), text compressed, and small files. Also
/// which keys are the base64 ones, and the pictures.
fn an_old_file(base64_files: u32) -> (Kept, Vec<String>, Vec<String>) {
    let (mut kept, mut b64, mut pictures) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..base64_files {
        let key = format!("INBOX/1/{i}.eml");
        let n = match i % 4 {
            0 => 2_000,
            1 => 30_000,
            2 => 120_000,
            _ => 9_000,
        } + (i as usize % 7) * 500;
        kept.push((key.clone(), base64(&noise(n, u64::from(i) + 1))));
        b64.push(key);
    }
    for i in 0..base64_files / 4 {
        let key = format!("Pictures/1/{i}.eml");
        kept.push((key.clone(), noise(5_000 + i as usize * 3_000, u64::from(i) + 7_000)));
        pictures.push(key);
    }
    for i in 0..base64_files / 4 {
        kept.push((format!("Notes/1/{i}.eml"), super::super::pack(&text(6_000, u64::from(i))).into_owned()));
        kept.push((format!("Tiny/1/{i}.eml"), b"Subject: x\r\n\r\nshort".to_vec()));
    }
    // One bigger than a batch: a batch of its own.
    let big = "Big/1/1.eml".to_string();
    kept.push((big.clone(), base64(&noise(9 << 20, 99))));
    b64.push(big);
    (kept, b64, pictures)
}

/// A note for a file in pieces, with none of them there.
fn damaged(s: &Store, key: &str) {
    let db = packs(s).db("a1", false).unwrap().unwrap();
    let tx = db.begin_write().unwrap();
    {
        let mut note = super::super::PIECED.to_vec();
        note.extend_from_slice(&(200_000u64).to_le_bytes());
        tx.open_table(super::super::FILES).unwrap().insert(key, note.as_slice()).unwrap();
    }
    tx.commit().unwrap();
}

/// A data file from before compressed again: the base64 is kept compressed now, in about three
/// quarters of the room; the pictures and what was compressed already are left as they are; a
/// damaged file doesn't stop it; every file reads back the same. Its progress goes up to the
/// whole. Then the file says it's all kept as this build keeps it, and a second run has nothing
/// to do. One run at a time: another asking meanwhile is refused.
#[test]
fn an_old_data_file_is_compressed_again_and_reads_back_the_same() {
    let t = temp("recompress");
    let (kept, b64, pictures) = an_old_file(240);
    let s = old_store(&t, &kept);
    damaged(&s, "Damaged/1/1.eml");
    let before = read_back(&s);
    assert_eq!(before.get("Damaged/1/1.eml"), Some(&None));
    assert_eq!(meta(&s).deflate, 0, "a file from before says nothing");
    let (mut heard, mut refused) = (Vec::<Recompressing>::new(), None);
    let stop = AtomicBool::new(false);
    let r = s
        .recompress_data_file("a1", &stop, &mut |w| {
            if refused.is_none() {
                refused = Some(s.recompress_data_file("a1", &AtomicBool::new(false), &mut |_| {}).is_err());
            }
            heard.push(w);
        })
        .unwrap()
        .unwrap();
    eprintln!("{r:?}");
    assert_eq!(refused, Some(true), "a second run at once");
    assert!(r.finished);
    assert_eq!((r.files, r.stayed_raw, r.changed), (b64.len() as u64, pictures.len() as u64, 0));
    assert_eq!((r.passed_over, r.passed_over_bytes), (1, 200_000), "the damaged file, counted");
    let b64_bytes: u64 = kept.iter().filter(|(k, _)| b64.contains(k)).map(|(_, v)| v.len() as u64).sum();
    assert_eq!(r.before, b64_bytes);
    assert!(r.after * 100 <= r.before * 80, "kept in {} of {}", r.after, r.before);
    assert_eq!(r.saved_in_all, r.before - r.after);
    assert_eq!(read_back(&s), before, "everything reads back the same");
    assert!(b64.iter().all(|k| kept_compressed(&s, k)));
    assert!(pictures.iter().all(|k| !kept_compressed(&s, k)), "left as they were");

    // Progress: up to the whole, never back.
    let last = *heard.last().unwrap();
    assert!(heard.windows(2).all(|w| w[0].done <= w[1].done));
    assert_eq!(last.done, last.total);
    assert_eq!(last.total, before.len() as u64);
    assert!(heard.len() > 3, "{} batches", heard.len() - 1);

    let m = meta(&s);
    assert!(m.deflate == super::super::DEFLATE_NOW && m.cursor.is_none(), "{m:?}");
    let again = s.recompress_data_file("a1", &stop, &mut |_| {}).unwrap().unwrap();
    assert!(again.finished && again.files == 0 && again.saved_in_all == r.saved_in_all, "{again:?}");
    drop(s);

    // A data file this build made has nothing to do.
    let t2 = temp("recompress-new");
    let s = Store::open_packed(t2.join("app"), t2.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    s.write_body(&k, &base64(&noise(50_000, 1))).unwrap();
    assert_eq!(meta(&s).deflate, super::super::DEFLATE_NOW, "said as it was made");
    let r = s.recompress_data_file("a1", &stop, &mut |_| {}).unwrap().unwrap();
    assert!(r.finished && r.files == 0);
    assert_eq!(s.recompress_data_file("nobody", &stop, &mut |_| {}).unwrap(), None);
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
    let _ = std::fs::remove_dir_all(&t2);
}

/// Every write made, as for an account never removed.
fn always(write: &mut dyn FnMut() -> std::io::Result<()>) -> Option<std::io::Result<()>> {
    Some(write())
}

/// A run whose first batch has files written again (whole and in pieces), and one removed,
/// between reading them and writing them back compressed: those are left as written (none
/// brought back), and the others in the batch are compressed all the same.
struct Meddling<'a> {
    inner: Account<'a>,
    commits: AtomicU32,
}

impl Target for Meddling<'_> {
    fn db(&self) -> std::io::Result<std::sync::Arc<redb::Database>> {
        self.inner.db()
    }

    fn commit(&self, f: &mut dyn FnMut(&mut Tables<'_>, &redb::WriteTransaction) -> std::io::Result<()>) -> std::io::Result<()> {
        // The first batch's (the run's start is the first commit).
        if self.commits.fetch_add(1, Ordering::Relaxed) == 1 {
            self.inner.packs.change(self.inner.account, |t| {
                t.put("A/1.eml", b"Subject: written again\r\n\r\nnew")?;
                t.put_kept("A/2.eml", &base64(&noise(40_000, 77)))?;
                t.take_kept("A/3.eml").map(drop)
            })?;
        }
        self.inner.commit(f)
    }

    // As the store's: what's written meanwhile is seen as written, though the batch was read with
    // nothing under way.
    fn writes(&self) -> Option<u64> {
        self.inner.writes()
    }

    fn none_since(&self, seen: u64) -> bool {
        self.inner.none_since(seen)
    }
}

#[test]
fn a_file_written_while_it_is_compressed_is_left_as_written() {
    let t = temp("recompress-race");
    let mut kept: Kept = vec![
        ("A/1.eml".into(), base64(&noise(2_000, 1))),
        ("A/2.eml".into(), base64(&noise(30_000, 2))),
        ("A/3.eml".into(), base64(&noise(9_000, 3))),
        ("A/4.eml".into(), base64(&noise(20_000, 4))),
    ];
    kept.extend((0..40u32).map(|i| (format!("B/1/{i}.eml"), base64(&noise(60_000, u64::from(i) + 10)))));
    let s = old_store(&t, &kept);
    let mut before = read_back(&s);
    let p = packs(&s);
    let target = Meddling { inner: Account { packs: p, account: "a1", kept: &always }, commits: AtomicU32::new(0) };
    let r = super::super::recompress::recompress(&target, &AtomicBool::new(false), &mut |_| {}).unwrap();
    eprintln!("{r:?}");
    assert!(r.finished);
    assert_eq!((r.changed, r.files), (3, kept.len() as u64 - 3));
    before.insert("A/1.eml".into(), Some(b"Subject: written again\r\n\r\nnew".to_vec()));
    before.insert("A/2.eml".into(), Some(base64(&noise(40_000, 77))));
    before.remove("A/3.eml");
    assert_eq!(read_back(&s), before);
    assert!(!kept_compressed(&s, "A/2.eml"), "the file as written meanwhile");
    assert!(kept_compressed(&s, "A/4.eml") && kept_compressed(&s, "B/1/39.eml"));
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// A run whose account is removed between reading a batch and writing it back (`Removing`), or
/// between two batches: the run stops, and neither brings back a data file for the account gone
/// nor names one for it again (the removed account's file stays, as Outlook keeps a `.pst`).
struct Removing<'a> {
    inner: Account<'a>,
    store: &'a Store,
    commits: AtomicU32,
}

impl Target for Removing<'_> {
    fn db(&self) -> std::io::Result<std::sync::Arc<redb::Database>> {
        self.inner.db()
    }

    fn commit(&self, f: &mut dyn FnMut(&mut Tables<'_>, &redb::WriteTransaction) -> std::io::Result<()>) -> std::io::Result<()> {
        // The first batch's (after the run's start).
        if self.commits.fetch_add(1, Ordering::Relaxed) == 1 {
            self.store.remove_account("a1").unwrap();
        }
        self.inner.commit(f)
    }
}

/// The data files in `Files/`.
fn data_files(t: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(t.join("Files")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

#[test]
fn a_recompress_never_brings_back_the_data_file_of_an_account_removed() {
    let t = temp("recompress-removed");
    let (kept, _, _) = an_old_file(80);
    let s = old_store(&t, &kept);
    let p = packs(&s);
    let seen = s.removals("a1");
    let kept_while = |w: &mut dyn FnMut() -> std::io::Result<()>| s.while_kept("a1", seen, w);
    let target = Removing { inner: Account { packs: p, account: "a1", kept: &kept_while }, store: &s, commits: AtomicU32::new(0) };
    let r = super::super::recompress::recompress(&target, &AtomicBool::new(false), &mut |_| {});
    assert_eq!(r.map_err(|e| e.kind()).err(), Some(std::io::ErrorKind::NotFound), "stopped");
    assert_eq!(data_files(&t), ["old.quatrelle"], "no file made");
    assert_eq!(p.file("a1"), None, "none named");
    drop(s);

    // Through the store, removed between two batches: no run, no file; and Compact Now the same.
    for whole in [false, true] {
        let t2 = temp(&format!("recompress-removed-between-{whole}"));
        let s = old_store(&t2, &kept);
        let mut heard = 0;
        let mut removing = || {
            heard += 1;
            if heard == 2 {
                s.remove_account("a1").unwrap();
            }
        };
        if whole {
            let r = s.compact_now("a1", true, &AtomicBool::new(false), &mut |_| removing()).unwrap();
            assert_eq!(r, None);
        } else {
            let r = s.recompress_data_file("a1", &AtomicBool::new(false), &mut |_| removing()).unwrap();
            assert_eq!(r, None);
        }
        assert_eq!(data_files(&t2), ["old.quatrelle"]);
        assert_eq!(packs(&s).file("a1"), None);
        drop(s);
        let _ = std::fs::remove_dir_all(&t2);
    }
    let _ = std::fs::remove_dir_all(&t);
}

/// A run whose first batch is read, then files kept raw that it hasn't reached are moved behind
/// where it is (to keys that batch covers, or before it), before the batch is written back.
struct Moving<'a> {
    inner: Account<'a>,
    moves: Vec<(&'static str, &'static str)>,
    commits: AtomicU32,
}

impl Target for Moving<'_> {
    fn db(&self) -> std::io::Result<std::sync::Arc<redb::Database>> {
        self.inner.db()
    }

    fn commit(&self, f: &mut dyn FnMut(&mut Tables<'_>, &redb::WriteTransaction) -> std::io::Result<()>) -> std::io::Result<()> {
        if self.commits.fetch_add(1, Ordering::Relaxed) == 1 {
            let app = &self.inner.packs.root;
            for (from, to) in &self.moves {
                self.inner.packs.rename(&app.join("a1").join(from), &app.join("a1").join(to)).unwrap();
            }
        }
        self.inner.commit(f)
    }
}

/// Files kept raw moved during a run (mail moved from a folder it hasn't reached to one it has
/// passed) are compressed all the same before the run says the file is done: moved as a batch is
/// being written, between two batches, or while the run is stopped.
#[test]
fn a_file_moved_behind_a_recompress_is_compressed_too() {
    let moves = [("Z/1/119.eml", "A/1/0b.eml"), ("Z/1/118.eml", "A/0/1.eml")];
    let t = temp("recompress-moved");
    let kept: Kept =
        (0..120u32).map(|i| (format!("{}/1/{i}.eml", if i < 60 { "A" } else { "Z" }), base64(&noise(150_000, u64::from(i) + 1)))).collect();
    let s = old_store(&t, &kept);
    let mut before = read_back(&s);
    let p = packs(&s);
    // As a batch is written: a run under way (as `Packs::recompress` says it is).
    Packs::lock(&p.recompressing).insert("a1".into());
    let target = Moving { inner: Account { packs: p, account: "a1", kept: &always }, moves: moves.to_vec(), commits: AtomicU32::new(0) };
    let r = super::super::recompress::recompress(&target, &AtomicBool::new(false), &mut |_| {}).unwrap();
    Packs::lock(&p.recompressing).remove("a1");
    eprintln!("{r:?}");
    assert!(r.finished);
    for (from, to) in moves {
        let v = before.remove(from).unwrap();
        before.insert(to.to_string(), v);
        assert!(kept_compressed(&s, to), "{to}");
    }
    assert_eq!(read_back(&s), before);
    assert_eq!(meta(&s).deflate, super::super::DEFLATE_NOW);
    drop(s);

    // Between two batches, and while stopped: through the store.
    let t2 = temp("recompress-moved-between");
    let s = old_store(&t2, &kept);
    let mut before = read_back(&s);
    let app = t2.join("app").join("a1");
    let stop = AtomicBool::new(false);
    let mut heard = 0;
    let first = s
        .recompress_data_file("a1", &stop, &mut |_| {
            heard += 1;
            if heard == 3 {
                packs(&s).rename(&app.join(moves[0].0), &app.join(moves[0].1)).unwrap();
            }
            stop.store(heard >= 3, Ordering::Relaxed);
        })
        .unwrap()
        .unwrap();
    assert!(!first.finished);
    packs(&s).rename(&app.join(moves[1].0), &app.join(moves[1].1)).unwrap();
    let rest = s.recompress_data_file("a1", &AtomicBool::new(false), &mut |_| {}).unwrap().unwrap();
    assert!(rest.finished, "{rest:?}");
    for (from, to) in moves {
        let v = before.remove(from).unwrap();
        before.insert(to.to_string(), v);
        assert!(kept_compressed(&s, to), "{to}");
    }
    assert_eq!(read_back(&s), before);
    assert_eq!(meta(&s).deflate, super::super::DEFLATE_NOW);
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
    let _ = std::fs::remove_dir_all(&t2);
}

/// A run stopped after its first batch says so in `META`, with what it did on the disk; the next
/// run, from a store opened again, goes on from where it stopped (its progress starts there), and
/// between them every file was compressed once. Compact Now stopped there isn't compacted.
#[test]
fn a_stopped_recompress_goes_on_from_where_it_stopped() {
    let t = temp("recompress-stop");
    let (kept, b64, _) = an_old_file(160);
    let s = old_store(&t, &kept);
    let before = read_back(&s);
    let stop = AtomicBool::new(false);
    let mut heard = 0;
    let first = s
        .compact_now("a1", true, &stop, &mut |_| {
            heard += 1;
            stop.store(heard >= 2, Ordering::Relaxed);
        })
        .unwrap()
        .unwrap();
    eprintln!("stopped: {first:?}");
    let first = first.recompressed;
    assert!(!first.finished && first.files > 0 && first.files < b64.len() as u64);
    let m = meta(&s);
    assert!(m.cursor.is_some() && m.deflate == 0 && m.looked > 0, "{m:?}");
    assert_eq!(read_back(&s), before);
    drop(s);

    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    let mut starts = None;
    let rest = s
        .recompress_data_file("a1", &AtomicBool::new(false), &mut |w| {
            starts.get_or_insert(w);
        })
        .unwrap()
        .unwrap();
    eprintln!("went on: {rest:?} from {starts:?}");
    let starts = starts.unwrap();
    assert!(starts.done > 0 && starts.done < starts.total, "{starts:?}");
    assert!(rest.finished);
    assert_eq!(first.files + rest.files, b64.len() as u64, "each compressed once");
    assert_eq!(rest.saved_in_all, first.before - first.after + rest.before - rest.after);
    assert_eq!(read_back(&s), before);
    assert!(b64.iter().all(|k| kept_compressed(&s, k)));
    let m = meta(&s);
    assert!(m.deflate == super::super::DEFLATE_NOW && m.cursor.is_none(), "{m:?}");
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// Killed between two batches (after a different one each time): the data file opens without
/// being gone through first (each batch is sealed), every file reads back the same, and a run
/// then finishes the job. The one killed is this test run again.
#[test]
fn a_recompress_killed_part_way_loses_nothing() {
    const CHILD: &str = "QUATRELLE_TEST_KILLED_RECOMPRESS";
    if let Some(dir) = std::env::var_os(CHILD) {
        let dir = std::path::PathBuf::from(dir);
        let batches: u32 = std::env::var("QUATRELLE_TEST_BATCHES").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
        let s = Store::open_packed(dir.join("app"), dir.join("Files")).unwrap();
        let heard = AtomicU32::new(0);
        let _ = s.recompress_data_file("a1", &AtomicBool::new(false), &mut |_| {
            // Heard as it starts, then after each batch. No destructor runs.
            if heard.fetch_add(1, Ordering::Relaxed) == batches {
                std::process::exit(3);
            }
        });
        std::process::exit(0);
    }
    for batches in [1u32, 2, 4] {
        let t = temp(&format!("recompress-kill-{batches}"));
        let (kept, b64, _) = an_old_file(400);
        let before = {
            let s = old_store(&t, &kept);
            read_back(&s)
        };
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "packed::tests::recompress::a_recompress_killed_part_way_loses_nothing", "--test-threads=1"])
            .env(CHILD, &t)
            .env("QUATRELLE_TEST_BATCHES", batches.to_string())
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(3), "{batches}: the child was to stop dead");
        let repaired = std::rc::Rc::new(std::cell::Cell::new(false));
        let seen = repaired.clone();
        let db = redb::Database::builder().set_repair_callback(move |_| seen.set(true)).create(t.join("Files").join("old.quatrelle")).unwrap();
        assert!(!repaired.get(), "{batches}: no repair was needed");
        drop(db);
        let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
        assert_eq!(read_back(&s), before, "{batches}: after the kill");
        assert!(meta(&s).cursor.is_some(), "{batches}: where it got to");
        let r = s.recompress_data_file("a1", &AtomicBool::new(false), &mut |_| {}).unwrap().unwrap();
        assert!(r.finished, "{batches}: {r:?}");
        assert_eq!(read_back(&s), before, "{batches}: once finished");
        assert!(b64.iter().all(|k| kept_compressed(&s, k)), "{batches}");
        drop(s);
        let _ = std::fs::remove_dir_all(&t);
    }
}

/// A hostile `META` (values of the wrong length, a cursor that isn't text or is longer than any
/// key, a count past everything, a pull-back longer than any key): the file reads back the same,
/// and Compact Now compresses it again from the start, then compacts. One whose cursor is past
/// every key stops cleanly: it reads back the same, compacted, nothing compressed.
#[test]
fn a_hostile_meta_never_stops_compact_now() {
    let kept: Kept = (0..30u32).map(|i| (format!("INBOX/1/{i}.eml"), base64(&noise(60_000, u64::from(i) + 1)))).collect();
    let long = "x".repeat(5000);
    let cases: Vec<(&str, Vec<(&str, Vec<u8>)>, bool)> = vec![
        ("short values", vec![("deflate", vec![2, 0]), ("recompress.saved", vec![1]), ("recompress.looked", vec![9; 3])], true),
        ("not text", vec![("recompress.cursor", vec![0xff, 0xfe, 0x00]), ("recompress.looked", u64::MAX.to_le_bytes().to_vec())], true),
        ("too long", vec![("recompress.cursor", long.clone().into_bytes()), ("recompress.back", long.clone().into_bytes())], true),
        ("past everything", vec![("recompress.cursor", b"zzzz".to_vec()), ("recompress.looked", u64::MAX.to_le_bytes().to_vec())], false),
    ];
    for (name, values, compressed) in cases {
        let t = temp(&format!("recompress-hostile-{}", name.replace(' ', "-")));
        let s = old_store(&t, &kept);
        {
            let db = packs(&s).db("a1", false).unwrap().unwrap();
            let tx = db.begin_write().unwrap();
            {
                let mut m = tx.open_table(super::super::META).unwrap();
                for (k, v) in &values {
                    m.insert(*k, v.as_slice()).unwrap();
                }
            }
            tx.commit().unwrap();
        }
        let before = read_back(&s);
        assert!(before.values().all(Option::is_some), "{name}: readable");
        let mut heard = Vec::new();
        let done = s.compact_now("a1", true, &AtomicBool::new(false), &mut |step| heard.push(step)).unwrap().unwrap();
        eprintln!("{name}: {done:?}");
        assert!(heard.iter().all(|h| !matches!(h, CompactStep::Recompressing(w) if w.done > w.total)), "{name}: {heard:?}");
        assert_eq!(heard.last(), Some(&CompactStep::Compacting), "{name}: compacted");
        assert_eq!(read_back(&s), before, "{name}: reads back the same");
        assert_eq!(done.recompressed.files == kept.len() as u64, compressed, "{name}");
        drop(s);
        let _ = std::fs::remove_dir_all(&t);
    }

    // `META` that isn't a table of this kind: Compact Now compacts all the same.
    let t = temp("recompress-hostile-kind");
    let s = old_store(&t, &kept);
    {
        let db = packs(&s).db("a1", false).unwrap().unwrap();
        let tx = db.begin_write().unwrap();
        let wrong: redb::TableDefinition<u64, u64> = redb::TableDefinition::new("meta");
        tx.open_table(wrong).unwrap().insert(1, 2).unwrap();
        tx.commit().unwrap();
    }
    let before = read_back(&s);
    let done = s.compact_now("a1", true, &AtomicBool::new(false), &mut |_| {}).unwrap().unwrap();
    assert_eq!(read_back(&s), before);
    if room_told(done.before) {
        assert!(done.after <= done.before);
    }
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// A fuzz input of records (`fuzz_db`): a value in `FILES`, `PIECES` or `META`.
fn record(out: &mut Vec<u8>, kind: u8, key: &str, piece: Option<u8>, value: &[u8], times: u8) {
    out.push(kind);
    out.push(key.len() as u8);
    out.extend_from_slice(key.as_bytes());
    if let Some(n) = piece {
        out.push(n);
    }
    out.extend_from_slice(&(value.len() as u16).to_le_bytes());
    out.push(times);
    out.extend_from_slice(value);
}

/// The fuzz target's input that every kind of file is in (its seed, `fuzz/seeds/data_file`).
fn fuzz_seed() -> Vec<u8> {
    let mut i = Vec::new();
    let line = b"QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVphYmNkZWZnaGlqa2xtbm9wcXJzdHV2d3h5ejAxMjM0\r\n";
    record(&mut i, b'F', "INBOX/1/1.eml", None, line, 40);
    let mut note = b"\0Quatrelle:pieces\0".to_vec();
    note.extend_from_slice(&(line.len() as u64 * 100).to_le_bytes());
    record(&mut i, b'F', "INBOX/1/2.eml", None, &note, 1);
    record(&mut i, b'P', "INBOX/1/2.eml", Some(0), line, 50);
    record(&mut i, b'P', "INBOX/1/2.eml", Some(1), line, 50);
    record(&mut i, b'F', "INBOX/1/3.eml", None, b"\0Quatrelle:pieces\0\xff\xff\xff\xff\0\0\0\0", 1);
    record(&mut i, b'F', "INBOX/1/4.eml", None, b"\0Quatrelle:deflate\0\x03\0", 1);
    record(&mut i, b'F', "INBOX/search.idx", None, line, 30);
    record(&mut i, b'F', "Sent/1/1.eml", None, &noise(3000, 5), 1);
    record(&mut i, b'M', "recompress.cursor", None, b"INBOX/1/1.eml", 1);
    record(&mut i, b'M', "recompress.back", None, b"INBOX", 1);
    record(&mut i, b'M', "deflate", None, &1u64.to_le_bytes(), 1);
    i
}

/// The fuzz target's hook, on its seed (every file reads back the same once compressed again), and
/// on that seed damaged every way a few hundred times (never a panic).
#[test]
fn the_fuzz_targets_data_files_read_back_the_same() {
    let seed = fuzz_seed();
    assert!(crate::fuzz_data_file(&seed));
    let written = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds/data_file/kinds");
    if std::env::var_os("QUATRELLE_WRITE_SEED").is_some() {
        std::fs::create_dir_all(written.parent().unwrap()).unwrap();
        std::fs::write(&written, &seed).unwrap();
    }
    assert_eq!(std::fs::read(&written).ok().as_deref(), Some(seed.as_slice()), "fuzz/seeds/data_file/kinds is the seed made here");
    let mut x = 0x2545_f491_4f6c_dd1du64;
    for round in 0..300 {
        let mut input = seed.clone();
        for _ in 0..1 + round % 8 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let at = (x >> 8) as usize % input.len();
            input[at] = (x >> 40) as u8;
        }
        assert!(crate::fuzz_data_file(&input), "round {round}");
    }
}
