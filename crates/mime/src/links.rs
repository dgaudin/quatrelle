//! Web and mail addresses written out in a message's text (`https://…`, `www.…`, `name@host`):
//! found so they open as links do, the way Outlook makes them links though the message didn't
//! (a plain-text message, or an HTML one whose sender typed the address without a link).

use std::ops::Range;

use crate::Span;

/// Most addresses found in one text.
const MAX: usize = 500;
/// Longest address taken as one.
const MAX_LEN: usize = 4096;

/// Characters that end an address.
fn ends(c: char) -> bool {
    c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`' | '{' | '}' | '|' | '\\' | '^' | '«' | '»' | '“' | '”')
}

/// Sentence punctuation after an address isn't part of it, nor a closing bracket it didn't open.
fn trim_end(link: &str) -> &str {
    let mut link = link;
    loop {
        let t = link.trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '*']);
        let t = [(')', '('), (']', '['), ('}', '{')]
            .iter()
            .find(|(close, open)| t.ends_with(*close) && t.matches(*open).count() < t.matches(*close).count())
            .and_then(|_| t.get(..t.len() - 1))
            .unwrap_or(t);
        if t.len() == link.len() {
            return link;
        }
        link = t;
    }
}

/// A host name as one is written: labels of letters, digits and hyphens, a dot between them,
/// the last of two letters or more.
fn host_ok(host: &str) -> bool {
    let labels: Vec<&str> = host.split('.').collect();
    host.len() < 256
        && labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.len() < 64 && !l.starts_with('-') && !l.ends_with('-') && l.chars().all(|c| c.is_alphanumeric() || c == '-'))
        && labels.last().is_some_and(|tld| tld.chars().count() >= 2 && tld.chars().all(char::is_alphabetic))
}

/// The addresses written in `text`: where each is (a byte range) and what it opens
/// (`https://…` as written, `http://www.…`, `mailto:…`), in order, never overlapping.
pub fn find_links(text: &str) -> Vec<(Range<usize>, String)> {
    let mut out: Vec<(Range<usize>, String)> = Vec::new();
    let lower = text.to_ascii_lowercase();
    // A word starts here: the text's start, or after something that isn't part of a word.
    let word_start =
        |at: usize| text.get(..at).and_then(|b| b.chars().last()).is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '.' | '@' | '/' | '-' | '_')));
    // Where each beginning comes next, looked for again only once passed: looking for all of
    // them from every place the scan stops made hostile text (a line of `www.www.www.…`)
    // cost seconds.
    const BEGINNINGS: [&str; 4] = ["https://", "http://", "www.", "@"];
    let mut next: [Option<Option<usize>>; 4] = [None; 4];
    let mut at = 0;
    while at < text.len() && out.len() < MAX {
        for (k, p) in BEGINNINGS.iter().enumerate() {
            if next[k].is_none_or(|n| n.is_some_and(|n| n < at)) {
                next[k] = Some(lower.get(at..).and_then(|r| r.find(p)).map(|i| at + i));
            }
        }
        let web = BEGINNINGS.iter().take(3).zip(next).filter_map(|(p, n)| n.flatten().map(|i| (i, *p))).min_by_key(|(i, _)| *i);
        let mail = next[3].flatten();
        match (web, mail) {
            (Some((start, prefix)), m) if m.is_none_or(|m| start < m) => {
                let end = text.get(start..).and_then(|r| r.find(ends)).map_or(text.len(), |e| start + e);
                let link = trim_end(text.get(start..end).unwrap_or(""));
                let after = if prefix == "www." { link } else { link.get(prefix.len()..).unwrap_or("") };
                let host = after.split(['/', '?', '#', ':']).next().unwrap_or("");
                if word_start(start) && link.len() > prefix.len() && link.len() <= MAX_LEN && host_ok(host) {
                    let url = if prefix == "www." { format!("http://{link}") } else { link.to_string() };
                    out.push((start..start + link.len(), url));
                    at = start + link.len().max(1);
                } else {
                    // Nothing in the rest of this word starts one either.
                    at = end.max(start + prefix.len());
                }
            }
            (_, Some(m)) => {
                // `name@host.tld`: the name back to where it starts, the host on to its end.
                let before = text.get(..m).unwrap_or("");
                let name_len: usize =
                    before.chars().rev().take_while(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-')).map(char::len_utf8).sum();
                let start = m - name_len;
                let host_len: usize =
                    text.get(m + 1..).unwrap_or("").chars().take_while(|c| c.is_alphanumeric() || matches!(c, '.' | '-')).map(char::len_utf8).sum();
                let host = text.get(m + 1..m + 1 + host_len).unwrap_or("").trim_end_matches(['.', '-']);
                let name = text.get(start..m).unwrap_or("").trim_start_matches('.');
                let start = m - name.len();
                if !name.is_empty() && name.len() <= 64 && host_ok(host) && word_start(start) {
                    let end = m + 1 + host.len();
                    out.push((start..end, format!("mailto:{}", text.get(start..end).unwrap_or(""))));
                    at = end;
                } else {
                    at = m + 1;
                }
            }
            _ => break,
        }
    }
    out
}

