// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What a message carries to learn about its reader. A newsletter's 1-by-1 picture tells its
//! sender when (and from where) it was opened; its links go through a click counter, or carry
//! tags (`utm_source`, `fbclid`…) that say which mailing and which reader clicked.
//!
//! [`trackers`] counts both in an HTML body, and lists the tracking pictures so they stay
//! unloaded even when the user shows the message's pictures. [`clean_link`] takes a link back
//! to where it really goes before it's opened: out of the redirect wrappers that carry the real
//! address in them (Microsoft's Safe Links, Google's `/url`, Proofpoint, Amazon SES and others),
//! and without the tags that only identify the reader. Links counted through a service that
//! keeps the real address to itself can't be cleaned, only counted.

use serde::{Deserialize, Serialize};

/// Pictures and links that report back.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trackers {
    /// The tracking pictures' addresses (never loaded).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pictures: Vec<String>,
    /// Links that report the click (through a counter, or with tags naming the reader).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub links: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl Trackers {
    pub fn is_empty(&self) -> bool {
        self.pictures.is_empty() && self.links == 0
    }
}

/// Query parameters that only say who clicked, from which mailing (`utm_*` too).
const TAGS: &[&str] = &[
    "_hsenc",
    "_hsmi",
    "_kx",
    "__hsfp",
    "__hssc",
    "__hstc",
    "ck_subscriber_id",
    "dclid",
    "fbclid",
    "gbraid",
    "gclid",
    "hsctatracking",
    "igshid",
    "mc_cid",
    "mc_eid",
    "mkt_tok",
    "msclkid",
    "oly_anon_id",
    "oly_enc_id",
    "s_cid",
    "vero_conv",
    "vero_id",
    "wbraid",
    "yclid",
];

/// Services that count clicks and opens for mailings (a host ending in one of these).
const COUNTERS: &[&str] = &[
    "awstrack.me",
    "clicks.aweber.com",
    "clicks.mlsend.com",
    "convertkit-mail.com",
    "convertkit-mail2.com",
    "createsend1.com",
    "ct.sendgrid.net",
    "exct.net",
    "hubspotemail.net",
    "hubspotlinks.com",
    "klclick.com",
    "klclick1.com",
    "klclick2.com",
    "klclick3.com",
    "list-manage.com",
    "mailtrack.io",
    "mandrillapp.com",
    "mcsv.net",
    "mjt.lu",
    "pstmrk.it",
    "rs6.net",
    "sendgrid.net",
    "sparkpostmail.com",
    "t.yesware.com",
    "track.customer.io",
    "track.hubspot.com",
    "via.intercom-mail.com",
];

/// Wrappers that carry the real address in a query parameter: (host or host suffix, path
/// prefix, parameter).
const WRAPPERS: &[(&str, &str, &str)] = &[
    ("www.google.com", "/url", "q"),
    ("www.google.com", "/url", "url"),
    ("google.com", "/url", "q"),
    ("l.facebook.com", "/l.php", "u"),
    ("lm.facebook.com", "/l.php", "u"),
    ("l.instagram.com", "/", "u"),
    ("www.youtube.com", "/redirect", "q"),
    ("out.reddit.com", "/", "url"),
    ("slack-redir.net", "/link", "url"),
    ("www.linkedin.com", "/redir/redirect", "url"),
    ("steamcommunity.com", "/linkfilter/", "url"),
];

/// Wrappers an organisation puts around the links of the mail it receives, to check the real
/// address at the moment it's clicked (Microsoft Defender's Safe Links): the same shape as
/// [`WRAPPERS`], but a link opens through them. Taking one off would open a page the check
/// would have stopped. Proofpoint's URL Defense is one too (its address is read in `unwrapped`).
const GATES: &[(&str, &str, &str)] = &[("safelinks.protection.outlook.com", "/", "url")];

/// The longest link looked at (longer ones are left as they are).
const LINK_MAX: usize = 8192;

/// A web address split where cleaning needs it.
struct Url<'a> {
    scheme: &'a str,
    host: String,
    /// Everything after the host up to `?` (a port included, as written).
    path: &'a str,
    query: Option<&'a str>,
    fragment: Option<&'a str>,
}

fn split(url: &str) -> Option<Url<'_>> {
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
        return None;
    }
    let (rest, fragment) = match rest.split_once('#') {
        Some((r, f)) => (r, Some(f)),
        None => (rest, None),
    };
    let (rest, query) = match rest.split_once('?') {
        Some((r, q)) => (r, Some(q)),
        None => (rest, None),
    };
    // An IPv6 address is in brackets, colons and all.
    let host_end = match rest.strip_prefix('[') {
        Some(r) => r.find(']').map_or(rest.len(), |e| e + 2),
        None => rest.find(['/', ':']).unwrap_or(rest.len()),
    };
    let authority = rest.get(..host_end)?;
    // `user@host`: the host is after the `@`.
    let host = authority.rsplit('@').next().unwrap_or(authority).to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    Some(Url { scheme, host, path: rest.get(host_end..).unwrap_or(""), query, fragment })
}

fn host_is(host: &str, name: &str) -> bool {
    host == name || host.strip_suffix(name).is_some_and(|h| h.ends_with('.'))
}

/// `%xx` and (in a query) `+` as the characters they stand for; `None` when that isn't text.
fn percent_decode(s: &str, plus: bool) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        match c {
            b'%' => {
                let hex = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                    None => out.push(c),
                }
            }
            b'+' if plus => out.push(b' '),
            _ => out.push(c),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// The address a wrapper carries, when `url` is one. `gates`: also one that checks the address
