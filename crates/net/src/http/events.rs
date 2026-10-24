//! Answers that stay open and are read a slice at a time, so the caller can do other work in
//! between: server-sent events (`text/event-stream`, as JMAP push uses), and any body the server
//! sends in pieces as it has something to say (Exchange's streaming notifications).

use super::*;

/// Longest event (or piece of a streamed body not taken yet) accepted.
pub(crate) const MAX_EVENT: usize = 1 << 20;

/// One server-sent event (the type, `message` when the server gave none, and its data).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sse {
    pub event: String,
    pub data: String,
}

/// An answer's body as it comes off the connection: chunk framing removed, `\r` dropped.
pub struct BodyStream<S> {
    s: S,
    /// Bytes as they came off the connection.
    raw: Vec<u8>,
    /// The body decoded so far and not taken.
    text: Vec<u8>,
    chunked: bool,
    /// Bytes left in the current chunk, and whether its closing CRLF is still to come.
    left: usize,
    crlf: bool,
    /// The last chunk came: nothing more will.
    ended: bool,
}

impl<S: Read + stream::Timeouts> BodyStream<S> {
    /// A body on `s`, `rest` being what of it came with the headers.
    pub fn new(s: S, chunked: bool, rest: Vec<u8>) -> BodyStream<S> {
        BodyStream { s, raw: rest, text: Vec::new(), chunked, left: 0, crlf: false, ended: false }
    }

    /// What has come and not been taken (the caller drains what it understood).
    pub fn text(&mut self) -> &mut Vec<u8> {
        &mut self.text
    }

