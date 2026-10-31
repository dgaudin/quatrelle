//! What a crafted `winmail.dat` can cost: the output, the memory and the passes over it stay
//! bounded however it's built (measured on crafted inputs, review of 2026-10-31).

use super::*;

/// A link around text whose formatting changes again and again: the link is written once.
#[test]
fn a_link_stays_open_while_its_formatting_changes() {
    let rtf = format!(
        "{{\\rtf1{{\\field{{\\*\\fldinst HYPERLINK \"http://x.example/{}\"}}{{\\fldrslt {}}}}}}}",
        "a".repeat(4000),
        "\\b x\\b0 y".repeat(2000)
    );
    let TnefBody::Html(h) = rtf_body(rtf.as_bytes()) else { panic!("no HTML") };
    assert_eq!(h.matches("<a href=").count(), 1);
    assert_eq!(h.matches("<b>x</b>").count(), 2000);
    // Before: the 4 KB link written for each of the 4000 changes, 16 MB.
    assert!(h.len() < 64 << 10, "{} bytes", h.len());
}

/// The pictures a body shows, found in one pass: by `src`, by a style's `url()`, never by a
/// longer reference that starts like one.
#[test]
fn the_pictures_a_body_shows_are_found_in_one_pass() {
    let body = TnefBody::Html("<img src=\"cid:one@x\"><div style=\"background:url(cid:two@x)\"></div><img src='cid:three@xy'>".into());
    let shown = shown_cids(&body, ["one@x", "two@x", "three@x", "four@x"]);
    let mut shown: Vec<_> = shown.into_iter().collect();
    shown.sort();
    assert_eq!(shown, ["one@x", "two@x"]);
    assert!(shown_cids(&TnefBody::Text("cid:one@x".into()), ["one@x"]).is_empty());
    // References one after the other with nothing between: each read no further than the longest
    // Content-ID (before: to the end of the body each time).
    let html = TnefBody::Html("cid:".repeat(4 << 20));
    let started = std::time::Instant::now();
    assert!(shown_cids(&html, ["a@x"]).is_empty());
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
}

/// The review's case: a big body (16 MB of HTML from 2.8 KB of RTF there, 8 MB of HTML here)
/// and 500 files with Outlook-length Content-IDs the body never shows. Each file looked through
/// the whole body (13 s a pass in the review's release build); the body is now read once.
#[test]
fn many_pictures_and_a_big_body_read_in_one_pass() {
    let cids: Vec<String> = (0..500).map(|i| format!("{}{i}", "a".repeat(40))).collect();
    let mut a = Vec::new();
    attr(&mut a, 1, ATT_MSG_PROPS, &props(&[(0x0102, PR_HTML, binary(format!("<p>{}</p>", "x".repeat(8 << 20)).as_bytes()))]));
    for c in &cids {
        attr(&mut a, 2, ATT_REND_DATA, &[]);
        attr(&mut a, 2, ATT_ATTACHMENT, &props(&[(0x001E, PR_ATTACH_CONTENT_ID, str8(c))]));
        attr(&mut a, 2, ATT_DATA, b"x");
    }
    let raw = outlook_message(stream(&a));
    let started = std::time::Instant::now();
    let data = crate::attachment_data(&raw, 499);
    assert_eq!(data.as_deref(), Some(&b"x"[..]));
    assert!(started.elapsed() < std::time::Duration::from_secs(2), "{:?}", started.elapsed());
}

/// Compressed RTF as the review's bench writes it: literals, and references repeating the last
/// `period` bytes (MS-OXRTFCP), so a few kilobytes make megabytes.
struct Lz {
    out: Vec<u8>,
    control: usize,
    bit: u8,
    write: usize,
    raw: usize,
}

impl Lz {
    fn new() -> Self {
        Lz { out: Vec::new(), control: 0, bit: 8, write: 207, raw: 0 }
    }
    fn slot(&mut self, reference: bool) {
        if self.bit == 8 {
            self.control = self.out.len();
            self.out.push(0);
            self.bit = 0;
        }
        if reference && let Some(c) = self.out.get_mut(self.control) {
            *c |= 1 << self.bit;
        }
        self.bit += 1;
    }
    fn lit(&mut self, b: &[u8]) {
        for c in b {
            self.slot(false);
            self.out.push(*c);
            self.write = (self.write + 1) % 4096;
            self.raw += 1;
        }
    }
    fn repeat(&mut self, period: usize, mut n: usize) {
        while n >= 2 {
            let len = n.min(17);
            let offset = (self.write + 4096 - period) % 4096;
            self.slot(true);
            self.out.extend((((offset as u16) << 4) | (len as u16 - 2)).to_be_bytes());
            self.write = (self.write + len) % 4096;
            self.raw += len;
            n -= len;
        }
    }
    fn finish(self) -> Vec<u8> {
        let mut v = ((self.out.len() + 12) as u32).to_le_bytes().to_vec();
        v.extend((self.raw as u32).to_le_bytes());
        v.extend(0x7546_5A4Cu32.to_le_bytes());
        v.extend(0u32.to_le_bytes());
        v.extend(self.out);
        v
    }
}

