// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: mail made to pass for someone else's. A signature vouches for the sender shown,
//! never for one named inside by whoever signed; a key or certificate that came with the mail
//! isn't proof of who sent it; and what a message brings never replaces what was trusted.

use super::*;
use crate::session::Learnt;
use quatrelle_pgp as openpgp;

/// After GnuPG made the test keys.
const PGP_NOW: i64 = 1_800_000_000;

fn pgpdata(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../pgp/testdata").join(name)).unwrap()
}

fn ana_key() -> openpgp::Unlocked {
    openpgp::read_secret_key(&pgpdata("ana-secret.asc"), "secret").unwrap()
}

fn shown_from(s: &Session) -> String {
    s.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.from.email.clone()).unwrap_or_default()
}

fn shown(s: &Session) -> quatrelle_mime::Parsed {
    s.reading.as_ref().and_then(|r| r.parsed.as_deref().cloned()).unwrap()
}

fn seal(s: &Session) -> Seal {
    security(s).signed.expect("signed").seal
}

/// The header fields of a message from `from`, each line ending CRLF, then `extra`.
fn outer(id: &str, from: &str, extra: &str) -> String {
    format!(
        "From: {from}\r\nTo: riley@studio.example\r\nSubject: Hello\r\nDate: Sat, 24 Oct 2026 09:00:00 +0000\r\nMessage-ID: <{id}@forged.example>\r\n{extra}"
    )
}

/// A PGP/MIME message signed with `key` over `entity`.
fn pgp_signed(outer: &str, entity: &str, key: &openpgp::Unlocked, now: i64) -> Vec<u8> {
    let signature = openpgp::sign(entity.as_bytes(), &key.secret, now, &mut rand_core::OsRng).unwrap().replace('\n', "\r\n");
    format!(
        "{outer}MIME-Version: 1.0\r\nContent-Type: multipart/signed; micalg=pgp-sha256; protocol=\"application/pgp-signature\"; boundary=\"sig\"\r\n\r\n\
--sig\r\n{entity}\r\n--sig\r\nContent-Type: application/pgp-signature; name=\"signature.asc\"\r\n\r\n{signature}\r\n--sig--\r\n"
    )
    .into_bytes()
}

/// The entity someone signs to pass for the director: header protection names another sender.
const AS_THE_DIRECTOR: &str = "Content-Type: text/plain; charset=utf-8; hp=\"clear\"\r\nFrom: The Director <director@victim.example>\r\n\
Subject: Urgent transfer\r\n\r\nPay Eve today.\r\n";

/// S/MIME: Riley's own (trusted) certificate signs an entity that names the director as its
/// sender. The sender shown stays the one the message came from, whom the certificate is for.
#[test]
fn a_sender_named_inside_isnt_vouched_for_by_anothers_certificate() {
    let mut s = with_certificate("forged-smime-inside");
    let signature = smime::sign(AS_THE_DIRECTOR.as_bytes(), &riley(), LATER, &mut rand_core::OsRng).unwrap();
    let sent = quatrelle_mime::smime::signed_message(
        outer("f1", "Riley <riley@studio.example>", "").as_bytes(),
        AS_THE_DIRECTOR.as_bytes(),
        &signature,
        "=_f1",
    );
    open_raw(&mut s, 9701, &sent);
    assert!(!(shown_from(&s) == "director@victim.example" && seal(&s) == Seal::Trusted), "a trusted seal over a sender the certificate isn't for");
    assert_eq!(shown_from(&s), "riley@studio.example");
    assert_eq!(seal(&s), Seal::Trusted, "signed by whom it came from");
    assert_eq!(shown(&s).subject, "Urgent transfer", "the rest of what was signed shows");
    done(s);
}

/// OpenPGP: the same, with Ana's key (imported by hand).
#[test]
fn a_sender_named_inside_isnt_vouched_for_by_anothers_key() {
    let mut s = sample_session("forged-pgp-inside");
    s.clock.now = PGP_NOW;
    s.run("pgp.importPublic", &json!({"data": pgpdata("ana.asc")})).unwrap();
    let sent = pgp_signed(&outer("f2", "Ana Lima <ana@posteo.example>", ""), AS_THE_DIRECTOR, &ana_key(), PGP_NOW);
    open_raw(&mut s, 9702, &sent);
    assert!(!(shown_from(&s) == "director@victim.example" && seal(&s) == Seal::Trusted), "a trusted seal over a sender the key isn't for");
    assert_eq!(shown_from(&s), "ana@posteo.example");
    assert_eq!(seal(&s), Seal::Trusted);
    done(s);
}

