// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: reading OpenPGP mail (adapted from upstream f32b54b), with GnuPG's keys and mail
//! (`crates/pgp/testdata`: Riley's key, and Ana's, who writes to Riley). A signature says whose
//! key made it, whether it holds and how its key was learnt; encrypted mail is decrypted by the
//! network layer (played here with the pgp crate, as the pool does) and kept in memory only;
//! mail encrypted to a key Quatrelle doesn't have says so; a protected subject shows once
//! decrypted.

use super::*;
use quatrelle_model::Private;
use quatrelle_model::job::{NOT_FOR_YOU, PgpSignature};

/// After GnuPG made the test keys.
const PGP_NOW: i64 = 1_800_000_000;

/// The sample session, at [`PGP_NOW`].
fn session(name: &str) -> Session {
    let mut s = sample_session(name);
    s.clock.now = PGP_NOW;
    s
}

fn pgpdata(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../pgp/testdata").join(name)).unwrap()
}

fn riley_key() -> pgp::Unlocked {
    pgp::read_secret_key(&pgpdata("riley-secret.asc"), "").unwrap()
}

fn ana_key() -> pgp::Unlocked {
    pgp::read_secret_key(&pgpdata("ana-secret.asc"), "secret").unwrap()
}

/// The sample session with Riley's GnuPG key imported (the network layer's answer played).
fn with_riley(name: &str) -> Session {
    let mut s = session(name);
    let k = riley_key();
    let changed = Some(PgpChange::Imported { fingerprint: k.info.fingerprint.clone() });
    s.apply(Event::PgpKeys { account: "sample".into(), keys: vec![k.public], changed, error: None });
    assert!(s.pgp_active("sample"));
    s
}

/// Ana's mail about `subject`: `body` (a MIME entity's header and body), with her key in an
/// `Autocrypt:` header when `autocrypt`.
fn from_ana_about(subject: &str, autocrypt: bool, content_type: &str, body: &str) -> Vec<u8> {
    let mut m = format!(
        "From: Ana Lima <ana@posteo.example>\r\nTo: riley@studio.example\r\nSubject: {subject}\r\nDate: Sat, 24 Oct 2026 09:00:00 +0000\r\n\
Message-ID: <{}@posteo.example>\r\n",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    if autocrypt {
        m.push_str(&pgp::autocrypt::header("ana@posteo.example", &ana_key().public, false));
    }
    m.push_str(&format!("MIME-Version: 1.0\r\nContent-Type: {content_type}\r\n\r\n{body}"));
    m.into_bytes()
}

fn from_ana(autocrypt: bool, content_type: &str, body: &str) -> Vec<u8> {
    from_ana_about("Proof", autocrypt, content_type, body)
}

/// Ana's PGP/MIME signed message (GnuPG's detached signature over the part).
fn signed_by_ana(autocrypt: bool) -> String {
    let body = format!(
        "--sig\r\n{}\r\n--sig\r\nContent-Type: application/pgp-signature; name=\"signature.asc\"\r\n\r\n{}\r\n--sig--\r\n",
        String::from_utf8_lossy(&pgpdata("ana-part.txt")),
        String::from_utf8_lossy(&pgpdata("ana-part.sig"))
    );
    let raw = from_ana(autocrypt, "multipart/signed; micalg=pgp-sha512; protocol=\"application/pgp-signature\"; boundary=\"sig\"", &body);
    String::from_utf8(raw).unwrap()
}

/// PGP/MIME encrypted mail from Ana holding `armored`.
fn encrypted_from_ana(subject: &str, autocrypt: bool, armored: &str) -> Vec<u8> {
    let body = format!(
        "--enc\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n\r\n--enc\r\n\
Content-Type: application/octet-stream; name=\"encrypted.asc\"\r\n\r\n{armored}\r\n--enc--\r\n"
    );
    from_ana_about(subject, autocrypt, "multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=\"enc\"", &body)
}

/// Plays the network layer: decrypts what was asked for with Riley's key, as the pool does
/// (keys attached inside handed back, a message for another key said so).
fn answer_decrypt(s: &mut Session) -> usize {
    let riley = riley_key();
    let mut n = 0;
    for j in s.take_jobs() {
        let Job::Pgp { work: PgpWork::Decrypt { key, data, inline, charset, verify_with }, .. } = j else { continue };
        n += 1;
        let keys: Vec<&[u8]> = verify_with.iter().map(Vec::as_slice).collect();
        let event = match pgp::decrypt(&data, &[&riley.secret], &keys) {
            Ok(d) => {
                let entity = if inline { quatrelle_mime::pgp::text_entity(&d.data, &charset) } else { d.data };
                let inside = quatrelle_mime::pgp::key_parts(&entity).iter().flat_map(|k| pgp::public_keys(k).unwrap_or_default()).collect();
                Event::PgpDecrypted {
                    key,
                    content: Some(Private(entity)),
                    signature: d.check.map(|c| match c {
                        pgp::Check::Good { key, at, weak, until } => PgpSignature::Good { key, at, weak, until },
                        pgp::Check::Bad => PgpSignature::Bad,
                        pgp::Check::Unknown(key_id) => PgpSignature::Unknown { key_id },
                    }),
                    keys: inside,
                    error: None,
                }
            }
            Err(pgp::Error::Decrypt) => {
                let error = format!("{NOT_FOR_YOU} ({})", pgp::recipients(&data).join(", "));
                Event::PgpDecrypted { key, content: None, signature: None, keys: Vec::new(), error: Some(error) }
            }
            Err(e) => Event::PgpDecrypted { key, content: None, signature: None, keys: Vec::new(), error: Some(e.to_string()) },
        };
        s.apply(event);
    }
    n
}

fn text(s: &Session) -> String {
    s.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.text.clone()).unwrap_or_default()
}

