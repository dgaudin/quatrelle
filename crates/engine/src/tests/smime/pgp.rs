// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: OpenPGP on the sample mailbox (Riley's account). Nothing is active until the
//! account has a key; a key made here asks for its backup and says when it runs out; an account
//! removed takes its keys with it. The network layer's part (making a key, its keychain) is
//! played here with the pgp crate itself.

use super::*;
use crate::session::{AccountPgp, PGP_DOC};
use quatrelle_model::job::{PgpChange, PgpWork};
use quatrelle_pgp as pgp;

const THREE_YEARS: i64 = 94_608_000;
const DAY: i64 = 86_400;

/// Plays the network layer for `pgp.create`: the key made from what the job asks, its public
/// part answered. Returns it whole, as the keychain would keep it.
fn answer_make(s: &mut Session) -> pgp::Unlocked {
    let jobs = s.take_jobs();
    let (account, name, email, now) = jobs
        .iter()
        .find_map(|j| match j {
            Job::Pgp { account, work: PgpWork::Make { name, email, now } } => Some((account.id.clone(), name.clone(), email.clone(), *now)),
            _ => None,
        })
        .expect("a key asked for");
    let key = pgp::generate(&name, &email, now, &mut rand_core::OsRng).unwrap();
    let changed = Some(PgpChange::Made { fingerprint: key.info.fingerprint.clone() });
    s.apply(Event::PgpKeys { account, keys: vec![key.public.clone()], changed, error: None });
    key
}

/// The sample session with a key made for Riley.
fn with_key(name: &str) -> (Session, pgp::Unlocked) {
    let mut s = sample_session(name);
    s.run("pgp.create", &json!({})).unwrap();
    let key = answer_make(&mut s);
    (s, key)
}

#[test]
fn nothing_is_active_without_a_key() {
    let mut s = sample_session("pgp-inactive");
    assert!(!s.pgp_active("sample"));
    let card = s.pgp_card("sample");
    assert!(card.keys.is_empty() && !card.needs_backup && card.expires_soon.is_none());
    s.run("mail.new", &json!({})).unwrap();
    let r = s.run("compose.security", &json!({"scheme": "pgp", "sign": true}));
    assert!(matches!(r, Err(CmdError::Disabled(_))), "{r:?}");
    let keys = s.run("pgp.keys", &json!({})).unwrap();
    assert_eq!(keys["inUse"], Value::Null);
    // What needs a key says so, and asks nothing of the network.
    assert!(matches!(s.run("pgp.exportPublic", &json!({"path": "/nowhere/k.asc"})), Err(CmdError::Disabled(_))));
    assert!(matches!(s.run("pgp.extend", &json!({"fingerprint": "AB"})), Err(CmdError::Params(_))));
    assert!(s.take_jobs().is_empty());
    done(s);
}

#[test]
fn a_key_made_is_active_and_asks_for_its_backup() {
    let (mut s, key) = with_key("pgp-made");
    let fp = key.info.fingerprint.clone();
    assert!(s.pgp_active("sample"));
    let card = s.pgp_card("sample");
    assert!(card.needs_backup, "made here: no backup yet");
    assert!(card.autocrypt, "the public key goes with mail once a key is made");
    assert_eq!(card.keys.len(), 1);
    assert!(card.keys[0].in_use && !card.keys[0].backed_up);
    assert_eq!(card.keys[0].info.emails, ["riley@studio.example"]);
    assert_eq!(card.keys[0].info.user_ids, ["Riley Moreno <riley@studio.example>"]);
    assert!(s.notices.iter().any(|n| n.template.contains("OpenPGP key") && !n.error));
    // Kept with the account: a new session says the same.
    let store = s.store.clone().unwrap();
    let again = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    assert!(again.pgp_active("sample") && again.pgp_card("sample").needs_backup);
    // The backup: asked of the network layer, with a passphrase, somewhere.
    assert!(matches!(s.run("pgp.backup", &json!({"fingerprint": fp, "passphrase": "", "path": "/x/k.asc"})), Err(CmdError::Params(_))));
    assert!(matches!(s.run("pgp.backup", &json!({"fingerprint": fp, "passphrase": "pw", "path": " "})), Err(CmdError::Params(_))));
    s.run("pgp.backup", &json!({"fingerprint": key.info.spaced(), "passphrase": "correct horse", "path": "/x/k.asc"})).unwrap();
    let asked = s.take_jobs();
    assert!(
        asked.iter().any(|j| matches!(j, Job::Pgp { work: PgpWork::Backup { fingerprint, passphrase, path }, .. }
            if *fingerprint == fp && passphrase.0 == "correct horse" && path == "/x/k.asc")),
        "{asked:?}"
    );
    // Not saved: still asked for.
    s.apply(Event::PgpBackedUp { account: "sample".into(), fingerprint: fp.clone(), error: Some("disk full".into()) });
    assert!(s.pgp_card("sample").needs_backup);
    s.apply(Event::PgpBackedUp { account: "sample".into(), fingerprint: fp.clone(), error: None });
    assert!(!s.pgp_card("sample").needs_backup);
    let again = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    assert!(!again.pgp_card("sample").needs_backup, "kept with the account");
    // Its public part written as a key file.
    let file = s.store.as_ref().unwrap().root().join("riley.asc");
    s.run("pgp.exportPublic", &json!({"path": file.display().to_string()})).unwrap();
    assert_eq!(pgp::public_keys(&std::fs::read(&file).unwrap()).unwrap(), [key.public.clone()]);
    // Sent with mail, or not.
    s.run("pgp.options", &json!({"autocrypt": false})).unwrap();
    assert!(!s.pgp_card("sample").autocrypt);
    // A failed answer changes nothing; an empty one (a keychain that gave nothing back) doesn't
    // take the keys away.
    s.apply(Event::PgpKeys {
        account: "sample".into(),
        keys: Vec::new(),
        changed: None,
        error: Some("the system keychain isn't available: locked".into()),
    });
    s.apply(Event::PgpKeys { account: "sample".into(), keys: Vec::new(), changed: None, error: None });
    assert!(s.pgp_active("sample") && s.pgp_card("sample").keys.len() == 1);
    assert!(s.notices.iter().any(|n| n.error && n.text.contains("the system keychain isn't available")));
    // Extended: asked of the network layer for now.
    s.run("pgp.extend", &json!({"fingerprint": fp.to_lowercase()})).unwrap();
    assert!(
        s.take_jobs().iter().any(|j| matches!(j, Job::Pgp { work: PgpWork::Extend { fingerprint, now }, .. } if *fingerprint == fp && *now == NOW))
    );
    let keys = s.run("pgp.keys", &json!({})).unwrap();
    assert_eq!(keys["inUse"]["fingerprint"], json!(fp));
    // Forgotten: nothing active, the keychain's part asked to go.
    s.run("pgp.forget", &json!({"fingerprint": fp})).unwrap();
    assert!(!s.pgp_active("sample"));
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Pgp { work: PgpWork::Forget { fingerprint }, .. } if *fingerprint == fp)));
    done(s);
}

