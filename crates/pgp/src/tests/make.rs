// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A key Quatrelle makes: the form GnuPG 2.2+, Thunderbird and Proton read (v4, Ed25519 and
//! Curve25519 in their legacy form), three years, extended, and backed up behind a passphrase.

use proptest::prelude::*;

use super::*;

const THREE_YEARS: i64 = 94_608_000;

/// One key for the whole file: making one is cheap, but a proptest case needn't.
fn riley() -> &'static Unlocked {
    static KEY: std::sync::OnceLock<Unlocked> = std::sync::OnceLock::new();
    KEY.get_or_init(|| generate("Riley Moreno", "riley@studio.example", NOW, &mut rand_core::OsRng).unwrap())
}

#[test]
fn a_made_key_reads_back_as_the_spec_says() {
    assert_eq!(VALID_FOR_SECS, 94_608_000);
    let k = riley();
    let i = info(&k.public).unwrap();
    assert_eq!(i.algorithm, "Ed25519");
    assert!(i.encrypts && i.signs);
    assert_eq!(i.emails, ["riley@studio.example"]);
    assert_eq!(i.user_ids, ["Riley Moreno <riley@studio.example>"]);
    assert_eq!(i.name, "Riley Moreno");
    assert_eq!(i.created, NOW);
    assert_eq!(i.expires, Some(NOW + THREE_YEARS));
    assert_eq!(i.encrypt_until, Some(NOW + THREE_YEARS));
    assert!(i.revoked.is_none() && i.problems(NOW).is_empty() && i.encryptable(NOW).is_ok());
    assert_eq!(i.encryptable(NOW + THREE_YEARS), Err(Problem::Expired(NOW + THREE_YEARS)));
    assert_eq!(k.info, i);
    // What goes to the keychain opens without a passphrase, and is the same key.
    let again = read_secret_key(&k.secret, "").unwrap();
    assert_eq!(again.info.fingerprint, k.info.fingerprint);
    assert_eq!(again.public, k.public);
    assert!(!format!("{k:?}").contains("secret: ["), "never in a log");
    // Without a name, the address alone; an address that isn't one makes nothing.
    let bare = generate("", "Bo@Studio.example", NOW, &mut rand_core::OsRng).unwrap();
    assert_eq!(bare.info.user_ids, ["<Bo@Studio.example>"]);
    assert_eq!(bare.info.emails, ["bo@studio.example"]);
    assert_eq!(bare.info.name, "bo@studio.example");
    // A name can't break out of its user ID.
    let odd = generate("Eve <eve@evil.example>\r\nX", "eve@studio.example", NOW, &mut rand_core::OsRng).unwrap();
    assert_eq!(odd.info.emails, ["eve@studio.example"]);
    for bad in ["", "riley", "a b@c.example", "<riley@studio.example>"] {
        assert!(generate("Riley", bad, NOW, &mut rand_core::OsRng).is_err(), "{bad:?}");
    }
}

/// The signatures on the key say when they were made: `now`, not the system's clock.
#[test]
fn a_made_key_is_stamped_with_the_time_given() {
    let k = riley();
    let key: pgp::composed::SignedPublicKey = pgp::composed::Deserializable::from_bytes(&k.public[..]).unwrap();
    let mut stamps = Vec::new();
    for u in &key.details.users {
        stamps.extend(u.signatures.iter().map(|s| s.created().map(|t| i64::from(t.as_secs()))));
    }
    for s in &key.public_subkeys {
        stamps.extend(s.signatures.iter().map(|s| s.created().map(|t| i64::from(t.as_secs()))));
    }
    assert_eq!(stamps, [Some(NOW), Some(NOW)], "one user ID and one subkey, each signed once");
    // The primary user ID, and the preferences: AES-256, SHA-256, uncompressed or ZLIB.
    let uid_sig = &key.details.users[0].signatures[0];
    assert!(uid_sig.is_primary());
    assert_eq!(uid_sig.preferred_symmetric_algs(), [pgp::crypto::sym::SymmetricKeyAlgorithm::AES256]);
    assert_eq!(uid_sig.preferred_hash_algs(), [pgp::crypto::hash::HashAlgorithm::Sha256]);
    assert_eq!(uid_sig.preferred_compression_algs(), [pgp::types::CompressionAlgorithm::Uncompressed, pgp::types::CompressionAlgorithm::ZLIB]);
    let flags = uid_sig.key_flags();
    assert!(flags.certify() && flags.sign() && !flags.encrypt_comms());
    let sub_flags = key.public_subkeys[0].signatures[0].key_flags();
    assert!(sub_flags.encrypt_comms() && sub_flags.encrypt_storage() && !sub_flags.sign());
}

