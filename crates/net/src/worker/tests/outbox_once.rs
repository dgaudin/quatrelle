// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A message sent from the Outbox goes out once, even when its files can't be taken out of the
//! Outbox (held by another program, a failing disk, a removal that answers as if done and leaves
//! them); and one refused for good isn't tried again when the mark saying so can't be written.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use quatrelle_model::{Security, ServerConfig};

use super::*;

/// What the SMTP server below took: each message's subject, and each recipient it refused.
#[derive(Default)]
struct Took {
    messages: Vec<String>,
    refused: Vec<String>,
}

/// An SMTP server on localhost taking any number of messages, refusing for good (550) the
/// recipients whose address starts with `refused@`.
fn smtp_server() -> (u16, Arc<Mutex<Took>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let took = Arc::new(Mutex::new(Took::default()));
    let t = took.clone();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(s) = s else { return };
            let mut w = s.try_clone().unwrap();
            let mut r = BufReader::new(s);
            let _ = w.write_all(b"220 test ESMTP\r\n");
            let (mut line, mut data, mut subject) = (String::new(), false, String::new());
            while r.read_line(&mut line).unwrap_or(0) > 0 {
                let cmd = line.trim_end().to_string();
                let upper = cmd.to_ascii_uppercase();
                let answer: &[u8] = if data {
                    if let Some(s) = cmd.strip_prefix("Subject: ") {
                        subject = s.to_string();
                    }
                    if cmd == "." {
                        data = false;
                        t.lock().unwrap().messages.push(std::mem::take(&mut subject));
                        b"250 queued\r\n"
                    } else {
                        b""
                    }
                } else if upper.starts_with("EHLO") {
                    b"250-test\r\n250 AUTH PLAIN\r\n"
                } else if upper.starts_with("AUTH") {
                    b"235 ok\r\n"
                } else if upper.starts_with("RCPT TO:<REFUSED@") {
                    t.lock().unwrap().refused.push(cmd.clone());
                    b"550 no such user\r\n"
                } else if upper == "DATA" {
                    data = true;
                    b"354 go\r\n"
                } else if upper == "QUIT" {
                    let _ = w.write_all(b"221 bye\r\n");
                    break;
                } else {
                    b"250 ok\r\n"
                };
                let _ = w.write_all(answer);
                line.clear();
            }
        }
    });
    (port, took)
}

fn queue(store: &Store, id: u64, to: &str, subject: &str) {
    let raw = format!("From: me@studio.example\r\nTo: {to}\r\nSubject: {subject}\r\nMessage-ID: <{id}@studio.example>\r\n\r\nhi\r\n");
    let item = OutboxItem {
        id,
        from: "me@studio.example".into(),
        recipients: vec![to.into()],
        sent_folder: Some("Sent".into()),
        queued: i64::try_from(id).unwrap(),
        ..Default::default()
    };
    store.outbox_add("a1", &item, raw.as_bytes()).unwrap();
}

/// A worker for the account on these servers, as a new start of Quatrelle makes one.
fn worker(pool: &Pool, store: &Store, imap: u16, smtp: u16) -> Worker {
    let acc = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port: imap, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: smtp, security: Security::None },
        ..Default::default()
    };
    Worker::new(acc, Auth::shared("pw"), store.clone(), pool.sink.clone(), SyncOptions::default())
}

fn subjects(took: &Mutex<Took>) -> Vec<String> {
    took.lock().unwrap().messages.clone()
}

fn in_sent(st: &Arc<Mutex<crate::fake::State>>) -> usize {
    st.lock().unwrap().mailbox("Sent").unwrap().msgs.len()
}

/// The removals of the files `files` ends with fail from now on, or (`quietly`) answer as if done
/// and leave them, as a data file does when it can't look whether they're there; `None`: they go.
fn hold(store: &Store, quietly: bool, files: Option<&str>) {
    if quietly { store.fail_quietly(files) } else { store.fail(None, files) }
}

