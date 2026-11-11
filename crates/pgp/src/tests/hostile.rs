// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Trying to break it: what decompresses to too much, packets nested without end, a user ID
//! someone else certified, and keys by the hundred.

use pgp::types::CompressionAlgorithm;

use super::*;

/// A new-format packet: `tag`, a length, the body.
fn packet(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![0xC0 | tag, 0xFF];
    out.extend_from_slice(&u32::try_from(body.len()).unwrap().to_be_bytes());
    out.extend_from_slice(body);
    out
}

/// A literal data packet holding `data`.
fn literal(data: &[u8]) -> Vec<u8> {
    let mut body = vec![b'b', 0, 0, 0, 0, 0];
    body.extend_from_slice(data);
    packet(11, &body)
}

#[test]
fn what_decompresses_to_too_much_is_refused() {
    let ana = unlocked("Ana <ana@posteo.example>");
    let big = vec![0u8; MAX + 1024];
    let mut b = pgp::composed::MessageBuilder::from_bytes("", big).seipd_v1(rand_core::OsRng, pgp::crypto::sym::SymmetricKeyAlgorithm::AES256);
    let key: pgp::composed::SignedPublicKey = pgp::composed::Deserializable::from_bytes(&ana.public[..]).unwrap();
    b.encrypt_to_key(rand_core::OsRng, &key.public_subkeys[0]).unwrap();
    b.compression(CompressionAlgorithm::ZLIB);
    let sealed = b.to_vec(rand_core::OsRng).unwrap();
    assert!(sealed.len() < 1 << 20, "a small message: {}", sealed.len());
    assert_eq!(decrypt(&sealed, &[&ana.secret], &[]).unwrap_err(), Error::TooBig);
}

/// A compression bomb to a key GnuPG made: 70 MiB of zeros, compressed (ZLIB) to a few hundred
/// kilobytes and encrypted to Riley. Reading stops at [`MAX`]: never the 70 MiB. And a part of
/// 70 MiB said to be encrypted isn't read at all.
#[test]
fn a_message_that_decompresses_past_max_is_refused() {
    let riley = read_secret_key(&gnupg::testdata("riley-secret.asc"), "").unwrap();
    let public = public_keys(&gnupg::testdata("riley.asc")).unwrap().remove(0);
    assert!(same_key(&riley.public, &public));
    let key: pgp::composed::SignedPublicKey = pgp::composed::Deserializable::from_bytes(&public[..]).unwrap();
    let mut b = pgp::composed::MessageBuilder::from_bytes("", vec![0u8; 70 << 20])
        .seipd_v1(rand_core::OsRng, pgp::crypto::sym::SymmetricKeyAlgorithm::AES256);
    b.encrypt_to_key(rand_core::OsRng, &key.public_subkeys[0]).unwrap();
    b.compression(CompressionAlgorithm::ZLIB);
    let sealed = b.to_vec(rand_core::OsRng).unwrap();
    assert!(sealed.len() < 1 << 20, "a small message: {}", sealed.len());
    assert_eq!(decrypt(&sealed, &[&riley.secret], &[&public]).unwrap_err(), Error::TooBig);
    // Seventy mebibytes said to be an encrypted message: refused before rPGP sees it.
    let mut part = b"-----BEGIN PGP MESSAGE-----\n\n".to_vec();
    part.resize(70 << 20, b'A');
    assert_eq!(decrypt(&part, &[&riley.secret], &[]).unwrap_err(), Error::TooBig);
    assert!(!is_encrypted(&part));
    assert!(recipients(&part).is_empty());
}

/// What decompresses to too much is found before the buffer holding it grows past [`MAX`]:
/// `read_to_end` on a reader cut at `MAX + 1` doubled it to twice `MAX` first.
#[test]
fn reading_stops_at_max_without_growing_past_it() {
    /// A reader that hands out a thousand bytes at a time, as a decompressor may.
    struct Dribble<R>(R);
    impl<R: std::io::Read> std::io::Read for Dribble<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(1000);
            self.0.read(&mut buf[..n])
        }
    }
    let fail = Error::Decrypt;
    assert_eq!(crate::message::read_capped(&mut std::io::repeat(0), MAX, &fail).unwrap_err(), Error::TooBig);
    let all = crate::message::read_capped(&mut Dribble(std::io::Read::take(std::io::repeat(7), MAX as u64)), MAX, &fail).unwrap();
    assert_eq!(all.len(), MAX);
    assert!(all.capacity() <= MAX, "{}", all.capacity());
    assert_eq!(crate::message::read_capped(&mut Dribble(&b"short"[..]), MAX, &fail).unwrap(), b"short");
}

