//! Automatic replies and inbox rules on IMAP servers: a small ManageSieve client (RFC 5804)
//! that keeps a Sieve script (RFC 5228) named `quatrelle` with a `vacation` action (RFC 5230),
//! limited to a date range with `currentdate` (RFC 5260) when one is set, and the rules marked
//! "also run on the server" (filing mail with `fileinto`, marking it with `imap4flags`).
//!
//! The script is Quatrelle's own: the settings it was made from are kept in its first lines, so
//! reading them back never means parsing Sieve. A script the user already runs stays in charge
//! of their other rules: ours `include`s it (RFC 6609) and becomes the active one; when ours does
//! nothing any more (no replies, no rule) theirs becomes active again.
//!
//! Server input is hostile: lines and literals are capped.

use std::io::{BufRead, BufReader, Read, Write};

use quatrelle_model::job::{Vacation, is_day};
use quatrelle_model::organize::{Condition, Rule, RuleAction};
use quatrelle_model::{PINNED, Security, ServerConfig, keyword_for, utf7};

use crate::stream::{self, Stream};
use crate::{NetError, Result};

/// ManageSieve's registered port.
pub const PORT: u16 = 4190;
/// Our script's name.
pub const SCRIPT: &str = "quatrelle";
const MARK: &str = "# quatrelle-vacation ";
const RULES_MARK: &str = "# quatrelle-rules ";
const MAX_LINE: usize = 64 << 10;
const MAX_LITERAL: usize = 1 << 20;

/// One server response: data lines before it, and the final `OK`/`NO`/`BYE` with its text.
struct Reply {
    data: Vec<Vec<String>>,
    ok: bool,
    text: String,
}

pub struct Client<S: Read + Write> {
    io: BufReader<S>,
    caps: Vec<(String, String)>,
}

/// A token of a response line: an atom, or a string (quoted or literal).
fn tokens<S: Read + Write>(io: &mut BufReader<S>, first: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut line = first.to_string();
    loop {
        let mut chars = line.char_indices().peekable();
        let mut literal: Option<usize> = None;
        while let Some((i, c)) = chars.next() {
            match c {
                ' ' | '\t' | '\r' | '\n' => {}
                '"' => {
                    let mut s = String::new();
                    loop {
                        match chars.next() {
                            Some((_, '\\')) => {
                                if let Some((_, e)) = chars.next() {
                                    s.push(e);
                                }
                            }
                            Some((_, '"')) | None => break,
                            Some((_, c)) => s.push(c),
                        }
                    }
                    out.push(s);
                }
                '{' => {
                    let rest = line.get(i + 1..).unwrap_or("");
                    let n: String = rest.chars().take_while(char::is_ascii_digit).collect();
                    literal = Some(n.parse().map_err(|_| NetError::Protocol("ManageSieve: bad literal".into()))?);
                    break;
                }
                '(' | ')' => {}
                _ => {
                    let mut atom = String::from(c);
                    while let Some(&(_, c)) = chars.peek() {
                        if c == ' ' || c == '\r' || c == '\n' || c == '(' || c == ')' {
                            break;
                        }
                        atom.push(c);
                        chars.next();
                    }
                    out.push(atom);
                }
            }
        }
        let Some(n) = literal else { return Ok(out) };
        if n > MAX_LITERAL {
            return Err(NetError::Protocol("ManageSieve: literal too large".into()));
        }
        let mut buf = vec![0u8; n];
        io.read_exact(&mut buf)?;
        out.push(String::from_utf8_lossy(&buf).into_owned());
        line = read_line(io)?;
    }
}

