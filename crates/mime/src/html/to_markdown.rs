// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A received message's formatting written as Markdown, for the reading pane's Markdown view:
//! made from the lines the formatted view shows ([`html_to_rich`]: the same reading of the HTML,
//! within the same caps, the hidden text and the tracking pictures already left out), so the two
//! say the same. Headings as `#`, emphasis as `**`, `*` and `~~`, code in backticks and fenced
//! blocks, lists with their bullets, numbers and indents, quotes as `>`, links as `[text](url)`
//! (as the message has them, or cleaned of their tracking tags as the formatted view shows them),
//! tables as `| … |` rows with a `|---|` rule under the first, pictures as `![description](url)`
//! text, never fetched.
//!
//! What is hostile in it: the HTML, read by html2text as for the formatted view. Here the lines
//! are walked once, without recursion, and what is written stops at [`MARKDOWN_WRITTEN_MAX`].

use super::*;

/// The most Markdown written of one message, in bytes.
pub const MARKDOWN_WRITTEN_MAX: usize = 4 << 20;

/// What a formatted line is, for Markdown.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Blank,
    Rule,
    /// A table's border (`─┬─`, `─┼─`): left out.
    Border,
    /// A table's row: its cells between `│`.
    Row,
    /// All code: a line of a fenced block.
    Code,
    Text,
}

/// Box-drawing characters (html2text's table borders).
fn is_box(c: char) -> bool {
    ('\u{2500}'..='\u{257F}').contains(&c)
}

fn kind_of(line: &[Span]) -> Kind {
    if line.iter().any(|s| s.rule) {
        return Kind::Rule;
    }
    if line.iter().all(|s| s.text.trim().is_empty() && s.image.is_none()) {
        return Kind::Blank;
    }
    let whole: String = line.iter().map(|s| s.text.as_str()).collect();
    if whole.contains('│') {
        if whole.chars().all(|c| is_box(c) || c == ' ') && whole.chars().any(|c| c != '│' && c != ' ') {
            return Kind::Border;
        }
        return Kind::Row;
    }
    // A picture alone on its line, without a description (its text empty): one all the same.
    if line.iter().any(|s| s.image.is_some()) {
        return Kind::Text;
    }
    if whole.chars().all(|c| is_box(c) || c == ' ') {
        return Kind::Border;
    }
    if line.iter().all(|s| s.code || s.text.is_empty()) && line.iter().all(|s| s.image.is_none() && s.heading == 0) {
        return Kind::Code;
    }
    Kind::Text
}

/// A formatted message's lines as Markdown (`clean_links`: links without their tracking tags,
/// as the formatted view shows them). At most [`MARKDOWN_WRITTEN_MAX`] bytes.
pub fn rich_markdown(lines: &RichText, clean_links: bool) -> String {
    let kinds: Vec<Kind> = lines.iter().map(Vec::as_slice).map(kind_of).collect();
    // The kind of the next line that isn't blank, for each line: a blank line between two lines
    // of code stays inside their block.
    let mut next_filled = vec![Kind::Blank; kinds.len()];
    let mut seen = Kind::Blank;
    for (i, k) in kinds.iter().enumerate().rev() {
        if let Some(n) = next_filled.get_mut(i) {
            *n = seen;
        }
        if *k != Kind::Blank {
            seen = *k;
        }
    }
    let mut out = String::new();
    let (mut fenced, mut in_table) = (false, false);
    for (i, line) in lines.iter().enumerate() {
        if out.len() >= MARKDOWN_WRITTEN_MAX {
            break;
        }
        let kind = kinds.get(i).copied().unwrap_or(Kind::Blank);
        let after = next_filled.get(i).copied().unwrap_or(Kind::Blank);
        if fenced && kind != Kind::Code && !(kind == Kind::Blank && after == Kind::Code) {
            out.push_str("```\n");
            fenced = false;
        }
        if !matches!(kind, Kind::Row | Kind::Border) {
            in_table = false;
        }
        match kind {
            Kind::Blank => out.push('\n'),
            Kind::Rule => out.push_str("---\n"),
            Kind::Border => {}
            Kind::Row => {
                let cells = row_cells(line, clean_links);
                out.push_str("| ");
                out.push_str(&cells.join(" | "));
                out.push_str(" |\n");
                if !in_table {
                    out.push('|');
                    out.push_str(&"---|".repeat(cells.len().max(1)));
                    out.push('\n');
                    in_table = true;
                }
            }
            Kind::Code => {
                if !fenced {
                    out.push_str("```\n");
                    fenced = true;
                }
                for sp in line {
                    out.push_str(&sp.text.replace('\n', " "));
                }
                out.push('\n');
            }
            Kind::Text => {
                let (prefix, _, skip) = structure(line);
                let heading = line.iter().map(|s| s.heading).max().unwrap_or(0).min(6);
                if heading > 0 {
                    out.push_str(&"#".repeat(usize::from(heading)));
                    out.push(' ');
                }
                out.push_str(&prefix);
                let body = inline(line, skip, clean_links, false);
                // What would start a heading or a rule where the message has text.
                let t = body.trim_start();
                if heading == 0 && (t.starts_with('#') || (t.len() >= 3 && t.chars().all(|c| matches!(c, '-' | '=' | '+')))) {
                    out.push('\\');
                }
                out.push_str(&body);
                // A line followed by another of text stays a line of its own (a hard break), as
                // a signature's lines are, rather than run into one paragraph.
                let next_text =
                    kinds.get(i + 1) == Some(&Kind::Text) && lines.get(i + 1).is_some_and(|n| n.iter().all(|s| s.heading == 0) && says_more(n));
                if heading == 0 && !body.trim().is_empty() && next_text {
                    out.push_str("  ");
                }
                out.push('\n');
            }
        }
    }
    if fenced && out.len() < MARKDOWN_WRITTEN_MAX {
        out.push_str("```\n");
    }
    let end = out.trim_end_matches('\n').len();
    out.truncate(end);
    super::markdown::cut(&out, MARKDOWN_WRITTEN_MAX).to_string()
}