/// Sent, but its two files can't be removed (held, or their removal answering as if done and
/// leaving them): it leaves the Outbox all the same, and neither the next round nor the next
/// start sends it again. Its files go once they can.
#[test]
fn a_sent_message_whose_files_cant_be_removed_isnt_sent_again() {
    for quietly in [false, true] {
        let st = crate::fake::State::new();
        let imap = crate::fake::serve_tcp(&st);
        let (smtp, took) = smtp_server();
        let store = Store::failing();
        let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
        queue(&store, 7, "ana@contoso.example", "Held");
        hold(&store, quietly, Some("outbox/7.json|outbox/7.eml"));
        let mut w = worker(&pool, &store, imap, smtp);
        w.flush_outbox();
        assert_eq!(subjects(&took), ["Held"], "quietly {quietly}");
        assert_eq!(in_sent(&st), 1, "quietly {quietly}: its copy filed");
        w.flush_outbox();
        assert_eq!(subjects(&took), ["Held"], "quietly {quietly}: sent once");
        assert!(store.outbox("a1").is_empty(), "quietly {quietly}: out of the Outbox: {:?}", store.outbox("a1"));
        worker(&pool, &store, imap, smtp).flush_outbox();
        assert_eq!(subjects(&took), ["Held"], "quietly {quietly}: sent once, after a new start too");
        assert_eq!(in_sent(&st), 1, "quietly {quietly}");
        // The files can go now: the next round takes them.
        hold(&store, quietly, None);
        queue(&store, 8, "ana@contoso.example", "Next");
        worker(&pool, &store, imap, smtp).flush_outbox();
        assert_eq!(subjects(&took), ["Held", "Next"], "quietly {quietly}");
        assert!(store.outbox("a1").is_empty(), "quietly {quietly}");
        assert!(store.outbox_raw("a1", 7).is_none(), "quietly {quietly}: its source removed");
        assert!(store.account_files("a1", "outbox").unwrap().is_empty(), "quietly {quietly}: what was left removed");
        assert_eq!(in_sent(&st), 2, "quietly {quietly}");
    }
}

/// Nothing in the Outbox can be changed at all: this run of Quatrelle doesn't send it again, and
/// says the message is still there. Once the Outbox can be changed, the next round takes it out:
/// a new start doesn't send it either.
#[test]
fn a_sent_message_nothing_can_take_out_isnt_sent_again_while_running() {
    let st = crate::fake::State::new();
    let imap = crate::fake::serve_tcp(&st);
    let (smtp, took) = smtp_server();
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    queue(&store, 7, "ana@contoso.example", "Stuck");
    store.fail(None, Some("outbox/7.json|outbox/7.eml|outbox/7.gone"));
    let mut w = worker(&pool, &store, imap, smtp);
    w.flush_outbox();
    w.flush_outbox();
    assert_eq!(subjects(&took), ["Stuck"], "sent once");
    let mut said = Vec::new();
    while let Some(e) = pool.wait(Duration::from_millis(100)) {
        said.push(e);
    }
    assert!(said.iter().any(|e| matches!(e, Event::Notice { message, .. } if message.contains("Outbox"))), "{said:?}");
    assert_eq!(said.iter().filter(|e| matches!(e, Event::Sent { id: 7, .. })).count(), 1, "{said:?}");
    store.fail(None, None);
    w.flush_outbox();
    assert!(store.outbox("a1").is_empty(), "still in the Outbox: {:?}", store.outbox("a1"));
    assert!(w.outbox_done.is_empty(), "not forgotten once taken out");
    worker(&pool, &store, imap, smtp).flush_outbox();
    assert_eq!(subjects(&took), ["Stuck"], "sent again after a new start");
}

