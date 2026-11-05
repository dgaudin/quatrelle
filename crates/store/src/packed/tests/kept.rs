// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! How a file is kept: in pieces, compressed when that helps.

use super::*;

/// Big files go in pieces that fill their pages (a whole value took the next power of two
/// of pages: imported mail came to more than twice its size), and come back whole, through
/// renames, removals, overwrites and copies into another data file.
#[test]
fn big_files_are_kept_in_pieces_and_come_back_whole() {
    let t = temp("pieces");
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    // Bytes that don't compress, so they're kept in pieces as they are.
    let big = noise(1_234_567, 1);
    let edge = noise(super::super::WHOLE_MAX + 1, 2);
    let small = b"Subject: small

x"
    .to_vec();
    let key = |uid: u32| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
    s.write_bodies(&[(key(1), big.clone()), (key(2), edge.clone()), (key(3), small.clone())]).unwrap();
    assert_eq!(s.read_body(&key(1)).unwrap(), big);
    assert_eq!(s.read_body(&key(2)).unwrap(), edge);
    assert_eq!(s.read_body(&key(3)).unwrap(), small);
    assert!(s.has_body(&key(1)));
    let dir = s.body_path(&key(1)).parent().unwrap().to_path_buf();
    // Sizes are what the files take (compressed: this test's bytes repeat), not the notes'.
    let kept = s.files.size(&dir);
    assert!(kept > 1_000 && kept <= (big.len() + edge.len() + small.len()) as u64, "{kept}");
    // Overwritten smaller: no pieces left behind; and bigger again.
    s.write_body(&key(1), &small).unwrap();
    assert_eq!(s.read_body(&key(1)).unwrap(), small);
    s.write_body(&key(1), &big).unwrap();
    assert_eq!(s.read_body(&key(1)).unwrap(), big);
    // A folder renamed takes its pieces.
    s.save_folders("a1", &[quatrelle_model::Folder { account: "a1".into(), path: "INBOX".into(), selectable: true, ..Default::default() }]).unwrap();
    s.apply_op("a1", &quatrelle_model::Op::RenameFolder { from: "INBOX".into(), to: "Kept".into() }).unwrap();
    let kept = |uid: u32| MessageKey { folder: "Kept".into(), ..key(uid) };
    assert_eq!(s.read_body(&kept(1)).unwrap(), big);
    assert!(s.read_body(&key(1)).is_none());
    // Removed: gone, pieces and all.
    s.remove_body(&kept(2));
    assert!(s.read_body(&kept(2)).is_none() && !s.has_body(&kept(2)));
    // Copied into another account's data file: whole.
    let file = s.data_file("a1").unwrap();
    drop(s);
    let other = Store::open_packed(t.join("app2"), t.join("Files2")).unwrap();
    other.save_accounts(&[account("b1", "ana@example.org")]).unwrap();
    other.import_data_file("b1", &file).unwrap();
    assert_eq!(other.read_body(&MessageKey { account: "b1".into(), ..kept(1) }).unwrap(), big);
    // The pieces fill their pages: many messages of awkward sizes leave little unused (a whole
    // value each left 40% of the file unused).
    let t2 = temp("pieces-waste");
    let s = Store::open_packed(t2.join("app"), t2.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    let bodies: Vec<(MessageKey, Vec<u8>)> =
        (0..120u32).map(|i| (key(i), noise(70_000 + (i as usize * 2_731) % 200_000, u64::from(i) + 9))).collect();
    let stored: usize = bodies.iter().map(|(_, b)| b.len()).sum();
    s.write_bodies(&bodies).unwrap();
    let file2 = s.data_file("a1").unwrap();
    drop(s);
    let pages = |path: &std::path::Path| {
        let db = redb::Database::open(path).unwrap();
        let st = db.begin_write().unwrap().stats().unwrap();
        st.allocated_pages() * st.page_size() as u64
    };
    let pieced = pages(&file2);
    // The same bodies as whole values, for comparison.
    let whole_file = t2.join("whole.redb");
    {
        let db = redb::Database::create(&whole_file).unwrap();
        let tx = db.begin_write().unwrap();
        {
            let mut table = tx.open_table(super::super::FILES).unwrap();
            for (k, b) in &bodies {
                table.insert(format!("m{}", k.uid).as_str(), b.as_slice()).unwrap();
            }
        }
        tx.commit().unwrap();
    }
    let whole = pages(&whole_file);
    assert!(pieced < stored as u64 * 112 / 100, "pieces took {pieced} for {stored}");
    assert!(pieced * 100 < whole * 85, "pieces {pieced}, whole values {whole}");
    let _ = std::fs::remove_dir_all(t2);
    let sizes = super::super::piece_sizes(big.len());
    assert_eq!(sizes.iter().sum::<usize>(), big.len());
    assert!(sizes.iter().all(|n| *n <= 16 * super::super::PAGE - super::super::PAGE_SLACK));
    assert!(super::super::piece_sizes(0).is_empty() && super::super::piece_sizes(1) == [1]);
    let _ = std::fs::remove_dir_all(t);
}

/// A file in pieces is read back as written whether it's kept compressed (uncompressed as its
/// pieces are read) or not, and every byte of a compressed one comes from its pieces in order;
/// one with a piece gone, at its start, in its middle or at its end, is told as damaged (taken
/// as missing), never handed back short.
#[test]
fn a_file_in_pieces_reads_back_compressed_or_not() {
    let t = temp("pieces-read");
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    let key = |uid: u32| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
    // An attachment's base64 (kept compressed, in about 40 pieces) and noise (kept as it is).
    let attached: Vec<u8> = b"Subject: sheet\r\n\r\n".iter().copied().chain(base64(&noise(2_000_000, 5))).collect();
    let raw = noise(1_500_000, 6);
    assert!(super::super::pack(&attached).starts_with(super::super::DEFLATED) && super::super::pack(&attached).len() > 16 * super::super::PAGE * 4);
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    let pieces_of = |uid: u32| -> (String, usize) {
        let inside = s.body_path(&key(uid)).strip_prefix(t.join("app").join("a1")).unwrap().to_str().unwrap().replace('\\', "/");
        let db = packs.db("a1", false).unwrap().unwrap();
        let tx = redb::ReadableDatabase::begin_read(&*db).unwrap();
        let n = tx.open_table(super::super::PIECES).unwrap().range((inside.as_str(), 0)..=(inside.as_str(), u32::MAX)).unwrap().count();
        (inside, n)
    };
    for (uid, bytes) in [(1, &attached), (2, &raw)] {
        s.write_body(&key(uid), bytes).unwrap();
        assert_eq!(s.read_body(&key(uid)).as_ref(), Some(bytes));
        let (_, n) = pieces_of(uid);
        assert!(n > 3, "{n} pieces");
        // A piece gone: the first, one in the middle, the last.
        for (i, gone) in [0, n / 2, n - 1].into_iter().enumerate() {
            let uid2 = uid * 10 + i as u32;
            s.write_body(&key(uid2), bytes).unwrap();
            let (inside, _) = pieces_of(uid2);
            let db = packs.db("a1", false).unwrap().unwrap();
            let tx = db.begin_write().unwrap();
            tx.open_table(super::super::PIECES).unwrap().remove((inside.as_str(), gone as u32)).unwrap();
            tx.commit().unwrap();
            assert_eq!(s.load_body(&key(uid2)).unwrap(), None, "uid {uid}: piece {gone} of {n} gone");
            let e = packs.read(&s.body_path(&key(uid2))).unwrap_err();
            assert_eq!(e.kind(), std::io::ErrorKind::InvalidData, "uid {uid}: piece {gone} of {n} gone: {e}");
        }
    }
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}

/// Text (mail, folder lists) is kept compressed and reads back as written; what doesn't
/// compress is kept as it is; files from before compression still read; and a crafted
/// compressed value that would come to more than the cap is refused, not inflated.
#[test]
fn text_is_kept_compressed() {
    let t = temp("deflate");
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    let key = |uid: u32| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
    let text: Vec<u8> = "Subject: report

"
    .bytes()
    .chain("The figures for October are in the attached sheet. ".repeat(4000).bytes())
    .collect();
    s.write_body(&key(1), &text).unwrap();
    assert_eq!(s.read_body(&key(1)).unwrap(), text);
    let dir = s.body_path(&key(1)).parent().unwrap().to_path_buf();
    assert!(s.files.size(&dir) * 20 < text.len() as u64, "kept in {} bytes", s.files.size(&dir));
    let random = noise(5000, 3);
    assert_eq!(super::super::pack(&random).as_ref(), random.as_slice(), "not compressed when it doesn't help");
    assert_eq!(super::super::pack(b"short").as_ref(), b"short");
    assert_eq!(super::super::unpack(b"plain old bytes".to_vec()).unwrap(), b"plain old bytes", "from before compression");
    // A crafted value: a deflate stream of more than the cap.
    let bomb = {
        use std::io::Write;
        let mut z = flate2::write::DeflateEncoder::new(super::super::DEFLATED.to_vec(), flate2::Compression::best());
        let zeros = vec![0u8; 1 << 20];
        for _ in 0..(super::super::INFLATE_MAX >> 20) + 2 {
            z.write_all(&zeros).unwrap();
        }
        z.finish().unwrap()
    };
    assert!(super::super::unpack(bomb).is_err());
    let _ = std::fs::remove_dir_all(t);
}

/// An attachment's base64 is kept compressed, at most 0.80 of its size: base64 carries six bits
/// in eight, so even random bytes shrink by about a quarter. (The fast level of flate2's zlib-rs
/// backend, which the app's build turned on, kept it at 0.995, so it was stored raw.)
#[test]
fn base64_attachments_are_kept_compressed() {
    let text = base64(&noise(1 << 20, 7));
    let kept = super::super::pack(&text);
    assert!(kept.starts_with(super::super::DEFLATED), "stored raw");
    assert!(kept.len() * 100 <= text.len() * 80, "kept {} of {} bytes", kept.len(), text.len());
    assert_eq!(super::super::unpack(kept.into_owned()).unwrap(), text);
}

/// `pack` keeps exactly what compressing the whole file and then applying the tenth rule would
/// have kept, right up to the line, from text that compresses well to noise that doesn't.
#[test]
fn packing_keeps_what_the_tenth_rule_would() {
    let (mut raw, mut deflated) = (0, 0);
    for noisy in (0..=100).step_by(2) {
        let len = 40_000;
        let noisy_len = len * noisy / 100;
        let mut bytes = noise(noisy_len, noisy as u64 + 1);
        bytes.extend("The figures for October are in the attached sheet. ".bytes().cycle().take(len - noisy_len));
        let stream = miniz_oxide::deflate::compress_to_vec(&bytes, 1);
        let expected = if super::super::DEFLATED.len() + stream.len() >= len - len / 10 {
            raw += 1;
            bytes.clone()
        } else {
            deflated += 1;
            [super::super::DEFLATED, &stream].concat()
        };
        assert_eq!(super::super::pack(&bytes).as_ref(), expected.as_slice(), "{noisy}% noise");
    }
    assert!(raw > 0 && deflated > 0, "both sides of the line: {raw} raw, {deflated} compressed");
}

/// A packed attachment holds no more memory than eight tenths of its source, and a file that
/// comes to between eight and nine tenths is still kept compressed, in no more than nine.
#[test]
fn packing_reserves_what_an_attachment_comes_to() {
    let text = base64(&noise(1 << 20, 11));
    let kept = super::super::pack(&text).into_owned();
    assert!(kept.starts_with(super::super::DEFLATED), "stored raw");
    assert!(kept.capacity() * 10 <= text.len() * 8, "{} held for {} bytes", kept.capacity(), text.len());
    let mut grown = 0;
    for noisy in (0..=100).step_by(2) {
        let len = 40_000;
        let mut bytes = noise(len * noisy / 100, noisy as u64 + 1);
        bytes.extend("The figures for October are in the attached sheet. ".bytes().cycle().take(len - bytes.len()));
        let kept = super::super::pack(&bytes).into_owned();
        if kept.starts_with(super::super::DEFLATED) && kept.len() * 10 > len * 8 {
            grown += 1;
            assert!(kept.capacity() * 10 <= len * 9, "{} held for {len} bytes", kept.capacity());
            assert_eq!(super::super::unpack(kept).unwrap(), bytes, "{noisy}% noise");
        }
    }
    assert!(grown > 0, "no file came to between eight and nine tenths");
}
