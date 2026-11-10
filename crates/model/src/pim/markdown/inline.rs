// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A line's words: code in backticks, links, bare addresses, and bold, italic and struck text
//! by their marks (CommonMark's runs of `*`, `_` and `~~`, simplified). A mark left open is text.

use std::ops::Range;

use super::{Kind, MAX_SPANS, Span, Style};

/// Most pieces a line is cut into before its marks are matched; past them, the rest is text.
const MAX_TOKENS: usize = 1024;
/// How many marks back a closing mark looks for its opening one.
const LOOKBACK: usize = 32;
/// Longest link text, in bytes.
const MAX_LINK_TEXT: usize = 512;
/// Longest address, in bytes.
pub(super) const MAX_URL: usize = 2048;

/// A piece of a line, before the marks are matched.
enum Tok {
    Text(Range<usize>),
    /// A run of `*`, `_` or `~` that may open or close (an index in the runs).
    Run(usize),
    Code {
        open: Range<usize>,
        body: Range<usize>,
        close: Range<usize>,
    },
    /// `[text](url)`, `end` after its `)`.
    Link {
        text: Range<usize>,
        url: Range<usize>,
        end: usize,
    },
    Url(Range<usize>),
}

/// A run of `*`, `_` or `~~`: what of it is left unmatched, and its parts matched as marks.
struct Run {
    ch: u8,
    open: bool,
    close: bool,
    left: Range<usize>,
    marks: Vec<Range<usize>>,
}

/// A style an opening and a closing mark put on what's between them.
#[derive(Clone, Copy)]
enum Flag {
    Bold,
    Italic,
    Strike,
}

/// The segments of `line` from byte `from` on, at `base` in the text, added to `out` (which
/// already holds the line's marks): at most [`MAX_SPANS`] in all.
pub(super) fn spans(line: &str, from: usize, base: usize, out: &mut Vec<Span>) {
    let mut runs = Vec::new();
    let toks = tokens(line, from, &mut runs);
    let styled = matched(&mut runs);
    // Each piece of the line, in order, with its kind.
    let mut pieces: Vec<(Range<usize>, Kind)> = Vec::new();
    for t in &toks {
        match t {
            Tok::Text(r) => pieces.push((r.clone(), Kind::Text)),
            Tok::Run(i) => {
                let Some(run) = runs.get(*i) else { continue };
                let mut parts: Vec<(Range<usize>, Kind)> = run.marks.iter().map(|m| (m.clone(), Kind::Mark)).collect();
                if !run.left.is_empty() {
                    parts.push((run.left.clone(), Kind::Text));
                }
                parts.sort_by_key(|(r, _)| r.start);
                pieces.extend(parts);
            }
            Tok::Code { open, body, close } => {
                pieces.extend([(open.clone(), Kind::Mark), (body.clone(), Kind::Code), (close.clone(), Kind::Mark)]);
            }
            Tok::Link { text, url, end } => {
                pieces.push((text.start.saturating_sub(1)..text.start, Kind::Mark));
                pieces.push((text.clone(), Kind::LinkText));
                pieces.extend([(text.end..url.start, Kind::Mark), (url.clone(), Kind::Url), (url.end..*end, Kind::Mark)]);
            }
            Tok::Url(r) => pieces.push((r.clone(), Kind::Url)),
        }
    }
    // Where each style starts and ends: always where a piece does.
    let mut events: Vec<(usize, i32, Flag)> = styled.iter().flat_map(|(a, z, f)| [(*a, 1, *f), (*z, -1, *f)]).collect();
    events.sort_by_key(|(at, delta, _)| (*at, *delta));
    let (mut bold, mut italic, mut strike) = (0i32, 0i32, 0i32);
    let mut next = 0;
    let first = out.len();
    let at = |r: &Range<usize>| r.start.saturating_add(base)..r.end.saturating_add(base);
    for (r, kind) in pieces.into_iter().filter(|(r, _)| !r.is_empty()) {
        while let Some((_, delta, flag)) = events.get(next).filter(|(p, _, _)| *p <= r.start) {
            let counter = match flag {
                Flag::Bold => &mut bold,
                Flag::Italic => &mut italic,
                Flag::Strike => &mut strike,
            };
            *counter = counter.saturating_add(*delta);
            next += 1;
        }
        let style = Style { bold: bold > 0, italic: italic > 0, strike: strike > 0 };
        // Joined with the one before when they look the same (not with the line's own marks).
        let joinable = out.len() > first;
        if let Some(last) = out.last_mut().filter(|l| joinable && l.kind == kind && l.style == style && l.range.end == at(&r).start) {
            last.range.end = at(&r).end;
            continue;
        }
        if out.len() >= MAX_SPANS.saturating_sub(1) {
            // Past the cap: the rest of the line as it's written.
            let rest = r.start..line.len();
            out.push(Span { kind: Kind::Text, style: Style::default(), range: at(&rest) });
            return;
        }
        out.push(Span { kind, style, range: at(&r) });
    }
}