fn read_line<S: Read + Write>(io: &mut BufReader<S>) -> Result<String> {
    let mut buf = Vec::new();
    let n = io.by_ref().take(MAX_LINE as u64).read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Err(NetError::Closed("the ManageSieve server closed the connection".into()));
    }
    if !buf.ends_with(b"\n") {
        return Err(NetError::Protocol("ManageSieve: line too long".into()));
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn reply<S: Read + Write>(io: &mut BufReader<S>) -> Result<Reply> {
    let mut data = Vec::new();
    loop {
        let line = read_line(io)?;
        let t = tokens(io, &line)?;
        let head = t.first().map(|s| s.to_ascii_uppercase()).unwrap_or_default();
        match head.as_str() {
            "OK" | "NO" | "BYE" => {
                let text = t
                    .iter()
                    .skip(1)
                    .filter(|x| !x.chars().all(|c| c.is_ascii_uppercase() || c == '-' || c == '/'))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ");
                if head == "BYE" {
                    return Err(NetError::Closed(text));
                }
                return Ok(Reply { data, ok: head == "OK", text });
            }
            _ => {
                if data.len() > 10_000 {
                    return Err(NetError::Protocol("ManageSieve: too many lines".into()));
                }
                data.push(t);
            }
        }
    }
}

/// A ManageSieve quoted string.
fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

impl<S: Read + Write> Client<S> {
    /// Read the greeting (the server's capabilities).
    pub fn from_stream(s: S) -> Result<Client<S>> {
        let mut c = Client { io: BufReader::new(s), caps: Vec::new() };
        c.read_caps()?;
        Ok(c)
    }

    fn read_caps(&mut self) -> Result<()> {
        let r = reply(&mut self.io)?;
        if !r.ok {
            return Err(NetError::No(r.text));
        }
        self.caps = r.data.into_iter().filter_map(|t| Some((t.first()?.to_ascii_uppercase(), t.get(1).cloned().unwrap_or_default()))).collect();
        Ok(())
    }

    fn cap(&self, name: &str) -> Option<&str> {
        self.caps.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Whether the server's Sieve has an extension.
    pub fn has_extension(&self, ext: &str) -> bool {
        self.cap("SIEVE").is_some_and(|v| v.split_whitespace().any(|e| e.eq_ignore_ascii_case(ext)))
    }

    fn run(&mut self, cmd: &str) -> Result<Reply> {
        let w = self.io.get_mut();
        w.write_all(cmd.as_bytes())?;
        w.write_all(b"\r\n")?;
        w.flush()?;
        reply(&mut self.io)
    }

    fn run_ok(&mut self, cmd: &str) -> Result<Reply> {
        let r = self.run(cmd)?;
        if r.ok { Ok(r) } else { Err(NetError::No(r.text)) }
    }

    pub fn login(&mut self, user: &str, pass: &str) -> Result<()> {
        let token = crate::imap::base64(format!("\0{user}\0{pass}").as_bytes());
        let r = self.run(&format!("AUTHENTICATE \"PLAIN\" {}", q(&token)))?;
        if r.ok { Ok(()) } else { Err(NetError::Auth(if r.text.is_empty() { "the Sieve server refused the password".into() } else { r.text })) }
    }

    pub fn login_xoauth2(&mut self, user: &str, token: &str) -> Result<()> {
        let r = self.run(&format!("AUTHENTICATE \"XOAUTH2\" {}", q(&crate::imap::base64(crate::oauth::xoauth2(user, token).as_bytes()))))?;
        if r.ok { Ok(()) } else { Err(NetError::Auth(r.text)) }
    }

    /// Script names, and which one is active.
    pub fn list(&mut self) -> Result<(Vec<String>, Option<String>)> {
        let r = self.run_ok("LISTSCRIPTS")?;
        let mut names = Vec::new();
        let mut active = None;
        for t in r.data {
            let Some(n) = t.first() else { continue };
            if t.get(1).is_some_and(|a| a.eq_ignore_ascii_case("ACTIVE")) {
                active = Some(n.clone());
            }
            names.push(n.clone());
        }
        Ok((names, active))
    }

    pub fn get(&mut self, name: &str) -> Result<String> {
        let r = self.run_ok(&format!("GETSCRIPT {}", q(name)))?;
        Ok(r.data.into_iter().next().and_then(|t| t.into_iter().next()).unwrap_or_default())
    }

    pub fn put(&mut self, name: &str, script: &str) -> Result<()> {
        let cmd = format!("PUTSCRIPT {} {{{}+}}\r\n{script}", q(name), script.len());
        self.run_ok(&cmd).map(|_| ())
    }

    /// Make a script the active one (`""`: none).
    pub fn activate(&mut self, name: &str) -> Result<()> {
        self.run_ok(&format!("SETACTIVE {}", q(name))).map(|_| ())
    }

    pub fn logout(&mut self) {
        let _ = self.run("LOGOUT");
    }

    /// The automatic replies this script holds (`None`: never set up here).
    pub fn vacation(&mut self) -> Result<Vacation> {
        let (names, active) = self.list()?;
        let (mut settings, _) = self.settings(&names, active.as_deref())?;
        // Someone switched our script off elsewhere: the replies don't run.
        settings.vacation.enabled &= active.as_deref() == Some(SCRIPT);
        Ok(settings.vacation)
    }

    /// Store the automatic replies and switch them on or off (the rules it holds are kept).
    pub fn set_vacation(&mut self, v: &Vacation) -> Result<()> {
        self.update(|s| s.vacation = v.clone())
    }

    /// Store the rules that run on the server (the automatic replies it holds are kept).
    pub fn set_rules(&mut self, rules: Vec<Rule>, categories: Vec<(String, String)>) -> Result<()> {
        self.update(|s| {
            s.rules = rules;
            s.categories = categories;
        })
    }

    /// What our script holds, and the script the user had before ours.
    fn settings(&mut self, names: &[String], active: Option<&str>) -> Result<(Settings, Option<String>)> {
        if !names.iter().any(|n| n == SCRIPT) {
            return Ok((Settings::default(), None));
        }
        let text = self.get(SCRIPT)?;
        let mut vacation = read_script(&text).unwrap_or_default();
        if active != Some(SCRIPT) {
            // Someone switched our script off elsewhere: the replies don't run (the rules are
            // kept as they are: rewriting them off would throw them away).
            vacation.enabled = false;
        }
        Ok((Settings { vacation, rules: read_rules(&text), categories: Vec::new() }, read_previous(&text)))
    }

    /// Rewrite our script with these settings, switching it on when it does anything and the
    /// user's own script back on when it doesn't.
    fn update(&mut self, change: impl FnOnce(&mut Settings)) -> Result<()> {
        let (names, active) = self.list()?;
        let (mut s, previous) = self.settings(&names, active.as_deref())?;
        change(&mut s);
        // The script the user had before ours: the one active now, or the one ours remembers.
        let theirs = match active.as_deref() {
            Some(a) if a != SCRIPT => Some(a.to_string()),
            _ => previous.filter(|p| names.contains(p)),
        };
        let dated = s.vacation.from.is_some() || s.vacation.until.is_some();
        let files = s.rules.iter().any(|r| r.actions.iter().any(|a| matches!(a, RuleAction::MoveTo(_))));
        let marks = s
            .rules
            .iter()
            .any(|r| r.actions.iter().any(|a| matches!(a, RuleAction::MarkRead | RuleAction::Flag | RuleAction::Pin | RuleAction::Categorize(_))));
        if s.vacation.enabled && !self.has_extension("vacation") {
            return Err(NetError::No("this server's Sieve has no automatic replies (vacation)".into()));
        }
        if s.vacation.enabled && dated && !self.has_extension("date") {
            return Err(NetError::No("this server can't limit automatic replies to dates".into()));
        }
        if files && !self.has_extension("fileinto") {
            return Err(NetError::No("this server's Sieve can't file mail into folders (no fileinto)".into()));
        }
        if marks && !self.has_extension("imap4flags") {
            return Err(NetError::No("this server's Sieve can't mark mail (no imap4flags)".into()));
        }
        if theirs.is_some() && s.any() && !self.has_extension("include") {
            return Err(NetError::No(
                "another Sieve script runs your filters and this server can't combine scripts (include); turn it off first".into(),
            ));
        }
        self.put(SCRIPT, &script(&s, theirs.as_deref()))?;
        if s.any() {
            self.activate(SCRIPT)
        } else {
            // Nothing of ours to run: the user's own script runs again (or none).
            self.activate(theirs.as_deref().unwrap_or(""))
        }
    }
}

impl Client<Stream> {
    /// Connect to the account's Sieve server: the IMAP host on port 4190, encrypted with
    /// STARTTLS unless the account is plain (local test servers).
    pub fn connect(imap: &ServerConfig, port: u16) -> Result<Client<Stream>> {
        // Not tried where it's known not to be: as a server that has none says.
        if quatrelle_model::without_managesieve(&imap.host) {
            return Err(NetError::Io(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, format!("{} keeps no ManageSieve", imap.host))));
        }
        let sock = stream::tcp(&imap.host, port)?;
        let mut c = Client::from_stream(Stream::Plain(sock))?;
        if imap.security == Security::None {
            return Ok(c);
        }
        if c.cap("STARTTLS").is_none() {
            return Err(NetError::Tls("the Sieve server does not offer STARTTLS".into()));
        }
        c.run_ok("STARTTLS")?;
        if !c.io.buffer().is_empty() {
            return Err(NetError::Tls("unexpected data before TLS negotiation".into()));
        }
        let Stream::Plain(sock) = c.io.into_inner() else { return Err(NetError::Tls("connection already encrypted".into())) };
        // The server repeats its capabilities over TLS (RFC 5804 §2.2); the earlier ones are void.
        Client::from_stream(stream::tls(sock, &imap.host)?)
    }
}

/// A Sieve string: quoted, with `\` and `"` escaped (line breaks are allowed inside).
fn sieve_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\r', ""))
}

