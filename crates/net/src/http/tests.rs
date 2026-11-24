// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
use std::io::{BufRead, BufReader};
use std::net::TcpListener;

use super::*;

/// The address alone stops a test, however it's written and whichever way it's reached, a
/// redirect included; only that refusal counts, not a failed connection.
#[test]
fn a_test_never_reaches_the_real_webex() {
    let refused = |r: Result<Response>| matches!(r, Err(NetError::Protocol(m)) if m == REAL_WEBEX);
    for url in [
        "https://webexapis.com/v1/people/me",
        "https://WEBEXAPIS.com/v1/access_token",
        "https://webexapis.com./v1/meetings",
        "https://api.webexapis.com/v1/meetings",
        "https://webexapis.com:8443/v1/meetings",
    ] {
        assert!(refused(send(&Request::get(url, 100))), "{url}");
        assert!(refused(send_public(&Request::post(url, "application/json", b"{}".to_vec(), 100))), "{url}");
        assert!(refused(get_public(url, 100)), "{url}");
    }
    let moved = b"HTTP/1.1 302 Found\r\nLocation: https://webexapis.com/v1/people/me\r\nContent-Length: 0\r\n\r\n".to_vec();
    let (base, served) = serve(vec![moved]);
    assert!(refused(get(&base, 100)), "a redirect is refused too");
    assert_eq!(served.join().unwrap().len(), 1);
}

/// How long [`serve`] waits for a request, or for the rest of one, before it gives up.
const WAIT: Duration = Duration::from_secs(20);

/// Serve `responses` (one per connection, in order) on localhost; returns the base URL and
/// the requests received. A request that doesn't come within [`WAIT`] ends it, the requests
/// seen so far returned: a test expecting more fails in seconds rather than hanging the run
/// (a race in a Graph renewal test hung CI for four hours, 29/10).
pub(crate) fn serve(responses: Vec<Vec<u8>>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let (l, base) = bind();
    (base, serve_on(l, responses))
}

/// A listener on localhost and its base URL, to serve on once the answers are known (two
/// servers that redirect to each other need each other's address first).
pub(crate) fn bind() -> (TcpListener, String) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    l.set_nonblocking(true).unwrap();
    let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    (l, base)
}

/// [`serve`] on a listener from [`bind`].
pub(crate) fn serve_on(l: TcpListener, responses: Vec<Vec<u8>>) -> std::thread::JoinHandle<Vec<String>> {
    std::thread::spawn(move || {
        let mut seen = Vec::new();
        for r in responses {
            let end = Instant::now() + WAIT;
            let mut s = loop {
                match l.accept() {
                    Ok((s, _)) => break s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < end => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => return seen,
                }
            };
            s.set_nonblocking(false).unwrap();
            s.set_read_timeout(Some(WAIT)).unwrap();
            let mut reader = BufReader::new(s.try_clone().unwrap());
            let mut req = String::new();
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                req.push_str(&line);
            }
            let mut body = vec![0u8; len];
            reader.read_exact(&mut body).unwrap();
            req.push_str(&String::from_utf8_lossy(&body));
            seen.push(req);
            s.write_all(&r).unwrap();
        }
        seen
    })
}

