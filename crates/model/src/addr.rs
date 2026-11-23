// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Email addresses: what the To/Cc boxes accept, and how names are shown.

use std::fmt;

pub mod entries;
pub(crate) mod providers;
pub mod trust;

use serde::{Deserialize, Serialize};

pub use entries::*;

/// A display name and an address (`Zoe Park <zoe@example.org>`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EmailAddress {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
}

impl EmailAddress {
    pub fn new(name: impl Into<String>, email: impl Into<String>) -> Self {
        EmailAddress { name: name.into(), email: email.into() }
    }

    /// The name if there is one, else the address.
    pub fn display(&self) -> &str {
        if self.name.trim().is_empty() { &self.email } else { self.name.trim() }
    }

    /// Up to two initials for the avatar (`Zoe Park` → `ZP`, `ops@x.org` → `O`).
    pub fn initials(&self) -> String {
        let src = self.display();
        let words: Vec<&str> = src.split(|c: char| c.is_whitespace() || c == '.' || c == '_' || c == '@').filter(|w| !w.is_empty()).collect();
        let pick = |w: &str| w.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
        match (self.name.trim().is_empty(), words.as_slice()) {
            (_, []) => "?".to_string(),
            (true, [first, ..]) => pick(first),
            (false, [only]) => pick(only),
            (false, [first, .., last]) => format!("{}{}", pick(first), pick(last)),
        }
    }

    /// A rough check: one `@`, something on both sides, a dot-free local part allowed, no spaces.
    pub fn is_valid(&self) -> bool {
        is_valid_email(&self.email)
    }
}

impl fmt::Display for EmailAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.name.trim().is_empty() {
            write!(f, "{}", self.email)
        } else if self.name.contains([',', ';', '"', '<', '>', '@']) {
            write!(f, "\"{}\" <{}>", self.name.replace('"', "'"), self.email)
        } else {
            write!(f, "{} <{}>", self.name.trim(), self.email)
        }
    }
}

pub fn is_valid_email(s: &str) -> bool {
    let s = s.trim();
    if entries::is_group_ref_text(s) {
        return false;
    }
    let Some((local, domain)) = s.rsplit_once('@') else { return false };
    !local.is_empty()
        && !domain.is_empty()
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !s.chars().any(|c| c.is_whitespace() || c == '<' || c == '>' || c == ',' || c == ';')
        && !domain.contains('@')
}

/// Parse what was typed in a To/Cc/Bcc box: addresses separated by `,` or `;`, each either bare
/// (`zoe@example.org`) or with a name (`Zoe Park <zoe@example.org>`, `"Park, Zoe" <zoe@…>`).
/// Returns the invalid entries as the error.
pub fn parse_list(input: &str) -> Result<Vec<EmailAddress>, String> {
    let mut out = Vec::new();
    let mut bad = Vec::new();
    for part in split_entries(input) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match parse_one(part) {
            Some(a) => out.push(a),
            None => bad.push(part.to_string()),
        }
    }
    if bad.is_empty() { Ok(out) } else { Err(bad.join(", ")) }
}

/// The addresses of [`parse_list`]'s reading that are whole: for a draft, which may hold one
/// still being typed. An entry that doesn't read (a quote left open swallowing the entries
/// after it) is read again with the quotes ignored, so that no address that reads whole is lost.
pub fn parse_valid(input: &str) -> Vec<EmailAddress> {
    let mut out = Vec::new();
    for entry in split_entries(input).into_iter().take(MAX_ENTRIES) {
        match parse_one(entry) {
            Some(a) => out.push(a),
            None => out.extend(entries::split_entries_blind(entry).into_iter().take(MAX_ENTRIES).filter_map(parse_one)),
        }
    }
    out
}

/// One entry typed (`zoe@x`, `Zoe Park <zoe@x>`, `"Park, Zoe" <zoe@x>`) as an address; `None`
/// when it isn't one (a contact group's reference included), or when it holds more than one
/// (a quote left open makes the next address part of this one's name, where it would be lost).
pub fn parse_one(s: &str) -> Option<EmailAddress> {
    if let (Some(lt), Some(gt)) = (s.rfind('<'), s.rfind('>')) {
        if gt < lt {
            return None;
        }
        // Nothing after it but what ends an entry (`;`, `,`) or closes a quote around all of it
        // (`"Zoe <zoe@x>"`): `Jo 6" Doe <jo@x>; bob@y` is one piece, and bob would go unsaid.
        let after = s.get(gt + 1..).unwrap_or_default().trim_matches(|c: char| c.is_whitespace() || matches!(c, ';' | ',' | '"'));
        if !after.is_empty() {
            return None;
        }
        // Nor a name with an address of its own in it, unless one pair of quotes around all of
        // it holds it: a quote anywhere else (`Jo 6" Doe`) may be left open, and what follows it
        // is read as part of the name.
        let name = s.get(..lt)?;
        let held = name.trim().strip_prefix('"').and_then(|n| n.strip_suffix('"')).is_some_and(|n| !n.contains('"'));
        if name.contains(['<', '>']) && !held {
            return None;
        }
        let email = s.get(lt + 1..gt)?.trim().to_string();
        let name = name.trim().trim_matches('"').trim().to_string();
        let a = EmailAddress { name, email };
        return a.is_valid().then_some(a);
    }
    let a = EmailAddress { name: String::new(), email: s.trim().to_string() };
    a.is_valid().then_some(a)
}

