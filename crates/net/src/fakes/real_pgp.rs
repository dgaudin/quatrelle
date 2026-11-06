// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! OpenPGP through a real mail server, by hand (issue #36; upstream's, from f32b54b, never run
//! here): GreenMail's SMTP and IMAP, with two made-up accounts and their keys, made by GnuPG.
//!
//! ```text
//! QUATRELLE_REAL_PGP="127.0.0.1:3025 127.0.0.1:3143 pia@studio.test piapw ole@studio.test olepw" \
//! QUATRELLE_PGP_DIR=<dir> cargo test -p quatrelle-net real_pgp -- --ignored --nocapture
//! ```
//!
//! `<dir>` holds `pia-secret.asc` (passphrase `piapass`), `ole-secret.asc` (none) and
//! `gpg-to-ole.eml`, a PGP/MIME message GnuPG encrypted to Ole and signed as Pia. The pool keeps
//! both keys (their passphrases taken off, in the keychain: here, memory); Pia's message is signed
//! and encrypted by the pool and sent, with her key in its `Autocrypt:` header, GnuPG's too; both
//! are fetched back over IMAP as the server delivered them to Ole, decrypted by the pool with
//! Ole's key, and their signatures checked. Quatrelle's copy is written to `<dir>/q-delivered.eml`,
//! for GnuPG to decrypt and verify in turn.

use std::time::{Duration, Instant};

use quatrelle_model::job::{PgpSignature, PgpWork, Secret};
use quatrelle_model::{AccountConfig, Event, Job, Private, Security, ServerConfig};

use crate::worker::{MemoryCredentials, Pool};

fn until(pool: &Pool, secs: u64, what: &str, mut f: impl FnMut(&Event) -> bool) -> Vec<Event> {
    let end = Instant::now() + Duration::from_secs(secs);
    let mut seen = Vec::new();
    while Instant::now() < end {
        if let Some(e) = pool.wait(Duration::from_millis(100)) {
            let hit = f(&e);
            seen.push(e);
            if hit {
                return seen;
            }
        }
    }
    panic!("timed out waiting for {what}; last events: {:?}", seen.iter().rev().take(5).collect::<Vec<_>>());
}

