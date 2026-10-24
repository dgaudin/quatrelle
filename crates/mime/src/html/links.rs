//! Web and mail addresses written out in a message's text (`https://…`, `www.…`, `name@host`),
//! and network shares (`\\server\share\folder`, Windows' way of naming a file on another
//! computer): found so they open as links do, the way Outlook makes them links though the
//! message didn't (a plain-text message, or an HTML one whose sender typed the address without
//! a link). A share is kept as `file://server/share/folder`, whatever form it came in, and
//! opened the way the system opens one (`smb://` on Linux and macOS).

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

/// `%xx` escapes undone (an address's path as its characters).
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        let hex = |k: usize| b.get(k).and_then(|h| (*h as char).to_digit(16));
        match (c, hex(i + 1), hex(i + 2)) {
            (b'%', Some(h), Some(l)) => {
                out.push(u8::try_from(h * 16 + l).unwrap_or(b'?'));
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// A path part as it goes in an address: what would end or change one escaped.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            c => out.push(c),
        }
    }
    out
}

/// A network share as a link: `\\server\share\folder`, `file://server/share/folder`,
/// `file:///\\server\share` (Outlook's HTML) or `smb://server/share`, as
/// `file://server/share/folder`. `None` for anything else: a file on this computer
/// (`file:///home/…`, `C:\…`), or a share without a server and a share name.
pub fn share_link(target: &str) -> Option<String> {
    let t = target.trim();
    let lower = t.to_ascii_lowercase();
    let rest = if let Some(r) = t.strip_prefix("\\\\") {
        r
    } else {
        let after = if lower.starts_with("file:") {
            t.get(5..)?
        } else if lower.starts_with("smb:") {
            t.get(4..)?
        } else {
            return None;
        };
        // `//server/…`; with no server (`file:///…`), a path on this computer, unless the path
        // itself names one (`file:///\\server\share`, `file:////server/share`).
        let s = after.strip_prefix("//")?;
        let server_end = s.find(['/', '\\']).unwrap_or(s.len());
        if server_end > 0 {
            s
        } else {
            let path = s.get(1..)?;
            if !path.starts_with(['/', '\\']) {
                return None;
            }
            path.trim_start_matches(['/', '\\'])
        }
    };
    let parts: Vec<String> = rest.split(['\\', '/']).filter(|p| !p.is_empty()).map(unescape).take(64).collect();
    let (host, share) = (parts.first()?, parts.get(1)?);
    let host_ok = !host.is_empty()
        && host.len() < 256
        && host.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && !host.eq_ignore_ascii_case("localhost")
        && !host.starts_with("127.");
    let names_ok = parts.iter().all(|p| p.len() < 256 && !p.contains(['<', '>', '"', '|', '*', '?', ':']) && !p.chars().any(char::is_control));
    (host_ok && names_ok && !share.is_empty())
        .then(|| format!("file://{}/{}", host.to_ascii_lowercase(), parts.iter().skip(1).map(|p| escape(p)).collect::<Vec<_>>().join("/")))
}

/// A share's link ([`share_link`]) as its server and Windows' way of writing it
/// (`\\server\share\folder`): to show, and to open on Windows.
pub fn share_parts(link: &str) -> Option<(String, String)> {
    let rest = link.strip_prefix("file://")?;
    let (host, path) = rest.split_once('/')?;
    let unc = format!("\\\\{host}\\{}", path.split('/').map(unescape).collect::<Vec<_>>().join("\\"));
    (!host.is_empty() && !path.is_empty()).then(|| (host.to_string(), unc))
}

/// The shares written in `text` (`\\server\share\…`, `<\\server\share\a folder>` with its
/// spaces, `file://…`, `smb://…`), each with its link.
fn find_shares(text: &str) -> Vec<(Range<usize>, String)> {
    let mut out: Vec<(Range<usize>, String)> = Vec::new();
    let lower = text.to_ascii_lowercase();
    for begin in ["\\\\", "file:", "smb://"] {
        let mut at = 0;
        while out.len() < MAX
            && let Some(i) = lower.get(at..).and_then(|r| r.find(begin))
        {
            let start = at + i;
            let before = text.get(..start).and_then(|b| b.chars().last());
            let rest = text.get(start..).unwrap_or("");
            // In angle brackets, as Outlook writes one with spaces: up to the closing one.
            let closed = (before == Some('<')).then(|| rest.find('>')).flatten();
            // (A share's path goes on through its backslashes, which end a web address.)
            let end = closed.unwrap_or_else(|| rest.find(|c: char| c != '\\' && (ends(c) || c == '*')).unwrap_or(rest.len()));
            let found = if closed.is_some() { rest.get(..end).unwrap_or("") } else { trim_end(rest.get(..end).unwrap_or("")) };
            let word = before.is_none_or(|c| !(c.is_alphanumeric() || c == '\\'));
            match share_link(found).filter(|_| word && found.len() <= MAX_LEN) {
                Some(url) => {
                    out.push((start..start + found.len(), url));
                    at = start + found.len().max(1);
                }
                // Nothing further in this word starts one either.
                None => at = start + end.max(begin.len()),
            }
        }
    }
    out
}

