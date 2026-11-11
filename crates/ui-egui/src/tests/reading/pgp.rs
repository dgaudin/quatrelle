// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the reading pane's security bar for OpenPGP mail (lot 3b), with GnuPG's keys and
//! mail (`crates/pgp/testdata`): a signature unknown, good (and how its key came), changed after
//! signing, by another address's key; a key that changed; mail decrypted, and mail encrypted to
//! a key Quatrelle doesn't have. With `QUATRELLE_FRAME_PNG` set to a folder, each is painted
//! there, light and dark.

use super::super::list::follow_up::{Textures, frame_with};
use super::*;
use quatrelle_engine::model::Private;
use quatrelle_engine::model::job::{PgpChange, PgpSignature, PgpWork};
use quatrelle_engine::pgp;

/// After GnuPG made the test keys.
const PGP_NOW: i64 = 1_800_000_000;

/// The app over the sample mailbox at [`PGP_NOW`] (the window's clock is its services').
fn app_later(name: &str) -> (MailApp, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("quatrelle-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    sample::seed_if_empty(&store, NOW).unwrap();
    let s = Session::open(Some(store), Clock::new(PGP_NOW, 0));
    (MailApp::new(s, Box::new(NullBackend), Box::new(NullServices(PGP_NOW))), dir)
}

fn pgpdata(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../pgp/testdata").join(name)).unwrap()
}

/// `raw` put at the top of the sample Inbox, opened.
fn open(a: &mut MailApp, raw: &[u8]) -> MessageKey {
    let store = a.session.store.clone().unwrap();
    let mut st = store.load_folder("sample", "INBOX").unwrap();
    let uid = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1;
    let key = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: st.validity.max(1), uid };
    st.messages.insert(0, quatrelle_engine::mime::summary_from_full(raw, key.clone(), Default::default(), raw.len() as u32, PGP_NOW));
    store.write_body(&key, raw).unwrap();
    store.save_folder("sample", "INBOX", &st).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    a.run("list.open", json!({"key": key.to_id()}));
    key
}

/// Ana's mail, its `Autocrypt:` header (when given) bringing `autocrypt`'s key.
fn from_ana(autocrypt: Option<&[u8]>, content_type: &str, body: &str) -> Vec<u8> {
    let mut m =
        String::from("From: Ana L. <ana@posteo.example>\r\nTo: riley@studio.example\r\nSubject: Proof\r\nDate: Sat, 24 Oct 2026 09:00:00 +0000\r\n");
    if let Some(k) = autocrypt {
        m.push_str(&pgp::autocrypt::header("ana@posteo.example", k, false));
    }
    m.push_str(&format!("MIME-Version: 1.0\r\nContent-Type: {content_type}\r\n\r\n{body}"));
    m.into_bytes()
}

fn signed_by_ana(autocrypt: Option<&[u8]>) -> String {
    let body = format!(
        "--sig\r\n{}\r\n--sig\r\nContent-Type: application/pgp-signature; name=\"signature.asc\"\r\n\r\n{}\r\n--sig--\r\n",
        String::from_utf8_lossy(&pgpdata("ana-part.txt")),
        String::from_utf8_lossy(&pgpdata("ana-part.sig"))
    );
    let raw = from_ana(autocrypt, "multipart/signed; micalg=pgp-sha512; protocol=\"application/pgp-signature\"; boundary=\"sig\"", &body);
    String::from_utf8(raw).unwrap()
}

/// Plays the network layer: what was asked to be decrypted, with Riley's key.
fn answer_decrypt(a: &mut MailApp) {
    let riley = pgp::read_secret_key(&pgpdata("riley-secret.asc"), "").unwrap();
    for j in a.session.take_jobs() {
        let quatrelle_engine::model::Job::Pgp { work: PgpWork::Decrypt { key, data, verify_with, .. }, .. } = j else { continue };
        let keys: Vec<&[u8]> = verify_with.iter().map(Vec::as_slice).collect();
        let d = pgp::decrypt(&data, &[&riley.secret], &keys).unwrap();
        let signature = d.check.map(|c| match c {
            pgp::Check::Good { key, at, weak, until } => PgpSignature::Good { key, at, weak, until },
            pgp::Check::Bad => PgpSignature::Bad,
            pgp::Check::Unknown(key_id) => PgpSignature::Unknown { key_id },
        });
        a.session.apply(MailEvent::PgpDecrypted { key, content: Some(Private(d.data)), signature, keys: Vec::new(), error: None });
    }
}