fn seal(s: &Session) -> Seal {
    security(s).signed.expect("signed").seal
}

/// Every file under `dir`, read.
fn files_under(dir: &std::path::Path) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else if let Ok(b) = std::fs::read(&p) {
                out.push(b);
            }
        }
    }
    out
}

#[test]
fn a_signature_says_whose_key_made_it_and_whether_it_holds() {
    let mut s = session("pgp-read-signed");
    // Ana's key isn't known: signed, but it can't be checked; the text shows.
    open_raw(&mut s, 9601, signed_by_ana(false).as_bytes());
    let sec = security(&s);
    assert!(sec.pgp, "{sec:?}");
    assert!(matches!(&sec.signed.as_ref().unwrap().seal, Seal::UnknownKey(id) if id.len() == 16), "{sec:?}");
    assert!(text(&s).contains("the proof is approved"));
    assert!(s.reading.as_ref().unwrap().parsed.as_ref().unwrap().attachments.is_empty(), "signature.asc isn't an attachment");
    // Her mail brings her key (Autocrypt): it holds, by her key, learnt from her mail.
    open_raw(&mut s, 9602, signed_by_ana(true).as_bytes());
    let signed = security(&s).signed.unwrap();
    assert_eq!(signed.seal, Seal::Trusted);
    assert_eq!(signed.key.as_ref().unwrap().emails, ["ana@posteo.example"]);
    assert_eq!(signed.learnt, Some(Learnt::FromMail));
    // Checking with it is using it: the least used goes first, not this one.
    s.clock.now = PGP_NOW + 5 * DAY;
    open_raw(&mut s, 9603, signed_by_ana(false).as_bytes());
    assert_eq!(seal(&s), Seal::Trusted, "her key is known from now on");
    assert_eq!(s.person_key("sample", "ana@posteo.example").unwrap().last_used, PGP_NOW + 5 * DAY);
    // Changed after it was signed.
    open_raw(&mut s, 9604, signed_by_ana(false).replace("is approved", "is rejected").as_bytes());
    assert!(matches!(seal(&s), Seal::Broken(_)), "{:?}", seal(&s));
    // Someone else with Ana's signature on their mail: her key, not for that sender.
    let posing = signed_by_ana(false).replace("From: Ana Lima <ana@posteo.example>", "From: Boss <boss@posteo.example>");
    open_raw(&mut s, 9605, posing.as_bytes());
    match seal(&s) {
        Seal::Untrusted(p) => assert_eq!(p, [smime::Problem::OtherAddress(vec!["ana@posteo.example".into()])]),
        other => panic!("{other:?}"),
    }
    assert_eq!(security(&s).signed.unwrap().key.unwrap().emails, ["ana@posteo.example"]);
    // Signed in the text itself (cleartext): the text without its armor.
    open_raw(&mut s, 9606, &from_ana(false, "text/plain; charset=us-ascii", &String::from_utf8(pgpdata("ana-clearsigned.txt")).unwrap()));
    assert_eq!(seal(&s), Seal::Trusted);
    assert!(text(&s).contains("signed in the text itself") && !text(&s).contains("BEGIN PGP"), "{}", text(&s));
    // Her key imported by hand: said so.
    s.run("pgp.importPublic", &json!({"data": pgpdata("ana.asc")})).unwrap();
    open_raw(&mut s, 9607, signed_by_ana(false).as_bytes());
    assert_eq!(security(&s).signed.unwrap().learnt, Some(Learnt::ByHand));
    // Nothing of it asked of the network layer.
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Pgp { .. })));
    done(s);
}

