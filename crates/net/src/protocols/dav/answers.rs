//! Reading a WebDAV `207 Multi-Status` answer (RFC 4918 §13): the resources and the properties
//! found. The XML is hostile: depth and size are capped, and no entity beyond XML's own is read.

use std::collections::HashMap;

use quick_xml::Reader;
use quick_xml::events::Event as X;

use super::MAX_BODY;

/// One `<response>` of a multistatus: the resource and the properties found (200).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    pub href: String,
    /// Text properties by local name (`displayname`, `getetag`, `getctag`, `calendar-data`…).
    pub props: HashMap<String, String>,
    /// Hrefs inside properties (`current-user-principal`, `calendar-home-set`…).
    pub hrefs: HashMap<String, Vec<String>>,
    /// The kinds in `resourcetype` (`collection`, `calendar`, `addressbook`).
    pub kinds: Vec<String>,
    /// `supported-calendar-component-set` (`VEVENT`, `VTODO`…).
    pub comps: Vec<String>,
    /// The user may write to it (a `write` privilege, or `all`); `None` when not said.
    pub writable: Option<bool>,
}

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or("").to_ascii_lowercase()
}

/// Read a `207 Multi-Status` body.
pub fn multistatus(xml: &str) -> Vec<Item> {
    let mut r = Reader::from_str(xml);
    let mut stack: Vec<String> = Vec::new();
    let mut out: Vec<Item> = Vec::new();
    let mut cur: Option<Item> = None;
    // The properties of the propstat being read, kept only if its status is 200.
    let mut pending = Item::default();
    let mut status = String::new();
    let mut text = String::new();
    let mut privileges: Vec<String> = Vec::new();
    while let Ok(ev) = r.read_event() {
        // `<x/>` is a start and an end at once.
        let empty = matches!(ev, X::Empty(_));
        match ev {
            X::Start(e) | X::Empty(e) if stack.len() < 64 => {
                let name = local(e.name().as_ref());
                let parent = stack.last().cloned().unwrap_or_default();
                if parent == "resourcetype" {
                    pending.kinds.push(name.clone());
                }
                if name == "comp" && parent == "supported-calendar-component-set" {
                    for a in e.attributes().flatten() {
                        if local(a.key.as_ref()) == "name" {
                            pending.comps.push(a.value.to_ascii_uppercase());
                        }
                    }
                }
                if stack.iter().any(|s| s == "privilege") {
                    privileges.push(name.clone());
                }
                match name.as_str() {
                    "response" => {
                        cur = Some(Item::default());
                    }
                    "propstat" => {
                        pending = Item::default();
                        status.clear();
                    }
                    _ => {}
                }
                text.clear();
                if !empty {
                    stack.push(name);
                } else {
                    end_tag(&name, &stack, &mut cur, &mut pending, &mut status, &mut privileges, &text, &mut out);
                }
            }
            X::Text(t) if text.len() < MAX_BODY => {
                text.push_str(&t);
            }
            X::CData(t) if text.len() < MAX_BODY => {
                text.push_str(&t);
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
                if let Some(c) = c {
                    text.push(c);
                }
            }
            X::End(e) => {
                let name = local(e.name().as_ref());
                if stack.last() == Some(&name) {
                    stack.pop();
                }
                end_tag(&name, &stack, &mut cur, &mut pending, &mut status, &mut privileges, &text, &mut out);
                text.clear();
            }
            X::Eof => break,
            _ => {}
        }
        if out.len() > 100_000 {
            break;
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn end_tag(
    name: &str,
    stack: &[String],
    cur: &mut Option<Item>,
    pending: &mut Item,
    status: &mut String,
    privileges: &mut Vec<String>,
    text: &str,
    out: &mut Vec<Item>,
) {
    let parent = stack.last().map(String::as_str).unwrap_or("");
    match name {
        "href" if parent == "response" => {
            if let Some(c) = cur.as_mut() {
                c.href = text.trim().to_string();
            }
        }
        "href" => {
            // An href inside a property: file it under the property.
            if let Some(prop) = stack.iter().rev().find(|s| s.as_str() != "href" && s.as_str() != "prop") {
                pending.hrefs.entry(prop.clone()).or_default().push(text.trim().to_string());
            }
        }
        "status" if parent == "propstat" => *status = text.to_string(),
        "current-user-privilege-set" => {
            pending.writable = Some(privileges.iter().any(|p| p == "write" || p == "all" || p == "write-content" || p == "bind"));
            privileges.clear();
        }
        "propstat" => {
            let ok = status.split_whitespace().nth(1).is_none_or(|c| c.starts_with('2'));
            if ok && let Some(c) = cur.as_mut() {
                let p = std::mem::take(pending);
                c.props.extend(p.props);
                for (k, v) in p.hrefs {
                    c.hrefs.entry(k).or_default().extend(v);
                }
                c.kinds.extend(p.kinds);
                c.comps.extend(p.comps);
                c.writable = c.writable.or(p.writable);
            }
        }
        "response" => {
            if let Some(c) = cur.take() {
                out.push(c);
            }
        }
        _ => {
            if parent == "prop" && !text.trim().is_empty() {
                pending.props.insert(name.to_string(), text.trim().to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_multistatus() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/">
  <d:response>
    <d:href>/dav/calendars/me/work/</d:href>
    <d:propstat>
      <d:prop>
        <d:displayname>Work &amp; studio</d:displayname>
        <d:resourcetype><d:collection/><c:calendar/></d:resourcetype>
        <cs:getctag>"42"</cs:getctag>
        <c:supported-calendar-component-set><c:comp name="VEVENT"/><c:comp name="VTODO"/></c:supported-calendar-component-set>
        <d:current-user-privilege-set><d:privilege><d:read/></d:privilege><d:privilege><d:write/></d:privilege></d:current-user-privilege-set>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
    <d:propstat>
      <d:prop><d:getetag/></d:prop>
      <d:status>HTTP/1.1 404 Not Found</d:status>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>/dav/principals/me/</d:href>
    <d:propstat><d:prop><d:current-user-principal><d:href>/dav/principals/me/</d:href></d:current-user-principal>
    <c:calendar-data><![CDATA[BEGIN:VCALENDAR
END:VCALENDAR]]></c:calendar-data></d:prop>
    <d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  </d:response>
</d:multistatus>"#;
        let items = multistatus(xml);
        assert_eq!(items.len(), 2);
        let w = &items[0];
        assert_eq!(w.href, "/dav/calendars/me/work/");
        assert_eq!(w.props.get("displayname").map(String::as_str), Some("Work & studio"));
        assert_eq!(w.props.get("getctag").map(String::as_str), Some("\"42\""));
        assert!(w.kinds.contains(&"calendar".to_string()) && w.kinds.contains(&"collection".to_string()));
        assert_eq!(w.comps, vec!["VEVENT", "VTODO"]);
        assert_eq!(w.writable, Some(true));
        assert!(!w.props.contains_key("getetag"), "404 properties are left out");
        assert_eq!(items[1].hrefs.get("current-user-principal"), Some(&vec!["/dav/principals/me/".to_string()]));
        assert!(items[1].props.get("calendar-data").is_some_and(|d| d.starts_with("BEGIN:VCALENDAR")));
        for junk in [
            "",
            "<",
            "<d:multistatus>",
            "<a><b><c>",
            &"<x>".repeat(1000),
            "<?xml?><!DOCTYPE x [<!ENTITY e \"boom\">]><d:multistatus>&e;</d:multistatus>",
        ] {
            let _ = multistatus(junk);
        }
    }
}
