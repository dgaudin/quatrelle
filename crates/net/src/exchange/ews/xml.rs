// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! EWS: an answer's XML read into elements, text written into a request, and base64 both ways.

use quick_xml::Reader;
use quick_xml::events::Event as X;

use super::*;

/// An element of an answer: its local name, attributes, text and children.
#[derive(Clone, Debug, Default)]
pub struct El {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub kids: Vec<El>,
}

impl El {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    /// The first child called `name`.
    pub fn child(&self, name: &str) -> Option<&El> {
        self.kids.iter().find(|k| k.name == name)
    }
    pub fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a El> + 'a {
        self.kids.iter().filter(move |k| k.name == name)
    }
    /// The first element called `name` below this one (depth first).
    pub fn find(&self, name: &str) -> Option<&El> {
        self.kids.iter().find_map(|k| if k.name == name { Some(k) } else { k.find(name) })
    }
    /// Every element called `name` below this one.
    pub fn all<'a>(&'a self, name: &str, out: &mut Vec<&'a El>) {
        for k in &self.kids {
            if k.name == name {
                out.push(k);
            } else {
                k.all(name, out);
            }
        }
    }
    pub fn text_of(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.as_str())
    }
    /// The folders in an answer, whatever their kind: Exchange gives each in an element of its
    /// own (`Folder`, `ContactsFolder`, `CalendarFolder`, `TasksFolder`, `SearchFolder`).
    pub fn folders(&self) -> Vec<&El> {
        fn walk<'a>(el: &'a El, out: &mut Vec<&'a El>) {
            for k in &el.kids {
                if FOLDER_KINDS.contains(&k.name.as_str()) {
                    out.push(k);
                } else {
                    walk(k, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(self, &mut out);
        out
    }
}

/// The elements a folder comes in (EWS's array of folders).
const FOLDER_KINDS: [&str; 5] = ["Folder", "ContactsFolder", "CalendarFolder", "TasksFolder", "SearchFolder"];

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or("").to_string()
}

/// An attribute's value with its entities replaced.
fn unescape(v: &str) -> String {
    if !v.contains('&') {
        return v.to_string();
    }
    let mut out = String::with_capacity(v.len());
    let mut rest = v;
    while let Some(i) = rest.find('&') {
        out.push_str(rest.get(..i).unwrap_or(""));
        let after = rest.get(i + 1..).unwrap_or("");
        let Some(end) = after.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = after;
            continue;
        };
        let name = after.get(..end).unwrap_or("");
        let c = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            n => n
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| n.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = after.get(end + 1..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Read XML into elements (namespaces dropped: the local names are enough here).
pub fn parse(xml: &str) -> El {
    let mut r = Reader::from_str(xml);
    let mut stack: Vec<El> = vec![El { name: "#doc".into(), ..Default::default() }];
    let mut count = 0usize;
    // Start tags left out and not ended yet: their ends close nothing kept.
    let mut skipped = 0usize;
    let close = |stack: &mut Vec<El>| {
        if stack.len() > 1
            && let Some(done) = stack.pop()
            && let Some(parent) = stack.last_mut()
        {
            parent.kids.push(done);
        }
    };
    while let Ok(ev) = r.read_event() {
        let empty = matches!(ev, X::Empty(_));
        match ev {
            X::Start(_) if skipped > 0 || stack.len() >= MAX_DEPTH || count >= MAX_ELEMENTS => {
                if skipped == 0 {
                    let cap = if stack.len() >= MAX_DEPTH { "depth" } else { "element count" };
                    log::warn!("Exchange XML: the {cap} cap was reached; what the element holds is left out");
                }
                skipped += 1;
            }
            X::End(_) if skipped > 0 => skipped -= 1,
            _ if skipped > 0 && !matches!(ev, X::Eof) => {}
            X::Start(e) | X::Empty(e) if stack.len() < MAX_DEPTH && count < MAX_ELEMENTS => {
                count += 1;
                let el = El {
                    name: local(e.name().as_ref()),
                    attrs: e
                        .attributes()
                        .flatten()
                        .map(|a| {
                            let mut value = String::new();
                            push_in_xml(&mut value, &unescape(&a.value));
                            (local(a.key.as_ref()), value)
                        })
                        .collect(),
                    ..Default::default()
                };
                stack.push(el);
                if empty {
                    close(&mut stack);
                }
            }
            X::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    push_in_xml(&mut top.text, &t);
                }
            }
            X::CData(t) => {
                if let Some(top) = stack.last_mut() {
                    push_in_xml(&mut top.text, &t);
                }
            }
            X::GeneralRef(g) => {
                let name = g.to_string();
                let c = match name.as_str() {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    n => n
                        .strip_prefix("#x")
                        .or_else(|| n.strip_prefix("#X"))
                        .and_then(|h| u32::from_str_radix(h, 16).ok())
                        .or_else(|| n.strip_prefix('#').and_then(|d| d.parse().ok()))
                        .and_then(char::from_u32),
                };
                if let (Some(c), Some(top)) = (c, stack.last_mut()) {
                    push_in_xml(&mut top.text, c.encode_utf8(&mut [0; 4]));
                }
            }
            X::End(_) => close(&mut stack),
            X::Eof => break,
            _ => {}
        }
    }
    while stack.len() > 1 {
        close(&mut stack);
    }
    stack.pop().unwrap_or_default()
}

/// Whether XML 1.0 allows `c` (the controls but tab and line ends it doesn't).
fn in_xml(c: char) -> bool {
    (c as u32) >= 0x20 || matches!(c, '\t' | '\n' | '\r')
}

/// `s` onto `out`, without what XML doesn't allow.
fn push_in_xml(out: &mut String, s: &str) {
    out.extend(s.chars().filter(|&c| in_xml(c)));
}

/// Text for XML.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if !in_xml(c) => {}
            c => out.push(c),
        }
    }
    out
}