/// Version 4, the primary key EdDSA-legacy (22), the subkey ECDH (18) on Curve25519: never
/// RFC 9580's version 6 nor its new algorithms (25, 27), which GnuPG 2.4 can't read.
#[test]
fn a_made_key_has_no_v6_algorithm() {
    use pgp::types::KeyDetails as _;
    let k = riley();
    let public: pgp::composed::SignedPublicKey = pgp::composed::Deserializable::from_bytes(&k.public[..]).unwrap();
    let secret: pgp::composed::SignedSecretKey = pgp::composed::Deserializable::from_bytes(&k.secret[..]).unwrap();
    let mut algs = vec![u8::from(public.primary_key.algorithm())];
    algs.extend(public.public_subkeys.iter().map(|s| u8::from(s.key.algorithm())));
    assert_eq!(algs, [22, 18]);
    let mut secret_algs = vec![u8::from(secret.primary_key.algorithm())];
    secret_algs.extend(secret.secret_subkeys.iter().map(|s| u8::from(s.key.algorithm())));
    assert_eq!(secret_algs, [22, 18]);
    assert_eq!(public.primary_key.version(), pgp::types::KeyVersion::V4);
    assert!(public.public_subkeys.iter().all(|s| s.key.version() == pgp::types::KeyVersion::V4));
    let sigs = public.details.users.iter().flat_map(|u| &u.signatures).chain(public.public_subkeys.iter().flat_map(|s| &s.signatures));
    for s in sigs {
        assert_eq!(s.version(), pgp::packet::SignatureVersion::V4);
        assert_eq!(s.hash_alg(), Some(pgp::crypto::hash::HashAlgorithm::Sha256));
    }
}

#[test]
fn extending_keeps_the_key_and_moves_its_end() {
    let k = riley();
    let sealed = encrypt(b"Before.", &[&k.public], None, NOW, &mut rand_core::OsRng).unwrap();
    let later = NOW + 1000;
    let x = extend(k, later, &mut rand_core::OsRng).unwrap();
    assert_eq!(x.info.fingerprint, k.info.fingerprint);
    assert_eq!(x.info.created, NOW);
    assert_eq!(x.info.expires, Some(later + THREE_YEARS));
    assert_eq!(x.info.encrypt_until, Some(later + THREE_YEARS));
    assert_eq!(info(&x.public).unwrap(), x.info);
    assert_eq!(x.info.user_ids, k.info.user_ids);
    // The same key material: what was encrypted to it before still opens.
    assert_eq!(decrypt(sealed.as_bytes(), &[&x.secret], &[]).unwrap().data, b"Before.");
    assert!(same_key(&x.public, &k.public));
    // The old self-signatures are replaced, not piled up.
    let key: pgp::composed::SignedPublicKey = pgp::composed::Deserializable::from_bytes(&x.public[..]).unwrap();
    assert_eq!(key.details.users[0].signatures.len(), 1);
    assert_eq!(key.public_subkeys[0].signatures.len(), 1);
    assert_eq!(key.details.users[0].signatures[0].created().map(|t| i64::from(t.as_secs())), Some(later));
    // Past the first end, the extended key still encrypts and signs.
    let then = NOW + THREE_YEARS + 10;
    assert!(encrypt(b"x", &[&k.public], None, then, &mut rand_core::OsRng).is_err());
    let sealed = encrypt(b"After.", &[&x.public], None, then, &mut rand_core::OsRng).unwrap();
    assert_eq!(decrypt(sealed.as_bytes(), &[&x.secret], &[]).unwrap().data, b"After.");
    let sig = sign(b"part", &x.secret, then, &mut rand_core::OsRng).unwrap();
    assert!(matches!(verify(b"part", sig.as_bytes(), &[&x.public]).unwrap(), Check::Good { key: 0, .. }));
    // Something that isn't an unlocked key isn't extended.
    let junk = Unlocked { secret: b"junk".to_vec(), public: Vec::new(), info: KeyInfo::default() };
    assert!(extend(&junk, later, &mut rand_core::OsRng).is_err());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn a_made_key_signs_and_encrypts_round_trip(data in proptest::collection::vec(any::<u8>(), 0..=64 << 10)) {
        let k = riley();
        let sig = sign(&data, &k.secret, NOW, &mut rand_core::OsRng).unwrap();
        prop_assert_eq!(verify(&data, sig.as_bytes(), &[&k.public]).unwrap(), Check::Good { key: 0, at: Some(NOW), weak: false, until: Some(NOW + VALID_FOR_SECS as i64) });
        let sealed = encrypt(&data, &[&k.public], Some(&k.secret), NOW, &mut rand_core::OsRng).unwrap();
        let d = decrypt(sealed.as_bytes(), &[&k.secret], &[&k.public]).unwrap();
        prop_assert_eq!(d.data, data);
        prop_assert_eq!(d.check, Some(Check::Good { key: 0, at: Some(NOW), weak: false, until: Some(NOW + VALID_FOR_SECS as i64) }));
    }
}

