// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Keys and messages GnuPG 2.4 made, run as a command-line tool in a keyring of its own
//! (`--quick-gen-key`, `--quick-add-key`; the expired key with `--faked-system-time 20200101T000000`
//! and a day's validity, the withdrawn one with its revocation certificate imported; the messages
//! with `--sign --encrypt`, `--encrypt`, `--clearsign` and `--detach-sign`), read here: what other
//! programs send reads in Quatrelle. Ana's passphrase is `secret`; Bo's key has none.

use super::*;

pub(super) fn testdata(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata").join(name)).unwrap()
}

fn public(name: &str) -> Vec<u8> {
    public_keys(&testdata(name)).unwrap().remove(0)
}

#[test]
fn gnupg_keys_read_as_they_are() {
    let ana = read_secret_key(&testdata("ana-secret.asc"), "secret").unwrap();
    assert_eq!(ana.info.emails, ["ana@posteo.example"]);
    assert!(ana.info.encrypts && ana.info.signs && ana.info.expires.is_none() && ana.info.revoked.is_none());
    assert_eq!(read_secret_key(&testdata("ana-secret.asc"), "Secret").unwrap_err(), Error::Passphrase);
    // Binary, without a passphrase.
    let bo = read_secret_key(&testdata("bo-secret.gpg"), "").unwrap();
    assert_eq!((bo.info.name.as_str(), bo.info.algorithm.as_str()), ("Bo Berg", "RSA"));
    assert!(same_key(&bo.public, &public("bo.asc")));
    // One that signs only, one that ran out, one withdrawn.
    let sig = info(&public("sign-only.asc")).unwrap();
    assert!(sig.signs && !sig.encrypts);
    let old = info(&public("expired.asc")).unwrap();
    assert_eq!(old.expires, Some(1_577_836_800 + 86_400));
    assert_eq!(old.encryptable(NOW), Err(Problem::Expired(1_577_836_800 + 86_400)));
    let rev = info(&public("revoked.asc")).unwrap();
    assert!(rev.revoked.is_some());
    assert!(matches!(rev.encryptable(NOW), Err(Problem::Revoked(_))));
    for k in ["sign-only.asc", "expired.asc", "revoked.asc"] {
        assert!(encrypt(b"x", &[&public(k)], None, NOW, &mut rand_core::OsRng).is_err(), "{k}");
    }
}

#[test]
fn gnupg_mail_decrypts_and_verifies() {
    let ana = read_secret_key(&testdata("ana-secret.asc"), "secret").unwrap();
    let bo = public("bo.asc");
    let d = decrypt(&testdata("to-ana-signed.asc"), &[&ana.secret], &[&bo]).unwrap();
    assert_eq!(d.data, testdata("part.txt"));
    assert!(matches!(d.check, Some(Check::Good { key: 0, .. })), "{:?}", d.check);
    // Encrypted to Bo alone: Ana can't, and the message says to whom it went.
    assert_eq!(decrypt(&testdata("to-bo.asc"), &[&ana.secret], &[]).unwrap_err(), Error::Decrypt);
    let bo_secret = read_secret_key(&testdata("bo-secret.gpg"), "").unwrap();
    assert_eq!(recipients(&testdata("to-bo.asc")).len(), 1);
    assert_eq!(decrypt(&testdata("to-bo.asc"), &[&ana.secret, &bo_secret.secret], &[]).unwrap().data, testdata("part.txt"));
    // A detached signature, and a cleartext-signed text with a dash-escaped line.
    assert!(matches!(verify(&testdata("part.txt"), &testdata("part.sig"), &[&bo]).unwrap(), Check::Good { key: 0, .. }));
    assert_eq!(verify(b"Content-Type: text/plain\r\n\r\nchanged\r\n", &testdata("part.sig"), &[&bo]).unwrap(), Check::Bad);
    let (text, check) = verify_cleartext(&testdata("clearsigned.txt"), &[&bo]).unwrap();
    assert!(matches!(check, Check::Good { .. }));
    assert!(text.contains("\r\n- this line starts with a dash\r\n"), "{text:?}");
}

