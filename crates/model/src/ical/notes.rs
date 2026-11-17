// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! An event's notes as they read: links under their words, addresses as links (Exchange's and Google's invitations).

/// Most web addresses taken from an event's notes.
pub(super) const MAX_LINKS: usize = 20;

/// The web addresses in a text, once each and in order: written out, or as the `[1]: https://…`
/// footnotes an HTML body's links become when it's made text (a Teams invitation's "Join the
/// meeting now"). Only `http` and `https`; a sentence's closing punctuation isn't part of one.
pub fn links_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let lower = text.to_ascii_lowercase();
    let mut at = 0;
    while out.len() < MAX_LINKS {
        let next = [lower.get(at..).and_then(|r| r.find("https://")), lower.get(at..).and_then(|r| r.find("http://"))].into_iter().flatten().min();
        let Some(rel) = next else { break };
        let start = at + rel;
        let rest = text.get(start..).unwrap_or("");
        let len = rest.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '[' | ']' | '{' | '}' | '|')).unwrap_or(rest.len());
        let mut link = rest.get(..len).unwrap_or("");
        // Closing punctuation, and a closing bracket the link didn't open.
        loop {
            let trimmed = link.trim_end_matches(['.', ',', ';', ':', '!', '?']);
            let trimmed = if trimmed.ends_with(')') && trimmed.matches('(').count() < trimmed.matches(')').count() {
                trimmed.get(..trimmed.len() - 1).unwrap_or(trimmed)
            } else {
                trimmed
            };
            if trimmed.len() == link.len() {
                break;
            }
            link = trimmed;
        }
        at = start + len.max(1);
        let host_ok = link.split_once("://").is_some_and(|(_, r)| r.split(['/', '?', '#']).next().is_some_and(|h| h.contains('.') && h.len() < 256));
        if host_ok && link.len() <= 4096 && !out.iter().any(|l| l == link) {
            out.push(link.to_string());
        }
    }
    out
}

/// A piece of an event's notes as they're shown ([`note_lines`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotePart {
    Text(String),
    /// A link to open: its words, or its address when it has none (`label == url`).
    Link {
        label: String,
        url: String,
    },
}

/// The most of an event's notes shown as lines with links (the rest as it is).
pub(super) const NOTE_MAX: usize = 200_000;

/// An event's notes as lines of text and links, the way an invitation reads (issue #1). Made
/// text from HTML by Exchange (`BodyType Text`), each link is written after its words in angle
/// brackets, `Join with Google Meet<https://meet.google.com/…>`: the words become the link and
/// the address goes, as in the HTML. Of the text before, the link's words are those after the
/// last `. `, `: `, `) ` or `– ` on the line (`(US) +1 555-0100<tel:…>` links `+1 555-0100`);
/// an address in brackets with no words before it on its line, or one written out, is a link of
/// its own. Web and `mailto:` links open; a `tel:` one stays its words.
pub fn note_lines(text: &str) -> Vec<Vec<NotePart>> {
    let cut = text.char_indices().nth(NOTE_MAX).map_or(text.len(), |(i, _)| i);
    let mut lines: Vec<Vec<NotePart>> = text.get(..cut).unwrap_or(text).lines().map(note_line).collect();
    // A few words on a line, then (past blank lines) an address alone on one: the words are its
    // link, as a Google invitation's Yes, No, Maybe and More options are.
    let mut i = 0;
    while i < lines.len() {
        let url = match lines.get(i).map(Vec::as_slice) {
            Some([NotePart::Link { label, url }]) if label == url => url.clone(),
            _ => {
                i += 1;
                continue;
            }
        };
        let words = (0..i).rev().find(|j| !lines.get(*j).is_some_and(|l| l.iter().all(|p| matches!(p, NotePart::Text(t) if t.trim().is_empty()))));
        let short = words.and_then(|j| match lines.get(j).map(Vec::as_slice) {
            Some([NotePart::Text(t)]) if (1..=24).contains(&t.trim().chars().count()) && !t.trim_end().ends_with(':') => {
                Some((j, t.trim().to_string()))
            }
            _ => None,
        });
        match short {
            Some((j, label)) => {
                if let Some(l) = lines.get_mut(j) {
                    *l = vec![NotePart::Link { label, url }];
                }
                lines.drain(j + 1..=i.min(lines.len().saturating_sub(1)));
                i = j + 1;
            }
            None => i += 1,
        }
    }
    lines
}