pub(super) fn envelope(body: &str) -> String {
    envelope_as("Exchange2010_SP2", body)
}

/// The request under a server version of its own: operations Exchange 2013 brought (reminders)
/// are refused under the version the rest asks for.
pub(super) fn envelope_as(version: &str, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\" \
xmlns:t=\"http://schemas.microsoft.com/exchange/services/2006/types\" \
xmlns:m=\"http://schemas.microsoft.com/exchange/services/2006/messages\">\
<soap:Header><t:RequestServerVersion Version=\"{version}\"/></soap:Header>\
<soap:Body>{body}</soap:Body></soap:Envelope>"
    )
}

/// Base64 without line breaks.
pub(super) fn b64(data: &[u8]) -> String {
    crate::imap::base64(data)
}

/// Base64 text to bytes (whitespace skipped); `None` when it isn't base64.
pub fn unb64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace()) {
        if c == b'=' {
            break;
        }
        acc = (acc << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xFF).unwrap_or(0));
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An element past the depth cap is left out with what it holds; its end doesn't close the
    /// element it's in, so what comes after stays where it was.
    #[test]
    fn past_the_depth_cap_the_tree_keeps_its_shape() {
        let opens = "<a>".repeat(MAX_DEPTH - 2);
        let closes = "</a>".repeat(MAX_DEPTH - 2);
        let doc = parse(&format!("<r>{opens}<deep>lost<x/><y>gone</y></deep>{closes}<sibling/></r>"));
        let r = doc.child("r").unwrap();
        assert!(r.child("sibling").is_some(), "the sibling stays in r");
        assert_eq!(doc.kids.len(), 1, "nothing climbs to the top");
        let mut innermost = r;
        while let Some(a) = innermost.child("a") {
            innermost = a;
        }
        assert!(innermost.kids.is_empty() && innermost.text.is_empty(), "what's past the cap is left out: {innermost:?}");
        assert!(doc.find("deep").is_none() && doc.find("x").is_none());
    }

    /// The same past the cap on how many elements are read.
    #[test]
    fn past_the_element_cap_the_tree_keeps_its_shape() {
        let items = "<i/>".repeat(MAX_ELEMENTS - 1);
        let doc = parse(&format!("<r>{items}<over>lost<x/><y>gone</y></over>tail<late/></r>after"));
        let r = doc.child("r").unwrap();
        assert_eq!((doc.kids.len(), doc.text.as_str()), (1, "after"), "nothing climbs to the top");
        assert_eq!(r.kids.len(), MAX_ELEMENTS - 1);
        assert!(r.kids.iter().all(|k| k.name == "i"), "what's past the cap is left out");
        assert_eq!(r.text, "tail", "r is still open after the element left out ends");
    }

    /// What XML doesn't allow is neither read nor written: a rule's folder id holding NULs was
    /// read with them and written back without.
    #[test]
    fn what_xml_does_not_allow_is_not_read() {
        let doc = parse("<r a=\"x\0y\">a&#1;b</r>");
        let r = doc.child("r").unwrap();
        assert_eq!(r.attr("a"), Some("xy"), "{:?}", r.attrs);
        assert_eq!(r.text, "ab", "{r:?}");
        assert_eq!(esc("a\0b\u{1}"), "ab");
    }
}
