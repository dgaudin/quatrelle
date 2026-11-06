// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: sending with OpenPGP on the sample mailbox (Riley's account). Signed, then
//! encrypted to every recipient and to the sender; a recipient without a key that can be
//! encrypted to is named and nothing goes; with both a certificate and a key, Automatic answers in
//! the scheme a message came in. Nothing changes for an account without a key, byte for byte.
//! The network layer's part (signing and encrypting) is played here with the pgp crate, as the
//! pool does.

use super::*;
use crate::{NO_KEY_TO_ENCRYPT, Scheme};
use quatrelle_model::Private;

/// What the sample account filed in Sent last.
fn last_sent(s: &Session) -> Vec<u8> {
    let store = s.store.clone().unwrap();
    let sent = s.account("sample").and_then(|a| a.folder(FolderRole::Sent)).map(|f| f.path.clone()).unwrap();
    let st = store.load_folder("sample", &sent).unwrap();
    let newest = st.messages.iter().max_by_key(|m| m.key.uid).unwrap();
    store.read_body(&newest.key).unwrap()
}

/// The source the sample account sent before OpenPGP could send anything (recorded before lot 4;
/// `{VERSION}`: Quatrelle's, in `X-Mailer`), without the empty line mail-builder 1 wrote before
/// the first boundary (mail-builder 2 writes none).
const BEFORE: &str = "From: \"Riley Moreno\" <riley@studio.example>\r\nSubject: Plan\r\nDate: Wed, 14 Oct 2026 12:00:00 +0000\r\n\
Message-ID: <6acf6ec0.c4810462bc19dca4@studio.example>\r\nTo: \"Bo\" <bo@studio.example>\r\nX-Mailer: Quatrelle {VERSION}\r\n\
MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"=_mcc6512b6a89c44875_1\"\r\n\r\n--=_mcc6512b6a89c44875_1\r\n\
Content-Type: text/plain; charset=\"utf-8\"\r\nContent-Transfer-Encoding: 7bit\r\n\r\nSee you at nine.\r\n--=_mcc6512b6a89c44875_1\r\n\
Content-Type: text/plain\r\nContent-Disposition: attachment; filename=\"plan.txt\"\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n\
nine=0A\r\n--=_mcc6512b6a89c44875_1--\r\n";

#[test]
fn nothing_changes_for_an_account_without_a_key() {
    let mut s = sample_session("pgp-no-key-send");
    // Bo's key is known, and encryption may turn itself on: still nothing, the account has none.
    s.run("pgp.importPublic", &json!({"data": bo_key().public})).unwrap();
    s.run("view.encryptWhenPossible", &json!({"on": true})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "Bo <bo@studio.example>", "subject": "Plan", "body": "See you at nine."})).unwrap();
    s.run("compose.attach", &json!({"name": "plan.txt", "data": b"nine\n".to_vec()})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let raw = last_sent(&s);
    assert_eq!(String::from_utf8_lossy(&raw), BEFORE.replace("{VERSION}", env!("CARGO_PKG_VERSION")));
    // Nothing of OpenPGP is offered: no scheme to choose, nothing to encrypt with.
    s.run("mail.new", &json!({})).unwrap();
    assert!(matches!(s.run("compose.security", &json!({"scheme": "pgp"})), Err(CmdError::Disabled(_))));
    assert_eq!(s.run("compose.encryption", &json!({})).unwrap()["setUp"], json!(false));
    assert_eq!(s.scheme_for(s.compose.as_ref().unwrap()), None);
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Pgp { .. })));
    done(s);
}

