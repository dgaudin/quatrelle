//! A small HTTP/1.1 client (RFC 9112) for the few things mail needs from the web: pictures in a
//! message once the user asks for them, account autoconfiguration, OAuth token requests and JMAP.
//!
//! One request per connection (`Connection: close`), or several on a [`Conn`] kept open (NTLM
//! signs in a connection, not a request); TLS through the same rustls setup as IMAP,
//! `Content-Length` and `chunked` bodies, a handful of redirects. Responses are hostile input:
//! header and body sizes are capped and every number is checked.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use crate::stream::{self, Stream};
use crate::{NetError, Result};

mod events;
mod streamed;
pub use events::{BodyStream, EventStream, Sse, Streamed, open_events};
pub use streamed::{exchange_into, get_into};

/// Longest header block accepted.
const MAX_HEAD: usize = 64 << 10;
const MAX_REDIRECTS: usize = 5;

/// A parsed `http(s)://host[:port]/path?query` URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub https: bool,
    pub host: String,
    pub port: u16,
    /// Path and query, starting with `/`.
    pub path: String,
}

impl Url {
    pub fn parse(s: &str) -> Result<Url> {
        let s = s.trim();
        let bad = || NetError::Protocol(format!("not a web address: {}", s.chars().take(80).collect::<String>()));
        let (https, rest) = if let Some(r) = strip_prefix_ci(s, "https://") {
            (true, r)
        } else if let Some(r) = strip_prefix_ci(s, "http://") {
            (false, r)
        } else {
            return Err(bad());
        };
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        if authority.is_empty() || authority.contains('@') || authority.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(bad());
        }
        let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
            let (h, after) = v6.split_once(']').ok_or_else(bad)?;
            let port = match after.strip_prefix(':') {
                Some(p) => p.parse().map_err(|_| bad())?,
                None if after.is_empty() => default_port(https),
                None => return Err(bad()),
            };
            (h.to_string(), port)
        } else {
            match authority.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), p.parse().map_err(|_| bad())?),
                None => (authority.to_string(), default_port(https)),
            }
        };
        if host.is_empty() || port == 0 {
            return Err(bad());
        }
        let tail = tail.split('#').next().unwrap_or("");
        let path = if tail.is_empty() {
            "/".to_string()
        } else if tail.starts_with('?') {
            format!("/{tail}")
        } else {
            tail.to_string()
        };
        // Spaces and controls are percent-encoded rather than sent raw.
        let path: String = path
            .chars()
            .flat_map(|c| if c == ' ' || c.is_control() { format!("%{:02X}", u32::from(c) & 0xff).chars().collect::<Vec<_>>() } else { vec![c] })
            .collect();
        Ok(Url { https, host: host.to_ascii_lowercase(), port, path })
    }

    /// `Location` resolved against this URL.
    pub fn join(&self, location: &str) -> Result<Url> {
        let l = location.trim();
        if strip_prefix_ci(l, "http://").is_some() || strip_prefix_ci(l, "https://").is_some() {
            return Url::parse(l);
        }
        let origin = self.origin();
        if let Some(rest) = l.strip_prefix("//") {
            return Url::parse(&format!("{}://{rest}", if self.https { "https" } else { "http" }));
        }
        if l.starts_with('/') {
            return Url::parse(&format!("{origin}{l}"));
        }
        let dir = self.path.split('?').next().unwrap_or("/").rsplit_once('/').map_or("", |(d, _)| d);
        Url::parse(&format!("{origin}{dir}/{l}"))
    }

    pub fn origin(&self) -> String {
        let scheme = if self.https { "https" } else { "http" };
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        if self.port == default_port(self.https) { format!("{scheme}://{host}") } else { format!("{scheme}://{host}:{}", self.port) }
    }
}

fn default_port(https: bool) -> u16 {
    if https { 443 } else { 80 }
}

