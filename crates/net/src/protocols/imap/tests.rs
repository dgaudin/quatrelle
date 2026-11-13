// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
use std::io;

use super::*;

/// A canned server: reads are served from `input`; writes are recorded.
pub(crate) struct Script {
    input: io::Cursor<Vec<u8>>,
    pub written: Vec<u8>,
}

impl Script {
    pub(crate) fn new(s: &[u8]) -> Self {
        Script { input: io::Cursor::new(s.to_vec()), written: Vec::new() }
    }
}

impl Read for Script {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Deliver a few bytes at a time to exercise framing across reads.
        let n = buf.len().min(7);
        self.input.read(&mut buf[..n])
    }
}

impl Write for Script {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.written.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn sets_and_dates() {
    assert_eq!(uid_set(&[5, 1, 2, 3, 9, 10, 3, 0]), "1:3,5,9:10");
    assert_eq!(uid_set(&[]), "");
    assert_eq!(uid_set(&[u32::MAX, u32::MAX - 1]), format!("{}:{}", u32::MAX - 1, u32::MAX));
    assert_eq!(parse_internal_date("14-Oct-2026 09:30:00 +0200"), Some(1_791_963_000));
    assert_eq!(parse_internal_date(" 4-Jan-1970 00:00:00 -0100"), Some(3 * 86_400 + 3600));
    assert_eq!(parse_internal_date("junk"), None);
    assert_eq!(parse_internal_date("99-Oct-2026 09:30:00 +0200"), None);
    assert_eq!(base64(b"\0user\0pass"), "AHVzZXIAcGFzcw==");
    assert_eq!(base64(b"ab"), "YWI=");
    assert_eq!(quoted("a\"b\\c").as_deref(), Some("\"a\\\"b\\\\c\""));
    assert_eq!(quoted("pässword"), None);
}

#[test]
fn literals() {
    assert_eq!(literal_len(b"* 1 FETCH (BODY[] {12}"), Some(12));
    assert_eq!(literal_len(b"A1 APPEND x {3+}"), Some(3));
    assert_eq!(literal_len(b"* OK {abc}"), None);
    assert_eq!(literal_len(b"* OK done"), None);
    assert_eq!(literal_len(b"{99999999999999999}"), None);
}

#[test]
fn greeting_login_fetch_with_literals() {
    let server = b"* OK [CAPABILITY IMAP4rev1 MOVE UIDPLUS] ready\r\n\
M1 OK [CAPABILITY IMAP4rev1 MOVE UIDPLUS CONDSTORE] logged in\r\n\
* 1 FETCH (UID 7 FLAGS (\\Seen) RFC822.SIZE 12 BODY[HEADER] {12}\r\nSubject: x\r\n)\r\n\
* 2 FETCH (FLAGS (\\Flagged))\r\n\
M2 OK done\r\n";
    let mut c = Client::from_stream(Script::new(server)).unwrap();
    assert!(c.has("move") && !c.has("CONDSTORE"));
    c.login("zoe@x", "pässword").unwrap_err(); // literal path needs a '+' the script lacks
}

#[test]
fn login_quoted_then_fetch() {
    let server = b"* OK hi\r\n\
M1 OK [CAPABILITY IMAP4rev1 MOVE UIDPLUS] logged in\r\n\
* 1 FETCH (UID 7 FLAGS (\\Seen) RFC822.SIZE 12 BODY[HEADER] {12}\r\nSubject: x\r\n)\r\n\
* 2 FETCH (FLAGS (\\Flagged))\r\n\
M2 OK done\r\n";
    let mut c = Client::from_stream(Script::new(server)).unwrap();
    c.login("zoe@x", "pw").unwrap();
    assert!(c.has("UIDPLUS"));
    let f = c.uid_fetch("7", "UID FLAGS RFC822.SIZE BODY.PEEK[HEADER]", "").unwrap();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].uid, 7);
    assert_eq!(f[0].header.as_deref(), Some(&b"Subject: x\r\n"[..]));
    assert_eq!(f[0].flags.as_deref(), Some(&["\\Seen".to_string()][..]));
    let written = String::from_utf8(c.conn.into_inner().written).unwrap();
    assert!(written.starts_with("M1 LOGIN \"zoe@x\" \"pw\"\r\nM2 UID FETCH 7 ("), "{written}");
}