/// The server's clock, from its answer: hostile values are none, never a panic.
#[test]
fn the_servers_date() {
    let with = |d: &str| Response { status: 200, headers: vec![("date".into(), d.into())], body: Vec::new(), head_at: None, url: None }.date();
    assert_eq!(with("Thu, 29 Oct 2026 11:30:24 GMT"), Some(1_793_273_424));
    assert_eq!(with("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
    for junk in [
        "",
        "Thu,",
        "Thu, 29 Foo 2026 11:30:24 GMT",
        "Thu, 29 Oct 99999999999 11:30:24 GMT",
        "Thu, 29 Oct 2026 11:30",
        "Thu, 99 Oct 2026 25:00:00 GMT",
    ] {
        assert_eq!(with(junk), None, "{junk}");
    }
}

#[test]
fn urls() {
    let u = Url::parse("HTTPS://Mail.Example.com/a/b?x=1#frag").unwrap();
    assert_eq!((u.https, u.host.as_str(), u.port, u.path.as_str()), (true, "mail.example.com", 443, "/a/b?x=1"));
    assert_eq!(Url::parse("http://h:8080").unwrap().path, "/");
    assert_eq!(Url::parse("http://[::1]:99/x").unwrap().host, "::1");
    assert_eq!(Url::parse("http://h/a b").unwrap().path, "/a%20b");
    for bad in ["ftp://h/", "http://", "http://user@h/", "http://h:0/", "http://h:x/", "javascript:alert(1)", "http://h h/"] {
        assert!(Url::parse(bad).is_err(), "{bad}");
    }
    let u = Url::parse("https://h.example/a/b/c?q").unwrap();
    assert_eq!(u.join("/x").unwrap().path, "/x");
    assert_eq!(u.join("d").unwrap().path, "/a/b/d");
    assert_eq!(u.join("//other.example/p").unwrap().host, "other.example");
    assert!(!u.join("http://x.example/").unwrap().https);
    assert_eq!(pct("a b&c=é"), "a%20b%26c%3D%C3%A9");
    assert_eq!(form(&[("a", "1 2"), ("b", "/")]), b"a=1%202&b=%2F".to_vec());
}

#[test]
fn lengths_chunks_redirects_and_limits() {
    let (base, h) = serve(vec![
        b"HTTP/1.1 302 Found\r\nLocation: /two\r\n\r\n".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Type: text/plain\r\n\r\nhello".to_vec(),
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5;ext=1\r\npedia\r\n0\r\n\r\n".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 999999\r\n\r\nxx".to_vec(),
        b"HTTP/1.0 200 OK\r\n\r\nuntil the end".to_vec(),
        b"HTTP/1.1 201 Created\r\nContent-Length: 2\r\n\r\nok".to_vec(),
        b"garbage\r\n\r\n".to_vec(),
    ]);
    let r = get(&format!("{base}/one"), 1000).unwrap();
    assert_eq!((r.status, r.body.as_slice(), r.header("content-type")), (200, &b"hello"[..], Some("text/plain")));
    assert_eq!(get(&base, 1000).unwrap().body, b"Wikipedia");
    assert!(matches!(get(&base, 1000), Err(NetError::Protocol(m)) if m.contains("larger")));
    assert_eq!(get(&base, 1000).unwrap().body, b"until the end");
    let r = send(
        &Request::post(&format!("{base}/token"), "application/x-www-form-urlencoded", form(&[("code", "x y")]), 100)
            .header("Authorization", "Bearer t"),
    )
    .unwrap();
    assert!(r.ok() && r.body == b"ok");
    assert!(get(&base, 1000).is_err());
    let seen = h.join().unwrap();
    assert!(seen[0].starts_with("GET /one HTTP/1.1\r\nHost: 127.0.0.1:"), "{}", seen[0]);
    assert!(seen[1].starts_with("GET /two "), "redirect followed: {}", seen[1]);
    assert!(seen[5].starts_with("POST /token ") && seen[5].contains("Authorization: Bearer t") && seen[5].ends_with("code=x%20y"), "{}", seen[5]);
    // Header injection is refused before anything is sent.
    assert!(send(&Request::get(&base, 10).header("X", "a\r\nEvil: 1")).is_err());
}

/// Whether a request as the server saw it carried the token or the cookie.
pub(crate) fn credentials_seen(request: &str) -> (bool, bool) {
    let lower = request.to_ascii_lowercase();
    (lower.contains("authorization: bearer x"), lower.contains("cookie: s=1"))
}

/// Hands out each piece on its own read, as a TLS record at a time.
struct Pieces(std::collections::VecDeque<Vec<u8>>);

impl Read for Pieces {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let Some(p) = self.0.pop_front() else { return Ok(0) };
        let n = p.len().min(buf.len());
        buf[..n].copy_from_slice(&p[..n]);
        if n < p.len() {
            self.0.push_front(p[n..].to_vec());
        }
        Ok(n)
    }
}

/// A chunked answer's last empty line, arriving on its own, belongs to that answer: left
/// unread, the next answer on the kept connection began with it ("bad status line: ").
/// And empty lines before a status line are skipped anyway.
#[test]
fn a_chunked_answer_is_read_to_its_last_line() {
    let mut s = Pieces(
        [
            &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n"[..],
            b"X-Trailer: 1\r\n",
            b"\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi",
            b"\r\n\r\nHTTP/1.1 204 No Content\r\n\r\n",
        ]
        .into_iter()
        .map(<[u8]>::to_vec)
        .collect(),
    );
    assert_eq!(read_response(&mut s, 100, false).unwrap().body, b"abc");
    assert_eq!(read_response(&mut s, 100, false).unwrap().body, b"hi");
    assert_eq!(read_response(&mut s, 100, false).unwrap().status, 204);
}

/// The path encoder before it wrote each character's UTF-8 bytes, as it was.
fn old_spaces_escaped(path: &str) -> String {
    path.chars()
        .flat_map(|c| if c == ' ' || c.is_control() { format!("%{:02X}", u32::from(c) & 0xff).chars().collect::<Vec<_>>() } else { vec![c] })
        .collect()
}

/// A control past ASCII is written as its UTF-8 bytes, not one byte of its number.
#[test]
fn a_paths_controls_are_written_as_their_utf8_bytes() {
    assert_eq!(Url::parse("http://h/a\u{85}b\u{9f}").unwrap().path, "/a%C2%85b%C2%9F");
    assert_eq!(old_spaces_escaped("/a\u{85}b"), "/a%85b", "one byte of the number, no character");
    for path in ["/", "/dav/cal/1.ics", "/a b/c\td", "/é €/x\r\ny", "/?q=a b&r=\u{7f}", "/\u{0}\u{1f}", "/日本 語"] {
        assert_eq!(Url::parse(&format!("https://h{path}")).unwrap().path, old_spaces_escaped(path), "{path:?}");
    }
}