/// Plays the network layer for [`PgpWork::Protect`] with `secret` (the account's key), as the
/// pool does: signed, then encrypted (PGP/MIME). What it was asked: the keys it encrypted to, and
/// whether it signed.
fn answer_protect(s: &mut Session, secret: &[u8]) -> (Vec<Vec<u8>>, bool) {
    let jobs = s.take_jobs();
    let (id, raw, sign, encrypt_to, hide) = jobs
        .into_iter()
        .find_map(|j| match j {
            Job::Pgp { work: PgpWork::Protect { id, raw, sign, encrypt_to, hide_subject }, .. } => Some((id, raw.0, sign, encrypt_to, hide_subject)),
            _ => None,
        })
        .expect("asked to sign or encrypt");
    let keys: Vec<&[u8]> = encrypt_to.iter().map(Vec::as_slice).collect();
    let now = s.clock.now;
    let mut sign_with = |e: &[u8]| pgp::sign(e, secret, now, &mut rand_core::OsRng).map(String::into_bytes).map_err(|e| e.to_string());
    let mut encrypt_with = |e: &[u8]| pgp::encrypt(e, &keys, None, now, &mut rand_core::OsRng).map(String::into_bytes).map_err(|e| e.to_string());
    let signer: Option<quatrelle_mime::smime::Crypto<'_>> = if sign { Some(&mut sign_with) } else { None };
    let encrypter: Option<quatrelle_mime::smime::Crypto<'_>> = if keys.is_empty() { None } else { Some(&mut encrypt_with) };
    let sent = quatrelle_mime::pgp::protect(&raw, signer, encrypter, hide, "=_test").unwrap();
    s.apply(Event::Protected { account: "sample".into(), id, raw: Some(Private(sent)), error: None });
    (encrypt_to, sign)
}

/// What `sent` (signed, then encrypted) holds, decrypted with `secret`: the signed part, and what
/// its signature says checked with `signer`.
fn open_sent(sent: &[u8], secret: &[u8], signer: &[u8]) -> (String, pgp::Check) {
    let quatrelle_mime::pgp::Protection::Encrypted(data) = quatrelle_mime::pgp::protection(sent) else { panic!("not encrypted") };
    let inside = pgp::decrypt(&data, &[secret], &[]).unwrap().data;
    let quatrelle_mime::pgp::Protection::Signed { content, signature } = quatrelle_mime::pgp::protection(&inside) else {
        panic!("not signed inside")
    };
    let check = pgp::verify(&content, &signature, &[signer]).unwrap();
    (String::from_utf8_lossy(&content).into_owned(), check)
}

/// Plays the network layer for [`PgpWork::Decrypt`] with `secret`.
fn answer_decrypt_with(s: &mut Session, secret: &[u8]) {
    for j in s.take_jobs() {
        let Job::Pgp { work: PgpWork::Decrypt { key, data, .. }, .. } = j else { continue };
        let d = pgp::decrypt(&data, &[secret], &[]).unwrap();
        s.apply(Event::PgpDecrypted { key, content: Some(Private(d.data)), signature: None, keys: Vec::new(), error: None });
    }
}