/// when it's clicked ([`GATES`]), which is taken off only to say where a link goes.
fn unwrapped(url: &str, gates: bool) -> Option<String> {
    let u = split(url)?;
    let inner = if host_is(&u.host, "urldefense.com") {
        // Proofpoint v3: `/v3/__https://real.example/path__;…`.
        u.path.strip_prefix("/v3/__").filter(|_| gates).and_then(|p| p.split_once("__;")).map(|(real, _)| real.to_string())
    } else if host_is(&u.host, "awstrack.me") {
        // Amazon SES: `/L0/<the address, percent-encoded>/1/…`.
        u.path.strip_prefix("/L0/").and_then(|p| p.split('/').next()).and_then(|p| percent_decode(p, false))
    } else {
        let known = WRAPPERS.iter().chain(GATES.iter().filter(|_| gates));
        let (_, _, param) = known.into_iter().find(|(host, path, _)| host_is(&u.host, host) && u.path.starts_with(path))?;
        u.query?.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            k.eq_ignore_ascii_case(param).then(|| percent_decode(v, true)).flatten()
        })
    }?;
    let inner = inner.trim().to_string();
    split(&inner).is_some().then_some(inner)
}

fn is_tag(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("utm_") || TAGS.contains(&name.as_str())
}

/// `url` as it's opened: out of the redirect wrappers that carry the real address (a few deep at
/// most), and without the query parameters that only identify the reader. A wrapper that checks
/// the address when it's clicked stays ([`GATES`]): the link opens through the organisation's
/// check. Anything that isn't an `http(s)` address comes back as it was.
pub fn clean_link(url: &str) -> String {
    cleaned(url, false)
}

/// Where `url` goes in the end, for showing: [`clean_link`], out of the wrappers that check the
/// address when it's clicked too. Never what's opened.
pub fn link_destination(url: &str) -> String {
    cleaned(url, true)
}

fn cleaned(url: &str, gates: bool) -> String {
    let mut url = url.trim().to_string();
    if url.len() > LINK_MAX {
        return url;
    }
    for _ in 0..4 {
        match unwrapped(&url, gates) {
            Some(inner) if inner != url => url = inner,
            _ => break,
        }
    }
    let Some(u) = split(&url) else { return url };
    let Some(query) = u.query else { return url };
    let kept: Vec<&str> = query.split('&').filter(|p| !p.is_empty() && !is_tag(p.split('=').next().unwrap_or(p))).collect();
    let mut out = format!("{}://{}{}", u.scheme, original_authority(&url, u.scheme), u.path);
    if !kept.is_empty() {
        out.push('?');
        out.push_str(&kept.join("&"));
    }
    if let Some(f) = u.fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// The host part of `url` as it was written (case and any `user@` kept).
fn original_authority<'a>(url: &'a str, scheme: &str) -> &'a str {
    let rest = url.get(scheme.len() + 3..).unwrap_or("");
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let auth = rest.get(..end).unwrap_or("");
    // A port stays with the path (as `split` keeps it): only the name part here.
    let name_end = match auth.strip_prefix('[') {
        Some(r) => r.find(']').map_or(auth.len(), |e| e + 2),
        None => auth.rfind(':').unwrap_or(auth.len()),
    };
    auth.get(..name_end).unwrap_or(auth)
}

/// Whether following `url` tells someone the reader clicked it.
pub fn tracks_click(url: &str) -> bool {
    let Some(u) = split(url.trim()) else { return false };
    COUNTERS.iter().any(|c| host_is(&u.host, c))
        || unwrapped(url, true).is_some()
        || u.query.is_some_and(|q| q.split('&').any(|p| is_tag(p.split('=').next().unwrap_or(p))))
}

/// The trackers in an HTML body: tiny pictures (1 or 2 pixels across, hidden, or on a counting
/// service) and links that report the click. A message's pictures are asked in it as it came and
/// once its style sheet is written on its elements ([`crate::html::Pictures`]).
pub fn trackers(html: &str) -> Trackers {
    let mut p = crate::html::Pictures::default();
    p.read(html, true);
    Trackers { pictures: p.trackers(), links: tracking_links(html) }
}

/// How many of the links of an HTML body report the click.
pub(crate) fn tracking_links(html: &str) -> usize {
    crate::html_links(html).iter().filter(|l| tracks_click(l)).count()
}

/// Whether `src` is a web address (the only kind a tracking picture has).
pub(crate) fn on_the_web(src: &str) -> bool {
    split(src).is_some()
}

/// Whether a picture at `src` reports back, by what its tag or the box it's behind says
/// (`width`, `height`, `style`, as a browser reads them): 1 or 2 pixels across or down
/// ([`crate::html::tiny_size`]), hidden, or on a counting service. By its own properties (a
/// signature's icon `border-width: 0px` was taken for `width:0`, and stayed hidden when the
/// message's pictures were shown); hidden by any of them, when in doubt.
pub(crate) fn tracking_picture(src: &str, width: Option<&str>, height: Option<&str>, style: Option<&str>) -> bool {
    let Some(u) = split(src) else { return false };
    let style = style.map(crate::css::declarations).unwrap_or_default();
    let hidden = style.iter().any(|d| {
        let v = d.value.trim();
        match d.prop.as_str() {
            "display" => v.eq_ignore_ascii_case("none"),
            "visibility" => v.eq_ignore_ascii_case("hidden") || v.eq_ignore_ascii_case("collapse"),
            // Seen through: none of it, or next to none (`0`, `0%`, `0.01`).
            "opacity" => match v.strip_suffix('%') {
                Some(pc) => pc.trim().parse::<f32>().is_ok_and(|n| n < 5.0),
                None => v.parse::<f32>().is_ok_and(|n| n < 0.05),
            },
            _ => false,
        }
    });
    crate::html::tiny_size(width, height, &style) || hidden || COUNTERS.iter().any(|c| host_is(&u.host, c))
}

#[cfg(test)]
mod tests;
