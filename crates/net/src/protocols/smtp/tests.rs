// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The submission tests: local SMTP servers that take one message, and scripted ones.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::*;

/// A minimal SMTP server on localhost that accepts one message.
pub(crate) fn serve_once() -> (u16, std::thread::JoinHandle<String>) {
    serve_once_on(TcpListener::bind("127.0.0.1:0").unwrap())
}

/// The same, on a given listener.
pub(crate) fn serve_once_on(l: TcpListener) -> (u16, std::thread::JoinHandle<String>) {
    let port = l.local_addr().unwrap().port();
    let h = std::thread::spawn(move || {
        let (s, _) = l.accept().unwrap();
        let mut w = s.try_clone().unwrap();
        let mut r = BufReader::new(s);
        let mut log = String::new();
        w.write_all(b"220 test ESMTP\r\n").unwrap();
        let mut line = String::new();
        let mut in_data = false;
        while r.read_line(&mut line).unwrap_or(0) > 0 {
            log.push_str(&line);
            let cmd = line.trim_end().to_ascii_uppercase();
            if in_data {
                if cmd == "." {
                    in_data = false;
                    w.write_all(b"250 queued\r\n").unwrap();
                }
            } else if cmd.starts_with("EHLO") {
                w.write_all(b"250-test\r\n250 AUTH PLAIN\r\n").unwrap();
            } else if cmd.starts_with("AUTH") {
                w.write_all(b"235 ok\r\n").unwrap();
            } else if cmd == "DATA" {
                in_data = true;
                w.write_all(b"354 go\r\n").unwrap();
            } else if cmd == "QUIT" {
                w.write_all(b"221 bye\r\n").unwrap();
                break;
            } else {
                w.write_all(b"250 ok\r\n").unwrap();
            }
            line.clear();
        }
        log
    });
    (port, h)
}

/// The EHLO names this computer's address on the way to the server, as an address literal:
/// the loopback address only for a server on this computer, and this computer's own network
/// address for one reached through it (lettre's default is `[127.0.0.1]` for all). Adapted
/// from a6f00b41.
#[test]
fn ehlo_names_this_computers_address_on_the_way() {
    let raw = b"From: z@x.example\r\nTo: a@y.example\r\nSubject: hi\r\n\r\nbody\r\n";
    let to = ["a@y.example".to_string()];
    let (port, h) = serve_once();
    let server = ServerConfig { host: "127.0.0.1".into(), port, security: Security::None };
    send(&server, "", "", "z@x.example", &to, raw).unwrap();
    let log = h.join().unwrap();
    assert!(log.starts_with("EHLO [127.0.0.1]\r\n"), "{log}");
    // IPv6 loopback, where the system has it.
    if let Ok(l) = TcpListener::bind("[::1]:0") {
        let (port, h) = serve_once_on(l);
        let server = ServerConfig { host: "::1".into(), port, security: Security::None };
        send(&server, "", "", "z@x.example", &to, raw).unwrap();
        let log = h.join().unwrap();
        assert!(log.starts_with("EHLO [IPv6:::1]\r\n"), "{log}");
    }
    // This computer's network address (the one it would reach a documentation address from;
    // nothing is sent there), where it has one: a server reached through it hears that.
    let own = UdpSocket::bind("0.0.0.0:0").and_then(|s| {
        s.connect("192.0.2.1:9")?;
        s.local_addr()
    });
    let Some(own) = own.ok().map(|a| a.ip()).filter(|ip| !ip.is_loopback() && !ip.is_unspecified()) else {
        eprintln!("no network address: the rest skipped");
        return;
    };
    let (port, h) = serve_once_on(TcpListener::bind((own, 0)).unwrap());
    let server = ServerConfig { host: own.to_string(), port, security: Security::None };
    send(&server, "", "", "z@x.example", &to, raw).unwrap();
    let log = h.join().unwrap();
    assert!(log.starts_with(&format!("EHLO [{own}]\r\n")), "{log}");
    assert!(!log.contains("127.0.0.1"), "{log}");
}