/// Spans with the addresses written in their text made links of their own (the text before and
/// after as it was). Spans that are links, pictures or code already stay as they are.
pub fn link_written_addresses(spans: Vec<Span>) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for sp in spans {
        if sp.link.is_some() || sp.image.is_some() || sp.code || !(sp.text.contains("://") || sp.text.contains("www.") || sp.text.contains('@')) {
            out.push(sp);
            continue;
        }
        let found = find_links(&sp.text);
        if found.is_empty() {
            out.push(sp);
            continue;
        }
        let mut at = 0;
        for (range, url) in found {
            if range.start > at {
                out.push(Span { text: sp.text.get(at..range.start).unwrap_or("").to_string(), ..sp.clone() });
            }
            out.push(Span { text: sp.text.get(range.clone()).unwrap_or("").to_string(), link: Some(url), ..sp.clone() });
            at = range.end;
        }
        if at < sp.text.len() {
            out.push(Span { text: sp.text.get(at..).unwrap_or("").to_string(), ..sp.clone() });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> Vec<(&str, String)> {
        find_links(text).into_iter().map(|(r, u)| (text.get(r).unwrap_or(""), u)).collect()
    }

    #[test]
    fn addresses_written_out_are_found() {
        assert_eq!(
            found("Voir http://wdfdfds.sdds.fr, ou (https://example.com/a?b=1#c). Et www.contoso.example!"),
            [
                ("http://wdfdfds.sdds.fr", "http://wdfdfds.sdds.fr".to_string()),
                ("https://example.com/a?b=1#c", "https://example.com/a?b=1#c".to_string()),
                ("www.contoso.example", "http://www.contoso.example".to_string()),
            ]
        );
        assert_eq!(found("Écrire à jean.dupont@example.fr."), [("jean.dupont@example.fr", "mailto:jean.dupont@example.fr".to_string())]);
        // A bracket the address opened stays; one it didn't goes.
        assert_eq!(found("(voir https://fr.wikipedia.org/wiki/Rust_(langage))")[0].0, "https://fr.wikipedia.org/wiki/Rust_(langage)");
        // Not addresses.
        for text in [
            "http://",
            "www.",
            "a@b",
            "x@localhost",
            "@user",
            "http://nohost",
            "version 1.2.3",
            "awww.example.com",
            "mailto",
            "x@-.fr",
            "Tél.:\u{a0}+33",
        ] {
            assert!(found(text).is_empty(), "{text}: {:?}", found(text));
        }
        // Two in a row, and an address inside a web one isn't found again.
        let both = found("https://a.example/p?u=me@x.example www.b.example");
        assert_eq!(both.iter().map(|(t, _)| *t).collect::<Vec<_>>(), ["https://a.example/p?u=me@x.example", "www.b.example"]);
        // Hostile input: a long run without spaces, many '@', multibyte text.
        let junk = "a@".repeat(10_000) + &"é".repeat(10_000) + &"www.".repeat(5_000);
        assert!(find_links(&junk).len() <= MAX);
        let accents = "Café — rendez-vous sur https://café.example/menu ou écrivez à zoé@café.example";
        assert_eq!(found(accents).len(), 2);
    }

    #[test]
    fn spans_get_links_of_their_own() {
        let sp = Span { text: "Site : www.example.com, merci".into(), bold: true, ..Default::default() };
        let out = link_written_addresses(vec![sp, Span { text: "lien".into(), link: Some("https://x.example".into()), ..Default::default() }]);
        let texts: Vec<(&str, Option<&str>)> = out.iter().map(|s| (s.text.as_str(), s.link.as_deref())).collect();
        assert_eq!(
            texts,
            [("Site : ", None), ("www.example.com", Some("http://www.example.com")), (", merci", None), ("lien", Some("https://x.example"))]
        );
        assert!(out[1].bold, "its formatting kept");
    }
}
