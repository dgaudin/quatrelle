// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Requests with a deadline for all of their exchange: the connections, the TLS handshakes, the
//! redirects and every byte of the answer ([`stream::Timed`], under TLS: [`stream::tls_within`]). Past it, the request fails as a
//! connection cut does (`TimedOut`): what a server that answers a byte at a time gets, however
//! steadily it answers. For what one thread waits on alone: a sign-in's token, a picture a message
//! points at.

use super::*;

/// [`send`] within `within`, its redirects included.
pub fn send_within(req: &Request, within: Duration) -> Result<Response> {
    send_to(req, false, Instant::now().checked_add(within))
}

/// [`get_public`] within `within`, its redirects included.
pub fn get_public_within(url: &str, max: usize, within: Duration) -> Result<Response> {
    send_to(&Request::get(url, max), true, Instant::now().checked_add(within))
}

/// One request on a connection of its own, made and answered by `until`.
pub(super) fn once(req: &Request, url: &Url, public_only: bool, until: Instant) -> Result<Response> {
    let sock = if public_only { stream::tcp_public_within(&url.host, url.port, until)? } else { stream::tcp_within(&url.host, url.port, until)? };
    let mut s = if url.https { stream::tls_within(sock, &url.host, until)? } else { Stream::PlainWithin(stream::Timed::new(sock, until)) };
    exchange(&mut s, req, url)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    use super::*;

    /// A server on this computer that reads a request, then answers its head and a byte of its
    /// body every `every`, without end, until the connection goes: its address.
    fn trickling(every: Duration) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let at = format!("http://127.0.0.1:{}/slow", l.local_addr().unwrap().port());
        std::thread::spawn(move || {
            for conn in l.incoming() {
                let Ok(mut s) = conn else { continue };
                std::thread::spawn(move || {
                    let _ = s.read(&mut [0u8; 4096]);
                    let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1000000\r\n\r\n");
                    while s.write_all(b" ").is_ok() {
                        std::thread::sleep(every);
                    }
                });
            }
        });
        at
    }

    /// Review 1.5, network 1: an answer that trickles a byte every 50 ms, without end, is given up
    /// at its deadline as a connection cut is (`TimedOut`), where each read alone would wait 60 s.
    #[test]
    fn an_answer_that_trickles_ends_at_its_deadline() {
        let url = trickling(Duration::from_millis(50));
        let asked = Instant::now();
        let (done, came) = std::sync::mpsc::channel();
        std::thread::spawn(move || drop(done.send(send_within(&Request::get(&url, 8 << 20), Duration::from_secs(1)))));
        let got = came.recv_timeout(Duration::from_secs(5)).expect("the request still waits on the trickle");
        let e = got.unwrap_err();
        assert!(matches!(&e, NetError::Io(io) if io.kind() == std::io::ErrorKind::TimedOut), "{e}");
        assert!(asked.elapsed() < Duration::from_secs(2), "{:?}", asked.elapsed());
    }

    /// A server that takes the connection and says nothing: a TLS handshake waits no longer than
    /// the deadline either.
    #[test]
    fn a_handshake_that_never_ends_ends_at_its_deadline() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("https://127.0.0.1:{}/", l.local_addr().unwrap().port());
        let held = std::thread::spawn(move || l.accept().map(|(s, _)| s));
        let asked = Instant::now();
        let (done, came) = std::sync::mpsc::channel();
        std::thread::spawn(move || drop(done.send(send_within(&Request::get(&url, 1 << 10), Duration::from_secs(1)))));
        let got = came.recv_timeout(Duration::from_secs(5)).expect("the handshake still waits");
        let e = got.unwrap_err();
        assert!(matches!(&e, NetError::Io(io) if io.kind() == std::io::ErrorKind::TimedOut), "{e}");
        assert!(asked.elapsed() < Duration::from_secs(2), "{:?}", asked.elapsed());
        drop(held.join());
    }

    /// Review 1.5, network 1: a sign-in service that trickles its answer, a byte every 50 ms
    /// without end, holds a renewal no longer than its deadline: given up as the network's
    /// (`TimedOut`), never a refusal that asks to sign in again.
    #[test]
    fn a_token_answer_that_trickles_is_given_up() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let p = crate::oauth::tests::local(&format!("http://127.0.0.1:{}/token", l.local_addr().unwrap().port()));
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let _ = std::io::Read::read(&mut s, &mut [0u8; 4096]);
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100000\r\n\r\n{");
                while s.write_all(b" ").is_ok() {
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        });
        let (done, came) = std::sync::mpsc::channel();
        let asked = Instant::now();
        std::thread::spawn(move || {
            let fields = vec![("grant_type", "refresh_token".to_string()), ("refresh_token", "r1".to_string())];
            let _ = done.send(crate::oauth::token_request_within(&p, fields, "", Duration::from_secs(1)));
        });
        let got = came.recv_timeout(Duration::from_secs(5)).expect("the renewal still waits on the trickle");
        assert!(matches!(&got, Err(NetError::Io(e)) if e.kind() == std::io::ErrorKind::TimedOut), "{got:?}");
        assert!(asked.elapsed() < Duration::from_secs(2), "{:?}", asked.elapsed());
    }

    /// Review 1.5, tour 1: under TLS, a server (or anyone on the way) that answers the ClientHello
    /// with a record's header saying 16 KiB, then a byte of it every 50 ms, holds the handshake no
    /// longer than the deadline: each read of the socket under rustls looks at it, where a TLS read
    /// waited for its whole record (13 minutes here, 11 days at a byte every 59 s).
    #[test]
    fn a_tls_record_that_trickles_ends_at_its_deadline() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("https://127.0.0.1:{}/", l.local_addr().unwrap().port());
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let _ = s.read(&mut [0u8; 4096]);
                let _ = s.write_all(&[0x16, 0x03, 0x03, 0x40, 0x00]);
                while s.write_all(&[0x02]).is_ok() {
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        });
        let asked = Instant::now();
        let (done, came) = std::sync::mpsc::channel();
        std::thread::spawn(move || drop(done.send(send_within(&Request::get(&url, 1 << 10), Duration::from_secs(1)))));
        let got = came.recv_timeout(Duration::from_secs(5)).expect("the TLS record still trickles in");
        let e = got.unwrap_err();
        assert!(matches!(&e, NetError::Io(io) if io.kind() == std::io::ErrorKind::TimedOut), "{e}");
        assert!(asked.elapsed() < Duration::from_secs(2), "{:?}", asked.elapsed());
    }

    /// Within its deadline, an answer comes as it would without one.
    #[test]
    fn an_answer_in_time_comes_whole() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/", l.local_addr().unwrap().port());
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let _ = s.read(&mut [0u8; 4096]);
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello");
            }
        });
        let r = send_within(&Request::get(&url, 1 << 10), Duration::from_secs(5)).unwrap();
        assert_eq!((r.status, r.body.as_slice()), (200, &b"hello"[..]));
    }
}