#[test]
fn submits_to_a_local_server() {
    let (port, h) = serve_once();
    let server = ServerConfig { host: "127.0.0.1".into(), port, security: Security::None };
    let raw = b"From: z@x.example\r\nTo: a@y.example\r\nSubject: hi\r\n\r\nbody\r\n";
    send(&server, "z@x.example", "pw", "z@x.example", &["a@y.example".into(), "b@y.example".into()], raw).unwrap();
    let log = h.join().unwrap();
    assert!(log.contains("MAIL FROM:<z@x.example>"), "{log}");
    assert!(log.contains("RCPT TO:<a@y.example>") && log.contains("RCPT TO:<b@y.example>"));
    assert!(log.contains("Subject: hi"));
}

/// A refusal is classified by its reply code, not by the word "auth" in its text.
#[test]
fn refusals_are_classified_by_code() {
    let to = ["bob@y.example".to_string(), "ann@y.example".to_string()];
    let class = |code: u16, text: &str| refusal(code, text, &to);
    // Sign-in refusals: kept for after signing in again, not refused for good.
    for (code, text) in [
        (535, "5.7.8 Username and Password not accepted"),
        (534, "5.7.9 Please log in with your web browser"),
        (530, "5.7.0 Authentication required"),
        (550, "5.7.8 authentication credentials invalid"),
    ] {
        let e = class(code, text);
        assert!(matches!(e, NetError::Auth(_)), "{code} {text}: {e:?}");
        assert!(e.is_retryable(), "{code}: kept in the Outbox");
    }
    // The message refused for good.
    for (code, text) in [
        (553, "5.7.1 sender not owned by authenticated user"),
        (554, "5.7.1 relay access denied: authentication needed for relaying elsewhere"),
        (552, "5.3.4 message too big"),
    ] {
        let e = class(code, text);
        assert!(matches!(e, NetError::Smtp(_)), "{code} {text}: {e:?}");
        assert!(!e.is_retryable());
    }
    // The refused recipient is named when the server says which.
    let e = class(550, "5.1.1 <Ann@y.example>: Recipient address rejected: User unknown");
    assert!(matches!(&e, NetError::Smtp(m) if m.contains("ann@y.example") && !m.contains("bob@")), "{e:?}");
}

/// What a scripted server heard: each line (upper-cased, its end removed) and when, `<START>` and
/// `<END>` around each connection.
#[derive(Clone, Default)]
pub(crate) struct Heard(Arc<Mutex<Vec<(Instant, String)>>>);

impl Heard {
    fn push(&self, line: &str) {
        self.0.lock().unwrap().push((Instant::now(), line.to_string()));
    }