/// A line's start as Markdown reads it: indents, quotes' `>` and a bullet (`• ` written `- `) or
/// a number; whether it is a list item; and how many of the line's characters it took.
fn structure(line: &[Span]) -> (String, bool, usize) {
    let whole: String = line.iter().take(4).map(|s| s.text.as_str()).collect();
    let chars: Vec<char> = whole.chars().take(256).collect();
    let mut i = 0;
    loop {
        while chars.get(i) == Some(&' ') {
            i += 1;
        }
        if chars.get(i) == Some(&'>') {
            i += 1;
            if chars.get(i) == Some(&' ') {
                i += 1;
            }
            continue;
        }
        break;
    }
    let mut prefix: String = chars.get(..i).unwrap_or_default().iter().collect();
    let rest = chars.get(i..).unwrap_or_default();
    let bullet = matches!(rest, ['•' | '*' | '-' | '+', ' ', ..]);
    let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
    let number = (1..=9).contains(&digits) && matches!(rest.get(digits..), Some(['.' | ')', ' ', ..]));
    if bullet {
        match rest.first() {
            Some('•') | None => prefix.push('-'),
            Some(c) => prefix.push(*c),
        }
        prefix.push(' ');
        i += 2;
    } else if number {
        prefix.extend(rest.get(..digits + 2).unwrap_or_default());
        i += digits + 2;
    }
    (prefix, bullet || number, i)
}

/// Whether a line holds text past its start ([`structure`]) and isn't a list item: a line a
/// hard break leads to.
fn says_more(line: &[Span]) -> bool {
    let (_, listed, skip) = structure(line);
    !listed && line.iter().flat_map(|s| s.text.chars()).skip(skip).any(|c| !c.is_whitespace())
}

/// The cells of a table's row, each its spans as Markdown (`|` in them escaped).
fn row_cells(line: &[Span], clean_links: bool) -> Vec<String> {
    let mut cells: Vec<Vec<Span>> = vec![Vec::new()];
    for sp in line {
        let mut parts = sp.text.split('│');
        if let Some(first) = parts.next()
            && let Some(cell) = cells.last_mut()
        {
            cell.push(Span { text: first.to_string(), ..sp.clone() });
        }
        for part in parts {
            cells.push(vec![Span { text: part.to_string(), ..sp.clone() }]);
        }
    }
    cells.iter().map(|c| inline(c, 0, clean_links, true).trim().to_string()).collect()
}