#[test]
fn errors_and_hostile_input() {
    let mut c = Client::from_stream(Script::new(b"* OK hi\r\nM1 NO [AUTHENTICATIONFAILED] bad password\r\n")).unwrap();
    assert!(matches!(c.login("u", "p"), Err(NetError::Auth(m)) if m.contains("bad password")));
    assert!(matches!(Client::from_stream(Script::new(b"* BYE go away\r\n")), Err(NetError::Closed(_))));
    assert!(Client::from_stream(Script::new(b"")).is_err());
    assert!(Client::from_stream(Script::new(b"garbage\r\n")).is_err());
    let mut c = Client::from_stream(Script::new(b"* OK hi\r\n* 1 FETCH (BODY[] {999999999999}\r\n")).unwrap();
    assert!(matches!(c.uid_fetch("1", "BODY.PEEK[]", ""), Err(NetError::Protocol(_))));
    let mut c = Client::from_stream(Script::new(b"* OK hi\r\n* WEIRD stuff\r\nM1 OK fine\r\n")).unwrap();
    assert!(c.noop().is_ok());
    // Bare LF line ends are tolerated.
    let mut c = Client::from_stream(Script::new(b"* OK hi\n* SEARCH 3 1 2\nM1 OK done\n")).unwrap();
    assert_eq!(c.uid_search("ALL").unwrap(), vec![1, 2, 3]);
}

/// A response code followed straight by a non-ASCII character (found by `fuzz/`): imap-proto
/// sliced inside the character and panicked, so a server could crash the client.
#[test]
fn a_code_right_before_non_ascii_text_never_panics() {
    let c = Client::from_stream(Script::new(b"* OK [CAPABILITY IMAP4rev1]\xd7\n")).unwrap();
    assert!(c.has("IMAP4REV1"));
    let mut c = Client::from_stream(Script::new("* OK hi\r\nM1 NO [ALERT]é quota full\r\n".as_bytes())).unwrap();
    assert!(matches!(c.noop(), Err(NetError::No(m)) if m.contains("é quota full")), "the text is kept whole");
    let mut c = Client::from_stream(Script::new("* OK hi\r\n+ [X]ü\r\nM1 OK [APPENDUID 7 9]ö\r\n".as_bytes())).unwrap();
    assert_eq!(c.append("Drafts", "", b"x").unwrap(), Some(9));
    let mut c = Client::from_stream(Script::new(b"* OK hi\r\n+[UIDNEXT 9]\xff\nM1 OK done\r\n")).unwrap();
    assert!(c.append("Drafts", "", b"x").is_ok(), "a continuation without its space");
    // Inside a quoted string the bracket isn't a code: left as it was.
    assert_eq!(super::conn::space_after_code(b"* 1 FETCH (X \"[a]\xc3\xa9\")\r\n".to_vec()), b"* 1 FETCH (X \"[a]\xc3\xa9\")\r\n");
}

/// A connection the keepalive finds dead while IDLE waits ends the wait at once: Linux and macOS
/// say so as `ETIMEDOUT` (`TimedOut`), which isn't IDLE's own read timeout there (`WouldBlock`).
#[cfg(unix)]
#[test]
fn idle_ends_when_the_connection_is_found_dead() {
    /// The script, then the keepalive's verdict, then nothing more.
    struct Dying(Script, bool);
    impl Read for Dying {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.0.read(buf)? {
                0 if !self.1 => {
                    self.1 = true;
                    Err(io::Error::from(io::ErrorKind::TimedOut))
                }
                0 => Err(io::Error::from(io::ErrorKind::WouldBlock)),
                n => Ok(n),
            }
        }
    }
    impl Write for Dying {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Timeouts for Dying {
        fn set_read_timeout(&mut self, _: Option<Duration>) -> io::Result<()> {
            Ok(())
        }
    }
    let server = b"* OK hi\r\nM1 OK [CAPABILITY IMAP4rev1 IDLE] logged in\r\n+ idling\r\n";
    let mut c = Client::from_stream(Dying(Script::new(server), false)).unwrap();
    c.login("zoe@x", "pw").unwrap();
    let start = Instant::now();
    let r = c.idle(Duration::from_secs(2), || false);
    assert!(matches!(&r, Err(NetError::Io(e)) if e.kind() == io::ErrorKind::TimedOut), "{r:?}");
    assert!(start.elapsed() < Duration::from_millis(1500), "waited {:?}", start.elapsed());
}