    /// Every line heard, once each connection made is over: the client let go of it, and the
    /// server read all it was sent.
    pub(crate) fn lines(&self) -> Vec<(Instant, String)> {
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            let heard = self.0.lock().unwrap().clone();
            let count = |marker: &str| heard.iter().filter(|(_, l)| l == marker).count();
            if count("<START>") == count("<END>") || Instant::now() > until {
                return heard;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The lines the client sent, in order.
    pub(crate) fn said(&self) -> Vec<String> {
        self.lines().into_iter().map(|(_, l)| l).filter(|l| !l.starts_with('<')).collect()
    }

    /// How many messages went to it whole: their end (a line `.`) heard.
    pub(crate) fn messages(&self) -> usize {
        self.said().iter().filter(|l| *l == ".").count()
    }
}

/// How long a scripted server takes over an answer said `"SLOW …"`: longer than [`TIMEOUT`],
/// shorter than [`SENT_TIMEOUT`].
const SLOW: Duration = Duration::from_secs((TIMEOUT.as_secs() + SENT_TIMEOUT.as_secs()) / 2);

/// Plays a scripted SMTP server on a free local port, for every connection made to it, one after
/// the other: greets, then answers each line (upper-cased, its end removed) with what `script`
/// returns, until QUIT is answered or the connection's end. `"CLOSE"`: the connection drops
/// there, nothing said; `"SLOW …"`: the rest is said after [`SLOW`]; `""`: nothing is said (a
/// line of the message, or no answer).
pub(crate) fn scripted_heard(script: fn(&str) -> &'static str) -> (u16, Heard) {
    let socket = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = socket.local_addr().unwrap().port();
    let heard = Heard::default();
    let kept = heard.clone();
    std::thread::spawn(move || {
        for sock in socket.incoming().map_while(std::result::Result::ok) {
            kept.push("<START>");
            play(sock, script, &kept);
            kept.push("<END>");
        }
    });
    (port, heard)
}

/// [`scripted_heard`], what it heard left aside.
fn scripted(script: fn(&str) -> &'static str) -> u16 {
    scripted_heard(script).0
}

/// One connection of [`scripted_heard`].
fn play(sock: TcpStream, script: fn(&str) -> &'static str, heard: &Heard) {
    let mut out = sock.try_clone().unwrap();
    let _ = out.write_all(b"220 scripted.test ready\r\n");
    for command in BufReader::new(sock).lines().map_while(std::result::Result::ok) {
        let command = command.to_ascii_uppercase();
        heard.push(&command);
        let answer = script(&command);
        if answer == "CLOSE" {
            break;
        }
        let answer = answer.strip_prefix("SLOW ").map_or(answer, |later| {
            std::thread::sleep(SLOW);
            later
        });
        let _ = out.write_all(answer.as_bytes());
        if command == "QUIT" && !answer.is_empty() {
            break;
        }
    }
}

/// A server that takes the message whole, then drops the connection without a word.
pub(crate) fn cut_after_the_message(command: &str) -> &'static str {
    match command.split(' ').next().unwrap_or_default() {
        "EHLO" => "250-scripted.test\r\n250 AUTH PLAIN\r\n",
        "AUTH" => "235 2.7.0 welcome\r\n",
        "MAIL" | "RCPT" => "250 2.0.0 fine\r\n",
        "DATA" => "354 go on\r\n",
        "." => "CLOSE",
        "QUIT" => "221 2.0.0 closing\r\n",
        // A line of the message.
        _ => "",
    }
}

/// End to end: the server turns the sender away at MAIL FROM with a 553, which refuses this
/// message; it isn't taken for a failed sign-in.
#[test]
fn a_refused_sender_is_not_a_sign_in_failure() {
    let port = scripted(|command| match command.split(' ').next().unwrap_or_default() {
        "EHLO" => "250-scripted.test\r\n250 AUTH PLAIN\r\n",
        "AUTH" => "235 2.7.0 welcome\r\n",
        "MAIL" => "553 5.7.1 <z@x.example>: the authenticated account may not send as this address\r\n",
        "QUIT" => "221 2.0.0 closing\r\n",
        _ => "250 2.0.0 fine\r\n",
    });
    let server = ServerConfig { security: Security::None, port, host: "127.0.0.1".into() };
    let message = b"Subject: refused\r\n\r\nnot delivered\r\n";
    let e = send(&server, "z@x.example", "pw", "z@x.example", &["a@y.example".into()], message).unwrap_err();
    assert!(matches!(e, NetError::Smtp(_)), "{e:?}");
}

#[test]
fn rejects_bad_addresses_before_connecting() {
    let server = ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None };
    assert!(matches!(send(&server, "", "", "not an address", &["a@b.c".into()], b""), Err(NetError::Smtp(_))));
    assert!(matches!(send(&server, "", "", "a@b.c", &[], b""), Err(NetError::Smtp(_))));
}

/// The system's roots reach SMTP's TLS, and one the store holds but rustls can't read
/// doesn't stop sending: given to lettre as it is, it failed every message to every server.
#[test]
fn an_unreadable_root_in_the_store_does_not_stop_sending() {
    use crate::stream::trust::test_certs::{internal_ca, unreadable};
    use crate::stream::trust::{Distrusted, usable_roots};
    let store = vec![unreadable(), internal_ca()];
    assert!(tls_parameters("mail.corp.test", &usable_roots(store.clone(), &Distrusted::new())).is_ok());
    assert!(tls_parameters("mail.corp.test", &[]).is_ok());
    // Why the store is sorted first: lettre refuses the lot.
    assert!(tls_parameters("mail.corp.test", &store).is_err());
}