fn strip_prefix_ci<'a>(s: &'a str, p: &str) -> Option<&'a str> {
    s.get(..p.len()).filter(|h| h.eq_ignore_ascii_case(p)).and_then(|_| s.get(p.len()..))
}

/// A response.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// When its status line and headers were in: the server had answered, and the body came
    /// after (what a slow request spent waiting for the server, told from what it spent receiving).
    pub head_at: Option<Instant>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// A request.
#[derive(Clone, Debug, Default)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Largest body accepted.
    pub max: usize,
}

impl Request {
    pub fn get(url: &str, max: usize) -> Request {
        Request { method: "GET".into(), url: url.into(), headers: Vec::new(), body: Vec::new(), max }
    }

    pub fn post(url: &str, content_type: &str, body: Vec<u8>, max: usize) -> Request {
        Request { method: "POST".into(), url: url.into(), headers: vec![("Content-Type".into(), content_type.into())], body, max }
    }

    pub fn header(mut self, name: &str, value: &str) -> Request {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// Send a request; GET and HEAD follow redirects.
pub fn send(req: &Request) -> Result<Response> {
    send_to(req, false)
}

/// GET from the internet only ([`stream::tcp_public`]), every redirect included: what a message
/// asks for (its pictures) never reaches this computer or its network.
pub fn get_public(url: &str, max: usize) -> Result<Response> {
    send_to(&Request::get(url, max), true)
}

/// Send a request to the internet only, as [`get_public`]: an address a message or a certificate
/// names never reaches this computer or its network.
pub fn send_public(req: &Request) -> Result<Response> {
    send_to(req, true)
}

fn send_to(req: &Request, public_only: bool) -> Result<Response> {
    let mut url = Url::parse(&req.url)?;
    for _ in 0..=MAX_REDIRECTS {
        let resp = once(req, &url, public_only)?;
        let follow = matches!(resp.status, 301 | 302 | 303 | 307 | 308) && (req.method == "GET" || req.method == "HEAD");
        match resp.header("Location").filter(|_| follow) {
            Some(loc) => {
                let next = url.join(loc)?;
                // Never from HTTPS down to plain HTTP.
                if url.https && !next.https {
                    return Err(NetError::Tls("redirected from https to http".into()));
                }
                url = next;
            }
            None => return Ok(resp),
        }
    }
    Err(NetError::Protocol("too many redirects".into()))
}

pub fn get(url: &str, max: usize) -> Result<Response> {
    send(&Request::get(url, max))
}

fn once(req: &Request, url: &Url, public_only: bool) -> Result<Response> {
    let sock = if public_only { stream::tcp_public(&url.host, url.port)? } else { stream::tcp(&url.host, url.port)? };
    let mut s = if url.https { stream::tls(sock, &url.host)? } else { Stream::Plain(sock) };
    exchange(&mut s, req, url)
}

/// Write the request and read the response on an open connection.
pub fn exchange<S: Read + Write>(s: &mut S, req: &Request, url: &Url) -> Result<Response> {
    write_request(s, req, url, false)?;
    read_response(s, req.max, req.method == "HEAD")
}

/// How long a kept connection may sit unused and still be sent a request. Servers close idle
/// connections on their side (IIS after 120 s, proxies in front of it often sooner) without
/// telling: a request sent on one is answered by a reset or nothing at all.
pub const KEEP_IDLE: Duration = Duration::from_secs(45);

/// A connection kept open for several requests to one server (no redirects followed).
pub struct Conn {
    s: Stream,
    url: Url,
    /// The server said it closes the connection after the last answer.
    closing: bool,
    /// When the last answer was read (or the connection opened).
    last: Instant,
    /// Answers read on it so far.
    answered: u32,
    /// The last request failed the way a connection the server had already closed fails: on a
    /// reused connection, before a single byte of the answer. It never reached the server, so it
    /// can be sent again on a new connection.
    unanswered: bool,
}

impl Conn {
    pub fn open(url: &str) -> Result<Conn> {
        let url = Url::parse(url)?;
        let sock = stream::tcp(&url.host, url.port)?;
        let s = if url.https { stream::tls(sock, &url.host)? } else { Stream::Plain(sock) };
        Ok(Conn { s, url, closing: false, last: Instant::now(), answered: 0, unanswered: false })
    }

    /// Whether this connection reaches `url`'s server.
    pub fn reaches(&self, url: &str) -> bool {
        Url::parse(url).is_ok_and(|u| u.https == self.url.https && u.host.eq_ignore_ascii_case(&self.url.host) && u.port == self.url.port)
    }

    /// Whether another request can go on it: not closed, and not idle for so long the server
    /// has probably closed it on its side.
    pub fn usable(&self) -> bool {
        !self.closing && self.last.elapsed() < KEEP_IDLE
    }

    /// Whether the last request failed on a connection the server had closed, before it was
    /// read: sending it again on a new connection is safe.
    pub fn failed_unanswered(&self) -> bool {
        self.unanswered
    }

    /// Send a request to this server and read its answer.
    pub fn send(&mut self, req: &Request) -> Result<Response> {
        if self.closing {
            return Err(NetError::Closed("the server closed the connection".into()));
        }
        let url = Url::parse(&req.url)?;
        if !self.reaches(&req.url) {
            return Err(NetError::Protocol("a request for another server".into()));
        }
        // Whatever fails here leaves the connection half-read: the answer that didn't come (a
        // timeout, a reset) would be read as the next request's, and a stale 401 as a refused
        // password. It's dropped rather than reused — assumed closed until a full answer is in.
        self.closing = true;
        self.unanswered = false;
        let mut s = Counted { s: &mut self.s, read: 0 };
        let sent = write_request(&mut s, req, &url, true).and_then(|()| read_response(&mut s, req.max, req.method == "HEAD"));
        let nothing_read = s.read == 0;
        let resp = match sent {
            Ok(resp) => resp,
            Err(e) => {
                self.unanswered = self.answered > 0 && nothing_read && closed_unread(&e);
                return Err(timed_out(e));
            }
        };
        self.answered = self.answered.saturating_add(1);
        self.last = Instant::now();
        self.closing = resp.header("Connection").is_some_and(|c| c.eq_ignore_ascii_case("close"))
            || (resp.header("Content-Length").is_none() && resp.header("Transfer-Encoding").is_none() && resp.status != 204 && resp.status != 304);
        Ok(resp)
    }

    /// Send a request whose answer stays open (notifications sent as they happen), giving the
    /// connection up to it: its status and headers, and its body to read as it comes.
    pub fn stream(mut self, req: &Request) -> Result<Streamed> {
        if self.closing {
            return Err(NetError::Closed("the server closed the connection".into()));
        }
        let url = Url::parse(&req.url)?;
        if !self.reaches(&req.url) {
            return Err(NetError::Protocol("a request for another server".into()));
        }
        write_request(&mut self.s, req, &url, true).map_err(timed_out)?;
        let (status, headers, rest) = read_head(&mut self.s).map_err(timed_out)?;
        let chunked = headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked"));
        Ok(Streamed { status, headers, body: BodyStream::new(self.s, chunked, rest) })
    }
}

/// A stream that counts the bytes read from it.
struct Counted<'a, S> {
    s: &'a mut S,
    read: usize,
}

impl<S: Read> Read for Counted<'_, S> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.s.read(buf)?;
        self.read = self.read.saturating_add(n);
        Ok(n)
    }
}