#[test]
fn packets_nested_without_end_dont_overflow() {
    // Thousands of compressed packets ("uncompressed" algorithm) around a literal one.
    let mut m = literal(b"deep");
    for _ in 0..5000 {
        let mut body = vec![0u8];
        body.extend_from_slice(&m);
        m = packet(8, &body);
    }
    assert!(open(&m, &[]).is_err());
    let _ = is_encrypted(&m);
    let _ = recipients(&m);
    // A few levels read.
    let mut m = literal(b"shallow");
    for _ in 0..2 {
        let mut body = vec![0u8];
        body.extend_from_slice(&m);
        m = packet(8, &body);
    }
    assert_eq!(open(&m, &[]).unwrap().data, b"shallow");
    // Thousands of one-pass signature packets, none ended.
    let ops = packet(4, &[3, 0, 8, 22, 1, 2, 3, 4, 5, 6, 7, 8, 0]);
    let many: Vec<u8> = std::iter::repeat_n(ops, 5000).flatten().chain(literal(b"x")).collect();
    assert!(open(&many, &[]).is_err() || open(&many, &[]).is_ok_and(|d| d.check.is_some()));
}

/// Eve's user ID, with her own signature, put on Ana's key: not Ana's, so mail to Eve's
/// address isn't encrypted to Ana's key, nor Ana's signatures taken as Eve's.
#[test]
fn a_user_id_someone_else_certified_doesnt_count() {
    let (ana, eve) = (unlocked("Ana <ana@posteo.example>"), unlocked("Eve <eve@posteo.example>"));
    let mut a: pgp::composed::SignedPublicKey = pgp::composed::Deserializable::from_bytes(&ana.public[..]).unwrap();
    let e: pgp::composed::SignedPublicKey = pgp::composed::Deserializable::from_bytes(&eve.public[..]).unwrap();
    a.details.users.extend(e.details.users.clone());
    let forged = pgp::ser::Serialize::to_bytes(&a).unwrap();
    let i = info(&forged).unwrap();
    assert_eq!(i.emails, ["ana@posteo.example"]);
    assert!(!i.is_for("eve@posteo.example"));
}

#[test]
fn a_file_of_many_keys_is_read_so_far() {
    let ana = unlocked("Ana <ana@posteo.example>");
    let many: Vec<u8> = std::iter::repeat_n(ana.public.clone(), KEYS_MAX * 3).flatten().collect();
    assert_eq!(public_keys(&many).unwrap().len(), KEYS_MAX);
    // Armor with mixed line ends and trailing spaces around it still reads.
    let p: pgp::composed::SignedPublicKey = pgp::composed::Deserializable::from_bytes(&ana.public[..]).unwrap();
    let armored = p.to_armored_string(Default::default()).unwrap().replace('\n', "\r\n");
    assert_eq!(public_keys(format!("  \r\n{armored}   \n").as_bytes()).unwrap().len(), 1);
}

/// A standalone or timestamp signature covers one byte of what it's checked over: the sender's
/// own, lifted from elsewhere, would hold over any text that starts alike. Only signatures over a
/// message (binary or text) count.
#[test]
fn only_signatures_over_a_message_count() {
    use pgp::packet::{SignatureConfig, SignatureType, Subpacket, SubpacketData};
    use pgp::types::{KeyDetails as _, SigningKey as _};
    let ana = unlocked("Ana <ana@posteo.example>");
    let key = crate::message::secret(&ana.secret).unwrap();
    for typ in [SignatureType::Standalone, SignatureType::Timestamp] {
        let mut config = SignatureConfig::from_key(rand_core::OsRng, &key.primary_key, typ).unwrap();
        config.hashed_subpackets = vec![
            Subpacket::regular(SubpacketData::IssuerFingerprint(key.primary_key.fingerprint())).unwrap(),
            Subpacket::regular(SubpacketData::SignatureCreationTime(crate::stamp(NOW))).unwrap(),
        ];
        // rPGP makes none itself: hashed and signed by hand, over the one byte such a signature covers.
        let mut hasher = config.hash_alg.new_hasher().unwrap();
        hasher.update(b"C");
        let len = config.hash_signature_data(&mut hasher).unwrap();
        hasher.update(&config.trailer(len).unwrap());
        let hash = hasher.finalize();
        let bytes = key.primary_key.sign(&pgp::types::Password::empty(), config.hash_alg, &hash).unwrap();
        let made = pgp::packet::Signature::from_config(config, [hash[0], hash[1]], bytes).unwrap();
        let signature = pgp::composed::DetachedSignature::new(made).to_armored_bytes(Default::default()).unwrap();
        let said = verify(b"Content-Type: text/plain\r\n\r\nPay Eve.\r\n", &signature, &[&ana.public]).unwrap();
        assert_eq!(said, Check::Bad, "{typ:?}");
    }
}