/// GnuPG, where it's installed, reads what Quatrelle makes, and Quatrelle what GnuPG makes with
/// that key: the backup imported with its passphrase, mail decrypted, signatures verified both
/// ways, the end of the key and of its extension as GnuPG lists them. Run by hand:
/// `cargo test -p quatrelle-pgp gnupg_reads -- --ignored --nocapture`. Its keyring is a directory
/// of its own under the temporary directory, given to each `gpg` run (`GNUPGHOME`) and removed
/// at the end: the user's own keyring is never touched.
#[test]
#[ignore = "needs GnuPG (gpg on PATH); run by hand"]
fn gnupg_reads_a_key_and_mail_quatrelle_made() {
    use std::process::Command;
    if Command::new("gpg").arg("--version").output().is_err() {
        eprintln!("gpg isn't on PATH: skipped");
        return;
    }
    // GnuPG refuses a key made in the future: this one is made now.
    let now = i64::try_from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()).unwrap();
    let mut rng = rand_core::OsRng;
    let k = generate("Riley Moreno", "riley@studio.example", now, &mut rng).unwrap();
    let fp = k.info.fingerprint.clone();
    let home = std::env::temp_dir().join(format!("quatrelle-gnupg-{}-{now}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let file = |name: &str, data: &[u8]| {
        let p = home.join(name);
        std::fs::write(&p, data).unwrap();
        p.to_string_lossy().into_owned()
    };
    let gpg = |args: &[&str]| {
        let o = Command::new("gpg")
            .env("GNUPGHOME", &home)
            // Status lines (`[GNUPG:] GOODSIG`) say what happened whatever the language.
            .args([
                "--batch",
                "--no-tty",
                "--status-fd",
                "2",
                "--pinentry-mode",
                "loopback",
                "--passphrase",
                "correct horse",
                "--trust-model",
                "always",
            ])
            .args(args)
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&o.stderr).into_owned();
        eprintln!("gpg {}: {}\n{err}", args.join(" "), o.status);
        (o.status.success(), o.stdout, err)
    };
    let expiry = |listing: &[u8], kind: &str| -> Option<i64> {
        String::from_utf8_lossy(listing).lines().find(|l| l.starts_with(kind)).and_then(|l| l.split(':').nth(6)).and_then(|e| e.parse().ok())
    };
    let check = || {
        // The backup, imported with its passphrase.
        let (ok, _, _) = gpg(&["--import", &file("backup.asc", backup(&k, "correct horse", &mut rng).unwrap().as_bytes())]);
        assert!(ok, "the backup imports");
        let (ok, listing, _) = gpg(&["--with-colons", "--list-secret-keys", &fp]);
        assert!(ok && String::from_utf8_lossy(&listing).contains(&fp));
        let end = now + i64::try_from(VALID_FOR_SECS).unwrap();
        assert_eq!(expiry(&listing, "sec:"), Some(end));
        assert_eq!(expiry(&listing, "ssb:"), Some(end));
        // Mail Quatrelle encrypted and signed, read by GnuPG.
        let part = b"Content-Type: text/plain; charset=utf-8\r\n\r\nFrom Quatrelle.\r\n";
        let sealed = encrypt(part, &[&k.public], Some(&k.secret), now, &mut rng).unwrap();
        let (ok, out, err) = gpg(&["--decrypt", &file("sealed.asc", sealed.as_bytes())]);
        assert!(ok && out == part, "GnuPG decrypts");
        assert!(err.contains("[GNUPG:] GOODSIG "), "and checks the signature inside");
        let sig = sign(part, &k.secret, now, &mut rng).unwrap();
        let (ok, _, err) = gpg(&["--verify", &file("part.sig", sig.as_bytes()), &file("part.txt", part)]);
        assert!(ok && err.contains("[GNUPG:] GOODSIG "), "GnuPG verifies a detached signature");
        // What GnuPG makes with the key, read by Quatrelle.
        let from = home.join("gpg.sig").to_string_lossy().into_owned();
        let (ok, _, _) = gpg(&["--local-user", &fp, "--armor", "--output", &from, "--detach-sign", &file("theirs.txt", part)]);
        assert!(ok);
        let theirs = std::fs::read(&from).unwrap();
        assert!(matches!(verify(part, &theirs, &[&k.public]).unwrap(), Check::Good { key: 0, .. }), "Quatrelle verifies GnuPG's signature");
        let to = home.join("gpg.asc").to_string_lossy().into_owned();
        let (ok, _, _) = gpg(&["--armor", "--recipient", &fp, "--output", &to, "--sign", "--local-user", &fp, "--encrypt", &file("plain.txt", part)]);
        assert!(ok);
        let d = decrypt(&std::fs::read(&to).unwrap(), &[&k.secret], &[&k.public]).unwrap();
        assert_eq!(d.data, part, "Quatrelle decrypts GnuPG's");
        assert!(matches!(d.check, Some(Check::Good { key: 0, .. })));
        // The public key GnuPG exports is the same key, and an extension moves its end there.
        let (ok, exported, _) = gpg(&["--armor", "--export", &fp]);
        assert!(ok);
        assert!(same_key(&public_keys(&exported).unwrap()[0], &k.public));
        let x = extend(&k, now + 1000, &mut rng).unwrap();
        let armored = {
            let p: pgp::composed::SignedPublicKey = pgp::composed::Deserializable::from_bytes(&x.public[..]).unwrap();
            p.to_armored_string(Default::default()).unwrap()
        };
        let (ok, _, _) = gpg(&["--import", &file("extended.asc", armored.as_bytes())]);
        assert!(ok);
        let (_, listing, _) = gpg(&["--with-colons", "--list-keys", &fp]);
        assert_eq!(expiry(&listing, "pub:"), Some(end + 1000), "GnuPG takes the extension");
        assert_eq!(expiry(&listing, "sub:"), Some(end + 1000));
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(check));
    let _ = Command::new("gpgconf").env("GNUPGHOME", &home).args(["--kill", "all"]).output();
    let _ = std::fs::remove_dir_all(&home);
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

