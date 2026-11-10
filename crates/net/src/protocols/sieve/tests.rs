// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Tests of the ManageSieve client and of the scripts it writes, against a small server.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use super::*;

/// Gmail keeps no ManageSieve and its port drops what it's sent: reading the rules waited
/// 29 s for the connect timeout, and the first sync's folders behind it. It isn't tried.
#[test]
fn a_service_known_to_keep_no_managesieve_is_not_tried() {
    let gmail = ServerConfig { host: "imap.gmail.com".into(), port: 993, security: Security::Tls };
    let at = std::time::Instant::now();
    let refused = Client::connect(&gmail, PORT).err();
    assert!(at.elapsed() < std::time::Duration::from_secs(1), "{:?}", at.elapsed());
    assert!(
        matches!(&refused, Some(NetError::Io(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused && e.to_string().contains("keeps no ManageSieve")),
        "{refused:?}"
    );
}

#[test]
fn scripts() {
    let v = Vacation {
        enabled: true,
        subject: "Away \"now\"".into(),
        text: "Back on Monday.\nZoe".into(),
        from: Some("2026-10-15".into()),
        until: None,
        ..Default::default()
    };
    let s = script(&Settings { vacation: v.clone(), ..Default::default() }, Some("filters"));
    assert!(s.starts_with(MARK));
    assert!(s.contains(r#"require ["vacation", "include", "date", "relational"];"#), "{s}");
    assert!(s.contains("include :personal \"filters\";"));
    assert!(s.contains(r#"if allof(currentdate :value "ge" "date" "2026-10-15") {"#));
    assert!(
        s.contains(
            r#":subject "Away \"now\"" "Back on Monday.
Zoe";"#
        ),
        "{s}"
    );
    assert_eq!(read_script(&s), Some(v.clone()));
    assert_eq!(read_previous(&s).as_deref(), Some("filters"));
    let off = script(&Settings { vacation: Vacation { enabled: false, ..v }, ..Default::default() }, None);
    assert!(!off.contains("vacation :days"), "off: no action");
    assert_eq!(read_script("require \"vacation\";"), None);
    // Settings never break out of the comment line.
    let evil = Vacation { enabled: true, subject: "x\r\nrequire \"fileinto\";".into(), text: "t".into(), ..Default::default() };
    assert_eq!(
        script(&Settings { vacation: evil, ..Default::default() }, None).lines().next().unwrap().matches("require").count(),
        1,
        "still one comment line"
    );
}

/// A rule marked for the server becomes Sieve: its conditions as tests (any or all), its
/// actions as fileinto/addflag/stop, and what Sieve can't look at is left out.
#[test]
fn rules_become_sieve() {
    let rule = Rule {
        name: "Invoices".into(),
        enabled: true,
        account: Some("a1".into()),
        conditions: vec![Condition::From("billing@x.example".into()), Condition::Subject("invoice".into())],
        match_all: true,
        actions: vec![RuleAction::MoveTo("INBOX/Invoices".into()), RuleAction::MarkRead, RuleAction::Categorize("Work".into())],
        stop: true,
        server: true,
        remote_id: None,
        priority: None,
        refused: None,
    };
    let categories = vec![("Work".to_string(), "$label2".to_string())];
    // The script is written with CRLF; the blocks below are read as written.
    let crlf = |s: String| s.replace("\r\n", "\n");
    let s = crlf(script(&Settings { rules: vec![rule.clone()], categories: categories.clone(), ..Default::default() }, None));
    assert!(s.contains(r#"require ["fileinto", "imap4flags"];"#), "{s}");
    assert!(
        s.contains(
            r#"if allof(header :contains "from" "billing@x.example", header :contains "subject" "invoice") {
    fileinto "INBOX/Invoices";
    addflag "\\Seen";
    addflag "$label2";
    stop;
}"#
        ),
        "{s}"
    );
    assert_eq!(read_rules(&s), vec![rule.clone()]);
    assert_eq!(read_script(&s), Some(Vacation::default()));
    // Any of the conditions, and the flag/pin actions Sieve knows with imap4flags.
    let any = Rule {
        conditions: vec![Condition::To("ann@x.example".into()), Condition::AnyText("urgent".into())],
        match_all: false,
        actions: vec![RuleAction::Flag, RuleAction::Pin],
        ..rule.clone()
    };
    let s = crlf(script(&Settings { rules: vec![any], ..Default::default() }, None));
    assert!(
        s.contains(
            r#"if anyof(header :contains ["to", "cc"] "ann@x.example", header :contains ["subject", "from", "to", "cc"] "urgent") {
    addflag "\\Flagged";
    addflag "$Pinned";
    stop;
}"#
        ),
        "{s}"
    );
    // A rule with nothing to match (an empty text) would match every message: left out.
    let empty = Rule { conditions: vec![Condition::From("  ".into())], actions: vec![RuleAction::MarkRead], stop: false, ..rule.clone() };
    let s = crlf(script(&Settings { rules: vec![empty], ..Default::default() }, None));
    assert!(!s.contains("if "), "{s}");
    // What Sieve can't see (attachments, newsletters) is never sent.
    assert!(!Rule { conditions: vec![Condition::HasAttachment], actions: vec![], ..rule.clone() }.sieve_ready());
    assert!(!Rule { conditions: vec![Condition::Bulk], actions: vec![], ..rule.clone() }.sieve_ready());
    assert!(rule.sieve_ready() && rule.server_ready(), "both can take it");
    let any_text = Rule { conditions: vec![Condition::AnyText("x".into())], actions: vec![RuleAction::MarkRead], ..rule.clone() };
    assert!(any_text.sieve_ready() && !any_text.server_ready(), "Exchange can't look in every header, Sieve can");
    // The automatic replies and the rules live in the same script, both kept.
    let v = Vacation { enabled: true, text: "Away".into(), ..Default::default() };
    let s = crlf(script(&Settings { vacation: v.clone(), rules: vec![rule.clone()], categories }, Some("filters")));
    assert!(s.contains(r#"require ["vacation", "include", "fileinto", "imap4flags"];"#), "{s}");
    assert_eq!(read_script(&s), Some(v));
    assert_eq!(read_rules(&s), vec![rule]);
    assert!(s.find("fileinto").unwrap() < s.find("include :personal").unwrap(), "our rules first, then the user's own");
}

/// A ManageSieve server for tests: scripts in memory, one connection at a time.
pub(crate) struct Fake {
    pub scripts: Vec<(String, String)>,
    pub active: Option<String>,
    pub sieve: &'static str,
    pub log: Vec<String>,
}

pub(crate) fn serve(fake: Fake) -> (u16, Arc<Mutex<Fake>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let st = Arc::new(Mutex::new(fake));
    let shared = st.clone();
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(mut s) = conn else { continue };
            let mut r = BufReader::new(s.try_clone().unwrap());
            let caps = |s: &mut std::net::TcpStream, sieve: &str| {
                let _ = write!(
                    s,
                    "\"IMPLEMENTATION\" \"Fake\"\r\n\"SASL\" \"PLAIN\"\r\n\"SIEVE\" \"{sieve}\"\r\n\"VERSION\" \"1.0\"\r\nOK \"ready\"\r\n"
                );
            };
            caps(&mut s, shared.lock().unwrap().sieve);
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                let mut f = shared.lock().unwrap();
                f.log.push(line.trim_end().to_string());
                let arg = |n: usize| line.split('"').nth(n).unwrap_or("").to_string();
                let up = line.to_ascii_uppercase();
                if up.starts_with("AUTHENTICATE") {
                    let good = crate::imap::base64(b"\0me@studio.example\0pw");
                    let _ = s.write_all(if line.contains(&good) { b"OK\r\n" as &[u8] } else { b"NO \"wrong password\"\r\n" });
                } else if up.starts_with("LISTSCRIPTS") {
                    for (n, _) in &f.scripts {
                        let act = if f.active.as_deref() == Some(n.as_str()) { " ACTIVE" } else { "" };
                        let _ = write!(s, "\"{n}\"{act}\r\n");
                    }
                    let _ = s.write_all(b"OK\r\n");
                } else if up.starts_with("GETSCRIPT") {
                    let name = arg(1);
                    match f.scripts.iter().find(|(n, _)| *n == name) {
                        Some((_, body)) => {
                            let _ = write!(s, "{{{}}}\r\n{body}\r\nOK\r\n", body.len());
                        }
                        None => {
                            let _ = s.write_all(b"NO \"no such script\"\r\n");
                        }
                    }
                } else if up.starts_with("PUTSCRIPT") {
                    let name = arg(1);
                    let n: usize = line.rsplit('{').next().unwrap_or("").trim_end_matches(['\r', '\n', '}', '+']).parse().unwrap_or(0);
                    let mut body = vec![0u8; n];
                    r.read_exact(&mut body).unwrap();
                    let mut end = String::new();
                    let _ = r.read_line(&mut end);
                    let body = String::from_utf8(body).unwrap();
                    f.scripts.retain(|(x, _)| *x != name);
                    f.scripts.push((name, body));
                    let _ = s.write_all(b"OK\r\n");
                } else if up.starts_with("SETACTIVE") {
                    let name = arg(1);
                    f.active = (!name.is_empty()).then_some(name);
                    let _ = s.write_all(b"OK\r\n");
                } else if up.starts_with("LOGOUT") {
                    let _ = s.write_all(b"OK\r\n");
                    break;
                } else {
                    let _ = s.write_all(b"NO \"unknown\"\r\n");
                }
            }
        }
    });
    (port, st)
}

fn connect(port: u16) -> Client<std::net::TcpStream> {
    let mut c = Client::from_stream(std::net::TcpStream::connect(("127.0.0.1", port)).unwrap()).unwrap();
    c.login("me@studio.example", "pw").unwrap();
    c
}

#[test]
fn replies_on_and_off_keep_the_users_filters() {
    let (port, st) = serve(Fake {
        scripts: vec![("filters".into(), "require \"fileinto\";\r\n".into())],
        active: Some("filters".into()),
        sieve: "fileinto vacation include date relational",
        log: Vec::new(),
    });
    let mut bad = Client::from_stream(std::net::TcpStream::connect(("127.0.0.1", port)).unwrap()).unwrap();
    assert!(matches!(bad.login("me@studio.example", "nope"), Err(NetError::Auth(_))));
    bad.logout();
    let mut c = connect(port);
    assert_eq!(c.vacation().unwrap(), Vacation::default(), "never set up");
    let v = Vacation { enabled: true, subject: "Away".into(), text: "Back soon".into(), until: Some("2026-10-20".into()), ..Default::default() };
    c.set_vacation(&v).unwrap();
    assert_eq!(c.vacation().unwrap(), v);
    {
        let f = st.lock().unwrap();
        assert_eq!(f.active.as_deref(), Some(SCRIPT));
        let ours = &f.scripts.iter().find(|(n, _)| n == SCRIPT).unwrap().1;
        assert!(ours.contains("include :personal \"filters\";"), "the user's filters still run");
    }
    c.set_vacation(&Vacation { enabled: false, ..v.clone() }).unwrap();
    assert_eq!(st.lock().unwrap().active.as_deref(), Some("filters"), "their script is back");
    let off = c.vacation().unwrap();
    assert!(!off.enabled && off.text == "Back soon", "the text is kept for next time");
    // On again: theirs is still found through our script's note.
    c.set_vacation(&v).unwrap();
    assert!(st.lock().unwrap().scripts.iter().any(|(n, s)| n == SCRIPT && s.contains("include :personal \"filters\"")));
    c.logout();
}

#[test]
fn servers_without_the_extensions_say_so() {
    let (port, _) = serve(Fake { scripts: Vec::new(), active: None, sieve: "fileinto", log: Vec::new() });
    let mut c = connect(port);
    let v = Vacation { enabled: true, text: "x".into(), ..Default::default() };
    assert!(matches!(c.set_vacation(&v), Err(NetError::No(e)) if e.contains("vacation")));
    let (port, _) =
        serve(Fake { scripts: vec![("filters".into(), String::new())], active: Some("filters".into()), sieve: "vacation", log: Vec::new() });
    let mut c = connect(port);
    assert!(matches!(c.set_vacation(&v), Err(NetError::No(e)) if e.contains("include")));
    let dated = Vacation { from: Some("2026-01-01".into()), ..v };
    let (port, _) = serve(Fake { scripts: Vec::new(), active: None, sieve: "vacation", log: Vec::new() });
    assert!(matches!(connect(port).set_vacation(&dated), Err(NetError::No(e)) if e.contains("dates")));
}

/// A server that never stops sending `piece`, and counts what it has handed over. It gives up
/// (an I/O error) past 64 MB, so a reader with no cap of its own fails the test instead of
/// taking the computer's memory.
struct Endless {
    piece: Vec<u8>,
    at: usize,
    served: Arc<Mutex<usize>>,
}

impl Read for Endless {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut served = self.served.lock().unwrap();
        if *served > 64 << 20 {
            return Err(std::io::Error::other("the reader never stopped"));
        }
        let mut n = 0;
        while let (Some(slot), Some(b)) = (buf.get_mut(n), self.piece.get(self.at)) {
            *slot = *b;
            n += 1;
            self.at = (self.at + 1) % self.piece.len();
        }
        *served += n;
        Ok(n)
    }
}

impl Write for Endless {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// One answer is capped as a whole, the greeting included (read before STARTTLS, so from
/// anyone on the network): literals one after the other, each under the cap for one, long
/// lines without an end, and lines of very many words are refused after a few megabytes.
#[test]
fn an_answer_without_an_end_is_refused_after_a_few_megabytes() {
    let literal = [b"{1048576}\r\n".to_vec(), vec![b'x'; 1 << 20], b"\r\n".to_vec()].concat();
    let chained = [b"\"A\" {1048576}\r\n".to_vec(), vec![b'x'; 1 << 20], b" ".to_vec()].concat();
    let line = [b"\"".to_vec(), vec![b'x'; 60_000], b"\"\r\n".to_vec()].concat();
    let words = b"a b c d e f g h i j k l m n o p q r s t u v w x y z\r\n".to_vec();
    for piece in [literal, chained, line, words] {
        let served = Arc::new(Mutex::new(0));
        let got = Client::from_stream(Endless { piece, at: 0, served: Arc::clone(&served) });
        assert!(matches!(&got, Err(NetError::Protocol(m)) if m.contains("ManageSieve")), "{:?}", got.err());
        assert!(*served.lock().unwrap() <= 8 << 20, "{} bytes read", *served.lock().unwrap());
    }
}