    /// Read what comes within `wait`; `false` when nothing did. Once what came is taken, the end
    /// of the body (or of the connection) is `NetError::Closed`.
    pub fn fill(&mut self, wait: Duration) -> Result<bool> {
        let before = self.text.len();
        self.decode()?;
        if self.text.len() > before {
            return Ok(true);
        }
        if self.ended {
            return Err(NetError::Closed("the stream ended".into()));
        }
        if self.text.len() > MAX_EVENT || self.raw.len() > MAX_EVENT {
            return Err(NetError::Protocol("an event is too long".into()));
        }
        let mut tmp = [0u8; 8 << 10];
        self.s.set_read_timeout(Some(wait.max(Duration::from_millis(1))))?;
        match self.s.read(&mut tmp) {
            Ok(0) => Err(NetError::Closed("the stream ended".into())),
            Ok(n) => {
                self.raw.extend_from_slice(tmp.get(..n).unwrap_or_default());
                self.decode()?;
                Ok(true)
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// Move what has arrived from `raw` to `text`, removing chunk framing.
    fn decode(&mut self) -> Result<()> {
        let take = |out: &mut Vec<u8>, bytes: &[u8]| out.extend(bytes.iter().copied().filter(|b| *b != b'\r'));
        if !self.chunked {
            let raw = std::mem::take(&mut self.raw);
            take(&mut self.text, &raw);
            return Ok(());
        }
        while !self.ended {
            if self.left > 0 {
                let n = self.left.min(self.raw.len());
                if n == 0 {
                    return Ok(());
                }
                let part: Vec<u8> = self.raw.drain(..n).collect();
                take(&mut self.text, &part);
                self.left -= n;
                self.crlf = self.left == 0;
                continue;
            }
            if self.crlf {
                if self.raw.len() < 2 {
                    return Ok(());
                }
                self.raw.drain(..2);
                self.crlf = false;
            }
            let Some(end) = self.raw.windows(2).position(|w| w == b"\r\n") else {
                if self.raw.len() > 1024 {
                    return Err(NetError::Protocol("bad chunk size line".into()));
                }
                return Ok(());
            };
            let line = String::from_utf8_lossy(self.raw.get(..end).unwrap_or_default()).to_string();
            let size =
                usize::from_str_radix(line.split(';').next().unwrap_or("").trim(), 16).map_err(|_| NetError::Protocol("bad chunk size".into()))?;
            self.raw.drain(..end + 2);
            if size > MAX_EVENT {
                return Err(NetError::Protocol("an event is too long".into()));
            }
            self.ended = size == 0;
            self.left = size;
        }
        Ok(())
    }
}

/// An answer that stays open (see [`Conn::stream`]): its status and headers, and its body to
/// read as it comes.
pub struct Streamed {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: BodyStream<Stream>,
}

/// A response that stays open and carries server-sent events, read an event at a time.
pub struct EventStream<S> {
    body: BodyStream<S>,
}

/// Open an event stream: a GET that must answer 200 with `text/event-stream`.
pub fn open_events(url: &str, headers: &[(&str, &str)]) -> Result<EventStream<Stream>> {
    let u = Url::parse(url)?;
    let sock = stream::tcp(&u.host, u.port)?;
    let mut s = if u.https { stream::tls(sock, &u.host)? } else { Stream::Plain(sock) };
    let mut req = Request::get(url, MAX_EVENT).header("Accept", "text/event-stream").header("Cache-Control", "no-cache");
    for (k, v) in headers {
        req = req.header(k, v);
    }
    write_request(&mut s, &req, &u, false)?;
    let (status, hs, rest) = read_head(&mut s)?;
    let get = |n: &str| hs.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.to_ascii_lowercase());
    match status {
        200 => {}
        401 | 403 => return Err(NetError::Auth("the server refused the user name or password".into())),
        s => return Err(NetError::Protocol(format!("the event stream answered HTTP {s}"))),
    }
    if !get("content-type").is_some_and(|t| t.contains("text/event-stream")) {
        return Err(NetError::Protocol("not an event stream".into()));
    }
    let chunked = get("transfer-encoding").is_some_and(|t| t.contains("chunked"));
    Ok(EventStream { body: BodyStream::new(s, chunked, rest) })
}

impl<S: Read + stream::Timeouts> EventStream<S> {
    pub fn new(s: S, chunked: bool) -> EventStream<S> {
        EventStream { body: BodyStream::new(s, chunked, Vec::new()) }
    }

    /// The next event, waiting at most `wait` (`None`: nothing yet).
    pub fn next(&mut self, wait: Duration) -> Result<Option<Sse>> {
        loop {
            if let Some(e) = self.take() {
                return Ok(Some(e));
            }
            if !self.body.fill(wait)? {
                return Ok(None);
            }
        }
    }

    /// One complete event from what came (events end with a blank line; `:` lines are comments).
    fn take(&mut self) -> Option<Sse> {
        let text = self.body.text();
        loop {
            let end = text.windows(2).position(|w| w == b"\n\n")?;
            let block = String::from_utf8_lossy(text.get(..end).unwrap_or_default()).to_string();
            text.drain(..end + 2);
            let mut event = String::new();
            let mut data: Vec<&str> = Vec::new();
            for line in block.lines() {
                let (field, value) = line.split_once(':').map_or((line, ""), |(f, v)| (f, v.strip_prefix(' ').unwrap_or(v)));
                match field {
                    "event" => event = value.to_string(),
                    "data" => data.push(value),
                    _ => {}
                }
            }
            if !data.is_empty() || !event.is_empty() {
                return Some(Sse { event: if event.is_empty() { "message".into() } else { event }, data: data.join("\n") });
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Hands out bytes a few at a time, then "times out".
    pub(crate) struct Drip(pub Vec<u8>, pub usize);

    impl Read for Drip {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.0.is_empty() {
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "nothing yet"));
            }
            let n = self.1.min(self.0.len()).min(buf.len());
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0.drain(..n);
            Ok(n)
        }
    }

    impl crate::stream::Timeouts for Drip {
        fn set_read_timeout(&mut self, _: Option<std::time::Duration>) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn event_streams() {
        let body = ": hello\n\nevent: state\r\ndata: {\"a\":\r\ndata: 1}\r\n\r\ndata: plain\n\n";
        // Chunked, split at awkward places.
        let mut chunked = Vec::new();
        for part in [&body[..5], &body[5..23], &body[23..]] {
            chunked.extend(format!("{:x};ext=1\r\n{part}\r\n", part.len()).as_bytes());
        }
        for step in [1, 3, 64] {
            let mut es = EventStream::new(Drip(chunked.clone(), step), true);
            let mut got = Vec::new();
            for _ in 0..200 {
                if let Some(e) = es.next(std::time::Duration::from_millis(1)).unwrap() {
                    got.push(e);
                }
            }
            assert_eq!(
                got,
                vec![Sse { event: "state".into(), data: "{\"a\":\n1}".into() }, Sse { event: "message".into(), data: "plain".into() }],
                "step {step}"
            );
        }
        let mut es = EventStream::new(Drip(body.as_bytes().to_vec(), 7), false);
        let first = (0..50).find_map(|_| es.next(std::time::Duration::from_millis(1)).unwrap());
        assert_eq!(first.map(|e| e.event), Some("state".into()), "plain (not chunked) too");
        // The end of the stream, and hostile input.
        assert!(matches!(EventStream::new(Drip(b"0\r\n\r\n".to_vec(), 9), true).next(std::time::Duration::ZERO), Err(NetError::Closed(_))));
        assert!(EventStream::new(Drip(b"zz\r\n".to_vec(), 9), true).next(std::time::Duration::ZERO).is_err());
        let endless = vec![b'x'; MAX_EVENT + 10];
        let mut es = EventStream::new(Drip(endless, 1 << 16), false);
        assert!((0..100).any(|_| es.next(std::time::Duration::ZERO).is_err()), "an event without an end is refused");
    }

    /// What came just before the last chunk is still had: the stream's end is told after it.
    #[test]
    fn the_last_piece_comes_before_the_end() {
        let mut es = EventStream::new(Drip(b"a\r\nevent: x\n\n\r\n0\r\n\r\n".to_vec(), 64), true);
        assert_eq!(es.next(std::time::Duration::ZERO).unwrap().map(|e| e.event), Some("x".into()));
        assert!(matches!(es.next(std::time::Duration::ZERO), Err(NetError::Closed(_))));
    }
}
