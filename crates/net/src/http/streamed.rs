//! A GET whose body goes to a writer as it comes, for a download too big to hold in memory (an
//! update's installer): the answer's head first, so the caller can choose where the body goes
//! (a partial file continued, or started again), then its bytes. Hostile input as every answer
//! is: the head and the body are capped, and chunk sizes checked.

use super::*;

/// Send `req` (a GET, following redirects as [`send`] does) and hand the final answer's status
/// and headers to `open`, which gives the writer for its body, or `None` to leave the body
/// unread. At most `req.max` bytes of body are accepted. The answer comes back without its body;
/// a body cut short is an error, after what came of it was written.
pub fn get_into<W: Write>(req: &Request, open: impl FnOnce(&Response) -> Result<Option<W>>) -> Result<Response> {
    let mut url = Url::parse(&req.url)?;
    let mut chain = Chain::new(req, &url);
    let mut open = Some(open);
    for _ in 0..=MAX_REDIRECTS {
        let sock = stream::tcp(&url.host, url.port)?;
        let mut s = if url.https { stream::tls(sock, &url.host)? } else { Stream::Plain(sock) };
        let head = exchange_into(&mut s, chain.current(), &url, |head| open.take().map_or(Ok(None), |open| open(head)))?;
        match redirect(&head) {
            Some(loc) => {
                let next = url.join(loc)?;
                if url.https && !next.https {
                    return Err(NetError::Tls("redirected from https to http".into()));
                }
                chain.follow(&next);
                url = next;
            }
            None => return Ok(Response { url: Some(format!("{}{}", url.origin(), url.path)), ..head }),
        }
    }
    Err(NetError::Protocol("too many redirects".into()))
}

/// Write the request on an open connection and read the answer's head; its body goes to the
/// writer `open` gives, as [`get_into`] says. A redirect's body is left unread (`open` isn't
/// asked): the caller follows it on a connection of its own.
pub fn exchange_into<S: Read + Write, W: Write>(
    s: &mut S,
    req: &Request,
    url: &Url,
    open: impl FnOnce(&Response) -> Result<Option<W>>,
) -> Result<Response> {
    write_request(s, req, url, false)?;
    let (status, headers, rest) = read_head(s)?;
    let head = Response { status, headers, body: Vec::new(), head_at: None, url: None };
    if redirect(&head).is_some() {
        return Ok(head);
    }
    if let Some(mut out) = open(&head)? {
        let mut body = Ahead { s, buf: rest, pos: 0 };
        copy_body(&mut body, &head, u64::try_from(req.max).unwrap_or(u64::MAX), &mut out)?;
        out.flush()?;
    }
    Ok(head)
}

/// Where a redirect sends a GET, if `head` is one.
fn redirect(head: &Response) -> Option<&str> {
    head.header("Location").filter(|_| matches!(head.status, 301 | 302 | 303 | 307 | 308))
}

/// The body of `head`'s answer, at most `max` bytes, into `out`: by its length, its chunks, or
/// to the end of the connection.
fn copy_body<S: Read, W: Write>(body: &mut Ahead<'_, S>, head: &Response, max: u64, out: &mut W) -> Result<u64> {
    let too_big = || NetError::Protocol(format!("the download is larger than {} MB", max >> 20));
    let ended = || NetError::Closed("the download ended early".into());
    if head.header("Transfer-Encoding").is_some_and(|t| t.to_ascii_lowercase().contains("chunked")) {
        let mut done = 0u64;
        loop {
            let line = body.line()?;
            let size =
                u64::from_str_radix(line.split(';').next().unwrap_or("").trim(), 16).map_err(|_| NetError::Protocol("bad chunk size".into()))?;
            if size == 0 {
                return Ok(done);
            }
            if done.saturating_add(size) > max {
                return Err(too_big());
            }
            if body.copy(size, out)? < size {
                return Err(ended());
            }
            done += size;
            if !body.line()?.is_empty() {
                return Err(NetError::Protocol("bad chunk end".into()));
            }
        }
    }
    if let Some(len) = head.header("Content-Length") {
        let len: u64 = len.trim().parse().map_err(|_| NetError::Protocol("bad Content-Length".into()))?;
        if len > max {
            return Err(too_big());
        }
        return if body.copy(len, out)? == len { Ok(len) } else { Err(ended()) };
    }
    // No length: the body runs to the end of the connection.
    let done = body.copy(max.saturating_add(1), out)?;
    if done > max { Err(too_big()) } else { Ok(done) }
}

/// What came with the head, then the connection.
struct Ahead<'a, S> {
    s: &'a mut S,
    buf: Vec<u8>,
    pos: usize,
}

