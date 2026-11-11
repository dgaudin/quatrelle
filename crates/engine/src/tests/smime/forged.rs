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