/// What Quatrelle keeps in its script: the automatic replies and the rules that run on the
/// server (their folders already resolved to the server's names).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub vacation: Vacation,
    pub rules: Vec<Rule>,
    /// The account's categories by name, with the IMAP keyword each is kept as (a rule adds the
    /// keyword of the category it files mail in).
    pub categories: Vec<(String, String)>,
}

impl Settings {
    /// Whether our script does anything (otherwise the user's own script runs again).
    pub fn any(&self) -> bool {
        self.vacation.enabled || !self.rules.is_empty()
    }
}

/// Our script for these settings. The settings themselves (and the user's own script, which it
/// includes) are kept in the first lines.
pub fn script(s: &Settings, include: Option<&str>) -> String {
    let v = &s.vacation;
    fn one_line<T: serde::Serialize>(v: &T) -> String {
        serde_json::to_string(v).unwrap_or_default().replace(['\r', '\n'], " ")
    }
    let mut out = format!("{MARK}{}\r\n", one_line(v));
    if !s.rules.is_empty() {
        out.push_str(&format!("{RULES_MARK}{}\r\n", one_line(&s.rules)));
    }
    if let Some(p) = include {
        out.push_str(&format!("# quatrelle-previous {}\r\n", p.replace(['\r', '\n'], " ")));
    }
    let from = v.from.as_deref().filter(|d| is_day(d));
    let until = v.until.as_deref().filter(|d| is_day(d));
    let dated = v.enabled && (from.is_some() || until.is_some());
    let files = s.rules.iter().any(|r| r.actions.iter().any(|a| matches!(a, RuleAction::MoveTo(_))));
    let marks = s
        .rules
        .iter()
        .any(|r| r.actions.iter().any(|a| matches!(a, RuleAction::MarkRead | RuleAction::Flag | RuleAction::Pin | RuleAction::Categorize(_))));
    let mut req: Vec<&str> = Vec::new();
    if v.enabled {
        req.push("vacation");
    }
    if include.is_some() {
        req.push("include");
    }
    if dated {
        req.extend(["date", "relational"]);
    }
    if files {
        req.push("fileinto");
    }
    if marks {
        req.push("imap4flags");
    }
    if !req.is_empty() {
        out.push_str(&format!("require [{}];\r\n", req.iter().map(|r| sieve_str(r)).collect::<Vec<_>>().join(", ")));
    }
    for r in &s.rules {
        out.push_str(&rule_script(r, &s.categories));
    }
    if let Some(p) = include {
        out.push_str(&format!("include :personal {};\r\n", sieve_str(p)));
    }
    if v.enabled {
        let mut tests = Vec::new();
        if let Some(d) = from {
            tests.push(format!("currentdate :value \"ge\" \"date\" {}", sieve_str(d)));
        }
        if let Some(d) = until {
            tests.push(format!("currentdate :value \"le\" \"date\" {}", sieve_str(d)));
        }
        let subject = if v.subject.trim().is_empty() { String::new() } else { format!(":subject {} ", sieve_str(v.subject.trim())) };
        let action = format!("vacation :days 1 {subject}{};\r\n", sieve_str(&v.text));
        if tests.is_empty() {
            out.push_str(&action);
        } else {
            out.push_str(&format!("if allof({}) {{\r\n    {action}}}\r\n", tests.join(", ")));
        }
    }
    out
}