/// The addresses written in `text`: where each is (a byte range) and what it opens
/// (`https://…` as written, `http://www.…`, `mailto:…`, `file://server/share/…`), in order,
/// never overlapping.
pub fn find_links(text: &str) -> Vec<(Range<usize>, String)> {
    let mut out: Vec<(Range<usize>, String)> = if text.contains("\\\\") || text.contains(':') { find_shares(text) } else { Vec::new() };
    let shares = out.len();
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
    // A web or mail address inside a share's path (`\\server\share\a@b.fr`) is the share's.
    if shares > 0 {
        let (shares, others) = out.split_at(shares);
        let mut all: Vec<(Range<usize>, String)> = shares.to_vec();
        all.extend(others.iter().filter(|(r, _)| !shares.iter().any(|(s, _)| s.start < r.end && r.start < s.end)).cloned());
        all.sort_by_key(|(r, _)| r.start);
        out = all;
    }
    out
}

/// Spans with the addresses written in their text made links of their own (the text before and
/// after as it was). Spans that are links, pictures or code already stay as they are.
pub fn link_written_addresses(spans: Vec<Span>) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for mut sp in spans {
        // A link to a share, however the message wrote it, as shares are kept.
        if let Some(share) = sp.link.as_deref().and_then(share_link) {
            sp.link = Some(share);
        }
        let maybe =
            sp.text.contains("://") || sp.text.contains("www.") || sp.text.contains('@') || sp.text.contains("\\\\") || sp.text.contains("file:");
        if sp.link.is_some() || sp.image.is_some() || sp.code || !maybe {
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
            found("Voir http://wdfdfds.sdds.fr, ou (https://example.com/a?b=1#c). Et www.proginov.com!"),
            [
                ("http://wdfdfds.sdds.fr", "http://wdfdfds.sdds.fr".to_string()),
                ("https://example.com/a?b=1#c", "https://example.com/a?b=1#c".to_string()),
                ("www.proginov.com", "http://www.proginov.com".to_string()),
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

    /// Network shares, Windows' `\\server\share\folder` as people write them (and Outlook's
    /// HTML links to them), become `file://server/share/folder` links; files on this computer
    /// and broken ones don't.
    #[test]
    fn shares_written_out_are_found() {
        assert_eq!(
            found(r"Les fichiers sont sur \\srv-fichiers\Commun\Projets\2026 et <\\nas.proginov.local\Partage RH\Mes docs\Note de frais.xlsx>."),
            [
                (r"\\srv-fichiers\Commun\Projets\2026", "file://srv-fichiers/Commun/Projets/2026".to_string()),
                (
                    r"\\nas.proginov.local\Partage RH\Mes docs\Note de frais.xlsx",
                    "file://nas.proginov.local/Partage%20RH/Mes%20docs/Note%20de%20frais.xlsx".to_string()
                ),
            ]
        );
        assert_eq!(found("ouvrir file://srv/commun/a%20b, merci")[0].1, "file://srv/commun/a%20b");
        assert_eq!(found("smb://nas/backup/2026")[0].1, "file://nas/backup/2026");
        // Outlook's HTML link, and the address inside a share's path is the share's.
        assert_eq!(share_link(r"file:///\\SRV\Commun\a.docx").as_deref(), Some("file://srv/Commun/a.docx"));
        assert_eq!(found(r"\\srv\boites\jean@example.fr\notes").len(), 1);
        // Shown, and opened on Windows, as Windows writes it.
        assert_eq!(share_parts("file://nas/Partage%20RH/a.txt"), Some(("nas".to_string(), r"\\nas\Partage RH\a.txt".to_string())));
        // Not shares: a file on this computer, no share name, junk.
        for text in [r"C:\Users\me\file.txt", "file:///home/me/notes.txt", r"\\server", r"\\localhost\c$", r"\\\\\\", r"a\\b\c", "file:/x/y"] {
            assert!(found(text).is_empty(), "{text}: {:?}", found(text));
        }
        // Hostile input stays linear and bounded.
        let junk = r"\\".repeat(50_000) + &"file:".repeat(20_000) + &r"\\a\b ".repeat(5_000);
        assert!(find_links(&junk).len() <= MAX * 2);
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