#[test]
fn a_key_running_out_says_so_a_month_before() {
    let (mut s, key) = with_key("pgp-expiry");
    let end = key.info.expires.unwrap();
    assert_eq!(end, NOW + THREE_YEARS);
    assert_eq!(s.pgp_card("sample").expires_soon, None);
    s.clock.now = end - 31 * DAY;
    assert_eq!(s.pgp_card("sample").expires_soon, None);
    s.clock.now = end - 29 * DAY;
    assert_eq!(s.pgp_card("sample").expires_soon, Some(end));
    // Run out: still said, until it's extended.
    s.clock.now = end + DAY;
    assert_eq!(s.pgp_card("sample").expires_soon, Some(end));
    // Extended (the network layer's answer): three years from that day.
    let longer = pgp::extend(&key, s.clock.now, &mut rand_core::OsRng).unwrap();
    let changed = Some(PgpChange::Extended { fingerprint: key.info.fingerprint.clone() });
    s.apply(Event::PgpKeys { account: "sample".into(), keys: vec![longer.public], changed, error: None });
    assert_eq!(s.pgp_card("sample").expires_soon, None);
    assert!(s.pgp_card("sample").needs_backup, "extending isn't backing up");
    done(s);
}

/// Review focus 5: an account removed asks for each of its keys' secret parts to go from the
/// keychain, and its OpenPGP documents go too, even from the data file that stays.
#[test]
fn removing_an_account_forgets_its_secrets() {
    let dir = std::env::temp_dir().join(format!("quatrelle-engine-pgp-remove-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open_packed(dir.join("app"), dir.join("Files")).unwrap();
    assert!(crate::sample::seed_if_empty(&store, NOW).unwrap());
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.run("pgp.create", &json!({})).unwrap();
    let first = answer_make(&mut s);
    s.run("pgp.create", &json!({})).unwrap();
    let second = answer_make(&mut s);
    let both = vec![second.public.clone(), first.public.clone()];
    s.apply(Event::PgpKeys { account: "sample".into(), keys: both, changed: None, error: None });
    assert_eq!(s.pgp_card("sample").keys.len(), 2);
    assert!(store.load_account_doc::<AccountPgp>("sample", PGP_DOC).unwrap().keys.len() == 2);
    let removed = s.remove_account("sample").unwrap();
    let jobs = s.take_jobs();
    for k in [&first, &second] {
        assert!(
            jobs.iter().any(|j| matches!(j, Job::Pgp { account, work: PgpWork::Forget { fingerprint } }
                if account.id == "sample" && *fingerprint == k.info.fingerprint)),
            "{jobs:?}"
        );
    }
    // The data file kept, opened again: no OpenPGP left in it.
    let kept = removed["dataFile"].as_str().expect("a data file kept").to_string();
    let other = Store::open_packed(dir.join("other"), dir.join("Other Files")).unwrap();
    other.import_data_file("again", std::path::Path::new(&kept)).unwrap();
    assert!(!other.load_folders("again").unwrap().is_empty(), "the mail is there");
    assert_eq!(other.load_account_doc::<Value>("again", PGP_DOC).unwrap(), Value::Null);
    let _ = std::fs::remove_dir_all(&dir);
}