#[test]
#[ignore]
fn real_pgp_round_trip() {
    let (Ok(v), Ok(dir)) = (std::env::var("QUATRELLE_REAL_PGP"), std::env::var("QUATRELLE_PGP_DIR")) else {
        eprintln!("QUATRELLE_REAL_PGP or QUATRELLE_PGP_DIR isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [smtp, imap, pia, pia_pw, ole, ole_pw] = p.as_slice() else { panic!("QUATRELLE_REAL_PGP: smtp imap pia pia-password ole ole-password") };
    let cfg = |a: &str| {
        let (host, port) = a.rsplit_once(':').unwrap();
        ServerConfig { host: host.into(), port: port.parse().unwrap(), security: Security::None }
    };
    let (smtp, imap) = (cfg(smtp), cfg(imap));
    let dir = std::path::PathBuf::from(dir);
    let file = |n: &str| std::fs::read(dir.join(n)).unwrap_or_else(|e| panic!("{n}: {e}"));
    let store = crate::fake::store("real-pgp");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    let account = |id: &str, email: &str| AccountConfig {
        id: id.into(),
        email: email.into(),
        username: email.into(),
        imap: imap.clone(),
        smtp: smtp.clone(),
        ..Default::default()
    };
    let (pia_acc, ole_acc) = (account("p1", pia), account("o1", ole));
    // Each one's secret key, imported: Pia's with its passphrase, which a wrong one doesn't open.
    let mut import = |acc: &AccountConfig, name: &str, passphrase: &str| -> std::result::Result<Vec<Vec<u8>>, String> {
        pool.submit(Job::Pgp { account: acc.clone(), work: PgpWork::Import { file: Private(file(name)), passphrase: Secret(passphrase.into()) } });
        let seen = until(&pool, 30, "the key", |e| matches!(e, Event::PgpKeys { account, .. } if *account == acc.id));
        match seen.last() {
            Some(Event::PgpKeys { error: None, keys, .. }) => Ok(keys.clone()),
            Some(Event::PgpKeys { error: Some(e), .. }) => Err(e.clone()),
            other => panic!("{other:?}"),
        }
    };
    assert!(import(&pia_acc, "pia-secret.asc", "wrong").unwrap_err().contains("passphrase"));
    let pia_pub = import(&pia_acc, "pia-secret.asc", "piapass").unwrap().remove(0);
    let ole_pub = import(&ole_acc, "ole-secret.asc", "").unwrap().remove(0);
    let tag = std::process::id();
    // Pia's message, signed and encrypted by the pool, her key in its Autocrypt header.
    let small = quatrelle_pgp::minimal(&pia_pub, pia).unwrap();
    let raw = format!(
        "{}From: Pia <{pia}>\r\nTo: Ole <{ole}>\r\nSubject: Plan {tag}\r\nDate: Wed, 04 Nov 2026 10:00:00 +0000\r\nMessage-ID: <p{tag}@studio.test>\r\n\
MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nThe plan =E2=9C=93, from Pia.\r\n",
        quatrelle_pgp::autocrypt::header(pia, &small, false)
    );
    let work = PgpWork::Protect {
        id: 7,
        raw: Private(raw.into_bytes()),
        sign: true,
        encrypt_to: vec![ole_pub.clone(), pia_pub.clone()],
        hide_subject: true,
    };
    pool.submit(Job::Pgp { account: pia_acc.clone(), work });
    let seen = until(&pool, 30, "the protected message", |e| matches!(e, Event::Protected { .. }));
    let Some(Event::Protected { raw: Some(sealed), error: None, .. }) = seen.last() else { panic!("{seen:?}") };
    crate::smtp::send(&smtp, pia, pia_pw, pia, &[ole.to_string()], &sealed.0).unwrap();
    // GnuPG's, sent as it is.
    let gpg = file("gpg-to-ole.eml");
    let gpg_subject = quatrelle_mime::parse(&gpg).map(|m| m.subject).unwrap_or_default();
    crate::smtp::send(&smtp, pia, pia_pw, pia, &[ole.to_string()], &gpg).unwrap();
    // Both as the server delivered them to Ole, decrypted with Ole's key.
    let mut c = crate::imap::Client::connect(&imap).unwrap();
    c.login(ole, ole_pw).unwrap();
    let mut raw_of = |subject: &str| -> Vec<u8> {
        let end = Instant::now() + Duration::from_secs(30);
        let uid = loop {
            c.select("INBOX", true).unwrap();
            if let Some(u) = c.uid_search(&format!("SUBJECT \"{subject}\"")).unwrap().last().copied() {
                break u;
            }
            assert!(Instant::now() < end, "{subject} wasn't delivered");
            std::thread::sleep(Duration::from_millis(300));
        };
        c.uid_fetch(&uid.to_string(), "BODY.PEEK[]", "").unwrap().into_iter().find_map(|f| f.body).unwrap()
    };
    // Encrypted, the subject outside is `[...]`: Quatrelle's is found by GnuPG's, then by elimination.
    let delivered_gpg = raw_of(&gpg_subject);
    let delivered_mc = raw_of("[...]");
    std::fs::write(dir.join("q-delivered.eml"), &delivered_mc).unwrap();
    assert!(String::from_utf8_lossy(&delivered_mc).contains(&format!("Autocrypt: addr={pia}; keydata=")));
    for (n, raw) in [(1u32, &delivered_mc), (2, &delivered_gpg)] {
        let quatrelle_mime::pgp::Protection::Encrypted(data) = quatrelle_mime::pgp::protection(raw) else { panic!("{n}: not PGP/MIME encrypted") };
        let key = quatrelle_model::MessageKey { account: "o1".into(), folder: "INBOX".into(), validity: 1, uid: n };
        let work = PgpWork::Decrypt { key, data, inline: false, charset: String::new(), verify_with: vec![pia_pub.clone()] };
        pool.submit(Job::Pgp { account: ole_acc.clone(), work });
        let seen = until(&pool, 30, "decrypted", |e| matches!(e, Event::PgpDecrypted { key, .. } if key.uid == n));
        let Some(Event::PgpDecrypted { content: Some(entity), error: None, signature, .. }) = seen.last() else { panic!("{n}: {seen:?}") };
        let inner = quatrelle_mime::smime::with_entity(raw, &entity.0);
        let (text, subject) = match quatrelle_mime::pgp::protection(&inner) {
            // Signed, then encrypted (RFC 3156 §6.1): Quatrelle's way.
            quatrelle_mime::pgp::Protection::Signed { content, signature } => {
                let check = quatrelle_pgp::verify(&content, &signature, &[&pia_pub]).unwrap();
                assert!(matches!(check, quatrelle_pgp::Check::Good { key: 0, .. }), "{n}: {check:?}");
                let read = quatrelle_mime::parse(&quatrelle_mime::smime::with_entity_protected(&inner, &content).0).unwrap();
                (read.text, read.subject)
            }
            // Signed inside the encryption (§6.2): GnuPG's way.
            _ => {
                assert!(matches!(signature, Some(PgpSignature::Good { key: 0, .. })), "{n}: {signature:?}");
                let read = quatrelle_mime::parse(&inner).unwrap();
                (read.text, read.subject)
            }
        };
        eprintln!("{n}: {subject}: {}", text.trim());
        if n == 1 {
            assert_eq!(subject, format!("Plan {tag}"), "the real subject, from inside");
            assert!(text.contains("The plan \u{2713}, from Pia."), "{text}");
        } else {
            assert!(text.contains("from GnuPG"), "{text}");
        }
    }
    let _ = std::fs::remove_dir_all(store.root());
}
