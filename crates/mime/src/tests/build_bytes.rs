// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What `build` writes, kept byte for byte.

use super::*;

/// What `build` writes, byte for byte, for a message with every kind of part: three files (one
/// an attached message with 8-bit text), two pictures its HTML shows, as written and as a
/// signed message must be (7-bit). The digests were taken before `build` stopped copying its
/// parts, and again when the pictures went out under their names (upstream's, the same here);
/// the `X-Mailer` line, which carries the version, is left out.
#[test]
fn build_writes_the_same_bytes_as_before() {
    // Its length and FNV-1a, the X-Mailer line left out of both.
    let digest = |raw: &[u8]| {
        let kept = || raw.split_inclusive(|b| *b == b'\n').filter(|l| !l.starts_with(b"X-Mailer:"));
        (
            kept().map(<[u8]>::len).sum::<usize>(),
            kept().flatten().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)),
        )
    };
    let all_bytes: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
    let d = Draft {
        to: vec![EmailAddress::new("Ann", "ann@studio.example")],
        cc: vec![EmailAddress::new("", "kim@studio.example")],
        subject: "Boards and notes ✏️".into(),
        body: "Hi Ann,\r\nthe boards, the notes and the old thread.\r\n\r\n> earlier\r\n".into(),
        in_reply_to: "m1@studio.example".into(),
        references: vec!["m1@studio.example".into()],
        html: Some("<p>Hi <b>Ann</b></p><img src=\"cid:a@x\"><img src=\"cid:b@x\">".into()),
        attachments: vec![
            OutAttachment { name: "board.pdf".into(), mime: String::new(), data: all_bytes.clone(), ..Default::default() },
            OutAttachment { name: "notes.txt".into(), mime: "text/plain".into(), data: "café\r\n".repeat(40).into_bytes(), ..Default::default() },
            OutAttachment {
                name: "old.eml".into(),
                mime: "message/rfc822".into(),
                data: "Subject: old\r\n\r\nnaïve\r\n".into(),
                ..Default::default()
            },
        ],
        inline: vec![
            InlinePicture {
                cid: "a@x".into(),
                name: "a.png".into(),
                mime: "image/png".into(),
                data: all_bytes.get(..700).unwrap_or_default().to_vec(),
            },
            InlinePicture { cid: "b@x".into(), name: "b.gif".into(), mime: String::new(), data: vec![0x47, 0x49, 0x46, 0, 255] },
        ],
        importance: 1,
        read_receipt: true,
        ..Default::default()
    };
    let from = EmailAddress::new("Zoe Park", "zoe@studio.example");
    let raw = build(&d, &from, 1_791_963_000, "g1@studio.example").unwrap();
    let signed = build(&Draft { seven_bit: true, ..d.clone() }, &from, 1_791_963_000, "g1@studio.example").unwrap();
    assert_eq!(digest(&raw), (10_095, 7_850_003_861_301_803_885));
    assert_eq!(digest(&signed), (10_107, 17_852_924_195_389_485_945));
}
