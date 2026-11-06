// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: OpenPGP on the sample mailbox (Riley's account). Nothing is active until the
//! account has a key; a key made here asks for its backup and says when it runs out; an account
//! removed takes its keys with it. Correspondents' keys are learnt from their own mail only, a
//! newer message's replacing an older one's, and a change is said once. The network layer's part
//! (making a key, its keychain) is played here with the pgp crate itself.

use super::*;
use crate::session::{AccountPgp, Learnt, PEOPLE_DOC, PGP_DOC};
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
    assert_eq!(pgp::public_keys(&std::fs::read(&file).unwrap()).unwrap(), std::slice::from_ref(&key.public));
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
    let bo = pgp::generate("Bo", "bo@studio.example", NOW, &mut rand_core::OsRng).unwrap();
    s.run("pgp.importPublic", &json!({"data": bo.public})).unwrap();
    assert_ne!(store.load_account_doc::<Value>("sample", PEOPLE_DOC).unwrap(), Value::Null);
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
    assert_eq!(other.load_account_doc::<Value>("again", PEOPLE_DOC).unwrap(), Value::Null, "nor the people's keys");
    let _ = std::fs::remove_dir_all(&dir);
}

const OCT_24: &str = "Sat, 24 Oct 2026 09:00:00 +0000";
const OCT_25: &str = "Sun, 25 Oct 2026 09:00:00 +0000";