impl<S: Write> Write for Counted<'_, S> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.s.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.s.flush()
    }
}

/// Whether an error is how a connection the server already closed fails: a reset, a broken
/// pipe, or the end of the stream. A timeout isn't: the server may be working on the request.
fn closed_unread(e: &NetError) -> bool {
    use std::io::ErrorKind as K;
    match e {
        NetError::Io(e) => matches!(e.kind(), K::BrokenPipe | K::ConnectionReset | K::ConnectionAborted | K::UnexpectedEof | K::NotConnected),
        NetError::Closed(_) => true,
        _ => false,
    }
}

/// A read that timed out says so ("Resource temporarily unavailable (os error 11)" didn't).
fn timed_out(e: NetError) -> NetError {
    match e {
        NetError::Io(io) if matches!(io.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => NetError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("the server didn't answer within {} s", stream::IO_TIMEOUT.as_secs()),
        )),
        e => e,
    }
}

fn write_request<S: Write>(s: &mut S, req: &Request, url: &Url, keep: bool) -> Result<()> {
    let host = if url.port == default_port(url.https) { url.host.clone() } else { format!("{}:{}", url.host, url.port) };
    let mut head = format!("{} {} HTTP/1.1\r\nHost: {host}\r\nConnection: {}\r\n", req.method, url.path, if keep { "keep-alive" } else { "close" });
    // Quatrelle's own name, unless the request gives another (an account's own User-Agent).
    if !req.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("User-Agent")) {
        head.push_str(&format!("User-Agent: Quatrelle/{}\r\n", env!("CARGO_PKG_VERSION")));
    }
    if !req.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("Accept")) {
        head.push_str("Accept: */*\r\n");
    }
    for (k, v) in &req.headers {
        if k.contains(['\r', '\n', ':']) || v.contains(['\r', '\n']) {
            return Err(NetError::Protocol("bad request header".into()));
        }
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if !req.body.is_empty() || req.method == "POST" {
        head.push_str(&format!("Content-Length: {}\r\n", req.body.len()));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes())?;
    s.write_all(&req.body)?;
    s.flush()?;
    Ok(())
}

