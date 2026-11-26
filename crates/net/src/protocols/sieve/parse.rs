// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Reading a Sieve script Quatrelle didn't write as the rules it holds: [`parse_rules`]. A small
//! tokenizer and a walk of its `if` blocks, bounded (tokens, nesting, rules, one string) since a
//! script from the server is hostile input.

use super::*;

/// The rules a script Quatrelle didn't write holds, in Quatrelle's terms where every part of
/// them can be said there: each `if` block is one rule (`elsif` and `else` blocks are entries
/// of their own, the `else` one never runnable — Quatrelle's rules have no "otherwise"). A test
/// or an action Sieve has that Quatrelle doesn't (a body test, a `redirect`…) makes its rule
/// not runnable, and its summary says what it was. `active`: whether the server runs this
/// script now. `categories`: the account's, to turn a keyword back into the category it is.
pub fn parse_rules(account: &str, script: &str, text: &str, active: bool, categories: &[(String, String)]) -> Vec<ForeignRule> {
    let mut p = P { t: lex(text), i: 0 };
    let mut out: Vec<ForeignRule> = Vec::new();
    while p.peek().is_some() && out.len() < MAX_PARSE_RULES {
        let Some(Word::Atom(a)) = p.peek().cloned() else {
            p.next();
            skip_rest(&mut p);
            continue;
        };
        if !a.eq_ignore_ascii_case("if") {
            // `require`, `include`, or a statement of an extension Quatrelle doesn't know.
            p.next();
            skip_rest(&mut p);
            continue;
        }
        p.next();
        let mut n = out.len();
        loop {
            if out.len() >= MAX_PARSE_RULES {
                break;
            }
            let t = match p.peek().cloned() {
                Some(Word::Atom(a)) if a.eq_ignore_ascii_case("elsif") => {
                    p.next();
                    parse_test(&mut p, 0)
                }
                Some(Word::Atom(a)) if a.eq_ignore_ascii_case("else") => {
                    p.next();
                    Test { conds: Vec::new(), all: true, text: "otherwise".into(), ok: false, combined: None }
                }
                _ => parse_test(&mut p, 0),
            };
            p.eat(&Word::Open('{'));
            let (actions, stop, ok, then) = parse_block(&mut p, categories);
            p.eat(&Word::Close('}'));
            out.push(foreign_rule(account, script, n, active, t, actions, stop, ok, &then));
            n += 1;
            if !matches!(p.peek(), Some(Word::Atom(a)) if a.eq_ignore_ascii_case("elsif") || a.eq_ignore_ascii_case("else")) {
                break;
            }
        }
    }
    out
}

/// A string from the script, made safe to keep and show: control characters and bidi overrides
/// out (they would only serve to spoof the text shown), capped in length.
fn clean(s: &str) -> String {
    const MAX_TEXT: usize = 200;
    let mut out = String::new();
    for c in s.chars() {
        if out.len() >= MAX_TEXT {
            break;
        }
        match c {
            c if c.is_control() => {}
            '\u{202A}' | '\u{202B}' | '\u{202C}' | '\u{202D}' | '\u{202E}' | '\u{2066}' | '\u{2067}' | '\u{2068}' | '\u{2069}' => {}
            c => out.push(c),
        }
    }
    out
}

/// One parsed `if` block as a [`ForeignRule`].
fn foreign_rule(
    account: &str,
    script: &str,
    n: usize,
    active: bool,
    t: Test,
    actions: Vec<RuleAction>,
    stop: bool,
    ok: bool,
    then: &[String],
) -> ForeignRule {
    let id = format!("sieve:{script}#{n}");
    // The id goes back to the server as it named the script; the name is shown, so it's cleaned.
    let name = format!("{} #{n}", clean(script));
    let when = if t.text.is_empty() { "every message".to_string() } else { t.text.clone() };
    let then = if then.is_empty() { "nothing".to_string() } else { then.join(", ") };
    let mut summary = format!("{when} → {then}");
    if stop {
        summary.push_str(", then stops");
    }
    // The summary is shown in the interface's hover: bounded, and safe characters only.
    let summary = clean(&summary);
    let summary = {
        let mut s = summary;
        const MAX_SUMMARY: usize = 500;
        if s.len() > MAX_SUMMARY {
            let end = s.floor_char_boundary(MAX_SUMMARY);
            s.truncate(end);
        }
        s
    };
    // A condition Quatrelle couldn't say entirely (`t.ok`) is not runnable, even when the
    // flattening left some conditions in it (a mixed `allof`/`anyof`, say).
    let runnable = ok && t.ok && !t.conds.is_empty() && !actions.is_empty();
    let rule = runnable.then(|| Rule {
        name: name.clone(),
        enabled: active,
        account: Some(account.to_string()),
        conditions: t.conds.clone(),
        match_all: t.all,
        actions: actions.clone(),
        stop,
        server: true,
        remote_id: Some(id.clone()),
        priority: None,
        refused: None,
    });
    ForeignRule { account: account.to_string(), id, name, enabled: active, runnable, rule, summary }
}

