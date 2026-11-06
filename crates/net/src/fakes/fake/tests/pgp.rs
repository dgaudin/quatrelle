// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! OpenPGP over the fake IMAP and EWS servers (lot 4): a message signed and encrypted by the
//! pool with a key it made, sent, its copy kept byte for byte where the server keeps sent mail;
//! then delivered back, synced and fetched whole as any message, decrypted by the pool, and its
//! signature checked. Real Exchange may change what it's given (`FORK.md` says the check the
//! owner makes before turning OpenPGP on there); the fake keeps it as it came.

use std::sync::Arc;

use quatrelle_model::Private;
use quatrelle_model::job::PgpWork;
use quatrelle_pgp as pgp;

use super::*;
use crate::fakes::fake_ews;
use crate::worker::{MemoryCredentials, Pool};

/// The account's key, made by the pool (its secret part in the keychain): its public part.
fn make_key(pool: &mut Pool, account: &AccountConfig) -> Vec<u8> {
    let work = PgpWork::Make { name: "Riley".into(), email: account.email.clone(), now: crate::worker::now() - 60 };
    pool.submit(Job::Pgp { account: account.clone(), work });
    let seen = until(pool, "the key", |e| matches!(e, Event::PgpKeys { .. }));
    match seen.last() {
        Some(Event::PgpKeys { keys, error: None, .. }) if keys.len() == 1 => keys[0].clone(),
        other => panic!("{other:?}"),
    }
}

/// `raw` signed and encrypted (to `to`) by the pool, as the engine asks before sending.
fn protect(pool: &mut Pool, account: &AccountConfig, raw: &[u8], to: Vec<Vec<u8>>) -> Vec<u8> {
    let work = PgpWork::Protect { id: 9, raw: Private(raw.to_vec()), sign: true, encrypt_to: to, hide_subject: true };
    pool.submit(Job::Pgp { account: account.clone(), work });
    let seen = until(pool, "signed and encrypted", |e| matches!(e, Event::Protected { id: 9, .. }));
    match seen.last() {
        Some(Event::Protected { raw: Some(r), error: None, .. }) => r.0.clone(),
        other => panic!("{other:?}"),
    }
}

/// `got` (as it came back) decrypted by the pool with the account's key, the signature inside
/// checked with its public part `mine`; and Bo's key opens it too.
fn opens(pool: &mut Pool, account: &AccountConfig, key: MessageKey, got: &[u8], mine: &[u8], bo: &pgp::Unlocked) {
    let quatrelle_mime::pgp::Protection::Encrypted(data) = quatrelle_mime::pgp::protection(got) else { panic!("not PGP/MIME encrypted") };
    let work = PgpWork::Decrypt { key: key.clone(), data: data.clone(), inline: false, charset: String::new(), verify_with: vec![mine.to_vec()] };
    pool.submit(Job::Pgp { account: account.clone(), work });
    let seen = until(pool, "decrypted", |e| matches!(e, Event::PgpDecrypted { key: k, .. } if *k == key));
    let Some(Event::PgpDecrypted { content: Some(entity), error: None, .. }) = seen.last() else { panic!("{seen:?}") };
    let quatrelle_mime::pgp::Protection::Signed { content, signature } = quatrelle_mime::pgp::protection(&entity.0) else {
        panic!("not signed inside")
    };
    let check = pgp::verify(&content, &signature, &[mine]).unwrap();
    assert!(matches!(check, pgp::Check::Good { key: 0, .. }), "{check:?}");
    let text = String::from_utf8_lossy(&content);
    assert!(text.contains("The plan for Tuesday") && text.contains("Subject: Plan"), "{text}");
    let theirs = pgp::decrypt(&data, &[&bo.secret], &[]).unwrap().data;
    assert!(String::from_utf8_lossy(&theirs).contains("The plan for Tuesday"));
}

fn message(from: &str) -> Vec<u8> {
    format!(
        "From: Riley <{from}>\r\nTo: Bo <bo@studio.example>\r\nSubject: Plan\r\nMessage-ID: <plan@x>\r\nMIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\r\nThe plan for Tuesday.\r\n"
    )
    .into_bytes()
}