/// The status, headers and whatever of the body came with them.
#[allow(clippy::type_complexity)]
fn read_head<S: Read>(s: &mut S) -> Result<(u16, Vec<(String, String)>, Vec<u8>)> {
    let mut buf: Vec<u8> = Vec::with_capacity(8 << 10);
    let mut tmp = [0u8; 16 << 10];
    let head_end = loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
        if buf.len() > MAX_HEAD {
            return Err(NetError::Protocol("response headers too long".into()));
        }
        let n = s.read(&mut tmp)?;
        if n == 0 {
            return Err(NetError::Closed("the server closed the connection before answering".into()));
        }
        buf.extend_from_slice(tmp.get(..n).unwrap_or_default());
        // Empty lines before the status line are skipped (RFC 9112 §2.2): the end of an earlier
        // answer's chunked body, left on a kept connection, read as "bad status line" here.
        let blank = buf.iter().take_while(|b| matches!(b, b'\r' | b'\n')).count();
        if blank > 0 {
            buf.drain(..blank);
        }
    };
    let head = String::from_utf8_lossy(buf.get(..head_end).unwrap_or_default()).to_string();
    let mut rest: Vec<u8> = buf.get(head_end + 4..).map(<[u8]>::to_vec).unwrap_or_default();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let mut parts = status_line.split_whitespace();
    let version = parts.next().unwrap_or("");
    let status: u16 = parts
        .next()
        .and_then(|c| c.parse().ok())
        .filter(|c| (100..600).contains(c))
        .ok_or_else(|| NetError::Protocol(format!("bad status line: {}", status_line.chars().take(60).collect::<String>())))?;
    if !version.starts_with("HTTP/1.") {
        return Err(NetError::Protocol(format!("not HTTP/1.x: {version}")));
    }
    let headers: Vec<(String, String)> =
        lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).take(200).collect();
    Ok((status, headers, std::mem::take(&mut rest)))
}

