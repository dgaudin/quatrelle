//! The fake IMAP server's `SEARCH`: a command's keys, and whether a message meets them.

use super::*;

/// A `SEARCH` command's keys and strings (quotes taken off).
pub(super) fn search_tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut escaped) = (false, false);
    for c in s.chars() {
        match c {
            _ if escaped => {
                cur.push(c);
                escaped = false;
            }
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            ' ' if !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// What a `SEARCH` key is checked against.
struct SearchCtx<'a> {
    m: &'a Msg,
    raw: String,
    head: String,
    body: String,
    max: u32,
}

impl SearchCtx<'_> {
    fn header(&self, name: &str) -> String {
        self.head.lines().filter(|l| l.starts_with(&format!("{name}:"))).collect::<Vec<_>>().join("\n")
    }
}

/// Whether a message matches every key (RFC 3501 §6.4.4; strings as substrings, any case).
pub(super) fn search_matches(m: &Msg, keys: &[String], max: u32) -> bool {
    let raw = String::from_utf8_lossy(&m.raw).to_lowercase();
    let (head, body) = match raw.split_once("\r\n\r\n") {
        Some((h, b)) => (h.to_string(), b.to_string()),
        None => (raw.clone(), String::new()),
    };
    let ctx = SearchCtx { m, raw, head, body, max };
    let mut i = 0;
    while i < keys.len() {
        let (ok, used) = search_key(&ctx, keys, i);
        if !ok {
            return false;
        }
        i += used.max(1);
    }
    true
}

/// One key at `i`: whether it holds, and how many tokens it took (`NOT` and `OR` take a key).
fn search_key(ctx: &SearchCtx<'_>, keys: &[String], i: usize) -> (bool, usize) {
    let Some(k) = keys.get(i) else { return (true, 1) };
    let arg = keys.get(i + 1).map(|a| a.to_lowercase()).unwrap_or_default();
    match k.to_ascii_uppercase().as_str() {
        "ALL" => (true, 1),
        "NOT" => {
            let (ok, used) = search_key(ctx, keys, i + 1);
            (!ok, used + 1)
        }
        "OR" => {
            let (one, used_one) = search_key(ctx, keys, i + 1);
            let (two, used_two) = search_key(ctx, keys, i + 1 + used_one);
            (one || two, used_one + used_two + 1)
        }
        "CHARSET" => (true, 2),
        "UNSEEN" => (!ctx.m.flags.iter().any(|f| f == "\\Seen"), 1),
        "SEEN" => (ctx.m.flags.iter().any(|f| f == "\\Seen"), 1),
        "FLAGGED" => (ctx.m.flags.iter().any(|f| f == "\\Flagged"), 1),
        "UNFLAGGED" => (!ctx.m.flags.iter().any(|f| f == "\\Flagged"), 1),
        "UID" => (in_set(ctx.m.uid, &arg, ctx.max), 2),
        "TEXT" => (ctx.raw.contains(&arg), 2),
        "BODY" => (ctx.body.contains(&arg), 2),
        "SUBJECT" => (ctx.header("subject").contains(&arg), 2),
        "FROM" => (ctx.header("from").contains(&arg), 2),
        "TO" => (ctx.header("to").contains(&arg), 2),
        "CC" => (ctx.header("cc").contains(&arg), 2),
        "KEYWORD" => (ctx.m.flags.iter().any(|f| f.eq_ignore_ascii_case(&arg)), 2),
        _ => (false, 1),
    }
}

pub(super) fn in_set(uid: u32, set: &str, max: u32) -> bool {
    set.split(',').any(|part| {
        let val = |x: &str| if x == "*" { max } else { x.parse().unwrap_or(0) };
        match part.split_once(':') {
            Some((a, b)) => {
                let (a, b) = (val(a), val(b));
                (a.min(b)..=a.max(b)).contains(&uid)
            }
            None => val(part) == uid,
        }
    })
}
