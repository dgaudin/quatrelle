// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The pool's OpenPGP: the account's keys in the keychain (the in-memory one here), never in
//! Quatrelle's files; mail decrypted in memory; a keychain that can't keep anything makes nothing.

use super::*;

/// When GnuPG's test keys are valid.
const NOW: i64 = 1_800_000_000;

fn testdata(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../pgp/testdata").join(name)).unwrap()
}

fn account(id: &str) -> AccountConfig {
    AccountConfig { id: id.into(), email: format!("{id}@posteo.example"), ..Default::default() }
}

fn ana() -> pgp::Unlocked {
    pgp::read_secret_key(&testdata("ana-secret.asc"), "secret").unwrap()
}

/// The pool's events until one `done` says it's the answer (ten seconds at most).
fn answer(p: &Pool, done: impl Fn(&Event) -> bool) -> Event {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if let Some(e) = p.wait(Duration::from_millis(100))
            && done(&e)
        {
            return e;
        }
    }
    panic!("no answer from the pool")
}

/// Every file under `dir`, read.
fn files_under(dir: &std::path::Path) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(files_under(&p));
        } else if let Ok(b) = std::fs::read(&p) {
            out.push(b);
        }
    }
    out
}

/// Whether some 16 bytes of `secret` that its public part doesn't have are in `hay`.
fn holds_secret(hay: &[u8], secret: &[u8], public: &[u8]) -> bool {
    secret.windows(16).filter(|w| !public.windows(16).any(|p| p == *w)).any(|w| hay.windows(16).any(|h| h == w))
}

/// A keychain that can't be reached: nothing it keeps, nothing it gives.
struct NoKeychain;

impl Credentials for NoKeychain {
    fn get(&self, _: &AccountConfig) -> Option<String> {
        None
    }
    fn set(&self, _: &AccountConfig, _: &str) -> std::result::Result<(), String> {
        Err("locked".into())
    }
    fn delete(&self, _: &AccountConfig) {}
    fn get_smtp(&self, _: &AccountConfig) -> Option<String> {
        None
    }
    fn set_smtp(&self, _: &AccountConfig, _: &str) -> std::result::Result<(), String> {
        Err("locked".into())
    }
    fn delete_smtp(&self, _: &AccountConfig) {}
    fn set_secret(&self, _: &AccountConfig, _: &str, _: &str) -> std::result::Result<(), String> {
        Err("the collection is locked".into())
    }
}

/// A keychain that keeps everything but the entry named `refused`.
#[derive(Default)]
struct Refusing {
    held: MemoryCredentials,
    refused: String,
}

impl Credentials for Refusing {
    fn get(&self, a: &AccountConfig) -> Option<String> {
        self.held.get(a)
    }
    fn set(&self, a: &AccountConfig, p: &str) -> std::result::Result<(), String> {
        self.held.set(a, p)
    }
    fn delete(&self, a: &AccountConfig) {
        self.held.delete(a)
    }
    fn get_smtp(&self, a: &AccountConfig) -> Option<String> {
        self.held.get_smtp(a)
    }
    fn set_smtp(&self, a: &AccountConfig, p: &str) -> std::result::Result<(), String> {
        self.held.set_smtp(a, p)
    }
    fn delete_smtp(&self, a: &AccountConfig) {
        self.held.delete_smtp(a)
    }
    fn get_secret(&self, a: &AccountConfig, what: &str) -> Option<String> {
        self.held.get_secret(a, what)
    }
    fn set_secret(&self, a: &AccountConfig, what: &str, secret: &str) -> std::result::Result<(), String> {
        if what == self.refused { Err("refused".into()) } else { self.held.set_secret(a, what, secret) }
    }
    fn delete_secret(&self, a: &AccountConfig, what: &str) {
        self.held.delete_secret(a, what)
    }
}

