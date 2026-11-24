// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a message kept signed or encrypted (a draft on the server, a message waiting in
//! the Outbox) opens as it was written, and goes as protected as it was to.

use super::*;

/// A draft kept encrypted (a message to be sent encrypted is, on the server) opened again forgot
/// that, and went in the clear unless the account encrypts everything. It's to be sent encrypted
/// still; and before it's been decrypted it doesn't open at all, rather than as the file that
/// encrypts it.
#[test]
fn a_draft_kept_encrypted_is_still_sent_encrypted() {
    let mut s = with_certificate("smime-draft");
    s.run("security.defaults", &json!({"sign": false, "encrypt": false})).unwrap();
    let sealed = smime::encrypt(
        b"Content-Type: text/plain; charset=utf-8\r\n\r\nThe figures, as agreed.\r\n",
        std::slice::from_ref(&riley().chain[0]),
        smime::Cipher::Aes256Gcm,
        &mut rand_core::OsRng,
    )
    .unwrap();
    let outer = b"From: Riley <riley@studio.example>\r\nTo: ana@posteo.example\r\nSubject: Figures\r\nDate: Sat, 24 Oct 2026 09:00:00 +0000\r\nMessage-ID: <d1@studio.example>\r\n";
    let raw = quatrelle_mime::smime::encrypted_message(outer, &sealed, true);
    let key = open_raw(&mut s, 9371, &raw);
    let err = s.run("draft.edit", &json!({"key": key.to_id()})).unwrap_err().to_string();
    assert!(err.contains("encrypted") && s.compose.is_none(), "{err}");
    answer_decrypt(&mut s);
    s.run("draft.edit", &json!({"key": key.to_id()})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert!(c.body.contains("The figures, as agreed."), "{}", c.body);
    assert!(c.encrypt && !c.encrypt_auto, "to be sent encrypted, as it was kept");
    s.run("compose.discard", &json!({})).unwrap();
    // One kept in the clear opens as the account's settings say.
    let plain =
        b"From: Riley <riley@studio.example>\r\nTo: ana@posteo.example\r\nSubject: Lunch\r\nMessage-ID: <d2@studio.example>\r\n\r\nAt one?\r\n";
    let key = open_raw(&mut s, 9372, plain);
    s.run("draft.edit", &json!({"key": key.to_id()})).unwrap();
    assert!(!s.compose.as_ref().unwrap().encrypt);
    done(s);
}