/// A signature trusted for the sender named inside does show that sender: a list that rewrote
/// the outer From doesn't hide who signed.
#[test]
fn a_sender_named_inside_shows_when_the_signature_is_theirs() {
    let mut s = with_certificate("forged-smime-theirs");
    let entity =
        "Content-Type: text/plain; charset=utf-8; hp=\"clear\"\r\nFrom: Riley <riley@studio.example>\r\nSubject: Minutes\r\n\r\nAttached.\r\n";
    let signature = smime::sign(entity.as_bytes(), &riley(), LATER, &mut rand_core::OsRng).unwrap();
    let sent = quatrelle_mime::smime::signed_message(
        outer("f3", "Riley via Team <team@lists.example>", "").as_bytes(),
        entity.as_bytes(),
        &signature,
        "=_f3",
    );
    let key = open_raw(&mut s, 9703, &sent);
    assert_eq!(shown_from(&s), "riley@studio.example");
    assert_eq!(seal(&s), Seal::Trusted);
    // What a reply reads is what's shown.
    let readable = s.readable_raw(&key).unwrap();
    assert_eq!(quatrelle_mime::parse(&readable).unwrap().from.email, "riley@studio.example");
    done(s);
}

/// Header protection without a signature says nothing of who sent it: an entity that isn't signed
/// (here inside a signature that doesn't hold) can't name another sender. And header fields the
/// receiving server wrote can't be replaced from inside.
#[test]
fn unsigned_header_protection_names_no_other_sender() {
    let mut s = with_certificate("forged-unsigned-inside");
    let entity = "Content-Type: text/plain; charset=utf-8; hp=\"clear\"\r\nFrom: The Director <director@victim.example>\r\n\
Authentication-Results: mx.studio.example; dmarc=pass\r\nSubject: Urgent transfer\r\n\r\nPay Eve today.\r\n";
    let signature = smime::sign(b"Content-Type: text/plain\r\n\r\nSomething else.\r\n", &riley(), LATER, &mut rand_core::OsRng).unwrap();
    let sent = quatrelle_mime::smime::signed_message(
        outer("f4", "Eve <eve@evil.example>", "Authentication-Results: mx.studio.example; dmarc=fail\r\nReply-To: eve@evil.example\r\n").as_bytes(),
        entity.as_bytes(),
        &signature,
        "=_f4",
    );
    let key = open_raw(&mut s, 9704, &sent);
    assert!(matches!(seal(&s), Seal::Broken(_)));
    assert_eq!(shown_from(&s), "eve@evil.example");
    assert!(!security(&s).headers, "nothing was protected by a signature that doesn't hold");
    let readable = String::from_utf8_lossy(&s.readable_raw(&key).unwrap()).to_string();
    assert!(readable.contains("dmarc=fail") && !readable.contains("dmarc=pass"), "{readable}");
    done(s);
}

/// With header protection, what the outside adds for the reader isn't taken: a Reply-To put on
/// the way would send the answer (and the decrypted text it quotes) to someone else.
#[test]
fn a_reply_to_added_outside_protected_headers_is_dropped() {
    let mut s = with_certificate("forged-reply-to");
    let entity = "Content-Type: text/plain; charset=utf-8; hp=\"clear\"\r\nFrom: Riley <riley@studio.example>\r\nTo: riley@studio.example\r\n\
Subject: Figures\r\n\r\nThe figures.\r\n";
    let signature = smime::sign(entity.as_bytes(), &riley(), LATER, &mut rand_core::OsRng).unwrap();
    let sent = quatrelle_mime::smime::signed_message(
        outer("f5", "Riley <riley@studio.example>", "Reply-To: eve@evil.example\r\nCc: eve@evil.example\r\n").as_bytes(),
        entity.as_bytes(),
        &signature,
        "=_f5",
    );
    open_raw(&mut s, 9705, &sent);
    assert_eq!(seal(&s), Seal::Trusted);
    let p = shown(&s);
    assert!(p.reply_to.is_empty() && p.cc.is_empty(), "{:?} {:?}", p.reply_to, p.cc);
    done(s);
}

