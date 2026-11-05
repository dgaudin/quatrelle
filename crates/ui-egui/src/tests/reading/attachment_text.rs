// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a text file in another charset previews in it.

/// The pane read its bytes as they came (Save's) as UTF-8, so Windows-1252 and UTF-16 files
/// showed replacement characters.
#[test]
fn a_text_attachment_previews_in_its_charset() {
    let raw = b"From: Sam <sam@studio.example>\r\nSubject: Notes\r\nMIME-Version: 1.0\r\n\
        Content-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n--b1\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHere.\r\n\
        --b1\r\nContent-Type: text/plain; charset=windows-1252\r\nContent-Disposition: attachment; filename=\"latin.txt\"\r\n\
        Content-Transfer-Encoding: 8bit\r\n\r\nCaf\xe9 au lait\r\n\
        --b1\r\nContent-Type: text/plain; charset=utf-16\r\nContent-Disposition: attachment; filename=\"wide.txt\"\r\n\
        Content-Transfer-Encoding: quoted-printable\r\n\r\n=FF=FEh=00i=00\r\n--b1--\r\n";
    let p = quatrelle_engine::mime::parse(raw).unwrap();
    assert_eq!(p.attachments.len(), 2);
    let shown: Vec<String> = p.attachments.iter().map(|a| crate::reading::previewed_text(raw, a).unwrap()).collect();
    assert!(shown[0].starts_with("Caf\u{e9} au lait"), "{:?}", shown[0]);
    assert_eq!(shown[1].trim_end(), "hi");
    assert!(shown.iter().all(|t| !t.contains('\u{fffd}')));
    // Saved, the Windows-1252 file is still its own bytes.
    assert!(quatrelle_engine::mime::attachment_data(raw, 0).unwrap().starts_with(b"Caf\xe9"));
}