pub(super) fn note_line(line: &str) -> Vec<NotePart> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some((before, url, after)) = bracketed_link(rest) {
        let (text, label) = split_label(before);
        bare_links(text, &mut out);
        let tel = url.get(..4).is_some_and(|s| s.eq_ignore_ascii_case("tel:"));
        match (label.is_empty(), tel) {
            (true, true) => {}
            (false, true) => push_text(&mut out, label),
            (true, false) => out.push(NotePart::Link { label: url.to_string(), url: url.to_string() }),
            (false, false) => out.push(NotePart::Link { label: label.to_string(), url: url.to_string() }),
        }
        rest = after;
    }
    bare_links(rest, &mut out);
    out
}

/// The first `<scheme:…>` in a line: the text before it, the address, the text after.
pub(super) fn bracketed_link(line: &str) -> Option<(&str, &str, &str)> {
    let mut from = 0;
    loop {
        let open = from + line.get(from..)?.find('<')?;
        let inner = line.get(open + 1..)?;
        let close = inner.find('>');
        let url = close.and_then(|c| inner.get(..c)).unwrap_or("");
        let lower = url.to_ascii_lowercase();
        let scheme = ["https://", "http://", "mailto:", "tel:"].iter().any(|s| lower.starts_with(s));
        if let Some(c) = close
            && scheme
            && url.len() > 7
            && !url.contains(char::is_whitespace)
        {
            return Some((line.get(..open)?, url, inner.get(c + 1..)?));
        }
        from = open + 1;
    }
}

/// Text before a link: what stays text, and the link's own words.
pub(super) fn split_label(before: &str) -> (&str, &str) {
    let at = [". ", ": ", ") ", "– ", "\t"].iter().filter_map(|d| before.rfind(d).map(|i| i + d.len())).max().unwrap_or(0);
    let (text, label) = before.split_at(at.min(before.len()));
    let trimmed = label.trim_start();
    (before.get(..text.len() + (label.len() - trimmed.len())).unwrap_or(text), trimmed.trim_end())
}

pub(super) fn push_text(out: &mut Vec<NotePart>, s: &str) {
    if s.is_empty() {
        return;
    }
    if let Some(NotePart::Text(t)) = out.last_mut() {
        t.push_str(s);
    } else {
        out.push(NotePart::Text(s.to_string()));
    }
}

/// Text, its written-out web addresses links of their own.
pub(super) fn bare_links(text: &str, out: &mut Vec<NotePart>) {
    let mut rest = text;
    for link in links_in(text) {
        let Some(at) = rest.find(&link) else { continue };
        push_text(out, rest.get(..at).unwrap_or(""));
        out.push(NotePart::Link { label: link.clone(), url: link.clone() });
        rest = rest.get(at + link.len()..).unwrap_or("");
    }
    push_text(out, rest);
}

/// Whether a web address is a Webex meeting or personal room (`https://<site>.webex.com/meet/<name>`
/// or `…/join/<name>`), as the Webex Meeting button may put one in an appointment.
pub fn is_webex_link(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let host = lower.strip_prefix("https://").and_then(|r| r.split(['/', ':', '?', '#']).next()).unwrap_or("");
    url.len() <= 300
        && !url.chars().any(|c| c.is_whitespace() || c.is_control() || c == '@')
        && (host == "webex.com" || host.ends_with(".webex.com"))
        && is_meeting_link(url)
}