/// A hostile server's lists nested a hundred thousand deep don't overflow the stack (they did,
/// in imap-proto's parser, from a thousand), even on a thread with Windows' 1 MB main stack;
/// real nesting parses.
#[test]
fn deeply_nested_lists_dont_overflow_the_stack() {
    let h = std::thread::Builder::new().stack_size(1 << 20).spawn(|| {
        let mut out = Vec::new();
        for depth in [30usize, conn::MAX_NESTING - 1, conn::MAX_NESTING + 1, 1000, 100_000] {
            let mut s = b"* OK ready\r\n* 1 FETCH (UID 1 BODYSTRUCTURE ".to_vec();
            s.extend(std::iter::repeat_n(b'(', depth));
            s.extend(b"\"TEXT\" \"PLAIN\" NIL NIL NIL \"7BIT\" 1 1");
            s.extend(std::iter::repeat_n(b')', depth));
            s.extend(b")\r\nM1 OK done\r\n");
            let mut c = Client::from_stream(Script::new(&s)).unwrap();
            out.push((depth, c.uid_fetch("1", "UID BODYSTRUCTURE", "").is_ok()));
        }
        out
    });
    let out = h.unwrap().join().expect("no overflow");
    assert!(out.iter().all(|(_, ok)| *ok), "{out:?}");
    // The count skips quoted strings and literals.
    assert_eq!(conn::nesting(b"* 1 FETCH (UID 1 BODY[] {5}\r\n(((((\r\n FLAGS (\"((\" \\Seen))"), 2);
}

/// RFC 3501's own example, with a zone; hostile ones are refused, never a panic.
#[test]
fn an_internal_date_reads_with_its_zone_and_hostile_ones_are_refused() {
    assert_eq!(parse_internal_date("17-Jul-1996 02:44:25 -0700"), Some(837_596_665));
    assert_eq!(parse_internal_date(" 4-Jan-1970 00:00:05 +0000"), Some(3 * 86_400 + 5));
    for bad in [
        "",
        "x",
        "17-Jul-99999999999999999999 02:44:25 -0700",
        "17-Jul-1996 02:44:25 -9999",
        "0-Jul-1996 02:44:25 +0000",
        "17-Jul-1996 02:44:25 \u{e9}\u{e9}",
    ] {
        assert_eq!(parse_internal_date(bad), None, "{bad}");
    }
    assert_eq!(parse_internal_date(&format!("17-Jul-1996 02:44:25 -0700{}", " ".repeat(10_000)).replace(' ', "x")), None);
}

proptest::proptest! {
    /// What the in-memory server writes reads back; any text at all is read or refused.
    #[test]
    fn an_internal_date_round_trips_and_junk_never_panics(t in 0i64..253_402_300_799, junk in "\\PC{0,80}") {
        let text = crate::fakes::fake::internal_date(t);
        proptest::prop_assert_eq!(parse_internal_date(&text), Some(t));
        let _ = parse_internal_date(&junk);
    }
}

/// A refusal that is passing (RFC 5530's `UNAVAILABLE`, `SERVERBUG`, `INUSE`, `LIMIT`, or Gmail's
/// `System Error` with no code) is for later; the others are the server's final word.
#[test]
fn a_passing_refusal_is_for_later_and_the_others_are_final() {
    let said = |line: &str| {
        let mut c = Client::from_stream(Script::new(format!("* OK hi\r\nM1 NO {line}\r\n").as_bytes())).unwrap();
        c.noop().unwrap_err()
    };
    for line in ["System Error (Failure)", "[UNAVAILABLE] try again", "[serverbug] oops", "[INUSE] busy", "[LIMIT] too many"] {
        assert!(matches!(said(line), NetError::Later(m) if m.contains(line)), "{line}: for later, with the server's words");
    }
    for line in [
        "[NONEXISTENT] no such folder",
        "[NOPERM] no",
        "[OVERQUOTA] full",
        "[TRYCREATE] make it",
        "[ALREADYEXISTS] there",
        "no reason",
        "[ALERT]é quota full",
    ] {
        assert!(matches!(said(line), NetError::No(_)), "{line}: final");
    }
    // A command the server finds malformed is not a passing refusal.
    let mut c = Client::from_stream(Script::new(b"* OK hi\r\nM1 BAD System Error\r\n")).unwrap();
    assert!(matches!(c.noop(), Err(NetError::Bad(_))));
}

/// Three answers the IMAP parsing library panicked on (0.17.0): a message's date that is `NIL`,
/// a `METADATA` value that isn't UTF-8, and a status code whose own text holds a `]`, closed
/// before a non-ASCII character. Each is a response not understood, and the command goes on to
/// its end.
#[test]
fn answers_the_parsing_library_panicked_on_are_not_understood() {
    let nil_date: &[u8] = b"* 1 FETCH (INTERNALDATE NIL)\r\n";
    let metadata: &[u8] = b"* METADATA \"INBOX\" (/private/comment \"\xff\")\r\n";
    let mailbox: &[u8] = b"* METADATA \"IN\xffBOX\" (/private/comment \"x\")\r\n";
    let badcharset = "* OK [BADCHARSET (a]b)]\u{e9}\r\n".as_bytes();
    let tagged = "M1 OK [BADCHARSET (a]b)]\u{e9}\r\n".as_bytes();
    for answer in [nil_date, metadata, mailbox, badcharset] {
        let server = [b"* OK hi\r\n", answer, b"* SEARCH 4 2\r\nM1 OK done\r\n"].concat();
        let mut c = Client::from_stream(Script::new(&server)).unwrap();
        assert_eq!(c.uid_search("ALL").unwrap(), vec![2, 4], "{}", String::from_utf8_lossy(answer));
    }
    // As the greeting (read before TLS on a STARTTLS account), and as a command's completion.
    assert!(Client::from_stream(Script::new(nil_date)).is_err() && Client::from_stream(Script::new(metadata)).is_err());
    assert!(Client::from_stream(Script::new(badcharset)).is_ok());
    let mut c = Client::from_stream(Script::new(&[b"* OK hi\r\n", tagged].concat())).unwrap();
    assert!(c.noop().is_ok(), "the code ends at its last bracket");
}