/// PGP/MIME mail from `from` to Riley (and `cc`), encrypted to `to_key` at `now`.
fn encrypted_mail(from: &str, cc: &str, to_key: &[u8], text: &str, now: i64) -> Vec<u8> {
    let entity = format!("Content-Type: text/plain; charset=utf-8\r\n\r\n{text}\r\n");
    let armored = pgp::encrypt(entity.as_bytes(), &[to_key], None, now, &mut rand_core::OsRng).unwrap();
    format!(
        "From: {from}\r\nTo: Riley Moreno <riley@studio.example>\r\nCc: {cc}\r\nSubject: Plan\r\nDate: {OCT_24}\r\nMessage-ID: <{}@studio.example>\r\n\
MIME-Version: 1.0\r\nContent-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=\"enc\"\r\n\r\n\
--enc\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n\r\n--enc\r\n\
Content-Type: application/octet-stream; name=\"encrypted.asc\"\r\n\r\n{armored}\r\n--enc--\r\n",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
    .into_bytes()
}

fn zoe_key() -> pgp::Unlocked {
    pgp::generate("Zoe", "zoe@studio.example", NOW, &mut rand_core::OsRng).unwrap()
}

#[test]
fn sending_signs_then_encrypts_to_everyone_and_the_sender() {
    let (mut s, riley) = with_key("pgp-send");
    let (bo, zoe) = (bo_key(), zoe_key());
    s.run("pgp.importPublic", &json!({"data": bo.public})).unwrap();
    s.run("pgp.importPublic", &json!({"data": zoe.public})).unwrap();
    s.clock.now = NOW + 2 * DAY;
    s.run("mail.new", &json!({})).unwrap();
    s.run(
        "compose.update",
        &json!({"to": "Bo <bo@studio.example>", "cc": "Zoe <zoe@studio.example>", "subject": "Plan", "body": "The quokkaplan for Tuesday."}),
    )
    .unwrap();
    let r = s.run("compose.security", &json!({"sign": true, "encrypt": true, "scheme": "pgp"})).unwrap();
    assert_eq!((r["scheme"].as_str(), r["uses"].as_str()), (Some("pgp"), Some("pgp")), "{r}");
    let reach = s.run("compose.encryption", &json!({})).unwrap();
    assert_eq!((reach["can"].clone(), reach["missing"].clone(), reach["scheme"].clone()), (json!(true), json!([]), json!("pgp")), "{reach}");
    assert!(s.seals_with_pgp(s.compose.as_ref().unwrap()), "its draft is encrypted with OpenPGP too");
    let sent = s.run("compose.send", &json!({})).unwrap();
    assert!(sent.get("protecting").is_some(), "{sent}");
    let (to, signed) = answer_protect(&mut s, &riley.secret);
    assert!(signed);
    assert_eq!(to.len(), 3, "Bo, Zoe, and Riley for the Sent copy");
    for k in [&bo.public, &zoe.public, &riley.public] {
        assert!(to.contains(k));
    }
    let raw = last_sent(&s);
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("multipart/encrypted") && text.contains("Subject: [...]") && !text.contains("quokkaplan"), "{text}");
    for secret in [&bo.secret, &zoe.secret, &riley.secret] {
        let (inside, check) = open_sent(&raw, secret, &riley.public);
        assert!(inside.contains("quokkaplan") && inside.contains("Subject: Plan"), "{inside}");
        assert!(matches!(check, pgp::Check::Good { key: 0, .. }), "{check:?}");
    }
    // Encrypting to them is using their keys.
    assert_eq!(s.person_key("sample", "bo@studio.example").unwrap().last_used, NOW + 2 * DAY);
    // Signed only: no one's key is needed.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "eve@studio.example", "subject": "Signed", "body": "Hi."})).unwrap();
    s.run("compose.security", &json!({"sign": true})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let (to, signed) = answer_protect(&mut s, &riley.secret);
    assert!(signed && to.is_empty());
    assert!(matches!(quatrelle_mime::pgp::protection(&last_sent(&s)), quatrelle_mime::pgp::Protection::Signed { .. }));
    done(s);
}

/// Review focus 4: a reply to encrypted mail where one recipient's key has run out. Encryption
/// can't be done, and says whom for; sending refuses, naming them, and nothing goes. Nothing goes
/// unencrypted until the user turns encryption off.
#[test]
fn a_recipient_without_a_usable_key_is_named() {
    let (mut s, riley) = with_key("pgp-missing");
    // Bo's key ran out a year ago; Zoe's holds.
    let old = pgp::generate("Bo", "bo@studio.example", NOW - 4 * 365 * DAY, &mut rand_core::OsRng).unwrap();
    assert!(old.info.encryptable(NOW).is_err());
    s.run("pgp.importPublic", &json!({"data": old.public})).unwrap();
    s.run("pgp.importPublic", &json!({"data": zoe_key().public})).unwrap();
    open_raw(&mut s, 9701, &encrypted_mail("Bo <bo@studio.example>", "Zoe <zoe@studio.example>", &riley.public, "The quokkaplan.", NOW));
    answer_decrypt_with(&mut s, &riley.secret);
    assert!(matches!(security(&s).lock, Some(Lock::Open { .. })), "{:?}", security(&s));
    s.run("mail.replyAll", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert!(c.encrypt && c.sign, "an answer to encrypted mail is encrypted");
    assert_eq!(c.came_as, Some(Scheme::Pgp));
    let reach = s.run("compose.encryption", &json!({})).unwrap();
    assert_eq!((reach["can"].clone(), reach["missing"].clone()), (json!(false), json!(["bo@studio.example"])), "{reach}");
    let sent_before = last_sent(&s);
    match s.run("compose.send", &json!({})) {
        Err(CmdError::Disabled(e)) => assert!(e.starts_with(NO_KEY_TO_ENCRYPT) && e.contains("bo@studio.example") && !e.contains("zoe"), "{e}"),
        other => panic!("{other:?}"),
    }
    let jobs = s.take_jobs();
    assert!(!jobs.iter().any(|j| matches!(j, Job::Pgp { work: PgpWork::Protect { .. }, .. } | Job::Send { .. })), "{jobs:?}");
    assert_eq!(last_sent(&s), sent_before, "nothing went");
    assert!(s.compose.as_ref().is_some_and(|c| !c.sending), "it stays to be sent");
    // A key that's withdrawn or absent is the same: Eve has none.
    s.run("compose.update", &json!({"cc": "Zoe <zoe@studio.example>; eve@studio.example"})).unwrap();
    let reach = s.run("compose.encryption", &json!({})).unwrap();
    assert_eq!(reach["missing"], json!(["bo@studio.example", "eve@studio.example"]), "{reach}");
    // Encryption turned off by the user: it goes, signed.
    s.run("compose.security", &json!({"encrypt": false})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let (to, signed) = answer_protect(&mut s, &riley.secret);
    assert!(signed && to.is_empty());
    done(s);
}

/// With both a certificate and a key, Automatic answers a message in the scheme it came in, and
/// otherwise takes the one every recipient can receive; chosen by hand, the choice holds.
#[test]
fn auto_answers_in_the_scheme_it_was_written_in() {
    let mut s = with_certificate("pgp-auto");
    s.run("pgp.create", &json!({})).unwrap();
    let riley = answer_make(&mut s);
    assert!(s.identity("sample").is_some() && s.pgp_active("sample"));
    // Ana has an OpenPGP key known, no certificate: a new message to her goes with OpenPGP.
    let ana = pgp::generate("Ana", "ana@posteo.example", s.clock.now, &mut rand_core::OsRng).unwrap();
    s.run("pgp.importPublic", &json!({"data": ana.public})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@posteo.example", "subject": "New"})).unwrap();
    assert_eq!(s.scheme_for(s.compose.as_ref().unwrap()), Some(Scheme::Pgp));
    // Chosen by hand: held.
    let r = s.run("compose.security", &json!({"scheme": "smime"})).unwrap();
    assert_eq!(r["uses"], "smime");
    s.run("compose.discard", &json!({})).unwrap();
    // Her S/MIME mail, answered: S/MIME, though only her OpenPGP key is known.
    open_raw(&mut s, 9711, &testdata("encrypted-cbc.eml"));
    answer_decrypt(&mut s);
    s.run("mail.reply", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!((c.scheme, c.came_as, c.encrypt), (Scheme::Auto, Some(Scheme::Smime), true));
    assert_eq!(s.scheme_for(&c), Some(Scheme::Smime));
    let reach = s.run("compose.encryption", &json!({})).unwrap();
    assert_eq!((reach["scheme"].clone(), reach["missing"].clone()), (json!("smime"), json!(["ana@posteo.example"])), "{reach}");
    s.run("compose.discard", &json!({})).unwrap();
    // Bo's OpenPGP mail, answered: OpenPGP (Bo has neither, which would have been S/MIME).
    let raw = encrypted_mail("Bo <bo@studio.example>", "", &riley.public, "The plan.", s.clock.now);
    open_raw(&mut s, 9712, &raw);
    answer_decrypt_with(&mut s, &riley.secret);
    s.run("mail.reply", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!((c.came_as, c.encrypt), (Some(Scheme::Pgp), true));
    assert_eq!(s.scheme_for(&c), Some(Scheme::Pgp));
    let mut fresh = c.clone();
    fresh.came_as = None;
    assert_eq!(s.scheme_for(&fresh), Some(Scheme::Smime), "by recipients alone, a tie: S/MIME");
    done(s);
}

/// The `Autocrypt:` headers of `raw`, read: the address each is for, and its key's fingerprint.
fn autocrypt_of(raw: &[u8]) -> Vec<(String, String)> {
    quatrelle_mime::pgp::autocrypt(raw)
        .iter()
        .filter_map(|v| pgp::autocrypt::parse(v))
        .map(|a| (a.addr.clone(), pgp::info(&a.keydata).map(|i| i.fingerprint).unwrap_or_default()))
        .collect()
}

#[test]
fn attaching_my_public_key_adds_it_once() {
    // Without a key: nothing to attach.
    let mut s = sample_session("pgp-attach-none");
    s.run("mail.new", &json!({})).unwrap();
    assert!(matches!(s.run("compose.attachPublicKey", &json!({})), Err(CmdError::Disabled(_))));
    assert!(s.compose.as_ref().unwrap().attachments.is_empty());
    done(s);
    let (mut s, riley) = with_key("pgp-attach");
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "Bo <bo@studio.example>", "subject": "My key", "body": "Here it is."})).unwrap();
    s.run("compose.attachPublicKey", &json!({})).unwrap();
    let again = s.run("compose.attachPublicKey", &json!({})).unwrap();
    assert_eq!(again["already"], json!(true));
    let c = s.compose.clone().unwrap();
    let keys: Vec<_> = c.attachments.iter().filter(|a| a.mime == "application/pgp-keys").collect();
    assert_eq!(keys.len(), 1, "once");
    assert_eq!(keys[0].name, "riley@studio.example.asc");
    assert!(String::from_utf8_lossy(&keys[0].data).starts_with("-----BEGIN PGP PUBLIC KEY BLOCK-----"));
    let attached = pgp::public_keys(&keys[0].data).unwrap();
    assert_eq!(pgp::info(&attached[0]).unwrap().fingerprint, riley.info.fingerprint, "the account's key");
    // Sent, it reads as a key attached to Riley's mail.
    s.run("compose.send", &json!({})).unwrap();
    let parts = quatrelle_mime::pgp::key_parts(&last_sent(&s));
    assert_eq!(parts.len(), 1);
    done(s);
}

#[test]
fn the_public_key_goes_with_mail_once_a_key_exists() {
    let (mut s, riley) = with_key("pgp-autocrypt");
    let send = |s: &mut Session, subject: &str| {
        s.run("mail.new", &json!({})).unwrap();
        s.run("compose.update", &json!({"to": "Bo <bo@studio.example>", "subject": subject, "body": "Hi."})).unwrap();
        s.run("compose.send", &json!({})).unwrap();
        last_sent(s)
    };
    // On once the key is made: one header, the account's key, no prefer-encrypt.
    let raw = send(&mut s, "With");
    assert_eq!(autocrypt_of(&raw), [("riley@studio.example".to_string(), riley.info.fingerprint.clone())]);
    let text = String::from_utf8_lossy(&raw);
    assert_eq!(text.matches("Autocrypt:").count(), 1);
    assert!(!text.contains("prefer-encrypt"), "{text}");
    // Turned off: none.
    s.run("pgp.options", &json!({"autocrypt": false})).unwrap();
    assert!(autocrypt_of(&send(&mut s, "Without")).is_empty());
    // On again, but the key ran out: none either (no one could encrypt to it).
    s.run("pgp.options", &json!({"autocrypt": true})).unwrap();
    s.clock.now = riley.info.expires.unwrap() + DAY;
    assert!(autocrypt_of(&send(&mut s, "Late")).is_empty());
    done(s);
}

#[test]
fn encryption_turns_itself_on_only_when_chosen() {
    let (mut s, riley) = with_key("pgp-encrypt-auto");
    let bo = bo_key();
    s.run("pgp.importPublic", &json!({"data": bo.public})).unwrap();
    let reach = |s: &mut Session| s.run("compose.encryption", &json!({})).unwrap();
    // The option off, as it starts: encryption stays off, though everyone could receive it.
    assert_eq!(s.run("view.encryptWhenPossible", &json!({})).unwrap()["on"], json!(true), "switched on from off");
    s.run("view.encryptWhenPossible", &json!({"on": false})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "Bo <bo@studio.example>", "subject": "Plan"})).unwrap();
    let r = reach(&mut s);
    assert_eq!((r["can"].clone(), r["encrypt"].clone()), (json!(true), json!(false)), "{r}");
    s.run("compose.discard", &json!({})).unwrap();
    // On: it turns on while everyone can, off when someone can't, on again.
    s.run("view.encryptWhenPossible", &json!({"on": true})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "Bo <bo@studio.example>", "subject": "Plan"})).unwrap();
    assert_eq!(reach(&mut s)["encrypt"], json!(true));
    s.run("compose.update", &json!({"to": "Bo <bo@studio.example>; eve@studio.example"})).unwrap();
    let r = reach(&mut s);
    assert_eq!((r["encrypt"].clone(), r["missing"].clone()), (json!(false), json!(["eve@studio.example"])), "{r}");
    s.run("compose.update", &json!({"to": "Bo <bo@studio.example>"})).unwrap();
    assert_eq!(reach(&mut s)["encrypt"], json!(true));
    // Chosen by hand: it holds, whoever it goes to.
    s.run("compose.security", &json!({"encrypt": false})).unwrap();
    assert_eq!(reach(&mut s)["encrypt"], json!(false));
    s.run("compose.discard", &json!({})).unwrap();
    // Sent without asking first: as the recipients are then.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "Bo <bo@studio.example>", "subject": "Plan", "body": "Hi."})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let (to, _) = answer_protect(&mut s, &riley.secret);
    assert_eq!(to.len(), 2, "Bo, and Riley for the Sent copy");
    done(s);
}