fn read_response<S: Read>(s: &mut S, max: usize, head_only: bool) -> Result<Response> {
    let mut tmp = [0u8; 16 << 10];
    let (status, headers, mut rest) = read_head(s)?;
    let head_at = Some(Instant::now());
    let resp = |body| Response { status, headers: headers.clone(), body, head_at };
    let get = |n: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.to_ascii_lowercase());
    if head_only || status == 204 || status == 304 || (100..200).contains(&status) {
        return Ok(resp(Vec::new()));
    }
    let too_big = || NetError::Protocol(format!("the response is larger than {} KB", max / 1024));
    let mut fill = |rest: &mut Vec<u8>, want: usize| -> Result<bool> {
        while rest.len() < want {
            let n = s.read(&mut tmp)?;
            if n == 0 {
                return Ok(false);
            }
            rest.extend_from_slice(tmp.get(..n).unwrap_or_default());
            if rest.len() > max.saturating_add(MAX_HEAD) {
                return Err(too_big());
            }
        }
        Ok(true)
    };
    if get("transfer-encoding").is_some_and(|t| t.contains("chunked")) {
        let mut body = Vec::new();
        let mut pos = 0usize;
        loop {
            // A size line.
            let line_end = loop {
                if let Some(p) = rest.get(pos..).and_then(|r| r.windows(2).position(|w| w == b"\r\n")) {
                    break pos + p;
                }
                if rest.len() - pos > 1024 {
                    return Err(NetError::Protocol("bad chunk size line".into()));
                }
                let want = rest.len() + 1;
                if !fill(&mut rest, want)? {
                    return Err(NetError::Closed("the response ended early".into()));
                }
            };
            let size_text = String::from_utf8_lossy(rest.get(pos..line_end).unwrap_or_default()).to_string();
            let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
                .map_err(|_| NetError::Protocol("bad chunk size".into()))?;
            pos = line_end + 2;
            if size == 0 {
                // The trailer: header lines (ignored), then an empty line, which belongs to this
                // answer — left unread on a kept connection, it began the next one.
                for _ in 0..64 {
                    let end = loop {
                        if let Some(p) = rest.get(pos..).and_then(|r| r.windows(2).position(|w| w == b"\r\n")) {
                            break Some(pos + p);
                        }
                        if rest.len() - pos > MAX_HEAD {
                            return Err(NetError::Protocol("bad chunked trailer".into()));
                        }
                        let want = rest.len() + 1;
                        if !fill(&mut rest, want)? {
                            break None;
                        }
                    };
                    let Some(end) = end else { break };
                    let empty = end == pos;
                    pos = end + 2;
                    if empty {
                        break;
                    }
                }
                return Ok(resp(body));
            }
            if body.len().saturating_add(size) > max {
                return Err(too_big());
            }
            let want = pos.saturating_add(size).saturating_add(2);
            if !fill(&mut rest, want)? {
                return Err(NetError::Closed("the response ended early".into()));
            }
            body.extend_from_slice(rest.get(pos..pos + size).unwrap_or_default());
            pos += size + 2;
        }
    }
    if let Some(len) = get("content-length") {
        let len: usize = len.trim().parse().map_err(|_| NetError::Protocol("bad Content-Length".into()))?;
        if len > max {
            return Err(too_big());
        }
        if !fill(&mut rest, len)? {
            return Err(NetError::Closed("the response ended early".into()));
        }
        rest.truncate(len);
        return Ok(resp(rest));
    }
    // No length: the body runs to the end of the connection.
    fill(&mut rest, usize::MAX)?;
    if rest.len() > max {
        return Err(too_big());
    }
    Ok(resp(rest))
}

/// `application/x-www-form-urlencoded` (for OAuth token requests).
pub fn form(pairs: &[(&str, &str)]) -> Vec<u8> {
    pairs.iter().map(|(k, v)| format!("{}={}", pct(k), pct(v))).collect::<Vec<_>>().join("&").into_bytes()
}

/// Percent-encode everything but the unreserved characters (RFC 3986 §2.3).
pub fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    use super::*;

    /// Serve `responses` (one per connection, in order) on localhost; returns the base URL and
    /// the requests received.
    pub(crate) fn serve(responses: Vec<Vec<u8>>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
        let h = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for r in responses {
                let (mut s, _) = l.accept().unwrap();
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
        });
        (base, h)
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
}