/// The secret scalars of `key` (binary, unprotected): each part's secret MPI, without its length.
fn scalars(key: &[u8]) -> Vec<Vec<u8>> {
    let k: pgp::composed::SignedSecretKey = pgp::composed::Deserializable::from_bytes(key).unwrap();
    let plain = |p: &pgp::types::SecretParams| {
        let pgp::types::SecretParams::Plain(p) = p else { panic!("locked") };
        let mut raw = Vec::new();
        p.to_writer(&mut raw, pgp::types::KeyVersion::V4).unwrap();
        raw[2..raw.len() - 2].to_vec()
    };
    let mut out = vec![plain(k.primary_key.secret_params())];
    out.extend(k.secret_subkeys.iter().map(|s| plain(s.key.secret_params())));
    out
}

#[test]
fn a_backup_reads_back_with_its_passphrase_only() {
    let k = riley();
    let file = backup(k, "correct horse", &mut rand_core::OsRng).unwrap();
    assert!(file.starts_with("-----BEGIN PGP PRIVATE KEY BLOCK-----"), "{file}");
    let back = read_secret_key(file.as_bytes(), "correct horse").unwrap();
    assert_eq!(back.info.fingerprint, k.info.fingerprint);
    assert_eq!(back.public, k.public);
    assert_eq!(scalars(&back.secret), scalars(&k.secret));
    assert_eq!(read_secret_key(file.as_bytes(), "wrong").unwrap_err(), Error::Passphrase);
    assert_eq!(read_secret_key(file.as_bytes(), "").unwrap_err(), Error::Passphrase);
    assert!(matches!(backup(k, "", &mut rand_core::OsRng), Err(Error::Failed(_))));
    // Every secret part locked as GnuPG 2.2 reads it: usage 254, AES-256, iterated and salted
    // SHA-256, each with a salt of its own.
    let (locked, _) = <pgp::composed::SignedSecretKey as pgp::composed::Deserializable>::from_string(&file).unwrap();
    let mut parts = vec![locked.primary_key.secret_params().clone()];
    parts.extend(locked.secret_subkeys.iter().map(|s| s.key.secret_params().clone()));
    assert_eq!(parts.len(), 2);
    let mut salts = Vec::new();
    for p in &parts {
        let pgp::types::SecretParams::Encrypted(e) = p else { panic!("a part in the clear") };
        assert_eq!(e.string_to_key_id(), 254);
        let pgp::types::S2kParams::Cfb { sym_alg, s2k, .. } = e.string_to_key_params() else { panic!("{:?}", e.string_to_key_params()) };
        assert_eq!(*sym_alg, pgp::crypto::sym::SymmetricKeyAlgorithm::AES256);
        let pgp::types::StringToKey::IteratedAndSalted { hash_alg, salt, .. } = s2k else { panic!("{s2k:?}") };
        assert_eq!(*hash_alg, pgp::crypto::hash::HashAlgorithm::Sha256);
        salts.push(*salt);
    }
    assert_ne!(salts[0], salts[1]);
    // No secret byte in the clear: no 16 bytes of either scalar anywhere in the packets.
    let packets = pgp::ser::Serialize::to_bytes(&locked).unwrap();
    for s in scalars(&k.secret) {
        assert!(s.len() >= 16);
        for w in s.windows(16) {
            assert!(!packets.windows(16).any(|p| p == w), "a secret in the clear");
        }
    }
}

/// The public key written as a key file (`pgp.exportPublic`): armored, the same key read back,
/// nothing of the secret in it; what isn't a key is an error.
#[test]
fn a_public_key_is_written_as_a_key_file() {
    let k = riley();
    let file = armored_public(&k.public).unwrap();
    assert!(file.starts_with("-----BEGIN PGP PUBLIC KEY BLOCK-----"), "{file}");
    assert_eq!(public_keys(file.as_bytes()).unwrap(), std::slice::from_ref(&k.public));
    assert!(read_secret_key(file.as_bytes(), "").is_err(), "no secret key in it");
    assert!(armored_public(b"not a key").is_err());
    assert!(armored_public(&k.secret).is_err(), "a secret key isn't written as a public one");
}