/// The links of a line from byte `from` on: each one's bytes, and its text's.
pub(super) fn links(line: &str, from: usize) -> Vec<(Range<usize>, Range<usize>)> {
    let mut runs = Vec::new();
    tokens(line, from, &mut runs)
        .into_iter()
        .filter_map(|t| match t {
            Tok::Link { text, end, .. } => Some((text.start.saturating_sub(1)..end, text)),
            _ => None,
        })
        .collect()
}

/// The line cut into pieces from `from` on; the runs of marks go into `runs`.
fn tokens(s: &str, from: usize, runs: &mut Vec<Run>) -> Vec<Tok> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut text_from = from;
    let mut i = from;
    // Lengths of backtick runs known to have no closing run from here on (each searched once).
    let mut no_closer: u64 = 0;
    let flush = |out: &mut Vec<Tok>, a: usize, z: usize| {
        if a < z {
            out.push(Tok::Text(a..z));
        }
    };
    while let Some(&c) = b.get(i) {
        if out.len() >= MAX_TOKENS {
            break;
        }
        match c {
            b'`' => {
                let n = run_of(b, i, b'`');
                let j = i.saturating_add(n);
                let known = n < 64 && no_closer & (1u64 << n) == 0;
                match known.then(|| closer(b, j, n)).flatten() {
                    Some(k) => {
                        flush(&mut out, text_from, i);
                        let end = k.saturating_add(n);
                        out.push(Tok::Code { open: i..j, body: j..k, close: k..end });
                        text_from = end;
                        i = end;
                    }
                    None => {
                        if known {
                            no_closer |= 1u64 << n;
                        }
                        i = j;
                    }
                }
            }
            b'*' | b'_' | b'~' => {
                let n = run_of(b, i, c);
                let j = i.saturating_add(n);
                let (open, close) = flanking(s, i, j, c);
                if (c != b'~' || n == 2) && (open || close) {
                    flush(&mut out, text_from, i);
                    out.push(Tok::Run(runs.len()));
                    runs.push(Run { ch: c, open, close, left: i..j, marks: Vec::new() });
                    text_from = j;
                }
                i = j;
            }
            b'[' => match link(s, i) {
                Some((text, url, end)) => {
                    flush(&mut out, text_from, i);
                    out.push(Tok::Link { text, url, end });
                    text_from = end;
                    i = end;
                }
                None => i = i.saturating_add(1),
            },
            b'h' | b'H' => match bare_url(s, i) {
                Some(end) => {
                    flush(&mut out, text_from, i);
                    out.push(Tok::Url(i..end));
                    text_from = end;
                    i = end;
                }
                None => i = i.saturating_add(1),
            },
            _ => i = i.saturating_add(1),
        }
    }
    flush(&mut out, text_from, b.len());
    out
}

/// How many `c` follow from byte `i` on.
fn run_of(b: &[u8], i: usize, c: u8) -> usize {
    b.get(i..).map_or(0, |r| r.iter().take_while(|x| **x == c).count())
}

/// The next run of exactly `n` backticks from byte `from` on, closing code opened by `n`.
fn closer(b: &[u8], from: usize, n: usize) -> Option<usize> {
    let mut k = from;
    while k < b.len() {
        let p = k.saturating_add(b.get(k..)?.iter().position(|c| *c == b'`')?);
        let m = run_of(b, p, b'`');
        if m == n {
            return Some(p);
        }
        k = p.saturating_add(m.max(1));
    }
    None
}

/// Whether a run of marks (bytes `i..j`, of `c`) may open and may close, by what's around it.
fn flanking(s: &str, i: usize, j: usize, c: u8) -> (bool, bool) {
    let before = s.get(..i).and_then(|x| x.chars().next_back()).unwrap_or(' ');
    let after = s.get(j..).and_then(|x| x.chars().next()).unwrap_or(' ');
    let punct = |x: char| x.is_ascii_punctuation();
    let left = !after.is_whitespace() && (!punct(after) || before.is_whitespace() || punct(before));
    let right = !before.is_whitespace() && (!punct(before) || after.is_whitespace() || punct(after));
    if c == b'_' {
        // Not inside a word (snake_case stays as written).
        (left && (!right || punct(before)), right && (!left || punct(after)))
    } else {
        (left, right)
    }
}

