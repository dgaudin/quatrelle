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

/// `written` signed by Riley, and encrypted to Riley, as they wait in the Outbox.
fn signed_and_encrypted(written: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let (outer, entity) = quatrelle_mime::smime::entity_of(written);
    let signature = smime::sign(&entity, &riley(), LATER, &mut rand_core::OsRng).unwrap();
    let signed = quatrelle_mime::smime::signed_message(&outer, &entity, &signature, "=_o1");
    let sealed = smime::encrypt(&entity, std::slice::from_ref(&riley().chain[0]), smime::Cipher::Aes256Gcm, &mut rand_core::OsRng).unwrap();
    (signed, quatrelle_mime::smime::encrypted_message(&outer, &sealed, true))
}

/// The message `n` from Riley to Ana, with `text`.
fn written(n: u32, text: &str) -> Vec<u8> {
    format!(
        "From: Riley <riley@studio.example>\r\nTo: ana@posteo.example\r\nSubject: Figures\r\nDate: Sat, 24 Oct 2026 09:00:00 +0000\r\n\
Message-ID: <o{n}@studio.example>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}\r\n"
    )
    .into_bytes()
}

fn outbox_item(id: u64) -> quatrelle_store::OutboxItem {
    quatrelle_store::OutboxItem { id, from: "riley@studio.example".into(), recipients: vec!["ana@posteo.example".into()], ..Default::default() }
}

/// The Outbox row message `id` is in (its key counts rows).
fn outbox_key(s: &Session, id: u64) -> quatrelle_model::MessageKey {
    let row = s.outbox.iter().position(|(_, i)| i.id == id).unwrap();
    quatrelle_model::MessageKey { account: "sample".into(), folder: crate::session::OUTBOX.into(), validity: 0, uid: u32::try_from(row + 1).unwrap() }
}