#[test]
fn encrypted_mail_is_decrypted_in_memory_only() {
    let mut s = with_riley("pgp-read-encrypted");
    let entity = b"Content-Type: text/plain; charset=utf-8\r\n\r\nThe quokkaplan is ready for Tuesday.\r\n";
    let sealed = pgp::encrypt(entity, &[&riley_key().public], Some(&ana_key().secret), PGP_NOW, &mut rand_core::OsRng).unwrap();
    let raw = encrypted_from_ana("Proof", true, &sealed);
    let key = open_raw(&mut s, 9611, &raw);
    assert_eq!(security(&s).lock, Some(Lock::Opening));
    assert!(!text(&s).contains("quokkaplan"), "nothing shows before it's decrypted");
    assert_eq!(answer_decrypt(&mut s), 1);
    let sec = security(&s);
    assert!(sec.pgp);
    assert_eq!(sec.lock, Some(Lock::Open { authenticated: true }));
    let signed = sec.signed.expect("signed inside");
    assert_eq!(signed.seal, Seal::Trusted, "by Ana, whose key came with it");
    assert_eq!(signed.learnt, Some(Learnt::FromMail));
    assert!(text(&s).contains("quokkaplan"));
    // Not asked for again; nothing from the web.
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Pgp { .. })));
    assert!(text(&s).contains("quokkaplan"));
    assert!(s.remote_pictures().is_empty());
    // GnuPG's own, and inline: decrypted the same way.
    open_raw(&mut s, 9612, &encrypted_from_ana("Proof", false, &String::from_utf8(pgpdata("ana-to-riley.asc")).unwrap()));
    assert_eq!(answer_decrypt(&mut s), 1);
    assert_eq!(security(&s).lock, Some(Lock::Open { authenticated: true }));
    assert!(text(&s).contains("the proof is approved"), "{}", text(&s));
    open_raw(&mut s, 9613, &from_ana(false, "text/plain; charset=us-ascii", &String::from_utf8(pgpdata("ana-inline.asc")).unwrap()));
    assert_eq!(answer_decrypt(&mut s), 1);
    assert_eq!(security(&s).lock, Some(Lock::Open { authenticated: true }));
    assert!(text(&s).contains("signed in the text itself"), "{}", text(&s));
    // The decrypted text is in no file: the cache keeps the message as it came.
    let store = s.store.clone().unwrap();
    assert_eq!(store.read_body(&key).unwrap(), raw);
    for f in files_under(store.root()) {
        assert!(!f.windows(10).any(|w| w == b"quokkaplan"), "decrypted text in a file");
    }
    // Nor in the search: found by its subject and sender, not its text.
    assert_eq!(s.run("search.run", &json!({"query": "quokkaplan"})).unwrap()["results"], 0);
    assert!(s.run("search.run", &json!({"query": "Proof"})).unwrap()["results"].as_u64().unwrap() >= 3);
    done(s);
}