/// Each message's own Message-ID.
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A text from `from`, written `date`, with `extra` header lines (each ending CRLF).
fn mail(from: &str, date: &str, extra: &str) -> Vec<u8> {
    format!(
        "From: {from}\r\nTo: Riley Moreno <riley@studio.example>\r\nSubject: Hello\r\nDate: {date}\r\nMessage-ID: <{}@studio.example>\r\n{extra}\
MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello.\r\n",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
    .into_bytes()
}

/// A message from `from` with `key` attached (`application/pgp-keys`).
fn with_key_attached(from: &str, key: &[u8]) -> Vec<u8> {
    let armored = pgp::armored_public(key).unwrap().replace('\n', "\r\n");
    format!(
        "From: {from}\r\nTo: riley@studio.example\r\nSubject: My key\r\nDate: {OCT_24}\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n--b1\r\nContent-Type: text/plain\r\n\r\nMy key.\r\n--b1\r\n\
Content-Type: application/pgp-keys; name=\"key.asc\"\r\nContent-Disposition: attachment; filename=\"key.asc\"\r\n\r\n{armored}\r\n--b1--\r\n"
    )
    .into_bytes()
}

fn bo_key() -> pgp::Unlocked {
    pgp::generate("Bo", "bo@studio.example", NOW, &mut rand_core::OsRng).unwrap()
}

/// Open another message, then `key` again: shown anew.
fn reopen(s: &mut Session, key: &quatrelle_model::MessageKey) {
    let other = s.list.iter().find(|m| m.key != *key).map(|m| m.key.clone()).unwrap();
    s.run("list.open", &json!({"key": other.to_id()})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
}

/// Review focus 1: a message that says it's Bo's (From spoofed) brings another key. Autocrypt
/// takes it, being newer, but the message says the key changed: it isn't silent. Said once.
#[test]
fn a_newer_messages_key_replaces_an_older_ones_and_says_so_once() {
    let mut s = sample_session("pgp-key-changed");
    let (a, forged) = (bo_key(), bo_key());
    let first = open_raw(&mut s, 9001, &mail("Bo <bo@studio.example>", OCT_24, &pgp::autocrypt::header("bo@studio.example", &a.public, false)));
    assert!(!security(&s).key_changed, "a first key changes nothing");
    assert_eq!(s.person_key("sample", "bo@studio.example").unwrap().info.fingerprint, a.info.fingerprint);
    let second = open_raw(&mut s, 9002, &mail("Bo <bo@studio.example>", OCT_25, &pgp::autocrypt::header("bo@studio.example", &forged.public, false)));
    assert!(security(&s).key_changed, "the change is said on the message that brought it");
    let k = s.person_key("sample", "bo@studio.example").unwrap().clone();
    assert_eq!(k.info.fingerprint, forged.info.fingerprint);
    assert_eq!(k.previous.as_deref(), Some(a.info.fingerprint.as_str()));
    assert_eq!(k.learnt, Learnt::FromMail);
    reopen(&mut s, &second);
    assert!(!security(&s).key_changed, "said once");
    // The older message again: its key doesn't come back.
    reopen(&mut s, &first);
    assert!(!security(&s).key_changed);
    assert_eq!(s.person_key("sample", "bo@studio.example").unwrap().info.fingerprint, forged.info.fingerprint);
    // Kept with the account: a new session knows it, and says nothing more.
    let again = Session::open(s.store.clone(), quatrelle_model::dates::Clock::new(NOW, 0));
    let k = again.person_key("sample", "bo@studio.example").unwrap();
    assert!(k.change_shown && k.info.fingerprint == forged.info.fingerprint);
    assert_eq!(again.pgp_card("sample").people.len(), 1);
    done(s);
}

#[test]
fn only_the_senders_address_is_learnt() {
    let mut s = sample_session("pgp-sender-only");
    let (bo, eve) = (bo_key(), pgp::generate("Eve", "eve@studio.example", NOW, &mut rand_core::OsRng).unwrap());
    // Bo's mail with an Autocrypt header for Eve's address, or Eve's key attached: nothing.
    open_raw(&mut s, 9011, &mail("Bo <bo@studio.example>", OCT_24, &pgp::autocrypt::header("eve@studio.example", &eve.public, false)));
    open_raw(&mut s, 9012, &with_key_attached("Bo <bo@studio.example>", &eve.public));
    // Riley's own mail brings no one's key, not even Riley's.
    let riley = pgp::generate("Riley", "riley@studio.example", NOW, &mut rand_core::OsRng).unwrap();
    open_raw(
        &mut s,
        9013,
        &mail("Riley Moreno <riley@studio.example>", OCT_24, &pgp::autocrypt::header("riley@studio.example", &riley.public, false)),
    );
    assert!(s.pgp_people("sample").is_empty(), "{:?}", s.pgp_people("sample"));
    // Bo's key attached to Bo's mail is Bo's.
    open_raw(&mut s, 9014, &with_key_attached("Bo <bo@studio.example>", &bo.public));
    let people = s.pgp_people("sample");
    assert_eq!(people.len(), 1);
    assert_eq!((people[0].email.as_str(), people[0].info.fingerprint.as_str()), ("bo@studio.example", bo.info.fingerprint.as_str()));
    // A header that isn't one Quatrelle reads, a key that isn't one: nothing, no panic.
    open_raw(&mut s, 9015, &mail("Zoe <zoe@studio.example>", OCT_24, "Autocrypt: addr=zoe@studio.example; keydata=!!!!\r\n"));
    open_raw(&mut s, 9016, &mail("Zoe <zoe@studio.example>", OCT_24, "Autocrypt: addr=zoe@studio.example; keydata=AAAA\r\n"));
    assert_eq!(s.pgp_people("sample").len(), 1);
    done(s);
}

#[test]
fn a_key_imported_by_hand_is_never_replaced_from_mail() {
    let mut s = sample_session("pgp-by-hand");
    let (a, b) = (bo_key(), bo_key());
    let file = pgp::armored_public(&a.public).unwrap();
    let r = s.run("pgp.importPublic", &json!({"data": file.as_bytes()})).unwrap();
    assert_eq!(r["added"], json!(["bo@studio.example"]));
    open_raw(&mut s, 9021, &mail("Bo <bo@studio.example>", OCT_25, &pgp::autocrypt::header("bo@studio.example", &b.public, false)));
    let k = s.person_key("sample", "bo@studio.example").unwrap();
    assert_eq!((k.info.fingerprint.as_str(), k.learnt), (a.info.fingerprint.as_str(), Learnt::ByHand));
    assert!(!security(&s).key_changed);
    let known = s.run("pgp.known", &json!({})).unwrap();
    assert_eq!(known["known"][0]["email"], "bo@studio.example");
    assert_eq!(known["known"][0]["learnt"], "byHand");
    // Forgotten: Bo's next message brings his key again.
    s.run("pgp.forgetPublic", &json!({"email": "Bo@Studio.example"})).unwrap();
    assert!(s.pgp_people("sample").is_empty());
    assert!(matches!(s.run("pgp.forgetPublic", &json!({"email": "bo@studio.example"})), Err(CmdError::Params(_))));
    open_raw(&mut s, 9022, &mail("Bo <bo@studio.example>", OCT_25, &pgp::autocrypt::header("bo@studio.example", &b.public, false)));
    assert_eq!(s.person_key("sample", "bo@studio.example").unwrap().learnt, Learnt::FromMail);
    // A key only for the account's own address, or no key: nothing kept, said.
    let riley = pgp::generate("Riley", "riley@studio.example", NOW, &mut rand_core::OsRng).unwrap();
    assert!(matches!(s.run("pgp.importPublic", &json!({"data": riley.public})), Err(CmdError::Params(_))));
    assert!(matches!(s.run("pgp.importPublic", &json!({"data": b"not a key".to_vec()})), Err(CmdError::Params(_))));
    // Attached to the open message: imported from there.
    open_raw(&mut s, 9023, &with_key_attached("Zoe <zoe@studio.example>", &a.public));
    let attachments = s.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.attachments.clone()).unwrap_or_default();
    let index = attachments.iter().find(|a| a.name == "key.asc").map(|a| a.index).expect("the key shows as an attachment");
    s.run("pgp.importAttached", &json!({"index": index})).unwrap();
    assert_eq!(s.person_key("sample", "bo@studio.example").unwrap().learnt, Learnt::ByHand);
    done(s);
}

#[test]
fn a_damaged_people_file_is_set_aside() {
    let mut s = sample_session("pgp-damaged");
    s.run("pgp.importPublic", &json!({"data": bo_key().public})).unwrap();
    let store = s.store.clone().unwrap();
    let mut found = Vec::new();
    let mut dirs = vec![store.root().to_path_buf()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.file_name().is_some_and(|n| n == PEOPLE_DOC) {
                found.push(p);
            }
        }
    }
    let [file] = found.as_slice() else { panic!("{found:?}") };
    std::fs::write(file, b"{\"keys\": {\"bo@studio.example\": {\"public\": \"zz\x00\xff").unwrap();
    let mut again = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    assert!(again.pgp_people("sample").is_empty(), "an empty set");
    assert!(file.with_extension("json.bad").exists(), "the damaged file set aside");
    // And the next key is kept anew.
    again.run("pgp.importPublic", &json!({"data": bo_key().public})).unwrap();
    assert_eq!(again.pgp_people("sample").len(), 1);
    done(s);
}

/// Where the window has a thread for bodies, the keys a message brings are read there with it
/// (`BodyWork`), and learnt from what it found: the window's thread doesn't read it again.
#[test]
fn keys_are_learnt_where_the_body_is_got_ready() {
    let mut s = sample_session("pgp-deferred");
    s.defer_bodies(true);
    let bo = bo_key();
    open_raw(&mut s, 9031, &mail("Bo <bo@studio.example>", OCT_24, &pgp::autocrypt::header("bo@studio.example", &bo.public, false)));
    assert!(s.pgp_people("sample").is_empty(), "nothing read on this thread");
    let prepared = s.take_body_work().expect("the reading lane's work").run();
    assert_eq!(prepared.offered.len(), 1, "read on the reading lane");
    s.finish_body(prepared);
    assert_eq!(s.person_key("sample", "bo@studio.example").unwrap().info.fingerprint, bo.info.fingerprint);
    assert!(s.reading.as_ref().is_some_and(|r| r.parsed.is_some()), "and shown");
    done(s);
}