/// A rule as Sieve statements (`""` for one that can never match).
fn rule_script(r: &Rule, categories: &[(String, String)]) -> String {
    let mut tests: Vec<String> = r.conditions.iter().filter_map(condition_test).collect();
    if tests.is_empty() || r.actions.is_empty() {
        return String::new();
    }
    let test = match tests.len() {
        1 => tests.remove(0),
        _ if r.match_all => format!("allof({})", tests.join(", ")),
        _ => format!("anyof({})", tests.join(", ")),
    };
    let mut actions = String::new();
    for a in &r.actions {
        if let Some(line) = action_line(a, categories) {
            actions.push_str("    ");
            actions.push_str(&line);
            actions.push_str("\r\n");
        }
    }
    if actions.is_empty() {
        return String::new();
    }
    if r.stop {
        actions.push_str("    stop;\r\n");
    }
    format!("if {test} {{\r\n{actions}}}\r\n")
}

/// A condition as a Sieve test (`None`: it can't be one, or an empty text matches nothing).
fn condition_test(c: &Condition) -> Option<String> {
    let text = |t: &str| {
        let t = t.trim();
        (!t.is_empty()).then(|| sieve_str(t))
    };
    let list = |v: &[String]| {
        let items: Vec<String> = v.iter().filter_map(|t| text(t)).collect();
        (!items.is_empty()).then(|| format!("[{}]", items.join(", ")))
    };
    Some(match c {
        Condition::From(t) => format!("header :contains \"from\" {}", text(t)?),
        Condition::To(t) => format!("header :contains [\"to\", \"cc\"] {}", text(t)?),
        Condition::Subject(t) => format!("header :contains \"subject\" {}", text(t)?),
        Condition::AnyText(t) => format!("header :contains [\"subject\", \"from\", \"to\", \"cc\"] {}", text(t)?),
        Condition::HasAttachment | Condition::Bulk => return None,
        // A list of values, any of which matches (`header :contains "from" ["a", "b"]`).
        Condition::FromAny(v) => format!("header :contains \"from\" {}", list(v)?),
        Condition::ToAny(v) => format!("header :contains [\"to\", \"cc\"] {}", list(v)?),
        Condition::SubjectAny(v) => format!("header :contains \"subject\" {}", list(v)?),
    })
}