/// The host a browser would really go to, lower-cased, and the path after it, of an `http(s)`
/// address (the scheme in any case). The host ends at the first `/`, `?`, `#` or `\`, loses its
/// `:port` (digits only), and is refused unless it is only letters, digits, `.` and `-`: an `@` (user
/// information) or any other character could point the browser somewhere else than the text
/// suggests. The path is empty unless the host ended at a `/`.
fn web_host_and_path(url: &str) -> Option<(String, &str)> {
    let rest = ["https://", "http://"].iter().find_map(|s| {
        let head = url.get(..s.len())?;
        head.eq_ignore_ascii_case(s).then(|| url.get(s.len()..)).flatten()
    })?;
    let end = rest.find(['/', '?', '#', '\\']).unwrap_or(rest.len());
    let authority = rest.get(..end)?;
    // `host` or `host:<digits>`; anything else after a colon (`:443@evil.example`) is user information.
    let (host, port) = authority.split_once(':').map_or((authority, None), |(h, p)| (h, Some(p)));
    if port.is_some_and(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let plain = !host.is_empty() && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if !plain {
        return None;
    }
    let path = if rest.as_bytes().get(end) == Some(&b'/') { rest.get(end + 1..).unwrap_or("") } else { "" };
    Some((host.to_ascii_lowercase(), path))
}

/// The host of a scheduled Webex meeting's join page (`https://<site>.webex.com/<site>/j.php?MTID=…`);
/// none for any other link, a personal room's among them.
pub fn webex_join_host(url: &str) -> Option<String> {
    let (host, path) = web_host_and_path(url)?;
    let page = path.split(['?', '#']).next()?.rsplit('/').next()?;
    page.eq_ignore_ascii_case("j.php").then_some(host)
}

/// Whether a web address is a Microsoft Teams meeting (`…teams.microsoft.com/l/meetup-join…` or
/// `…/meet/…`, `teams.live.com/meet…`), by the host the browser would really go to.
pub fn is_teams_link(url: &str) -> bool {
    let Some((host, path)) = web_host_and_path(url) else { return false };
    let path = path.to_ascii_lowercase();
    let on = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    (on("teams.microsoft.com") && (path.starts_with("l/meetup-join") || path.starts_with("meet/")))
        || (on("teams.live.com") && path.starts_with("meet"))
}

/// Whether an `https` address names no port: nothing after its host but the path, query or
/// fragment. Microsoft's own pages are never given with one, `:443` included.
fn https_without_port(url: &str) -> bool {
    let https = url.get(..8).is_some_and(|s| s.eq_ignore_ascii_case("https://"));
    let rest = url.get(8..).unwrap_or("");
    let authority = rest.split(['/', '?', '#', '\\']).next().unwrap_or("");
    https && !authority.contains(':')
}

/// Whether a web address opens a Teams chat or one of its messages (`https://teams.microsoft.com/l/chat/…`
/// or `…/l/message/…`, or the same on `teams.cloud.microsoft`): over `https`, on Teams' own host
/// exactly (no subdomain, no port), and launchable ([`launchable_link`]).
pub fn is_teams_chat_link(url: &str) -> bool {
    let Some((host, path)) = web_host_and_path(url) else { return false };
    let path = path.to_ascii_lowercase();
    https_without_port(url)
        && (host == "teams.microsoft.com" || host == "teams.cloud.microsoft")
        && (path.starts_with("l/chat/") || path.starts_with("l/message/"))
        && launchable_link(url)
}

/// The most people a Teams call link names.
pub const TEAMS_CALL_PEOPLE: usize = 20;

/// The start of every Teams call link.
const TEAMS_CALL: &str = "https://teams.microsoft.com/l/call/0/0?users=";

/// Whether `a` is a plain address a call link can carry: 3 to [`crate::pim::teams::ADDRESS_MAX`]
/// characters, exactly one `@` with something on each side, and none of white space, a control or
/// bidi character, nor `, : / \ ? # & = % " < >`.
fn plain_call_address(a: &str) -> bool {
    let n = a.chars().count();
    let Some((user, host)) = a.split_once('@') else { return false };
    (3..=crate::pim::teams::ADDRESS_MAX).contains(&n)
        && !user.is_empty()
        && !host.is_empty()
        && !host.contains('@')
        && !a.chars().any(|c| c.is_whitespace() || c.is_control() || crate::pim::teams::is_bidi(c) || ",:/\\?#&=%\"<>".contains(c))
}

/// The link that starts a Teams call with these people (`addresses`, each trimmed, once for each
/// whatever its case), with video when `video`; none when an address isn't plain, there are none
/// or more than [`TEAMS_CALL_PEOPLE`]. Each address is percent-encoded whole, so it can't add a
/// person or an option.
pub fn teams_call_link(addresses: &[&str], video: bool) -> Option<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut users: Vec<String> = Vec::new();
    for a in addresses {
        let a = a.trim();
        if !plain_call_address(a) {
            return None;
        }
        if seen.contains(&a.to_ascii_lowercase()) {
            continue;
        }
        seen.push(a.to_ascii_lowercase());
        let mut enc = String::new();
        for b in a.bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                enc.push(char::from(b));
            } else {
                enc.push_str(&format!("%{b:02X}"));
            }
        }
        users.push(enc);
    }
    if users.is_empty() || users.len() > TEAMS_CALL_PEOPLE {
        return None;
    }
    let link = format!("{TEAMS_CALL}{}{}", users.join(","), if video { "&withVideo=true" } else { "" });
    is_teams_call_link(&link).then_some(link)
}