impl<S: Read> Ahead<'_, S> {
    /// More bytes from the connection; false at its end.
    fn more(&mut self) -> Result<bool> {
        if self.pos >= self.buf.len() {
            self.buf.clear();
            self.pos = 0;
        }
        let mut tmp = [0u8; 16 << 10];
        let n = self.s.read(&mut tmp)?;
        self.buf.extend_from_slice(tmp.get(..n).unwrap_or_default());
        Ok(n > 0)
    }

    /// Up to `n` bytes (fewer when the connection ends first) into `out`: how many.
    fn copy<W: Write>(&mut self, n: u64, out: &mut W) -> Result<u64> {
        let mut done = 0u64;
        while done < n {
            if self.pos >= self.buf.len() && !self.more()? {
                break;
            }
            let have = self.buf.len().saturating_sub(self.pos);
            let take = usize::try_from(n - done).map_or(have, |left| left.min(have));
            out.write_all(self.buf.get(self.pos..self.pos + take).unwrap_or_default())?;
            self.pos += take;
            done += take as u64;
        }
        Ok(done)
    }

    /// A line ended by CRLF (at most 1 KB), without its end.
    fn line(&mut self) -> Result<String> {
        loop {
            if let Some(p) = self.buf.get(self.pos..).and_then(|r| r.windows(2).position(|w| w == b"\r\n")) {
                let line = String::from_utf8_lossy(self.buf.get(self.pos..self.pos + p).unwrap_or_default()).into_owned();
                self.pos += p + 2;
                return Ok(line);
            }
            if self.buf.len().saturating_sub(self.pos) > 1024 {
                return Err(NetError::Protocol("bad chunk size line".into()));
            }
            if !self.more()? {
                return Err(NetError::Closed("the download ended early".into()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{bind, credentials_seen, serve, serve_on};
    use super::*;

    fn fetch(url: &str, max: usize) -> (Result<Response>, Vec<u8>) {
        let mut got = Vec::new();
        let r = get_into(&Request::get(url, max), |_| Ok(Some(&mut got)));
        (r, got)
    }

    /// Lengths, chunks split anywhere, to the end, redirects, and every way a body goes wrong.
    #[test]
    fn bodies_are_written_as_they_come() {
        let (base, h) = serve(vec![
            b"HTTP/1.1 302 Found\r\nLocation: /file\r\nContent-Length: 3\r\n\r\nnop".to_vec(),
            b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 2-6/7\r\nContent-Length: 5\r\n\r\nhello".to_vec(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5;ext=1\r\npedia\r\n0\r\n\r\n".to_vec(),
            b"HTTP/1.0 200 OK\r\n\r\nuntil the end".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 99999\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWikiXX0\r\n\r\n".to_vec(),
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nnot found".to_vec(),
        ]);
        let (r, got) = fetch(&format!("{base}/start"), 1000);
        let r = r.unwrap();
        assert_eq!((r.status, r.header("content-range"), got.as_slice()), (206, Some("bytes 2-6/7"), &b"hello"[..]));
        assert_eq!(fetch(&base, 1000).1, b"Wikipedia");
        assert_eq!(fetch(&base, 1000).1, b"until the end");
        let (r, got) = fetch(&base, 1000);
        assert!(matches!(r, Err(NetError::Closed(_))), "{r:?}");
        assert_eq!(got, b"short", "what came is kept, to carry on from");
        assert!(matches!(fetch(&base, 1000).0, Err(NetError::Protocol(m)) if m.contains("larger")));
        assert!(fetch(&base, 1000).0.is_err());
        assert!(fetch(&base, 1000).0.is_err(), "a chunk without its end");
        // The caller can look at the answer and leave its body.
        let r = get_into::<Vec<u8>>(&Request::get(&base, 1000), |head| Ok((head.status == 200).then(Vec::new))).unwrap();
        assert_eq!(r.status, 404);
        let seen = h.join().unwrap();
        assert!(seen[1].starts_with("GET /file "), "redirect followed: {}", seen[1]);
    }

    fn fetch_with_credentials(url: &str) -> Vec<u8> {
        let mut got = Vec::new();
        let req = Request::get(url, 1000).header("Authorization", "Bearer x").header("cookie", "s=1");
        get_into(&req, |_| Ok(Some(&mut got))).unwrap();
        got
    }

    /// As [`send`]: the token and the cookie go with a redirect within the site, and with none
    /// that leaves it (here `localhost`, the same server under another site's name), nor with the redirect back to the first site after that.
    #[test]
    fn a_redirect_to_another_site_carries_no_credentials() {
        let ok = || b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec();
        let (a, ha) = serve(vec![b"HTTP/1.1 302 Found\r\nLocation: /two\r\n\r\n".to_vec(), ok()]);
        assert_eq!(fetch_with_credentials(&format!("{a}/one")), b"ok");
        let seen = ha.join().unwrap();
        assert!(seen.iter().all(|r| credentials_seen(r) == (true, true)), "{seen:?}");
        let (b, hb) = serve(vec![ok()]);
        let b = b.replace("127.0.0.1", "localhost");
        let (a, ha) = serve(vec![format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {b}/file\r\n\r\n").into_bytes()]);
        assert_eq!(fetch_with_credentials(&format!("{a}/one")), b"ok");
        assert_eq!(credentials_seen(&ha.join().unwrap()[0]), (true, true));
        assert_eq!(credentials_seen(&hb.join().unwrap()[0]), (false, false));
        let ((la, a), (lb, b)) = (bind(), bind());
        let b = b.replace("127.0.0.1", "localhost");
        let ha = serve_on(la, vec![format!("HTTP/1.1 302 Found\r\nLocation: {b}/two\r\n\r\n").into_bytes(), ok()]);
        let hb = serve_on(lb, vec![format!("HTTP/1.1 302 Found\r\nLocation: {a}/three\r\n\r\n").into_bytes()]);
        assert_eq!(fetch_with_credentials(&format!("{a}/one")), b"ok");
        let (seen_a, seen_b) = (ha.join().unwrap(), hb.join().unwrap());
        assert_eq!(credentials_seen(&seen_a[0]), (true, true));
        assert_eq!(credentials_seen(&seen_b[0]), (false, false));
        assert!(seen_a[1].starts_with("GET /three "), "{}", seen_a[1]);
        assert_eq!(credentials_seen(&seen_a[1]), (false, false), "back on the first site, still none: {}", seen_a[1]);
    }
}