/// Told apart outside strings and literals only: a message that says `INTERNALDATE NIL` is read.
#[test]
fn what_the_parser_isnt_given() {
    use super::conn::beyond_the_parser;
    assert!(beyond_the_parser(b"* 1 FETCH (UID 4 internaldate nil)\r\n"));
    assert!(beyond_the_parser(b"* 1 FETCH (BODY[] {3}\r\nabc INTERNALDATE NIL)\r\n"));
    assert!(beyond_the_parser(b"* metadata INBOX (/shared/comment NIL)\r\n"));
    assert!(!beyond_the_parser(b"* 1 FETCH (UID 4 INTERNALDATE \"01-Jan-2024 10:00:00 +0100\")\r\n"));
    assert!(!beyond_the_parser(b"* 1 FETCH (BODY[] {16}\r\nINTERNALDATE NIL)\r\n"));
    assert!(!beyond_the_parser(b"* 1 FETCH (ENVELOPE (NIL \"INTERNALDATE NIL\" NIL))\r\n"));
    assert!(!beyond_the_parser(b"* OK metadata\r\n") && !beyond_the_parser(b""));
    let server = b"* OK hi\r\n* 1 FETCH (UID 1 BODY[] {16}\r\nINTERNALDATE NIL)\r\nM1 OK done\r\n";
    let mut c = Client::from_stream(Script::new(server)).unwrap();
    assert_eq!(c.uid_fetch("1", "BODY.PEEK[]", "").unwrap().len(), 1);
}

/// A quoted string the parser wouldn't take (a NUL or a line end in it, a `\` before anything
/// but `\` or `"`, or never closed) hides nothing: the parser reads on past it, so a `NIL` date
/// or a deep nesting after it is looked for in every byte (the fuzzer found the first).
#[test]
fn a_broken_string_hides_nothing() {
    use super::conn::{beyond_the_parser, parser_takes};
    assert!(beyond_the_parser(b"* 1 FETCH (INTERNALDATE 1 +\"FI\x000 FETCH (INTERNALDATE NIL)\r\n"));
    assert!(beyond_the_parser(b"* 1 FETCH (X \"a INTERNALDATE NIL)\r\n"));
    assert!(beyond_the_parser(b"* 1 FETCH (X \"a\\b INTERNALDATE NIL\")\r\n"));
    assert!(beyond_the_parser(b"* 1 FETCH (X \"a\r\nINTERNALDATE NIL\")\r\n"));
    let deep = [&b"* 1 FETCH (X \"\x00"[..], &[b'('; 100], b"\r\n"].concat();
    assert!(!parser_takes(&deep));
    // A well-formed string still holds what it says, escapes included; an odd quote in a status
    // line's text costs nothing.
    assert!(!beyond_the_parser(b"* 1 FETCH (ENVELOPE (NIL \"say \\\"INTERNALDATE NIL\\\\\" NIL))\r\n"));
    assert!(parser_takes(b"* OK [ALERT] say \"hi\r\n"));
}

/// The fuzz target's seeds (`fuzz/seeds/imap`), each response given to the parser as a
/// connection gives it, without the fence around it: the fuzzer counts a caught panic too.
#[test]
fn the_fuzz_seeds_are_parsed_without_a_panic() {
    use super::conn::{Conn, parser_takes, ready};
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds/imap");
    let mut seeds = 0;
    for seed in std::fs::read_dir(dir).unwrap() {
        let mut conn = Conn::new(Script::new(&std::fs::read(seed.unwrap().path()).unwrap()));
        while let Ok(raw) = conn.read_raw() {
            let fixed = ready(raw);
            if parser_takes(&fixed) {
                let _ = imap_proto::Response::parse(&fixed);
            }
        }
        seeds += 1;
    }
    assert!(seeds >= 5, "the session and the four answers the library panicked on");
}