/// Whether a web address is a Teams call link: `https` without a port, on `teams.microsoft.com`
/// exactly, the path `l/call/0/0`, no fragment, and the query `users=<v>` or
/// `users=<v>&withVideo=true`, `<v>` not empty and only `A-Z a-z 0-9 - . _ ~ ,` and `%XX`; and
/// [`launchable_link`].
pub fn is_teams_call_link(url: &str) -> bool {
    let Some((host, rest)) = web_host_and_path(url) else { return false };
    let Some((path, query)) = rest.split_once('?') else { return false };
    let users = match query.strip_prefix("users=") {
        Some(u) => u.strip_suffix("&withVideo=true").unwrap_or(u),
        None => return false,
    };
    let b = users.as_bytes();
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if c == b'%' {
            if !(b.get(i + 1).is_some_and(u8::is_ascii_hexdigit) && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit)) {
                return false;
            }
            i += 3;
        } else if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_' | b'~' | b',') {
            i += 1;
        } else {
            return false;
        }
    }
    https_without_port(url) && host == "teams.microsoft.com" && path.eq_ignore_ascii_case("l/call/0/0") && !users.is_empty() && launchable_link(url)
}

/// Most path segments and query keys a link's shape names ([`link_shape`]); the rest is `…`.
const SHAPE_PARTS: usize = 8;