/// A signature says until when it counts: the end of the part of the key that made it (GnuPG's
/// key for Kim signs with a subkey valid for a day; the key itself has no end), or its own.
#[test]
fn a_signature_says_when_its_key_part_runs_out() {
    use pgp::packet::{SignatureConfig, SignatureType, Subpacket, SubpacketData};
    use pgp::types::KeyDetails as _;
    let kim = public("kim.asc");
    assert_eq!(info(&kim).unwrap().expires, None);
    let said = verify(&testdata("part.txt"), &testdata("kim-part.sig"), &[&kim]).unwrap();
    assert_eq!(said, Check::Good { key: 0, at: Some(1_780_315_200), weak: false, until: Some(1_780_358_400) });
    // A signature that runs out an hour after it was made.
    let ana = super::unlocked("Ana <ana@posteo.example>");
    let key = crate::message::secret(&ana.secret).unwrap();
    let mut config = SignatureConfig::from_key(rand_core::OsRng, &key.primary_key, SignatureType::Binary).unwrap();
    config.hashed_subpackets = vec![
        Subpacket::regular(SubpacketData::IssuerFingerprint(key.primary_key.fingerprint())).unwrap(),
        Subpacket::regular(SubpacketData::SignatureCreationTime(crate::stamp(NOW))).unwrap(),
        Subpacket::regular(SubpacketData::SignatureExpirationTime(pgp::types::Duration::from_secs(3600))).unwrap(),
    ];
    let made = config.sign(&key.primary_key, &pgp::types::Password::empty(), &b"part"[..]).unwrap();
    let signature = pgp::composed::DetachedSignature::new(made).to_armored_bytes(Default::default()).unwrap();
    let said = verify(b"part", &signature, &[&ana.public]).unwrap();
    assert_eq!(said, Check::Good { key: 0, at: Some(NOW), weak: false, until: Some(NOW + 3600) });
}
