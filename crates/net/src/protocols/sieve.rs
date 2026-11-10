// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
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
//! Server input is hostile: a line, a literal and one whole answer (its bytes, its words, its
//! time) are capped, from the greeting on, which is read before TLS.

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
/// One answer whole, lines and literals together: a script is one literal, and nothing else a
/// server says here comes near it.
const MAX_REPLY: usize = 4 << 20;
/// The atoms and strings of one answer (a script listed takes two).
const MAX_TOKENS: usize = 50_000;
/// How long one answer may take to come whole, checked between its lines.
const REPLY_TIME: std::time::Duration = std::time::Duration::from_secs(120);

/// What is left of the caps on the answer being read.
struct Budget {
    bytes: usize,
    tokens: usize,
    until: std::time::Instant,
}

impl Budget {
    fn new() -> Budget {
        Budget { bytes: MAX_REPLY, tokens: MAX_TOKENS, until: std::time::Instant::now() + REPLY_TIME }
    }

    /// `n` more bytes, before they are read into memory.
    fn bytes(&mut self, n: usize) -> Result<()> {
        self.bytes = self.bytes.checked_sub(n).ok_or_else(|| NetError::Protocol("ManageSieve: the answer is too large".into()))?;
        Ok(())
    }

    fn tokens(&mut self, n: usize) -> Result<()> {
        self.tokens = self.tokens.checked_sub(n).ok_or_else(|| NetError::Protocol("ManageSieve: the answer has too many words".into()))?;
        Ok(())
    }

    fn time(&self) -> Result<()> {
        if std::time::Instant::now() >= self.until {
            return Err(NetError::Protocol("ManageSieve: the server took too long to answer".into()));
        }
        Ok(())
    }
}

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
fn tokens<S: Read + Write>(io: &mut BufReader<S>, first: &str, budget: &mut Budget) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut line = first.to_string();
    loop {
        let before = out.len();
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
        budget.tokens(out.len().saturating_sub(before))?;
        let Some(n) = literal else { return Ok(out) };
        if n > MAX_LITERAL {
            return Err(NetError::Protocol("ManageSieve: literal too large".into()));
        }
        // Counted before it is made room for: literals follow one another on one line.
        budget.bytes(n)?;
        budget.tokens(1)?;
        let mut buf = vec![0u8; n];
        io.read_exact(&mut buf)?;
        out.push(String::from_utf8_lossy(&buf).into_owned());
        line = read_line(io, budget)?;
    }
}

fn read_line<S: Read + Write>(io: &mut BufReader<S>, budget: &mut Budget) -> Result<String> {
    budget.time()?;
    let mut buf = Vec::new();
    let n = io.by_ref().take(MAX_LINE.min(budget.bytes.saturating_add(1)) as u64).read_until(b'\n', &mut buf)?;
    budget.bytes(n)?;
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
    let mut budget = Budget::new();
    loop {
        let line = read_line(io, &mut budget)?;
        let t = tokens(io, &line, &mut budget)?;
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
pub(crate) mod tests;
