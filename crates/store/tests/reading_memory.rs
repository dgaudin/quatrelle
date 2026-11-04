// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A big message read back from a data file is held once: its pieces are uncompressed as they're
//! read, not joined whole, compressed, beside what they uncompress to. Measured by the process's
//! peak resident memory (Linux), in a test binary of its own so no other test adds to it.

#[cfg(target_os = "linux")]
#[test]
fn a_big_source_is_held_once_while_it_is_read() {
    use quatrelle_model::{AccountConfig, MessageKey};
    use quatrelle_store::Store;

    /// Bytes that don't compress (xorshift).
    fn noise(n: usize, seed: u64) -> Vec<u8> {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect()
    }
    /// Base64, as attachments travel in mail, in lines of 76.
    fn base64(bytes: &[u8]) -> Vec<u8> {
        const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::with_capacity(bytes.len() / 3 * 4 + bytes.len() / 57 * 2 + 8);
        for (i, c) in bytes.chunks(3).enumerate() {
            let n = c.iter().enumerate().fold(0u32, |n, (j, b)| n | (u32::from(*b) << (16 - 8 * j)));
            for j in 0..4 {
                out.push(if j <= c.len() { ABC[((n >> (18 - 6 * j)) & 63) as usize] } else { b'=' });
            }
            if i % 19 == 18 {
                out.extend_from_slice(b"\r\n");
            }
        }
        out
    }
    /// A line of /proc/self/status, in bytes.
    fn status(field: &str) -> u64 {
        let s = std::fs::read_to_string("/proc/self/status").unwrap();
        let line = s.lines().find(|l| l.starts_with(field)).unwrap();
        line.split_whitespace().nth(1).unwrap().parse::<u64>().unwrap() * 1024
    }

    let t = std::env::temp_dir().join(format!("quatrelle-read-memory-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    let account = AccountConfig { id: "a1".into(), email: "zoe@studio.example".into(), ..Default::default() };
    s.save_accounts(&[account]).unwrap();
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    // 60 MB of base64, kept compressed in about 46 MB of pieces: both past the size from which the
    // allocator maps (and gives back) each buffer of its own, so each is counted as it's filled.
    let source: Vec<u8> = b"Subject: figures\r\n\r\n".iter().copied().chain(base64(&noise(45 << 20, 3))).collect();
    s.write_body(&key, &source).unwrap();
    drop(source);
    // A first read fills the database's own page cache; the second is measured.
    let len = s.read_body(&key).unwrap().len() as u64;
    // Peak resident memory set back to what is resident now.
    std::fs::write("/proc/self/clear_refs", "5").unwrap();
    let before = status("VmRSS:");
    let read = s.read_body(&key).unwrap();
    let peak = status("VmHWM:").saturating_sub(before);
    eprintln!("read {len} bytes: peak {peak} bytes above before ({:.2} of the source)", peak as f64 / len as f64);
    assert_eq!(read.len() as u64, len);
    assert!(peak * 10 < len * 13, "reading a {len}-byte source peaked {peak} bytes above before");
    drop((read, s));
    let _ = std::fs::remove_dir_all(t);
}

/// A crafted data file whose note says a file in pieces is 1 GiB long, with no pieces at all,
/// doesn't make the reader set 1 GiB aside (on Windows, memory set aside is committed: a low
/// commit limit and the reservation fails, which ends the process).
#[cfg(target_os = "linux")]
#[test]
fn a_note_that_claims_a_huge_file_sets_little_aside() {
    use quatrelle_model::{AccountConfig, MessageKey};
    use quatrelle_store::Store;
    use redb::TableDefinition;
    const FILES: TableDefinition<&str, &[u8]> = TableDefinition::new("files");
    const PIECES: TableDefinition<(&str, u32), &[u8]> = TableDefinition::new("pieces");
    /// The process's peak virtual memory: what it has set aside at most, touched or not.
    fn peak() -> u64 {
        let s = std::fs::read_to_string("/proc/self/status").unwrap();
        let line = s.lines().find(|l| l.starts_with("VmPeak:")).unwrap();
        line.split_whitespace().nth(1).unwrap().parse::<u64>().unwrap() * 1024
    }

    let t = std::env::temp_dir().join(format!("quatrelle-read-reserve-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    let (root, files) = (t.join("app"), t.join("Files"));
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let (inside, file) = {
        let s = Store::open_packed(&root, &files).unwrap();
        let account = AccountConfig { id: "a1".into(), email: "zoe@studio.example".into(), ..Default::default() };
        s.save_accounts(&[account]).unwrap();
        s.write_body(&key, b"Subject: hello\r\n\r\nShort.\r\n").unwrap();
        let path = s.body_path(&key);
        (path.strip_prefix(root.join("a1")).unwrap().to_str().unwrap().replace('\\', "/"), s.data_file("a1").unwrap())
    };
    {
        let db = redb::Database::open(&file).unwrap();
        let tx = db.begin_write().unwrap();
        {
            let mut note = b"\0Quatrelle:pieces\0".to_vec();
            note.extend_from_slice(&(1u64 << 30).to_le_bytes());
            tx.open_table(FILES).unwrap().insert(inside.as_str(), note.as_slice()).unwrap();
            tx.open_table(PIECES).unwrap();
        }
        tx.commit().unwrap();
    }
    let s = Store::open_packed(&root, &files).unwrap();
    let before = peak();
    assert!(s.load_body(&key).is_err() || s.read_body(&key).is_none());
    let set_aside = peak().saturating_sub(before);
    eprintln!("a note claiming 1 GiB: the peak of memory set aside rose {set_aside} bytes");
    assert!(set_aside < 256 << 20, "{set_aside} bytes set aside for a file of no pieces");
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}