/// One Sieve test, parsed: Quatrelle's conditions when it can say them (`ok`), and what the
/// test was in words either way.
struct Test {
    conds: Vec<Condition>,
    all: bool,
    text: String,
    ok: bool,
    /// `Some(all)` when the test is an `allof`/`anyof` of several (`None`: a plain test), so a
    /// combinator nested in a different one is not flattened into a false rule (I-3).
    combined: Option<bool>,
}

/// How deep `allof`/`anyof`/`not` may nest before the rest is skipped (a hostile script's).
const MAX_TEST_DEPTH: usize = 32;

fn parse_test(p: &mut P, depth: usize) -> Test {
    if depth > MAX_TEST_DEPTH {
        skip_rest(p);
        return Test { conds: Vec::new(), all: true, text: String::new(), ok: false, combined: None };
    }
    let Some(Word::Atom(head)) = p.peek().cloned() else {
        skip_rest(p);
        return Test { conds: Vec::new(), all: true, text: String::new(), ok: false, combined: None };
    };
    if head.eq_ignore_ascii_case("allof") || head.eq_ignore_ascii_case("anyof") {
        let all = head.eq_ignore_ascii_case("allof");
        p.next();
        p.eat(&Word::Open('('));
        let mut subs: Vec<Test> = Vec::new();
        loop {
            if matches!(p.peek(), Some(Word::Close(')')) | None) {
                break;
            }
            subs.push(parse_test(p, depth + 1));
            if !p.eat(&Word::Comma) {
                break;
            }
        }
        p.eat(&Word::Close(')'));
        // Only same-operator nesting flattens: `allof` in `allof`, `anyof` in `anyof`. A
        // different one inside would change what matches, so the rule is not runnable.
        let ok = !subs.is_empty() && subs.iter().all(|s| s.ok) && subs.iter().all(|s| s.combined.is_none_or(|c| c == all));
        let text = subs.iter().map(|s| s.text.clone()).collect::<Vec<_>>().join(if all { " + " } else { " or " });
        let text = if subs.len() > 1 { format!("({text})") } else { text };
        Test { conds: subs.into_iter().flat_map(|s| s.conds).collect(), all, text, ok, combined: Some(all) }
    } else if head.eq_ignore_ascii_case("not") {
        p.next();
        let inner = parse_test(p, depth + 1);
        Test { conds: Vec::new(), all: true, text: format!("not {}", inner.text), ok: false, combined: None }
    } else if head.eq_ignore_ascii_case("header") {
        p.next();
        // Without an option a test is `:is` (RFC 5228), an exact match: only an explicit
        // `:contains` is what Quatrelle's rules look with; any other option (`:matches`,
        // `:regex`, `:comparator`…) makes the test one it can't say.
        let mut contains = false;
        let mut other = false;
        while let Some(Word::Atom(c)) = p.peek()
            && c.starts_with(':')
        {
            let c = c.clone();
            p.next();
            if c.eq_ignore_ascii_case(":contains") {
                contains = true;
            } else {
                other = true;
                if c.eq_ignore_ascii_case(":comparator") {
                    str_list(p);
                }
            }
        }
        let contains = contains && !other;
        match (str_list(p), str_list(p)) {
            (Some((f, _)), Some((v, was_list))) => header_test(&f, &v, was_list, contains),
            _ => {
                skip_rest(p);
                Test { conds: Vec::new(), all: true, text: "header".into(), ok: false, combined: None }
            }
        }
    } else {
        p.next();
        skip_rest(p);
        Test { conds: Vec::new(), all: true, text: head, ok: false, combined: None }
    }
}

