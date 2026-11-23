// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
//! The submission tests: local SMTP servers that take one message, and scripted ones.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, UdpSocket};

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

/// Plays a scripted SMTP server for one connection on a free local port: greets, then answers
/// each command line (upper-cased, line end removed) with what `script` returns, until QUIT.
fn scripted(script: fn(&str) -> &'static str) -> u16 {
    let socket = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = socket.local_addr().unwrap();
    let play = move || {
        let Ok((sock, _)) = socket.accept() else { return };
        let mut out = sock.try_clone().unwrap();
        let _ = out.write_all(b"220 scripted.test ready\r\n");
        for command in BufReader::new(sock).lines().map_while(std::result::Result::ok) {
            let command = command.to_ascii_uppercase();
            let _ = out.write_all(script(&command).as_bytes());
            if command == "QUIT" {
                break;
            }
        }
    };
    std::thread::spawn(play);
    address.port()
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