/// Format a list for a header-like line (`Zoe Park <zoe@…>; ops@…`).
pub fn format_list(list: &[EmailAddress]) -> String {
    list.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
}

/// Recipients to put in a field being written: the list with a separator after it, so an
/// address typed next starts an entry of its own (empty for no one).
pub fn format_field(list: &[EmailAddress]) -> String {
    if list.is_empty() { String::new() } else { format!("{}; ", format_list(list)) }
}

/// Short form for the reading pane (`Zoe Park; ops@example.org`).
pub fn display_list(list: &[EmailAddress]) -> String {
    list.iter().map(EmailAddress::display).collect::<Vec<_>>().join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mixed_lists() {
        let v = parse_list("Zoe Park <zoe@example.org>; ops@example.org, \"Park, Ann\" <ann@example.org>").unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], EmailAddress::new("Zoe Park", "zoe@example.org"));
        assert_eq!(v[1].email, "ops@example.org");
        assert_eq!(v[2].name, "Park, Ann");
        assert_eq!(parse_list("  ; ,").unwrap(), vec![]);
        assert_eq!(parse_list("good@x.org, not an address").unwrap_err(), "not an address");
        assert!(parse_list("a@b.c <").is_err());
    }

    /// Two addresses with nothing but a space or a line end between them (pasted from a list)
    /// are two: the first was taken for the second's name and never got the message.
    #[test]
    fn addresses_run_together_are_each_read() {
        let got = parse_list("Ann <a@b.example>\nBob <c@d.example> e@f.example; \"Park, Zoe\" <z@x.example>").unwrap();
        let got: Vec<(&str, &str)> = got.iter().map(|a| (a.name.as_str(), a.email.as_str())).collect();
        assert_eq!(got, [("Ann", "a@b.example"), ("Bob", "c@d.example"), ("", "e@f.example"), ("Park, Zoe", "z@x.example")]);
        assert_eq!(parse_valid("\"Park, Zoe\" <z@x.example>, half@").len(), 1);
        assert_eq!(parse_valid("Ann <a@b.example> Bob <c@d.example>").len(), 2);
    }

    /// A quote left open never loses an address unsaid: the entry that holds the next address is
    /// refused, naming it, and what reads whole is still kept in a draft.
    #[test]
    fn an_open_quote_never_loses_an_address() {
        let err = parse_list("Jo 6\" Doe <jo@x.example>; bob@y.example").unwrap_err();
        assert!(err.contains("bob@y.example"), "{err}");
        let err = parse_list("Jo 6\" Doe <jo@x.example>; Bob <bob@y.example>").unwrap_err();
        assert!(err.contains("bob@y.example"), "{err}");
        // A quote opened at the start of the text and never closed.
        let err = parse_list("\"Jo Doe <jo@x.example>; Bob <bob@y.example>").unwrap_err();
        assert!(err.contains("bob@y.example"), "{err}");
        // The quote before Jo closes at Lee's, and Lee's own opens a new one.
        let err = parse_list("Jo\" <jo@x.example>; Bob <bob@x.example>; \"Lee\" <lee@x.example>").unwrap_err();
        assert!(err.contains("bob@x.example") && err.contains("jo@x.example"), "{err}");
        assert!(parse_list("Jo <jo@x.example> and more").is_err());
        // What ends an entry may follow it, and a quote around all of it is a name's.
        assert_eq!(parse_list("Jo 6\" Doe <jo@x.example>; ").unwrap(), [EmailAddress::new("Jo 6\" Doe", "jo@x.example")]);
        assert_eq!(parse_list("\"Zoe <zoe@x.example>\"").unwrap()[0].email, "zoe@x.example");
        assert_eq!(parse_list("Jo <jo@x.example> ; bob@y.example").unwrap().len(), 2);
        // A draft keeps every address that reads whole, the open quote or not.
        let kept = |text: &str| parse_valid(text).into_iter().map(|a| a.email).collect::<Vec<_>>();
        assert_eq!(kept("Jo 6\" Doe <jo@x.example>; bob@y.example"), ["jo@x.example", "bob@y.example"]);
        assert_eq!(kept("Jo\" <jo@x.example>; Bob <bob@x.example>; \"Lee\" <lee@x.example>"), ["jo@x.example", "bob@x.example", "lee@x.example"]);
        assert_eq!(kept("\"Jo Doe <jo@x.example>; Bob <bob@y.example>"), ["jo@x.example", "bob@y.example"]);
    }

    #[test]
    fn display_and_initials() {
        let a = EmailAddress::new("Zoe Park", "zoe@example.org");
        assert_eq!(a.to_string(), "Zoe Park <zoe@example.org>");
        assert_eq!(a.initials(), "ZP");
        assert_eq!(EmailAddress::new("", "ops@example.org").initials(), "O");
        assert_eq!(EmailAddress::new("Park, Ann", "a@x.y").to_string(), "\"Park, Ann\" <a@x.y>");
        assert_eq!(EmailAddress::new("", "").initials(), "?");
        assert_eq!(EmailAddress::new("Ünal", "u@x.y").initials(), "Ü");
    }

    #[test]
    fn validity() {
        assert!(is_valid_email("a@b"));
        assert!(!is_valid_email("a@"));
        assert!(!is_valid_email("@b"));
        assert!(!is_valid_email("a b@c"));
        assert!(!is_valid_email("a@.c"));
    }
}