/// Compressed RTF of `n` tabs: a few kilobytes that read as about 24 bytes of HTML each.
fn tabs_rtf(n: usize) -> Vec<u8> {
    let mut lz = Lz::new();
    lz.lit(b"{\\rtf1 \\tab");
    lz.repeat(4, 4 * n.saturating_sub(1));
    lz.lit(b"}");
    lz.finish()
}

fn rtf_body_props(rtf: &[u8]) -> Vec<u8> {
    props(&[(0x0102, PR_RTF_COMPRESSED, binary(rtf))])
}

/// Attached messages, each with a body far bigger than its bytes: one budget for all of them
/// (before: each its own 16 MB, 30 of 3 KB held 500 MB of .eml in the review's bench). With a
/// small budget, what's held stays within it, and bodies past it aren't read at all.
#[test]
fn attached_messages_share_one_budget() {
    let mut inner = Vec::new();
    attr(&mut inner, 1, ATT_MSG_PROPS, &rtf_body_props(&tabs_rtf(40_000)));
    let inner = stream(&inner);
    let mut object = vec![0u8; 16];
    object.extend(&inner);
    let mut a = Vec::new();
    for _ in 0..30 {
        attr(&mut a, 2, ATT_REND_DATA, &[]);
        attr(
            &mut a,
            2,
            ATT_ATTACHMENT,
            &props(&[(0x0003, PR_ATTACH_METHOD, 5i32.to_le_bytes().to_vec()), (0x000D, PR_ATTACH_DATA, binary(&object))]),
        );
    }
    let outer = stream(&a);
    RTF_READ.with(|n| n.set(0));
    let mut budget = Budget { bytes: 1 << 20, files: MAX_FILES, cut: false };
    let t = read_with(&outer, &mut budget, true).unwrap_or_default();
    let held: usize = t.attachments.iter().map(|f| f.data.len()).sum();
    assert!(held <= (1 << 20) + 2 * outer.len(), "{held} bytes held from {} bytes", outer.len());
    assert!(budget.cut && !t.attachments.is_empty());
    assert!(RTF_READ.with(|n| n.get()) < 30);
    // With the real budget, one such message is read whole, body and all.
    let mut one = Vec::new();
    attr(&mut one, 2, ATT_REND_DATA, &[]);
    attr(&mut one, 2, ATT_ATTACHMENT, &props(&[(0x0003, PR_ATTACH_METHOD, 5i32.to_le_bytes().to_vec()), (0x000D, PR_ATTACH_DATA, binary(&object))]));
    let t = read_tnef(&stream(&one)).unwrap_or_default();
    assert!(t.attachments.first().is_some_and(|f| f.data.len() > 900_000));
}

/// A message with several winmail.dat parts: the body of the first only is read, and their files
/// count together against `MAX_FILES`; a part cut short stays listed as itself, one past the
/// budget too, so no file is lost.
#[test]
fn winmail_dat_parts_share_the_files_and_one_body() {
    let part = |n: usize| {
        let mut a = Vec::new();
        attr(&mut a, 1, ATT_MSG_PROPS, &rtf_body_props(&compress(b"{\\rtf1 Hello}")));
        for i in 0..n {
            attr(&mut a, 2, ATT_REND_DATA, &[]);
            attr(&mut a, 2, ATT_TITLE, format!("f{i}.txt\0").as_bytes());
            attr(&mut a, 2, ATT_DATA, b"x");
        }
        stream(&a)
    };
    let raw = MessageBuilder::new()
        .from("a@b.example")
        .subject("x")
        .text_body("")
        .attachment("application/ms-tnef", "winmail.dat", part(300))
        .attachment("application/ms-tnef", "winmail.dat", part(300))
        .attachment("application/ms-tnef", "winmail.dat", part(300))
        .write_to_vec()
        .unwrap_or_default();
    RTF_READ.with(|n| n.set(0));
    let p = crate::parse(&raw).unwrap_or_default();
    assert_eq!(RTF_READ.with(|n| n.get()), 1);
    assert!(p.text.contains("Hello"));
    let dat = p.attachments.iter().filter(|a| a.name == "winmail.dat").count();
    assert_eq!((p.attachments.len() - dat, dat), (MAX_FILES, 2));
}