/// A signed message waiting in the Outbox, taken back to be changed, opened as the signature's
/// own file, and an encrypted one as the encryption's: taken out of the Outbox, the message was
/// gone. It opens as it was written, to be signed or encrypted again as it goes; an encrypted
/// one once it's been decrypted, and never while it can't be read.
#[test]
fn a_protected_message_in_the_outbox_opens_as_it_was_written() {
    let mut s = with_certificate("smime-outbox");
    s.run("security.defaults", &json!({"sign": false, "encrypt": false})).unwrap();
    let store = s.store.clone().unwrap();
    let (signed, encrypted) = signed_and_encrypted(&written(1, "The figures, as agreed."));
    store.outbox_add("sample", &outbox_item(71), &encrypted).unwrap();
    store.outbox_add("sample", &outbox_item(72), &signed).unwrap();
    s.refresh_outbox();
    s.run("folder.outbox", &json!({})).unwrap();
    // The encrypted one: not while it can't be read, and it stays where it was.
    let k = outbox_key(&s, 71);
    let err = s.run("outbox.edit", &json!({"key": k.to_id()})).unwrap_err().to_string();
    assert!(err.contains("encrypted") && s.compose.is_none() && s.outbox.len() == 2, "{err}");
    // Shown, it's decrypted; then it opens as written, to be encrypted again.
    s.run("list.open", &json!({"key": k.to_id()})).unwrap();
    answer_decrypt(&mut s);
    s.run("outbox.edit", &json!({"key": k.to_id()})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert!(c.body.contains("The figures, as agreed.") && c.encrypt && !c.sign, "{}", c.body);
    assert!(c.attachments.is_empty() && c.subject == "Figures", "{:?}", c.attachments.iter().map(|a| &a.name).collect::<Vec<_>>());
    s.run("compose.discard", &json!({})).unwrap();
    // The signed one: as written, signed again as it goes.
    let k = outbox_key(&s, 72);
    s.run("outbox.edit", &json!({"key": k.to_id()})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert!(c.body.contains("The figures, as agreed.") && c.sign && !c.encrypt, "{}", c.body);
    assert!(c.attachments.is_empty(), "no signature's file in it");
    assert!(s.outbox.is_empty());
    done(s);
}

/// An Outbox key counts rows: what was decrypted under one is that message's, not the one the
/// row holds once a row above it goes (it was shown as the other, and opened to be changed as
/// it). An answer asked for before the rows moved is let pass.
#[test]
fn what_was_decrypted_in_the_outbox_stays_with_its_message() {
    let mut s = with_certificate("smime-outbox-rows");
    let store = s.store.clone().unwrap();
    for (id, text) in [(81, "The first."), (82, "The second."), (83, "The third.")] {
        let (_, encrypted) = signed_and_encrypted(&written(u32::try_from(id).unwrap(), text));
        store.outbox_add("sample", &outbox_item(id), &encrypted).unwrap();
    }
    s.refresh_outbox();
    s.run("folder.outbox", &json!({})).unwrap();
    let shown_text = |s: &Session| s.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.text.clone()).unwrap_or_default();
    // The first, decrypted; the second, asked for and not answered yet.
    s.run("list.open", &json!({"key": outbox_key(&s, 81).to_id()})).unwrap();
    answer_decrypt(&mut s);
    s.run("list.open", &json!({"key": outbox_key(&s, 81).to_id()})).unwrap();
    assert!(shown_text(&s).contains("The first."), "{}", shown_text(&s));
    s.run("list.open", &json!({"key": outbox_key(&s, 82).to_id()})).unwrap();
    let late: Vec<Job> = s.take_jobs().into_iter().filter(|j| matches!(j, Job::Decrypt { .. })).collect();
    assert_eq!(late.len(), 1);
    // The first deleted: the second is in its row, the third in the second's.
    s.run("list.open", &json!({"key": outbox_key(&s, 81).to_id()})).unwrap();
    s.run("mail.delete", &json!({})).unwrap();
    assert_eq!((outbox_key(&s, 82).uid, outbox_key(&s, 83).uid), (1, 2));
    s.take_jobs();
    s.run("list.open", &json!({"key": outbox_key(&s, 82).to_id()})).unwrap();
    assert!(!shown_text(&s).contains("The first."), "another's text: {}", shown_text(&s));
    assert_eq!(security(&s).lock, Some(Lock::Opening), "asked for anew");
    s.take_jobs();
    // The second's answer, asked under the row the third has now, comes: not the third's.
    for j in late {
        if let Job::Decrypt { key, cms, ask, .. } = j {
            let (c, authenticated) = smime::decrypt(&cms, &riley(), &mut rand_core::OsRng).unwrap();
            s.apply(Event::Decrypted { key, ask, content: Some(quatrelle_model::Private(c)), authenticated, error: None });
        }
    }
    s.run("list.open", &json!({"key": outbox_key(&s, 83).to_id()})).unwrap();
    assert!(!shown_text(&s).contains("The second."), "another's text: {}", shown_text(&s));
    answer_decrypt(&mut s);
    assert!(shown_text(&s).contains("The third."), "{}", shown_text(&s));
    done(s);
}

/// Each answer is decrypted on a thread of its own, so they come in any order. A message asked
/// for in the Outbox's first row, then sent or deleted, and the one after it asked for under the
/// same row: the second's answer came first and was let pass as the first's, then the first's
/// was taken for the row (its text under the other's headers, opened to be changed as it, the
/// other's text lost). Only the answer to the last asking is the row's.
#[test]
fn answers_for_an_outbox_row_coming_out_of_order_stay_with_their_messages() {
    let mut s = with_certificate("smime-outbox-order");
    let store = s.store.clone().unwrap();
    for (id, text) in [(91, "What Ana wrote."), (92, "What Bo wrote.")] {
        let (_, encrypted) = signed_and_encrypted(&written(u32::try_from(id).unwrap(), text));
        store.outbox_add("sample", &outbox_item(id), &encrypted).unwrap();
    }
    s.refresh_outbox();
    s.run("folder.outbox", &json!({})).unwrap();
    let decrypts = |s: &mut Session| -> Vec<Job> { s.take_jobs().into_iter().filter(|j| matches!(j, Job::Decrypt { .. })).collect() };
    // The first, asked for and not answered yet; then it goes, and the second, now in its row,
    // is asked for.
    s.run("list.open", &json!({"key": outbox_key(&s, 91).to_id()})).unwrap();
    let first = decrypts(&mut s);
    s.run("mail.delete", &json!({})).unwrap();
    let mut second = decrypts(&mut s);
    s.run("list.open", &json!({"key": outbox_key(&s, 92).to_id()})).unwrap();
    second.extend(decrypts(&mut s));
    assert_eq!((first.len(), second.len()), (1, 1));
    assert_eq!(outbox_key(&s, 92).uid, 1);
    // The second's answer first, then the first's.
    for j in second.into_iter().chain(first) {
        let Job::Decrypt { key, cms, ask, .. } = j else { continue };
        let (c, authenticated) = smime::decrypt(&cms, &riley(), &mut rand_core::OsRng).unwrap();
        s.apply(Event::Decrypted { key, ask, content: Some(quatrelle_model::Private(c)), authenticated, error: None });
    }
    let key = outbox_key(&s, 92);
    let raw = s.as_written(&key, store.outbox_raw("sample", 92).unwrap()).0;
    let text = String::from_utf8_lossy(&raw).into_owned();
    assert!(text.contains("What Bo wrote.") && !text.contains("What Ana wrote."), "{text}");
    let shown = s.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.text.clone()).unwrap_or_default();
    assert!(shown.contains("What Bo wrote."), "{shown}");
    s.run("outbox.edit", &json!({"key": key.to_id()})).unwrap();
    let body = s.compose.as_ref().map(|c| c.body.clone()).unwrap_or_default();
    assert!(body.contains("What Bo wrote.") && !body.contains("What Ana wrote."), "{body}");
    done(s);
}