/// A certificate no authority this computer trusts stands behind isn't kept to encrypt to: the
/// message that brings it could be anyone's.
#[test]
fn an_untrusted_certificate_isnt_kept_to_encrypt_to() {
    let mut s = sample_session("forged-smime-kept");
    s.clock.now = LATER;
    open_raw(&mut s, 9709, &testdata("signed-clear.eml"));
    assert_eq!(seal(&s), Seal::Untrusted(vec![smime::Problem::UnknownIssuer]));
    assert!(s.smime_settings("sample").known.is_empty(), "kept: {:?}", s.smime_settings("sample").known.keys());
    done(s);
}

/// A signing subkey that ran out doesn't sign for its owner any more, though the key it belongs
/// to is still valid.
#[test]
fn a_signing_subkey_that_ran_out_is_said() {
    let mut s = sample_session("forged-pgp-subkey");
    s.clock.now = PGP_NOW;
    s.run("pgp.importPublic", &json!({"data": pgpdata("kim.asc")})).unwrap();
    let sent = format!(
        "{}MIME-Version: 1.0\r\nContent-Type: multipart/signed; micalg=pgp-sha512; protocol=\"application/pgp-signature\"; boundary=\"sig\"\r\n\r\n\
--sig\r\n{}\r\n--sig\r\nContent-Type: application/pgp-signature; name=\"signature.asc\"\r\n\r\n{}\r\n--sig--\r\n",
        outer("f10", "Kim Park <kim@posteo.example>", ""),
        String::from_utf8_lossy(&pgpdata("part.txt")),
        String::from_utf8_lossy(&pgpdata("kim-part.sig"))
    );
    open_raw(&mut s, 9711, sent.as_bytes());
    assert_eq!(seal(&s), Seal::Untrusted(vec![smime::Problem::Expired(1_780_358_400)]));
    done(s);
}

/// A PGP block in a text, a few kilobytes that decompress to megabytes: not opened past what a
/// text is shown up to.
#[test]
fn a_compressed_block_in_a_text_isnt_opened_past_a_few_megabytes() {
    let mut s = sample_session("forged-pgp-bomb");
    s.clock.now = PGP_NOW;
    let block = String::from_utf8(pgpdata("compressed-inline.asc")).unwrap().replace('\n', "\r\n");
    assert!(block.len() < 16 << 10);
    let sent =
        format!("{}MIME-Version: 1.0\r\nContent-Type: text/plain; charset=us-ascii\r\n\r\n{block}", outer("f11", "Eve <eve@evil.example>", ""));
    open_raw(&mut s, 9712, sent.as_bytes());
    let text = shown(&s).text;
    assert!(text.len() < 1 << 20, "{} bytes shown", text.len());
    done(s);
}

/// A certificate kept before only trusted ones were (or one whose authority is trusted no
/// longer) isn't encrypted to: its chain is checked again when the message goes.
#[test]
fn a_kept_certificate_is_checked_again_before_encrypting() {
    let mut s = with_certificate("forged-smime-encrypt");
    let ana = der::pem::decode_vec(String::from_utf8(testdata("ana.pem")).unwrap().as_bytes()).unwrap().1;
    let eve = smime::read_p12(&testdata("zoe-aes.p12"), "secret").unwrap();
    // As an older file could hold them: Ana's, issued by the authority Riley's own came with,
    // and someone else's certificate under Bo's address.
    let known = |cert: &[u8]| crate::session::KnownCert { cert: cert.to_vec(), seen: LATER, ..Default::default() };
    let mut settings = s.smime_settings("sample");
    settings.known.insert("ana@posteo.example".into(), known(&ana));
    settings.known.insert("bo@posteo.example".into(), known(eve.certificate()));
    let store = s.store.clone().unwrap();
    store.save_account_doc("sample", "smime.json", &settings).unwrap();
    let mut s = Session::open(Some(store), quatrelle_model::dates::Clock::new(LATER, 0));
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@posteo.example, bo@posteo.example", "subject": "Plan"})).unwrap();
    s.run("compose.security", &json!({"encrypt": true})).unwrap();
    let err = s.run("compose.send", &json!({})).unwrap_err().to_string();
    assert!(err.contains("bo@posteo.example") && !err.contains("ana@posteo.example"), "{err}");
    done(s);
}