/// A key made for the account: its secret part in one keychain entry of its own (listed in the
/// account's `pgp` entry), its public part to the engine, nothing of the secret in Quatrelle's
/// files.
#[test]
fn a_key_made_goes_to_the_keychain_and_its_public_part_to_the_engine() {
    let store = crate::fake::tests_store("pgp-make");
    let creds = Arc::new(MemoryCredentials::default());
    let mut p = Pool::new(store.clone(), creds.clone());
    let a = account("riley");
    let work = PgpWork::Make { name: "Riley Moreno".into(), email: "riley@studio.example".into(), now: NOW };
    p.submit(Job::Pgp { account: a.clone(), work });
    let Event::PgpKeys { account: id, keys, changed, error } = answer(&p, |e| matches!(e, Event::PgpKeys { .. })) else { unreachable!() };
    assert_eq!((id.as_str(), error), ("riley", None));
    assert_eq!(keys.len(), 1);
    let info = pgp::info(&keys[0]).unwrap();
    assert_eq!(info.emails, ["riley@studio.example"]);
    assert_eq!(info.expires, Some(NOW + 94_608_000));
    assert_eq!(changed, Some(PgpChange::Made { fingerprint: info.fingerprint.clone() }));
    // One entry for the key, named by its fingerprint, and the list of the account's keys.
    assert_eq!(list(creds.as_ref(), &a), std::slice::from_ref(&info.fingerprint));
    let name = format!("pgp.{}", info.fingerprint.to_lowercase());
    let kept = unhex(&creds.get_secret(&a, &name).unwrap()).unwrap();
    let key = pgp::read_secret_key(&kept, "").unwrap();
    assert_eq!(key.public, keys[0], "the engine's public part is the keychain's key's");
    // The engine is given the public part only; the store's files hold nothing of the secret.
    assert!(!holds_secret(&keys[0], &kept, &key.public));
    std::fs::write(store.root().join("probe.json"), b"{}").unwrap();
    let files = files_under(store.root());
    assert!(!files.is_empty(), "the probe at least");
    for f in &files {
        assert!(!holds_secret(f, &kept, &key.public), "a secret byte in Quatrelle's files");
        assert!(!String::from_utf8_lossy(f).contains(&hex(&kept)[..64]));
    }
    // The secret part is no secret byte smaller for the probe: the check would find it.
    assert!(holds_secret(&kept, &kept, &key.public));
}

/// Review focus 3: the keychain can't be reached (GNOME's keyring locked): making a key says so
/// in Quatrelle's words, and nothing is kept, not half a key.
#[test]
fn a_missing_keychain_makes_nothing() {
    let store = crate::fake::tests_store("pgp-no-keychain");
    let mut p = Pool::new(store, Arc::new(NoKeychain));
    let a = account("riley");
    p.submit(Job::Pgp { account: a.clone(), work: PgpWork::Make { name: "Riley".into(), email: "riley@studio.example".into(), now: NOW } });
    let Event::PgpKeys { keys, changed, error, .. } = answer(&p, |e| matches!(e, Event::PgpKeys { .. })) else { unreachable!() };
    let e = error.expect("an error");
    assert!(e.starts_with(NO_KEYCHAIN), "{e}");
    assert!(e.contains("the collection is locked"), "and what the system said: {e}");
    assert!(keys.is_empty() && changed.is_none(), "no public key kept");
    // Importing says the same, never that the key or passphrase is wrong.
    let file = Private(testdata("ana-secret.asc"));
    p.submit(Job::Pgp { account: a.clone(), work: PgpWork::Import { file, passphrase: Secret("secret".into()) } });
    let Event::PgpKeys { error, keys, .. } = answer(&p, |e| matches!(e, Event::PgpKeys { .. })) else { unreachable!() };
    assert!(error.unwrap().starts_with(NO_KEYCHAIN) && keys.is_empty());
    // Decrypting with the keychain locked: the keychain, not a wrong key.
    let sealed = pgp::encrypt(b"x", &[&ana().public], None, NOW, &mut rand_core::OsRng).unwrap();
    let e = decrypt(&NoKeychain, &a, sealed.as_bytes(), false, "", &[]).unwrap_err();
    assert!(e.starts_with(NO_KEYCHAIN), "{e}");
    // The key's entry kept but its listing refused: the entry goes too.
    let creds = Refusing { refused: LIST.into(), ..Default::default() };
    let ana = ana();
    assert!(take(&creds, &a, &ana).unwrap_err().starts_with(NO_KEYCHAIN));
    assert!(creds.get_secret(&a, &entry(&ana.info.fingerprint)).is_none(), "nothing half-made");
    assert!(list(&creds, &a).is_empty());
}