/// The server takes the whole message and the connection drops before it answers: the
/// message may have gone, so it isn't one to send again unasked (it was, and arrived twice).
/// Dropped before the message is handed over, nothing has gone, and it's tried again.
#[test]
fn no_answer_after_the_message_is_not_tried_again_unasked() {
    let message = b"Subject: gone\r\n\r\ntext\r\n";
    let to = ["a@y.example".to_string()];
    let (port, heard) = scripted_heard(cut_after_the_message);
    let server = ServerConfig { security: Security::None, port, host: "127.0.0.1".into() };
    let e = send(&server, "z@x.example", "pw", "z@x.example", &to, message).unwrap_err();
    assert!(matches!(e, NetError::Unconfirmed(_)), "{e:?}");
    assert!(!e.is_retryable(), "{e}");
    assert_eq!(heard.messages(), 1);
    let port = scripted(|command| match command.split(' ').next().unwrap_or_default() {
        "EHLO" => "250-scripted.test\r\n250 AUTH PLAIN\r\n",
        "AUTH" => "235 2.7.0 welcome\r\n",
        "DATA" => "CLOSE",
        _ => "250 2.0.0 fine\r\n",
    });
    let server = ServerConfig { security: Security::None, port, host: "127.0.0.1".into() };
    let e = send(&server, "z@x.example", "pw", "z@x.example", &to, message).unwrap_err();
    assert!(matches!(e, NetError::Io(_)) && e.is_retryable(), "{e:?}");
}

/// A server that checks a message before it answers its end may take longer than any other
/// answer: it gets [`SENT_TIMEOUT`] (ten minutes), not [`TIMEOUT`] (one). Given up on after one,
/// the message was tried again, and arrived twice.
#[test]
fn the_answer_to_a_message_may_take_ten_minutes_not_one() {
    assert!(TIMEOUT < SLOW && SLOW < SENT_TIMEOUT);
    let (port, heard) = scripted_heard(|command| match command.split(' ').next().unwrap_or_default() {
        "EHLO" => "250-scripted.test\r\n250 AUTH PLAIN\r\n",
        "AUTH" => "235 2.7.0 welcome\r\n",
        "MAIL" | "RCPT" => "250 2.0.0 fine\r\n",
        "DATA" => "354 go on\r\n",
        "." => "SLOW 250 2.0.0 queued once checked\r\n",
        "QUIT" => "221 2.0.0 closing\r\n",
        _ => "",
    });
    let server = ServerConfig { security: Security::None, port, host: "127.0.0.1".into() };
    let message = b"Subject: checked\r\n\r\nscanned before it's taken\r\n";
    let sent = send(&server, "zoe@studio.example", "pw", "zoe@studio.example", &["ana@studio.example".into()], message);
    assert!(sent.is_ok(), "{sent:?}");
    assert_eq!(heard.messages(), 1);
}

/// A positive answer of the wrong class answers another command: the connection is out of step
/// (lettre's 04e3ac3 checks it, and so must Quatrelle). A 250 to DATA: nothing of the message
/// goes, or the server would read it as commands, and no QUIT either. A 354 to the message's
/// end: it may have gone or not, never "sent".
#[test]
fn an_answer_out_of_turn_never_passes_for_sent() {
    // A line of it a server would take for a command, were it read as one.
    let message = b"Subject: in step\r\n\r\nRCPT TO:<eve@studio.example>\r\n";
    let to = ["ana@studio.example".to_string()];
    let (port, heard) = scripted_heard(|command| match command.split(' ').next().unwrap_or_default() {
        "EHLO" => "250-scripted.test\r\n250 AUTH PLAIN\r\n",
        "AUTH" => "235 2.7.0 welcome\r\n",
        "QUIT" => "221 2.0.0 closing\r\n",
        _ => "250 2.0.0 fine\r\n",
    });
    let server = ServerConfig { security: Security::None, port, host: "127.0.0.1".into() };
    let sent = send(&server, "zoe@studio.example", "pw", "zoe@studio.example", &to, message);
    assert!(sent.as_ref().is_err_and(|e| e.is_retryable() && !matches!(e, NetError::Unconfirmed(_))), "nothing went: {sent:?}");
    let said = heard.said();
    let data = said.iter().position(|l| l == "DATA").unwrap();
    assert!(said.get(data + 1..).is_some_and(<[String]>::is_empty), "after DATA: {said:?}");
    // 354 to the message's end.
    let (port, heard) = scripted_heard(|command| match command.split(' ').next().unwrap_or_default() {
        "EHLO" => "250-scripted.test\r\n250 AUTH PLAIN\r\n",
        "AUTH" => "235 2.7.0 welcome\r\n",
        "MAIL" | "RCPT" => "250 2.0.0 fine\r\n",
        "DATA" | "." => "354 go on\r\n",
        "QUIT" => "221 2.0.0 closing\r\n",
        _ => "",
    });
    let server = ServerConfig { security: Security::None, port, host: "127.0.0.1".into() };
    let message = b"Subject: in step\r\n\r\nno command in it\r\n";
    let sent = send(&server, "zoe@studio.example", "pw", "zoe@studio.example", &to, message);
    assert!(sent.is_err(), "a 354 passed for sent");
    assert_eq!(heard.messages(), 1);
    assert!(matches!(sent, Err(NetError::Unconfirmed(_))), "{sent:?}");
}

