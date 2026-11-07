// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A message rebuilt from its item, and its summary: what the summary takes from the item before
//! the item goes into the message.

use super::super::item::{File, Item};
use super::super::rebuild::{message, summary_of, to_raw};
use super::*;

fn key() -> MessageKey {
    MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 }
}

fn summary(it: Item) -> quatrelle_model::Summary {
    let (raw, glance) = message(it, "s@x").unwrap();
    summary_of(&raw, glance, key(), u32::try_from(raw.len()).unwrap())
}

/// A reply's own words come after 10 KiB of the message it quotes: the preview finds them, in
/// its text and in the text made of its HTML (which the message's text part is too).
#[test]
fn a_preview_behind_ten_kib_of_quoted_text() {
    let quoted = "> The plan for the week, as it was agreed on Monday.\r\n".repeat(200);
    assert!(quoted.len() > 10 * 1024);
    let plain = Item { body: format!("{quoted}\r\nSounds good, see you then."), ..Default::default() };
    assert_eq!(summary(plain).preview, "Sounds good, see you then.");
    let lines = "<p>&gt; The plan for the week, as it was agreed on Monday.</p>".repeat(200);
    let html = Item { html: Some(format!("<html><body>{lines}<p>Sounds <span>good</span>.</p></body></html>")), ..Default::default() };
    assert_eq!(summary(html).preview, "Sounds good.");
    // A plain body is read through, as before: words past 64K characters of quoted text are found.
    let far = Item { body: format!("{}Too far in.", "> quoted\r\n".repeat(7000)), ..Default::default() };
    assert_eq!(summary(far).preview, "Too far in.");
}

/// The summary takes the item's flags, date and files, and the message is the one `to_raw`
/// writes: the item's files are in it, moved rather than copied.
#[test]
fn the_summary_and_the_message_come_from_one_item() {
    let picture = b"\x89PNG\r\n\x1a\npicture".to_vec();
    let item = || Item {
        subject: "Boards".into(),
        from: EmailAddress::new("Ana", "ana@example.com"),
        date: 1_704_067_200,
        body: "The boards are attached.".into(),
        flags: Flags { seen: true, flagged: true, ..Default::default() },
        files: vec![File::Data { name: "board.png".into(), mime: "image/png".into(), cid: String::new(), data: picture.clone() }],
        ..Default::default()
    };
    let (raw, glance) = message(item(), "s@x").unwrap();
    assert_eq!(raw, to_raw(item(), "s@x").unwrap());
    let s = summary_of(&raw, glance, key(), 7);
    assert!(s.flags.seen && s.flags.flagged && s.has_attachments, "{s:?}");
    assert_eq!((s.date, s.size, s.subject.as_str(), s.preview.as_str()), (1_704_067_200, 7, "Boards", "The boards are attached."));
    let p = mime::parse(&raw).unwrap();
    assert_eq!(p.attachments.len(), 1);
    assert!(!summary(Item { body: "x".into(), ..Default::default() }).has_attachments);
}

/// Rebuilding a message with a 32 MiB file, measured by the process's peak resident memory above
/// what was resident before (Linux). Alone in its process to mean anything:
/// `cargo test -p quatrelle-net --lib -- --ignored --test-threads=1 rebuilt::memory`.
#[cfg(target_os = "linux")]
#[test]
#[ignore]
fn memory_of_a_message_with_a_big_file() {
    fn status(field: &str) -> u64 {
        let s = std::fs::read_to_string("/proc/self/status").unwrap();
        let line = s.lines().find(|l| l.starts_with(field)).unwrap();
        line.split_whitespace().nth(1).unwrap().parse::<u64>().unwrap() * 1024
    }
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    let data: Vec<u8> = (0..32 << 20)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    let len = data.len() as u64;
    let it = Item {
        subject: "Boards".into(),
        from: EmailAddress::new("Ana", "ana@example.com"),
        date: 1_704_067_200,
        body: "The boards are attached.".into(),
        headers: "From: Ana <ana@example.com>\r\nSubject: Boards\r\nMessage-ID: <b@example.com>\r\n".into(),
        files: vec![File::Data { name: "boards.bin".into(), mime: "application/octet-stream".into(), cid: String::new(), data }],
        ..Default::default()
    };
    std::fs::write("/proc/self/clear_refs", "5").unwrap();
    let before = status("VmRSS:");
    let raw = to_raw(it, "s@x").unwrap();
    let peak = status("VmHWM:").saturating_sub(before);
    eprintln!("a {len}-byte file: peak {peak} bytes above before ({:.2} of the file), source {} bytes", peak as f64 / len as f64, raw.len());
}

/// A received item keeps when it was sent as its date (its header's `Date:`) and is placed by
/// when it was delivered; one delivered as it was dated keeps no second time (adapted from
/// upstream 62a459d3).
#[test]
fn a_summary_is_placed_by_delivery() {
    let header = Item {
        headers: "From: Ana <ana@example.com>\r\nDate: Mon, 1 Jan 2024 00:00:00 +0000\r\nSubject: Plan\r\n".into(),
        date: 1_704_070_800,
        received: 1_704_070_800,
        body: "b".into(),
        ..Default::default()
    };
    let s = summary(header);
    assert_eq!((s.date, s.received, s.received_at()), (1_704_067_200, 1_704_070_800, 1_704_070_800));
    let own = Item { date: 1_704_067_200, received: 1_704_067_200, body: "b".into(), ..Default::default() };
    let s = summary(own);
    assert_eq!((s.date, s.received, s.received_at()), (1_704_067_200, 0, 1_704_067_200));
}

/// A received message's header is put back in the built source's own buffer without doubling
/// it: the source waits in its import batch in that buffer, and a file that won't compress is
/// stored as it is.
#[test]
fn the_original_header_goes_back_without_doubling_the_source() {
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    let data: Vec<u8> = (0..2 << 20)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    let received = "Received: from relay.example.com by mx.example.org; Mon, 1 Jan 2024 00:00:00 +0000\r\n".repeat(80);
    let item = Item {
        headers: format!("{received}From: Ana <ana@example.com>\r\nSubject: Scans\r\nContent-Type: multipart/mixed\r\n"),
        body: "The scans.".into(),
        files: vec![File::Data { name: "scans.bin".into(), mime: "application/octet-stream".into(), cid: String::new(), data }],
        ..Default::default()
    };
    let raw = to_raw(item, "s@x").unwrap();
    assert!(raw.starts_with(b"Received: "), "the original header is back");
    assert!(raw.capacity() - raw.len() <= raw.len() / 100, "{} held for {} bytes", raw.capacity(), raw.len());
}