/// Refused for good, and the mark saying so can't be written: not tried again by this run. Once
/// it can be written, it is: a new start doesn't ask again either, and the message waits in the
/// Outbox to be changed.
#[test]
fn a_refused_message_whose_mark_cant_be_written_isnt_tried_again() {
    let st = crate::fake::State::new();
    let imap = crate::fake::serve_tcp(&st);
    let (smtp, took) = smtp_server();
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    queue(&store, 7, "refused@contoso.example", "Refused");
    store.fail(None, Some("outbox/7.json"));
    let mut w = worker(&pool, &store, imap, smtp);
    w.flush_outbox();
    w.flush_outbox();
    assert_eq!(took.lock().unwrap().refused.len(), 1, "asked once");
    assert!(subjects(&took).is_empty());
    assert_eq!(store.outbox("a1").len(), 1, "it waits in the Outbox to be changed");
    store.fail(None, None);
    w.flush_outbox();
    worker(&pool, &store, imap, smtp).flush_outbox();
    assert_eq!(took.lock().unwrap().refused.len(), 1, "asked again after a new start");
    let waiting = store.outbox("a1");
    assert!(waiting.first().and_then(|i| i.error.as_deref()).is_some_and(|e| e.starts_with("refused:")), "{waiting:?}");
    assert!(w.outbox_done.is_empty(), "not forgotten once marked");
}

/// What went is known from the mark on disk, not from the worker that sent it: sent with its
/// files held, the mark is beside them; a worker started again (its account changed, a new
/// start) sends nothing, and takes what's left out once it can.
#[test]
fn a_worker_started_again_still_knows_what_went() {
    let st = crate::fake::State::new();
    let imap = crate::fake::serve_tcp(&st);
    let (smtp, took) = smtp_server();
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    queue(&store, 7, "ana@contoso.example", "Once");
    store.fail(None, Some("outbox/7.json|outbox/7.eml"));
    worker(&pool, &store, imap, smtp).flush_outbox();
    let mut again = worker(&pool, &store, imap, smtp);
    again.flush_outbox();
    assert_eq!(subjects(&took), ["Once"], "sent again by a worker started again");
    let names = store.account_files("a1", "outbox").unwrap();
    assert!(names.iter().any(|n| n == "7.gone") && names.iter().any(|n| n == "7.eml"), "{names:?}");
    store.fail(None, None);
    again.flush_outbox();
    assert_eq!(subjects(&took), ["Once"]);
    assert!(store.account_files("a1", "outbox").unwrap().is_empty(), "what was left kept");
}

/// Through a real server on this computer, by hand: GreenMail's SMTP and IMAP (any password),
/// `QUATRELLE_REAL_OUTBOX="23025 23143"`. Delivered once to the recipient, one copy in Sent.
/// Without the setting, nothing is tried.
#[test]
#[ignore]
fn real_outbox_sends_once() {
    let Some((smtp, imap)) = std::env::var("QUATRELLE_REAL_OUTBOX").ok().and_then(|v| {
        let mut p = v.split_whitespace().map(|p| p.parse::<u16>().ok());
        Some((p.next()??, p.next()??))
    }) else {
        return;
    };
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let subject = format!("Outbox once {}", std::process::id());
    let to = format!("ana{}@contoso.example", std::process::id());
    queue(&store, 7, &to, &subject);
    store.fail(None, Some("outbox/7.json|outbox/7.eml"));
    let mut w = worker(&pool, &store, imap, smtp);
    w.flush_outbox();
    w.flush_outbox();
    worker(&pool, &store, imap, smtp).flush_outbox();
    let found = |user: &str, folder: &str| {
        let cfg = ServerConfig { host: "127.0.0.1".into(), port: imap, security: Security::None };
        let mut c = crate::imap::Client::connect(&cfg).unwrap();
        c.login(user, "pw").unwrap();
        c.select(folder, true).unwrap();
        c.uid_search(&format!("SUBJECT \"{subject}\"")).unwrap().len()
    };
    // GreenMail delivers at once.
    assert_eq!(found(&to, "INBOX"), 1, "delivered once");
    assert_eq!(found("me@studio.example", "Sent"), 1, "one copy in Sent");
    assert!(store.outbox("a1").is_empty());
}