/// `[text](address)` at byte `i`: its text, its address and where it ends.
fn link(s: &str, i: usize) -> Option<(Range<usize>, Range<usize>, usize)> {
    let b = s.as_bytes();
    let from = i.saturating_add(1);
    let k = from.saturating_add(b.get(from..)?.iter().take(MAX_LINK_TEXT).position(|c| *c == b']' || *c == b'[')?);
    if b.get(k) != Some(&b']') || b.get(k.saturating_add(1)) != Some(&b'(') {
        return None;
    }
    let u = k.saturating_add(2);
    let stop = |c: &u8| c.is_ascii_whitespace() || c.is_ascii_control() || matches!(c, b'(' | b')' | b'[' | b']' | b'<' | b'>' | b'"');
    let m = u.saturating_add(b.get(u..)?.iter().take(MAX_URL).position(stop)?);
    if b.get(m) != Some(&b')') {
        return None;
    }
    is_address(s.get(u..m)?).then(|| (from..k, u..m, m.saturating_add(1)))
}

/// An address a link may lead to: the web's, or an email address's.
pub(super) fn is_address(url: &str) -> bool {
    let starts = |p: &str| url.get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p));
    let host = |n: usize| url.as_bytes().get(n).is_some_and(|c| c.is_ascii_alphanumeric() || *c >= 0x80);
    url.len() <= MAX_URL && ((starts("https://") && host(8)) || (starts("http://") && host(7)) || (starts("mailto:") && url.contains('@')))
}

/// A bare address at byte `i` (`https://…`, not inside a word): where it ends, its last
/// punctuation left to the sentence.
fn bare_url(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    if i > 0 && b.get(i.saturating_sub(1)).is_some_and(|p| p.is_ascii_alphanumeric() || *p >= 0x80) {
        return None;
    }
    let rest = s.get(i..)?;
    let scheme = ["https://", "http://"].iter().find(|p| rest.get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p)))?.len();
    let start = i.saturating_add(scheme);
    let stop = |c: &u8| c.is_ascii_whitespace() || c.is_ascii_control() || matches!(c, b'<' | b'>' | b'"' | b'`');
    let len = b.get(start..)?.iter().take(MAX_URL.saturating_add(1)).take_while(|c| !stop(c)).count();
    if len > MAX_URL {
        return None;
    }
    let mut end = start.saturating_add(len);
    while end > start
        && b.get(end.saturating_sub(1)).is_some_and(|c| matches!(c, b'.' | b',' | b';' | b':' | b'!' | b'?' | b'\'' | b'*' | b'_' | b'~'))
    {
        end = end.saturating_sub(1);
    }
    let body = b.get(i..end)?;
    let opened = body.iter().filter(|c| **c == b'(').count();
    let closed = body.iter().filter(|c| **c == b')').count();
    if end > start && b.get(end.saturating_sub(1)) == Some(&b')') && closed > opened {
        end = end.saturating_sub(1);
    }
    let host = b.get(start).is_some_and(|c| c.is_ascii_alphanumeric() || *c >= 0x80);
    (host && end > start).then_some(end)
}

/// The runs of marks matched, opening with closing, from the inside out: what each pair styles.
fn matched(runs: &mut [Run]) -> Vec<(usize, usize, Flag)> {
    let mut styled = Vec::new();
    // The runs that may still open, in order.
    let mut stack: Vec<usize> = Vec::new();
    for d in 0..runs.len() {
        let Some((ch, open, close)) = runs.get(d).map(|r| (r.ch, r.open, r.close)) else { continue };
        // A closing run takes what it can from the opening ones before it.
        while close && let Some(closing) = runs.get(d).map(|r| r.left.clone()).filter(|l| !l.is_empty()) {
            let found = stack.iter().rev().take(LOOKBACK).position(|o| runs.get(*o).is_some_and(|r| r.ch == ch && !r.left.is_empty()));
            let Some(si) = found.map(|p| stack.len().saturating_sub(p.saturating_add(1))) else { break };
            let Some(o) = stack.get(si).copied() else { break };
            let Some(opening) = runs.get(o).map(|r| r.left.clone()) else { break };
            let n = if ch == b'~' || (opening.len() >= 2 && closing.len() >= 2) { 2 } else { 1 };
            if opening.len() < n || closing.len() < n {
                break;
            }
            let flag = match (ch, n) {
                (b'~', _) => Flag::Strike,
                (_, 2) => Flag::Bold,
                _ => Flag::Italic,
            };
            styled.push((opening.end, closing.start, flag));
            if let Some(r) = runs.get_mut(o) {
                r.marks.push(opening.end.saturating_sub(n)..opening.end);
                r.left.end = opening.end.saturating_sub(n);
            }
            if let Some(r) = runs.get_mut(d) {
                r.marks.push(closing.start..closing.start.saturating_add(n));
                r.left.start = closing.start.saturating_add(n);
            }
            // The marks between them can no longer match.
            stack.truncate(si.saturating_add(1));
            if runs.get(o).is_some_and(|r| r.left.is_empty()) {
                stack.pop();
            }
        }
        if open && runs.get(d).is_some_and(|r| !r.left.is_empty()) {
            stack.push(d);
        }
    }
    styled
}