/// A refused sign-in is one wherever it's said, AUTH included: a 535 is `Auth` (an OAuth token is
/// renewed for it, the account asks for its password), a 454 asks to wait, and no answer or no
/// way to sign in in common is no answer (`Io`), never a refused message.
#[test]
fn a_refused_sign_in_stays_a_sign_in_refusal() {
    let attempt = |script: fn(&str) -> &'static str| {
        let port = scripted(script);
        let server = ServerConfig { security: Security::None, port, host: "127.0.0.1".into() };
        send(&server, "zoe@studio.example", "pw", "zoe@studio.example", &["ana@studio.example".into()], b"Subject: hi\r\n\r\nhi\r\n").unwrap_err()
    };
    let e = attempt(|command| match command.split(' ').next().unwrap_or_default() {
        "EHLO" => "250-scripted.test\r\n250 AUTH PLAIN\r\n",
        "AUTH" => "535 5.7.8 credentials not accepted\r\n",
        "QUIT" => "221 2.0.0 closing\r\n",
        _ => "250 2.0.0 fine\r\n",
    });
    assert!(matches!(e, NetError::Auth(_)), "{e:?}");
    let e = attempt(|command| match command.split(' ').next().unwrap_or_default() {
        "EHLO" => "250-scripted.test\r\n250 AUTH PLAIN\r\n",
        "AUTH" => "454 4.7.0 try again in a moment\r\n",
        "QUIT" => "221 2.0.0 closing\r\n",
        _ => "250 2.0.0 fine\r\n",
    });
    assert!(matches!(e, NetError::Later(_)), "{e:?}");
    let e = attempt(|command| match command.split(' ').next().unwrap_or_default() {
        "EHLO" => "250-scripted.test\r\n250 AUTH PLAIN\r\n",
        "AUTH" => "CLOSE",
        _ => "250 2.0.0 fine\r\n",
    });
    assert!(matches!(e, NetError::Io(_)), "{e:?}");
    let e = attempt(|command| match command.split(' ').next().unwrap_or_default() {
        "EHLO" => "250-scripted.test\r\n250 AUTH CRAM-MD5\r\n",
        "QUIT" => "221 2.0.0 closing\r\n",
        _ => "250 2.0.0 fine\r\n",
    });
    assert!(matches!(e, NetError::Io(_)), "no mechanism in common: {e:?}");
}