/// Upstream's, through the keychain: a key imported (its passphrase never kept), mail encrypted
/// and signed to it decrypted with what the keychain has, its signature checked; the plain text
/// never reaches a file. A second key takes over, the first still decrypts; all forgotten,
/// nothing is left.
#[test]
fn encrypted_mail_is_decrypted_in_memory_only() {
    let store = crate::fake::tests_store("pgp-decrypt");
    let creds = Arc::new(MemoryCredentials::default());
    let mut p = Pool::new(store.clone(), creds.clone());
    let a = account("ana");
    p.submit(Job::Pgp {
        account: a.clone(),
        work: PgpWork::Import { file: Private(testdata("ana-secret.asc")), passphrase: Secret("secret".into()) },
    });
    let Event::PgpKeys { keys, changed, error, .. } = answer(&p, |e| matches!(e, Event::PgpKeys { .. })) else { unreachable!() };
    let ana = ana();
    assert_eq!(error, None);
    assert_eq!(changed, Some(PgpChange::Imported { fingerprint: ana.info.fingerprint.clone() }));
    assert_eq!(keys, [ana.public.clone()]);
    let kept = creds.get_secret(&a, &entry(&ana.info.fingerprint)).unwrap();
    assert!(!kept.contains("secret") && unhex(&kept).is_some());
    let entity = b"Content-Type: text/plain; charset=utf-8\r\n\r\nThe plan for Tuesday\r\n";
    let sealed = pgp::encrypt(entity, &[&ana.public], Some(&ana.secret), NOW, &mut rand_core::OsRng).unwrap();
    let work = PgpWork::Decrypt {
        key: MessageKey::default(),
        data: sealed.clone().into_bytes(),
        inline: false,
        charset: String::new(),
        verify_with: vec![ana.public.clone()],
    };
    p.submit(Job::Pgp { account: a.clone(), work });
    let Event::PgpDecrypted { content, signature, error, .. } = answer(&p, |e| matches!(e, Event::PgpDecrypted { .. })) else { unreachable!() };
    assert_eq!(error, None);
    assert_eq!(content.unwrap().0, entity);
    assert!(matches!(signature, Some(PgpSignature::Good { key: 0, .. })), "{signature:?}");
    for f in files_under(store.root()) {
        assert!(!String::from_utf8_lossy(&f).contains("The plan for Tuesday"), "decrypted text in a file");
    }
    // A second key takes over; the first still decrypts. Forgotten, nothing is left.
    let mut second = ana.clone();
    second.info.fingerprint = "00".repeat(20);
    take(creds.as_ref(), &a, &second).unwrap();
    assert_eq!(list(creds.as_ref(), &a).len(), 2);
    assert!(decrypt(creds.as_ref(), &a, sealed.as_bytes(), false, "", &[]).is_ok(), "the older key is tried too");
    use_key(creds.as_ref(), &a, &ana.info.fingerprint).unwrap();
    assert_eq!(list(creds.as_ref(), &a)[0], ana.info.fingerprint);
    forget_all(creds.as_ref(), &a);
    assert!(list(creds.as_ref(), &a).is_empty() && creds.get_secret(&a, &entry(&ana.info.fingerprint)).is_none());
    assert!(creds.get_secret(&a, &entry(&second.info.fingerprint)).is_none() && creds.get_secret(&a, LIST).is_none());
    assert!(decrypt(creds.as_ref(), &a, sealed.as_bytes(), false, "", &[]).is_err());
}

/// Mail GnuPG encrypted to Bo alone: Ana's keychain says to which key it went.
#[test]
fn mail_for_someone_else_says_so() {
    let creds = MemoryCredentials::default();
    let a = account("ana");
    take(&creds, &a, &ana()).unwrap();
    let e = decrypt(&creds, &a, &testdata("to-bo.asc"), false, "", &[]).unwrap_err();
    assert!(e.contains("encrypted to a key Quatrelle doesn't have ("), "{e}");
    // GnuPG's own signed and encrypted mail to Ana, read as inline PGP: a text, Bo's signature good.
    let bo = pgp::public_keys(&testdata("bo.asc")).unwrap();
    let (entity, signature) = decrypt(&creds, &a, &testdata("to-ana-signed.asc"), true, "iso-8859-1", &bo).unwrap();
    assert!(entity.starts_with(b"Content-Type: text/plain; charset=utf-8\r\n"));
    assert!(matches!(signature, Some(PgpSignature::Good { key: 0, .. })));
}

#[test]
fn at_most_so_many_keys() {
    let creds = MemoryCredentials::default();
    let a = account("ana");
    let ana = ana();
    for n in 0..KEYS_MAX {
        let mut k = ana.clone();
        k.info.fingerprint = format!("{n:040X}");
        take(&creds, &a, &k).unwrap();
    }
    let mut one_more = ana.clone();
    one_more.info.fingerprint = "F".repeat(40);
    assert!(take(&creds, &a, &one_more).unwrap_err().contains("at most"));
    // The same one again isn't one more.
    let mut again = ana;
    again.info.fingerprint = format!("{:040X}", 3);
    take(&creds, &a, &again).unwrap();
}