/// A `header` test as Quatrelle's condition: the headers it looks in decide which one it is.
/// A list of values (`["a", "b"]`, even one) is the "one of these" condition; a plain string
/// is the single one.
fn header_test(fields: &[String], values: &[String], values_was_list: bool, contains: bool) -> Test {
    let quoted = |v: &[String]| v.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(", ");
    let list = |v: &[String], was_list: bool| if was_list { format!("[{}]", quoted(v)) } else { quoted(v) };
    let mut f: Vec<String> = fields.iter().map(|s| s.to_ascii_lowercase()).collect();
    f.sort();
    f.dedup();
    let one = values.len() == 1 && !values_was_list;
    let cond = if !contains {
        None
    } else if f == ["from"] && one {
        Some(Condition::From(values[0].clone()))
    } else if f == ["from"] && !values.is_empty() {
        Some(Condition::FromAny(values.to_vec()))
    } else if f == ["cc", "to"] && one {
        Some(Condition::To(values[0].clone()))
    } else if f == ["cc", "to"] && !values.is_empty() {
        Some(Condition::ToAny(values.to_vec()))
    } else if f == ["subject"] && one {
        Some(Condition::Subject(values[0].clone()))
    } else if f == ["subject"] && !values.is_empty() {
        Some(Condition::SubjectAny(values.to_vec()))
    } else if f == ["cc", "from", "subject", "to"] && one {
        Some(Condition::AnyText(values[0].clone()))
    } else {
        None
    };
    let word = if f == ["from"] {
        "sender"
    } else if f == ["cc", "to"] {
        "recipient"
    } else if f == ["subject"] {
        "subject"
    } else if f == ["cc", "from", "subject", "to"] {
        "anywhere"
    } else {
        "header"
    };
    match cond {
        Some(c) => Test { conds: vec![c], all: true, text: format!("{word} {}", list(values, values_was_list)), ok: true, combined: None },
        None => Test {
            conds: Vec::new(),
            all: true,
            text: format!("header {} {}", list(fields, true), list(values, values_was_list)),
            ok: false,
            combined: None,
        },
    }
}

/// The statements of one `{ … }` block: the actions in Quatrelle's terms, whether it `stop`s,
/// whether every statement could be said (`ok`), and what each was in words.
fn parse_block(p: &mut P, categories: &[(String, String)]) -> (Vec<RuleAction>, bool, bool, Vec<String>) {
    let mut actions = Vec::new();
    let mut stop = false;
    let mut ok = true;
    let mut then: Vec<String> = Vec::new();
    while let Some(t) = p.peek().cloned() {
        if t == Word::Close('}') {
            break;
        }
        let Some(Word::Atom(a)) = p.peek().cloned() else {
            p.next();
            continue;
        };
        p.next();
        match a.to_ascii_lowercase().as_str() {
            "fileinto" => {
                // A `:copy` or other tag changes what it does; Quatrelle's move isn't that.
                let tagged = matches!(p.peek(), Some(Word::Atom(c)) if c.starts_with(':'));
                if tagged {
                    p.next();
                }
                match p.next() {
                    Some(Word::Str(name)) if !tagged => {
                        // The script names mailboxes as the server does (UTF-8); Quatrelle
                        // keeps folder paths in modified UTF-7. The path keeps the name as
                        // the server has it; only the words shown are cleaned.
                        actions.push(RuleAction::MoveTo(utf7::encode(&name)));
                        then.push(format!("file into \"{}\"", clean(&name)));
                    }
                    Some(Word::Str(name)) => {
                        ok = false;
                        then.push(format!("file into \"{}\" with a tag Quatrelle can't do", clean(&name)));
                    }
                    _ => {
                        ok = false;
                        then.push("fileinto".into());
                    }
                }
            }
            "addflag" => match p.next() {
                Some(Word::Str(kw)) => {
                    // IMAP flags are case-insensitive; a category's keyword is the account's
                    // own and stays compared as it is.
                    let action = if kw.eq_ignore_ascii_case("\\Seen") {
                        Some((RuleAction::MarkRead, "mark as read".to_string()))
                    } else if kw.eq_ignore_ascii_case("\\Flagged") {
                        Some((RuleAction::Flag, "flag".to_string()))
                    } else if kw.eq_ignore_ascii_case(PINNED) {
                        Some((RuleAction::Pin, "pin".to_string()))
                    } else {
                        categories.iter().find(|(_, k)| k == &kw).map(|(n, _)| (RuleAction::Categorize(n.clone()), format!("categorise \"{n}\"")))
                    };
                    match action {
                        Some((act, text)) => {
                            actions.push(act);
                            then.push(text);
                        }
                        None => {
                            ok = false;
                            then.push(format!("addflag \"{kw}\""));
                        }
                    }
                }
                _ => {
                    ok = false;
                    then.push("addflag".into());
                }
            },
            "discard" => {
                actions.push(RuleAction::Delete);
                then.push("delete".into());
            }
            "stop" => stop = true,
            // Keeping the message is what happens anyway.
            "keep" => {}
            other => {
                ok = false;
                then.push(other.to_string());
            }
        }
        if !p.eat(&Word::Semi) {
            skip_rest(p);
        }
    }
    (actions, stop, ok, then)
}