/// Spans as Markdown, from character `skip` of the first (what [`structure`] took): runs of one
/// look together, a link around the runs it covers.
fn inline(line: &[Span], skip: usize, clean_links: bool, in_table: bool) -> String {
    // The spans as Markdown sees them: what only a colour or a size told apart, one.
    let mut left = skip;
    let mut runs: Vec<Span> = Vec::new();
    for sp in line {
        let mut text: String = sp.text.replace('\n', " ");
        if left > 0 {
            let n = text.chars().count();
            text = text.chars().skip(left).collect();
            left = left.saturating_sub(n);
        }
        if sp.strike {
            text = text.replace('\u{336}', "");
        }
        if text.is_empty() && sp.image.is_none() {
            continue;
        }
        let code = sp.code && !in_table;
        match runs.last_mut() {
            Some(last)
                if last.image.is_none()
                    && sp.image.is_none()
                    && (last.bold, last.italic, last.strike, last.code, &last.link) == (sp.bold, sp.italic, sp.strike, code, &sp.link) =>
            {
                last.text.push_str(&text);
            }
            _ => runs.push(Span {
                text,
                bold: sp.bold,
                italic: sp.italic,
                strike: sp.strike,
                code,
                link: sp.link.clone(),
                image: sp.image.clone(),
                ..Default::default()
            }),
        }
    }
    let mut out = String::new();
    let mut i = 0;
    while let Some(first) = runs.get(i) {
        let link = first.link.clone();
        let end = runs.iter().skip(i).position(|r| r.link != link).map_or(runs.len(), |p| p + i);
        let group = runs.get(i..end).unwrap_or_default();
        let inner: String = group.iter().map(|r| run(r, in_table)).collect();
        match link {
            Some(url) => {
                let plain: String = group.iter().map(|r| r.text.as_str()).collect();
                let dest = if clean_links { link_destination(&url) } else { url.clone() };
                let bare = group.iter().all(|r| !(r.bold || r.italic || r.strike || r.code) && r.image.is_none())
                    && (plain.trim() == url.trim() || url.strip_prefix("mailto:").is_some_and(|m| m == plain.trim()));
                if bare {
                    out.push_str(&plain);
                } else {
                    // White space at its ends stays outside it.
                    let (lead, core, trail) = edges(&inner);
                    out.push_str(&format!("{lead}[{core}]({}){trail}", destination(&dest)));
                }
            }
            None => out.push_str(&inner),
        }
        i = end;
    }
    out
}

/// One run of text as Markdown: a picture as `![description](address)`, code in enough
/// backticks, emphasis around the words (the white space at its ends outside it).
fn run(r: &Span, in_table: bool) -> String {
    if let Some(src) = &r.image {
        let alt = escape(r.text.trim(), in_table).replace(['[', ']'], "");
        return format!("![{alt}]({})", destination(src));
    }
    if r.code {
        let ticks = longest_run(&r.text, '`') + 1;
        let fence = "`".repeat(ticks);
        let pad = if r.text.starts_with('`') || r.text.ends_with('`') { " " } else { "" };
        return format!("{fence}{pad}{}{pad}{fence}", r.text);
    }
    let text = escape(&r.text, in_table);
    let (lead, core, trail) = edges(&text);
    if core.is_empty() || !(r.bold || r.italic || r.strike) {
        return text;
    }
    let open = format!("{}{}{}", if r.bold { "**" } else { "" }, if r.italic { "*" } else { "" }, if r.strike { "~~" } else { "" });
    let close: String = open.chars().rev().collect();
    format!("{lead}{open}{core}{close}{trail}")
}

/// `s` as its white space before, what's between, and its white space after.
fn edges(s: &str) -> (&str, &str, &str) {
    let core = s.trim();
    let start = s.len() - s.trim_start().len();
    let end = start + core.len();
    (s.get(..start).unwrap_or(""), core, s.get(end..).unwrap_or(""))
}

/// The longest run of `c` in `s`.
fn longest_run(s: &str, c: char) -> usize {
    let (mut most, mut now) = (0, 0);
    for x in s.chars() {
        now = if x == c { now + 1 } else { 0 };
        most = most.max(now);
    }
    most
}

/// An address as Markdown takes it: between angle brackets when it has spaces or brackets.
fn destination(url: &str) -> String {
    let url = url.trim().replace(['\n', '\r'], "");
    if url.contains([' ', '(', ')', '<', '>']) { format!("<{}>", url.replace('<', "%3C").replace('>', "%3E")) } else { url }
}

/// Text with what Markdown would read as marks escaped: `\`, `*`, `` ` ``, `[`, `]`, a `_` at
/// a word's edge, a `<` that would open a tag, a `~~`, and `|` in a table.
fn escape(t: &str, in_table: bool) -> String {
    let chars: Vec<char> = t.chars().collect();
    let mut out = String::with_capacity(t.len() + 8);
    for (i, c) in chars.iter().copied().enumerate() {
        let prev = i.checked_sub(1).and_then(|p| chars.get(p)).copied();
        let next = chars.get(i + 1).copied();
        let edge = !prev.is_some_and(char::is_alphanumeric) || !next.is_some_and(char::is_alphanumeric);
        let escaped = match c {
            '\\' | '*' | '`' | '[' | ']' => true,
            '_' => edge,
            '<' => next.is_some_and(|n| n.is_ascii_alphabetic() || matches!(n, '/' | '!' | '?')),
            '~' => next == Some('~') || prev == Some('~'),
            '|' => in_table,
            _ => false,
        };
        if escaped {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// An HTML body as Markdown ([`rich_markdown`] of [`html_to_rich`]), its links as it has them.
pub fn html_markdown(html: &str) -> String {
    rich_markdown(&html_to_rich(html), false)
}

#[cfg(test)]
mod tests;