/// A key made here, extended (same fingerprint, three years from the day), then backed up to a
/// file that opens with its passphrase only.
#[test]
fn a_key_is_extended_and_backed_up() {
    let store = crate::fake::tests_store("pgp-extend");
    let creds = Arc::new(MemoryCredentials::default());
    let mut p = Pool::new(store.clone(), creds.clone());
    let a = account("riley");
    p.submit(Job::Pgp { account: a.clone(), work: PgpWork::Make { name: "Riley".into(), email: "riley@studio.example".into(), now: NOW } });
    let Event::PgpKeys { keys, .. } = answer(&p, |e| matches!(e, Event::PgpKeys { .. })) else { unreachable!() };
    let fp = pgp::info(&keys[0]).unwrap().fingerprint;
    let later = NOW + 1000 * 86_400;
    p.submit(Job::Pgp { account: a.clone(), work: PgpWork::Extend { fingerprint: fp.to_lowercase(), now: later } });
    let Event::PgpKeys { keys, changed, error, .. } = answer(&p, |e| matches!(e, Event::PgpKeys { .. })) else { unreachable!() };
    assert_eq!((error, changed), (None, Some(PgpChange::Extended { fingerprint: fp.clone() })));
    let info = pgp::info(&keys[0]).unwrap();
    assert_eq!((info.fingerprint.as_str(), info.expires), (fp.as_str(), Some(later + 94_608_000)));
    assert_eq!(list(creds.as_ref(), &a).len(), 1, "the same key, not another");
    // Backed up: the file opens with the passphrase, and is the extended key.
    let path = store.root().join("riley.asc");
    let backup = |pass: &str| PgpWork::Backup { fingerprint: fp.clone(), passphrase: Secret(pass.into()), path: path.display().to_string() };
    p.submit(Job::Pgp { account: a.clone(), work: backup("correct horse") });
    let done = answer(&p, |e| matches!(e, Event::PgpBackedUp { .. }));
    assert_eq!(done, Event::PgpBackedUp { account: "riley".into(), fingerprint: fp.clone(), error: None });
    let file = std::fs::read(&path).unwrap();
    assert!(file.starts_with(b"-----BEGIN PGP PRIVATE KEY BLOCK-----"));
    let back = pgp::read_secret_key(&file, "correct horse").unwrap();
    assert_eq!((back.info.fingerprint.as_str(), back.info.expires), (fp.as_str(), Some(later + 94_608_000)));
    assert!(pgp::read_secret_key(&file, "wrong").is_err());
    // No passphrase, no key, nowhere to write: said, nothing written.
    std::fs::remove_file(&path).unwrap();
    p.submit(Job::Pgp { account: a.clone(), work: backup("") });
    let Event::PgpBackedUp { error, .. } = answer(&p, |e| matches!(e, Event::PgpBackedUp { .. })) else { unreachable!() };
    assert!(error.is_some() && !path.exists());
    p.submit(Job::Pgp {
        account: a.clone(),
        work: PgpWork::Backup { fingerprint: "AB".repeat(20), passphrase: Secret("pw".into()), path: path.display().to_string() },
    });
    let Event::PgpBackedUp { error, .. } = answer(&p, |e| matches!(e, Event::PgpBackedUp { .. })) else { unreachable!() };
    assert!(error.is_some() && !path.exists());
    // A folder where the file would go.
    let nowhere = store.root().display().to_string();
    p.submit(Job::Pgp { account: a, work: PgpWork::Backup { fingerprint: fp, passphrase: Secret("pw".into()), path: nowhere } });
    let Event::PgpBackedUp { error, .. } = answer(&p, |e| matches!(e, Event::PgpBackedUp { .. })) else { unreachable!() };
    assert!(error.is_some());
}

/// An account removed: its keys' secret parts go from the keychain with it.
#[test]
fn removing_an_account_takes_its_keys_from_the_keychain() {
    let store = crate::fake::tests_store("pgp-remove");
    let creds = Arc::new(MemoryCredentials::default());
    let mut p = Pool::new(store, creds.clone());
    let a = account("ana");
    take(creds.as_ref(), &a, &ana()).unwrap();
    p.configs.insert(a.id.clone(), a.clone());
    p.submit(Job::Remove { account: a.id.clone() });
    assert!(list(creds.as_ref(), &a).is_empty());
    assert!(creds.get_secret(&a, &entry(&ana().info.fingerprint)).is_none(), "no orphan secret");
}