/// The texts drawn once the pane has settled, light and dark, each painted as `name` when
/// pictures are asked for; the light ones.
fn drawn(ctx: &egui::Context, a: &mut MailApp, tex: &mut Textures, name: &str) -> Vec<String> {
    let pictures = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let mut light = Vec::new();
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        a.ui.dark = Some(dark);
        for _ in 0..8 {
            frame_with(ctx, a, tex, vec![], None);
        }
        let png = pictures.as_ref().map(|d| d.join(format!("pgp-bar-{name}-{theme}.png")));
        let (_, shapes) = frame_with(ctx, a, tex, vec![], png);
        let texts: Vec<String> = shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect();
        if !dark {
            light = texts;
        }
    }
    light
}

fn has(texts: &[String], wanted: &str) -> bool {
    texts.iter().any(|t| t.contains(wanted))
}

#[test]
fn the_security_bar_says_what_openpgp_found() {
    let (mut a, dir) = app_later("pgp-bar");
    let ctx = egui::Context::default();
    let mut tex = Textures::default();
    frame_with(&ctx, &mut a, &mut tex, vec![], None);
    let ana = pgp::public_keys(&pgpdata("ana.asc")).unwrap().remove(0);
    // Ana's key isn't known.
    open(&mut a, signed_by_ana(None).as_bytes());
    let t = drawn(&ctx, &mut a, &mut tex, "unknown");
    assert!(has(&t, "Signed with the OpenPGP key ") && has(&t, "which Quatrelle doesn't have, so the signature can't be checked."), "{t:?}");
    // It came with her mail: the signature holds, and the bar says how the key came.
    open(&mut a, signed_by_ana(Some(&ana)).as_bytes());
    let t = drawn(&ctx, &mut a, &mut tex, "good");
    assert!(has(&t, "Signed by Ana Lima <ana@posteo.example>: the signature holds (OpenPGP key "), "{t:?}");
    assert!(has(&t, "Their key came with their own mail."), "{t:?}");
    // Changed after it was signed.
    open(&mut a, signed_by_ana(None).replace("is approved", "is rejected").as_bytes());
    let t = drawn(&ctx, &mut a, &mut tex, "broken");
    assert!(has(&t, "The signature doesn't hold: the message was changed after it was signed"), "{t:?}");
    // Ana's signature on someone else's mail.
    open(&mut a, signed_by_ana(None).replace("From: Ana L. <ana@posteo.example>", "From: Boss <boss@posteo.example>").as_bytes());
    let t = drawn(&ctx, &mut a, &mut tex, "other-address");
    assert!(has(&t, "Signed by Ana Lima <ana@posteo.example>, but its key is for ana@posteo.example, not for the sender."), "{t:?}");
    // Encrypted to Riley, who has no key yet: said, and how to read it.
    let to_riley = String::from_utf8(pgpdata("ana-to-riley.asc")).unwrap();
    let encrypted = |armored: &str| {
        let body = format!(
            "--enc\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n\r\n--enc\r\n\
Content-Type: application/octet-stream; name=\"encrypted.asc\"\r\n\r\n{armored}\r\n--enc--\r\n"
        );
        from_ana(None, "multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=\"enc\"", &body)
    };
    open(&mut a, &encrypted(&to_riley));
    let t = drawn(&ctx, &mut a, &mut tex, "no-key");
    assert!(has(&t, "This message is encrypted to a key Quatrelle doesn't have."), "{t:?}");
    assert!(has(&t, "make or import your OpenPGP key in File › Info › Security"), "{t:?}");
    assert!(!has(&t, "encrypted.asc"), "nothing of it but the bar: {t:?}");
    // With Riley's key: decrypted, the signature inside checked.
    let riley = pgp::read_secret_key(&pgpdata("riley-secret.asc"), "").unwrap();
    let changed = Some(PgpChange::Imported { fingerprint: riley.info.fingerprint.clone() });
    a.session.apply(MailEvent::PgpKeys { account: "sample".into(), keys: vec![riley.public], changed, error: None });
    answer_decrypt(&mut a);
    let t = drawn(&ctx, &mut a, &mut tex, "decrypted");
    assert!(has(&t, "Encrypted: only you and its other recipients can read it."), "{t:?}");
    assert!(has(&t, "Signed by Ana Lima <ana@posteo.example>: the signature holds"), "{t:?}");
    assert!(has(&t, "the proof is approved"), "{t:?}");
    // Her next message brings another key for her address: said on it, once.
    let other = pgp::public_keys(&pgpdata("bo.asc")).unwrap().remove(0);
    let key = open(&mut a, &from_ana(Some(&other), "text/plain", "New key.\r\n"));
    let t = drawn(&ctx, &mut a, &mut tex, "key-changed");
    assert!(has(&t, "This person's OpenPGP key changed"), "{t:?}");
    let first = a.session.list.iter().find(|m| m.key != key).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": first.to_id()}));
    a.run("list.open", json!({"key": key.to_id()}));
    assert!(!has(&drawn(&ctx, &mut a, &mut tex, "key-changed-again"), "This person's OpenPGP key changed"), "said once");
    let _ = std::fs::remove_dir_all(dir);
}