fn bo() -> pgp::Unlocked {
    pgp::generate("Bo", "bo@studio.example", crate::worker::now() - 60, &mut rand_core::OsRng).unwrap()
}

/// The message of `folder` whose subject is `subject`, once the account's cache has it.
fn listed(store: &Store, folder: &str, subject: &str) -> MessageKey {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(m) = store.load_folder("a1", folder).ok().and_then(|f| f.messages.into_iter().find(|m| m.subject == subject)) {
            return m.key;
        }
        assert!(Instant::now() < end, "{subject} never listed in {folder}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The whole of `key`, fetched from the server.
fn fetched(pool: &mut Pool, key: &MessageKey) -> Vec<u8> {
    pool.submit(Job::FetchBody { key: key.clone() });
    let seen = until(pool, "the message", |e| matches!(e, Event::Body { key: k, .. } if k == key));
    match seen.last() {
        Some(Event::Body { raw, .. }) => raw.clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn openpgp_mail_goes_and_comes_back_over_imap() {
    let st = State::new();
    let imap_port = serve_tcp(&st);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let smtp_port = listener.local_addr().unwrap().port();
    let (_, log) = crate::smtp::tests::serve_once_on(listener);
    let store = store("pgp-imap");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port: imap_port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: smtp_port, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let mine = make_key(&mut pool, &account);
    let bo = bo();
    let sealed = protect(&mut pool, &account, &message(&account.email), vec![bo.public.clone(), mine.clone()]);
    pool.submit(Job::Send {
        account: "a1".into(),
        id: 9,
        raw: sealed.clone(),
        from: account.email.clone(),
        recipients: vec!["bo@studio.example".into()],
        sent_folder: Some("Sent".into()),
    });
    until(&pool, "sent", |e| matches!(e, Event::Sent { id: 9, .. }));
    let said = log.join().unwrap();
    assert!(said.contains("multipart/encrypted") && !said.contains("The plan for Tuesday"), "what went over SMTP");
    // Its copy in Sent, byte for byte.
    let end = Instant::now() + Duration::from_secs(20);
    while st.lock().unwrap().mailbox("Sent").and_then(|b| b.msgs.last().map(|m| m.raw.clone())) != Some(sealed.clone()) {
        assert!(Instant::now() < end, "no copy in Sent, or not as it went");
        std::thread::sleep(Duration::from_millis(50));
    }
    // Delivered back to the Inbox, synced and fetched whole: as it went, and it opens.
    st.lock().unwrap().add_raw("INBOX", sealed.clone());
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    let key = listed(&store, "INBOX", "[...]");
    assert_eq!(fetched(&mut pool, &key), sealed);
    opens(&mut pool, &account, key, &sealed, &mine, &bo);
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn openpgp_mail_goes_and_comes_back_over_ews() {
    let mut d = fake_ews::Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Sent Items", None, Some("sentitems"), "IPF.Note");
    let server = fake_ews::serve(d);
    let store = store("pgp-ews");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{}\\{}", fake_ews::DOMAIN, fake_ews::USER),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret(fake_ews::PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let mine = make_key(&mut pool, &account);
    let bo = bo();
    let sealed = protect(&mut pool, &account, &message(&account.email), vec![bo.public.clone(), mine.clone()]);
    pool.submit(Job::Send {
        account: "a1".into(),
        id: 9,
        raw: sealed.clone(),
        from: account.email.clone(),
        recipients: vec!["bo@studio.example".into()],
        sent_folder: Some("Sent Items".into()),
    });
    until(&pool, "sent", |e| matches!(e, Event::Sent { id: 9, .. }));
    // What Exchange was given (CreateItem's MimeContent), byte for byte.
    assert_eq!(server.data.lock().unwrap().sent.last(), Some(&sealed));
    // Delivered back to the Inbox, synced and fetched whole: as it went, and it opens.
    server.data.lock().unwrap().deliver(&inbox, &sealed, false, "2026-11-06T08:00:00Z");
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    let key = listed(&store, "INBOX", "[...]");
    assert_eq!(fetched(&mut pool, &key), sealed);
    opens(&mut pool, &account, key, &sealed, &mine, &bo);
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}