/// A string or a list of strings (`"a"` or `["a", "b"]`): the values, and whether they were
/// written as a list (a list is the "one of these" condition, even with one value).
fn str_list(p: &mut P) -> Option<(Vec<String>, bool)> {
    match p.peek().cloned() {
        Some(Word::Str(s)) => {
            p.next();
            Some((vec![clean(&s)], false))
        }
        Some(Word::Open('[')) => {
            p.next();
            let mut out = Vec::new();
            loop {
                match p.next() {
                    Some(Word::Str(s)) => out.push(clean(&s)),
                    Some(Word::Comma) => continue,
                    Some(Word::Close(']')) => break,
                    _ => return None,
                }
            }
            Some((out, true))
        }
        _ => None,
    }
}

/// Skip the rest of a statement or test: to the `;` (consumed), or to a `,` or a closing
/// bracket that belongs to the caller (left).
fn skip_rest(p: &mut P) {
    let mut depth = 0usize;
    while let Some(t) = p.peek().cloned() {
        match t {
            // The block of the `if` this test belongs to is not part of the test.
            Word::Open('{') if depth == 0 => break,
            Word::Open(_) => {
                depth += 1;
                p.next();
            }
            Word::Close(_) => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
                p.next();
            }
            Word::Semi if depth == 0 => {
                p.next();
                break;
            }
            Word::Comma if depth == 0 => break,
            _ => {
                p.next();
            }
        }
    }
}

/// One word of a Sieve script: an atom, a quoted string, or one punctuation mark.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Word {
    Atom(String),
    Str(String),
    Open(char),
    Close(char),
    Semi,
    Comma,
}

/// A script's tokens, cut at a cap (the client caps a script on the way in already; this is
/// the second gate, for one read any other way).
const MAX_PARSE_TOKENS: usize = 100_000;
/// The rules one script is listed as (a server's script is small; the cap is for a hostile one).
const MAX_PARSE_RULES: usize = 1_000;

fn lex(s: &str) -> Vec<Word> {
    let mut out = Vec::new();
    let mut it = s.char_indices().peekable();
    while let Some((_, c)) = it.next() {
        if out.len() >= MAX_PARSE_TOKENS {
            break;
        }
        match c {
            c if c.is_whitespace() => {}
            '#' => {
                while let Some(&(_, c)) = it.peek() {
                    if c == '\n' {
                        break;
                    }
                    it.next();
                }
            }
            '"' => {
                let mut v = String::new();
                while let Some((_, c)) = it.next() {
                    match c {
                        '\\' => {
                            if let Some((_, e)) = it.next()
                                && v.len() < MAX_LINE
                            {
                                v.push(e);
                            }
                        }
                        '"' => break,
                        _ => {
                            if v.len() < MAX_LINE {
                                v.push(c);
                            }
                        }
                    }
                }
                out.push(Word::Str(v));
            }
            '(' | '[' | '{' => out.push(Word::Open(c)),
            ')' | ']' | '}' => out.push(Word::Close(c)),
            ';' => out.push(Word::Semi),
            ',' => out.push(Word::Comma),
            _ => {
                let mut v = String::from(c);
                while let Some(&(_, c)) = it.peek() {
                    if c.is_whitespace() || matches!(c, '"' | '(' | '[' | '{' | ')' | ']' | '}' | ';' | ',' | '#') {
                        break;
                    }
                    if v.len() < MAX_LINE {
                        v.push(c);
                    }
                    it.next();
                }
                out.push(Word::Atom(v));
            }
        }
    }
    out
}

/// A token cursor over a script.
struct P {
    t: Vec<Word>,
    i: usize,
}

impl P {
    fn peek(&self) -> Option<&Word> {
        self.t.get(self.i)
    }

    fn next(&mut self) -> Option<Word> {
        let t = self.t.get(self.i).cloned();
        if t.is_some() {
            self.i += 1;
        }
        t
    }

    fn eat(&mut self, t: &Word) -> bool {
        if self.peek() == Some(t) {
            self.i += 1;
            true
        } else {
            false
        }
    }
}