#[test]
fn mail_encrypted_to_a_key_quatrelle_doesnt_have_says_so() {
    // Without a key of the account's: nothing asked of the network layer.
    let mut s = session("pgp-read-nokey");
    let body = String::from_utf8(pgpdata("to-bo.asc")).unwrap();
    open_raw(&mut s, 9621, &from_ana(false, "text/plain", &body));
    assert!(matches!(&security(&s).lock, Some(Lock::NoKey(to)) if to.len() == 1), "{:?}", security(&s));
    assert!(security(&s).pgp);
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Pgp { .. })));
    done(s);
    // With Riley's key: the network layer says it went to another.
    let mut s = with_riley("pgp-read-other");
    open_raw(&mut s, 9622, &from_ana(false, "text/plain", &body));
    assert_eq!(security(&s).lock, Some(Lock::Opening));
    assert_eq!(answer_decrypt(&mut s), 1);
    assert!(matches!(&security(&s).lock, Some(Lock::NoKey(to)) if to.len() == 1), "{:?}", security(&s));
    assert!(!text(&s).contains("The plan"));
    // Another reason stays what it was.
    s.apply(Event::PgpDecrypted {
        key: s.reading.as_ref().unwrap().key.clone(),
        content: None,
        signature: None,
        keys: Vec::new(),
        error: Some("the system keychain isn't available: locked".into()),
    });
    assert!(matches!(&security(&s).lock, Some(Lock::Failed(e)) if e.contains("keychain")), "{:?}", security(&s));
    done(s);
}

/// RFC 9788: the subject sent as `...` outside, the real one inside, shown once decrypted; the
/// list keeps what came.
#[test]
fn a_protected_subject_shows_once_decrypted() {
    let mut s = with_riley("pgp-read-subject");
    let entity = b"Content-Type: text/plain; charset=utf-8; hp=\"cipher\"\r\nSubject: Budget 2027\r\n\r\nThe figures.\r\n";
    let sealed = pgp::encrypt(entity, &[&riley_key().public], None, PGP_NOW, &mut rand_core::OsRng).unwrap();
    let key = open_raw(&mut s, 9631, &encrypted_from_ana("...", false, &sealed));
    assert_eq!(s.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.subject.clone()).unwrap_or_default(), "...");
    answer_decrypt(&mut s);
    let parsed = s.reading.as_ref().and_then(|r| r.parsed.clone()).unwrap();
    assert_eq!(parsed.subject, "Budget 2027");
    assert!(security(&s).headers);
    assert_eq!(s.summary(&key).unwrap().subject, "...", "the list keeps what came");
    done(s);
}

/// A key attached inside an encrypted message is learnt once it's decrypted, for its sender
/// only, as one outside would be.
#[test]
fn a_key_inside_encrypted_mail_is_learnt_for_its_sender() {
    let mut s = with_riley("pgp-read-inside");
    let ana = pgp::armored_public(&ana_key().public).unwrap().replace('\n', "\r\n");
    let eve = pgp::generate("Eve", "eve@posteo.example", PGP_NOW, &mut rand_core::OsRng).unwrap();
    let eve = pgp::armored_public(&eve.public).unwrap().replace('\n', "\r\n");
    let entity = format!(
        "Content-Type: multipart/mixed; boundary=\"in\"\r\n\r\n--in\r\nContent-Type: text/plain\r\n\r\nMy keys.\r\n\
--in\r\nContent-Type: application/pgp-keys; name=\"ana.asc\"\r\n\r\n{ana}\r\n\
--in\r\nContent-Type: application/pgp-keys; name=\"eve.asc\"\r\n\r\n{eve}\r\n--in--\r\n"
    );
    let sealed = pgp::encrypt(entity.as_bytes(), &[&riley_key().public], None, PGP_NOW, &mut rand_core::OsRng).unwrap();
    open_raw(&mut s, 9641, &encrypted_from_ana("Keys", false, &sealed));
    assert!(s.pgp_people("sample").is_empty(), "nothing seen yet");
    answer_decrypt(&mut s);
    let people = s.pgp_people("sample");
    assert_eq!(people.len(), 1, "{people:?}");
    assert_eq!((people[0].email.as_str(), people[0].learnt), ("ana@posteo.example", Learnt::FromMail));
    done(s);
}