/// A web address's shape, for the log: its host (and port), each path segment only when it's a
/// short plain word (`[A-Za-z0-9_-]`, at most 16 characters; else `<id>`), the query's keys
/// alike and never a value, no fragment (`teams.microsoft.com/l/chat/<id>/conversations?context&tenantId`).
/// `(not a web address)` unless `http` or `https`; `(not a plain host)` for a host a browser
/// could take elsewhere ([`web_host_and_path`]). What it keeps is only short plain words, never a
/// value: a short id that is one (an all-digit one, an organisation named in one word) would show,
/// so it's for the links Quatrelle itself opens (a Teams chat's, Microsoft's page to sign in on),
/// whose ids are longer or punctuated.
pub fn link_shape(url: &str) -> String {
    let web = ["https://", "http://"].iter().any(|s| url.get(..s.len()).is_some_and(|h| h.eq_ignore_ascii_case(s)));
    if !web {
        return "(not a web address)".into();
    }
    let Some((host, path)) = web_host_and_path(url) else { return "(not a plain host)".into() };
    // The host as `web_host_and_path` read it, its port (digits only) beside it.
    let after = url.split_once("://").map_or("", |(_, r)| r);
    let authority = after.split(['/', '?', '#', '\\']).next().unwrap_or("");
    let mut out = host.chars().take(64).collect::<String>();
    if let Some((_, port)) = authority.split_once(':') {
        out.push(':');
        out.push_str(port.get(..port.len().min(5)).unwrap_or(""));
    }
    let unfragmented = url.split('#').next().unwrap_or("");
    if after.as_bytes().get(authority.len()) == Some(&b'/') {
        out.push('/');
        out.push_str(&shape_names(path.split(['?', '#']).next().unwrap_or("").split('/'), "/"));
    }
    if let Some((_, query)) = unfragmented.split_once('?') {
        out.push('?');
        out.push_str(&shape_names(query.split('&').map(|p| p.split('=').next().unwrap_or("")), "&"));
    }
    out
}

/// The first [`SHAPE_PARTS`] of a path's segments or a query's keys, each a plain word or `<id>`,
/// then `…` if there are more.
fn shape_names<'a>(mut parts: impl Iterator<Item = &'a str>, sep: &str) -> String {
    let word = |s: &str| {
        let plain = s.len() <= 16 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if plain { s.to_string() } else { "<id>".to_string() }
    };
    let mut out: Vec<String> = parts.by_ref().take(SHAPE_PARTS).map(word).collect();
    if parts.next().is_some() {
        out.push("…".into());
    }
    out.join(sep)
}

/// Whether a web address is Microsoft's page to sign in to an organisation, as the Teams sign-in
/// opens it: `https://login.microsoftonline.com/<tenant>/oauth2/v2.0/authorize?<query>`, on that
/// host exactly (no port), with an organisation the Options could name
/// ([`crate::valid_oauth_tenant`]), a query, nothing but ASCII (every parameter is
/// percent-encoded), and launchable ([`launchable_link`]).
pub fn is_microsoft_sign_in(url: &str) -> bool {
    let Some((host, path)) = web_host_and_path(url) else { return false };
    let Some((page, query)) = path.split_once('?') else { return false };
    let tenant = page.strip_suffix("/oauth2/v2.0/authorize").unwrap_or("");
    https_without_port(url)
        && host == "login.microsoftonline.com"
        && crate::valid_oauth_tenant(tenant)
        && !query.is_empty()
        && url.is_ascii()
        && launchable_link(url)
}

/// Whether a web address may be handed to a browser to open: `http` or `https`, no whitespace or
/// control character, at most 2,048 bytes, and a plain host ([`web_host_and_path`]).
pub fn launchable_link(url: &str) -> bool {
    url.len() <= 2048 && !url.chars().any(|c| c.is_whitespace() || c.is_control()) && web_host_and_path(url).is_some()
}

/// Whether a web address joins an online meeting: Microsoft Teams, Zoom, Google Meet, Webex,
/// GoTo Meeting, Jitsi, Whereby, Skype.
pub fn is_meeting_link(url: &str) -> bool {
    if is_teams_link(url) {
        return true;
    }
    let Some((host, path)) = web_host_and_path(url) else { return false };
    let path = path.to_ascii_lowercase();
    let on = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    (on("zoom.us") && (path.starts_with("j/") || path.starts_with("w/") || path.starts_with("my/")))
        || on("meet.google.com")
        || (on("webex.com") && (path.contains("j.php") || path.starts_with("meet") || path.contains("/meet") || path.starts_with("join/")))
        || (on("gotomeeting.com") && path.starts_with("join"))
        || on("meet.goto.com")
        || on("meet.jit.si")
        || on("whereby.com")
        || on("join.skype.com")
}