/// What an action runs as in Sieve (`None` when only the caller can resolve it: a folder).
fn action_line(a: &RuleAction, categories: &[(String, String)]) -> Option<String> {
    Some(match a {
        RuleAction::MoveTo(path) => format!("fileinto {};", sieve_str(&utf7::decode(path))),
        RuleAction::MoveToRole(_) | RuleAction::Delete => return None,
        RuleAction::MarkRead => "addflag \"\\\\Seen\";".into(),
        RuleAction::Flag => "addflag \"\\\\Flagged\";".into(),
        RuleAction::Pin => format!("addflag {};", sieve_str(PINNED)),
        // A category's keyword is the account's (its default "Work" is `$label2`); one made
        // since is the hash of its name.
        RuleAction::Categorize(name) => {
            let keyword = categories.iter().find(|(n, _)| n == name).map(|(_, k)| k.clone()).unwrap_or_else(|| keyword_for(name));
            format!("addflag {};", sieve_str(&keyword))
        }
    })
}

/// The settings our script was made from.
pub fn read_script(s: &str) -> Option<Vacation> {
    let first = s.lines().next()?;
    serde_json::from_str(first.strip_prefix(MARK)?.trim()).ok()
}

/// The rules our script was made from.
pub fn read_rules(s: &str) -> Vec<Rule> {
    s.lines().take(3).find_map(|l| l.strip_prefix(RULES_MARK)).and_then(|j| serde_json::from_str(j.trim()).ok()).unwrap_or_default()
}

fn read_previous(s: &str) -> Option<String> {
    s.lines().take(3).find_map(|l| l.strip_prefix("# quatrelle-previous ").map(|p| p.trim().to_string())).filter(|p| !p.is_empty())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::{BufRead, BufReader, Write};
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
}