/// An address outside ASCII (the sender's here, the recipients' all ASCII) goes where the server
/// says SMTPUTF8 (RFC 6531), with the parameter on MAIL FROM; elsewhere nothing of the message
/// goes, and it can go once the server takes it.
#[test]
fn addresses_outside_ascii_are_sent_where_the_server_takes_them() {
    let message = b"Subject: hi\r\n\r\nhi\r\n";
    let to = ["ana@studio.example".to_string()];
    let (port, heard) = scripted_heard(|command| match command.split(' ').next().unwrap_or_default() {
        "EHLO" => "250-scripted.test\r\n250 SMTPUTF8\r\n",
        "DATA" => "354 go on\r\n",
        "." => "250 2.0.0 queued\r\n",
        "QUIT" => "221 2.0.0 closing\r\n",
        "MAIL" | "RCPT" => "250 2.0.0 fine\r\n",
        _ => "",
    });
    let server = ServerConfig { security: Security::None, port, host: "127.0.0.1".into() };
    let sent = send(&server, "", "", "zoé@studio.example", &to, message);
    assert!(sent.is_ok(), "{sent:?}");
    let said = heard.said();
    assert!(said.iter().any(|l| l.starts_with("MAIL FROM:<ZOé@STUDIO.EXAMPLE>") && l.contains("SMTPUTF8")), "{said:?}");
    let (port, heard) = scripted_heard(|command| match command.split(' ').next().unwrap_or_default() {
        "EHLO" => "250-scripted.test\r\n250 8BITMIME\r\n",
        "QUIT" => "221 2.0.0 closing\r\n",
        _ => "250 2.0.0 fine\r\n",
    });
    let server = ServerConfig { security: Security::None, port, host: "127.0.0.1".into() };
    let e = send(&server, "", "", "zoé@studio.example", &to, message).unwrap_err();
    assert!(matches!(e, NetError::Io(_)), "{e:?}");
    let said = heard.said();
    let ehlo = said.iter().position(|l| l.starts_with("EHLO")).unwrap();
    assert!(said.get(ehlo + 1..).unwrap_or_default().iter().all(|l| l == "QUIT"), "after EHLO: {said:?}");
}

/// No answer after the message, then none to QUIT either: that goodbye waits [`TIMEOUT`], not
/// the ten minutes the message's answer was given (twenty in all).
#[test]
fn after_no_answer_the_goodbye_waits_a_minute_at_most() {
    let (port, heard) = scripted_heard(|command| match command.split(' ').next().unwrap_or_default() {
        "EHLO" => "250-scripted.test\r\n250 AUTH PLAIN\r\n",
        "AUTH" => "235 2.7.0 welcome\r\n",
        "MAIL" | "RCPT" => "250 2.0.0 fine\r\n",
        "DATA" => "354 go on\r\n",
        // The message's end and QUIT, and everything else: not a word.
        _ => "",
    });
    let server = ServerConfig { security: Security::None, port, host: "127.0.0.1".into() };
    let message = b"Subject: no word\r\n\r\nnothing heard\r\n";
    let e = send(&server, "zoe@studio.example", "pw", "zoe@studio.example", &["ana@studio.example".into()], message).unwrap_err();
    let back = Instant::now();
    assert!(matches!(e, NetError::Unconfirmed(_)), "{e:?}");
    let quit = heard.lines().into_iter().find(|(_, l)| l == "QUIT").map(|(at, _)| at).unwrap();
    let waited = back.saturating_duration_since(quit);
    assert!(waited < TIMEOUT + (SENT_TIMEOUT - TIMEOUT) / 2, "the goodbye waited {waited:?}");
}

/// The connection drops while the message is going, before its end: the server can't have taken
/// it (it takes a message at its end), so it can go again; it never passes for maybe gone.
#[test]
fn a_message_cut_before_its_end_can_go_again() {
    let (port, heard) = scripted_heard(|command| match command.split(' ').next().unwrap_or_default() {
        "EHLO" => "250-scripted.test\r\n250 AUTH PLAIN\r\n",
        "AUTH" => "235 2.7.0 welcome\r\n",
        "MAIL" | "RCPT" => "250 2.0.0 fine\r\n",
        "DATA" => "354 go on\r\n",
        "SUBJECT:" => "CLOSE",
        "QUIT" => "221 2.0.0 closing\r\n",
        _ => "",
    });
    let server = ServerConfig { security: Security::None, port, host: "127.0.0.1".into() };
    // More than the connection can hold on its way: the writing fails before the end goes.
    let mut message = b"Subject: big\r\n\r\n".to_vec();
    while message.len() < 32 << 20 {
        message.extend_from_slice(&[b'x'; 76]);
        message.extend_from_slice(b"\r\n");
    }
    let e = send(&server, "zoe@studio.example", "pw", "zoe@studio.example", &["ana@studio.example".into()], &message).unwrap_err();
    assert!(matches!(e, NetError::Io(_)) && e.is_retryable(), "{e:?}");
    assert_eq!(heard.messages(), 0);
}
