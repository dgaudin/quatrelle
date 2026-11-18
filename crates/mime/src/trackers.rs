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
mod tests {
    use super::*;

    #[test]
    fn links_lose_their_tags_and_wrappers() {
        for (link, clean) in [
            ("https://shop.example/sale?utm_source=news&utm_medium=email&id=7#top", "https://shop.example/sale?id=7#top"),
            ("https://shop.example/?fbclid=abc", "https://shop.example/"),
            ("https://Shop.Example:8443/a?MC_EID=1&q=x+y", "https://Shop.Example:8443/a?q=x+y"),
            ("https://www.google.com/url?q=https://site.example/page&sa=D&ust=1", "https://site.example/page"),
            ("https://abcd.r.us-east-1.awstrack.me/L0/https:%2F%2Fapp.example%2Fwelcome/1/0100-x/abc=", "https://app.example/welcome"),
            ("https://l.facebook.com/l.php?u=https%3A%2F%2Fevent.example%2F&h=AT0", "https://event.example/"),
            // Nothing to take out, or not a web address: as it was.
            ("https://site.example/a?b=c", "https://site.example/a?b=c"),
            ("mailto:ana@x.example?subject=Hi", "mailto:ana@x.example?subject=Hi"),
            // A wrapper whose parameter isn't a web address stays wrapped.
            ("https://www.google.com/url?q=javascript:alert(1)", "https://www.google.com/url?q=javascript:alert(1)"),
        ] {
            assert_eq!(clean_link(link), clean, "{link}");
        }
    }

    /// A wrapper that checks the address when it's clicked (Microsoft's Safe Links, Proofpoint's
    /// URL Defense) is the organisation's own protection: the link opens through it. Where it
    /// goes is still told, for showing.
    #[test]
    fn links_checked_when_clicked_open_through_their_check() {
        let safe = "https://eur01.safelinks.protection.outlook.com/?url=https%3A%2F%2Fdocs.example%2Fplan%3Fv%3D2%26utm_campaign%3Dq3&data=05%7C01&reserved=0";
        let defended = "https://urldefense.com/v3/__https://portal.example/login__;!!abc$";
        assert_eq!(clean_link(safe), safe);
        assert_eq!(clean_link(defended), defended);
        assert_eq!(link_destination(safe), "https://docs.example/plan?v=2");
        assert_eq!(link_destination(defended), "https://portal.example/login");
        // A redirect that only counts the click, around one that checks: taken off down to the check.
        let counted = format!("https://www.google.com/url?q={}", safe.replace('%', "%25").replace('&', "%26"));
        assert_eq!(clean_link(&counted), safe);
        assert_eq!(link_destination(&counted), "https://docs.example/plan?v=2");
        // Both still tell their sender the link was followed.
        assert!(tracks_click(safe) && tracks_click(defended));
        // Elsewhere the two say the same.
        assert_eq!(link_destination("https://shop.example/?fbclid=abc"), "https://shop.example/");
    }

    #[test]
    fn trackers_are_counted() {
        let html = r#"<p><a href="https://news.us1.list-manage.com/track/click?u=1&id=2">Read</a>
            <a href="https://shop.example/?utm_source=x">Shop</a>
            <a href="https://shop.example/about">About</a>
            <img src="https://shop.example/logo.png" width="120" alt="Shop">
            <img src="https://open.shop.example/o.gif?r=9" width="1" height="1">
            <img src="https://news.us1.list-manage.com/track/open.php?u=1" alt="">
            <img src="https://t.example/p.png" style="display: none">
            <img src="https://img.shop.example/phone.png" alt="phone" width="13" style="border-width: 0px; display: block">
            <img src="https://t.example/q.png" style="height: 1px !important; width: 1px !important">
            <img src="https://t.example/r.png" style="WIDTH:0"></p>"#;
        let t = trackers(html);
        assert_eq!(t.links, 2);
        // A signature's icon without a border isn't one (`border-width: 0px` was read as `width:0`):
        // shown when the message's pictures are.
        assert_eq!(
            t.pictures,
            [
                "https://open.shop.example/o.gif?r=9",
                "https://news.us1.list-manage.com/track/open.php?u=1",
                "https://t.example/p.png",
                "https://t.example/q.png",
                "https://t.example/r.png"
            ]
        );
        assert!(trackers("<p>plain</p>").is_empty());
    }

    /// Styles that hide a picture or make it a pixel, as a browser reads them: what the first rule
    /// (looking for `width:1px`, `width:0`… in the text) caught stays caught — when in doubt, a
    /// tracker.
    const PIXEL_STYLES: &[&str] = &[
        "width:1px",
        "height:1px",
        "max-width:1px",
        "max-height:1px",
        "max-width:0px",
        "width:1px !important",
        "WIDTH:1PX",
        "width : 1px",
        "width:0",
        "width:0.1em",
        "height:0",
        "width:1pt",
        "width:1",
        "width:2px",
        "width:\n1px",
        "display:none",
        "display:none!important",
        "DISPLAY: NONE",
        "height:1px !important;width:1px !important;border-width:0 !important;margin:0 !important",
        // `!` and `important` apart, an `!important` before a plain one, a zero in any unit,
        // a comment: all valid CSS, taken for pictures once the text was read as declarations.
        "width: 1px ! important; height: 1px ! important",
        "display: none ! important",
        "width:0%",
        "width:0vw",
        "width:1px !important; width:300px",
        "width:1px/**/",
        "width:1px; width:auto",
        "width:1px; width:300px",
        "display:none; display:block",
        // A `/*` in an address isn't a comment (the rest of the style went with it); one after
        // it is.
        "background:url(https://x.example/a/*.png); width:1px",
        "background:url('https://x.example/a/*.png'); width:1px",
        "background:url(https://x.example/a.png) /* plain */; width:1px",
        // Hidden, not sized.
        "visibility:hidden",
        "visibility: hidden !important",
        "opacity:0",
        "opacity: 0.0",
    ];

    /// Styles that make no picture small (an icon's `border-width: 0`).
    const PICTURE_STYLES: &[&str] = &[
        "min-width:1px",
        "min-height:1px",
        "line-height:1px",
        "border-width:0",
        "border-width: 0px; display: block",
        "width:13px",
        "width:3px;height:3px",
        "opacity:0.6",
        "visibility:visible",
        "width:50%",
        "background:url(https://x.example/a/*.png); width:13px",
    ];

    /// The styles of [`PIXEL_STYLES`] and [`PICTURE_STYLES`] that `tracker` (whether a picture of
    /// that style is taken for a tracker) reads wrong, each with what it should have said.
    fn read_wrong(tracker: impl Fn(&str) -> bool) -> Vec<(&'static str, bool)> {
        PIXEL_STYLES
            .iter()
            .map(|s| (*s, true))
            .chain(PICTURE_STYLES.iter().map(|s| (*s, false)))
            .filter(|(s, expected)| tracker(s) != *expected)
            .collect()
    }

    #[test]
    fn pixels_by_their_style() {
        let wrong = read_wrong(|style| !trackers(&format!("<p><img src=\"https://img.example/p.png\" style=\"{style}\"></p>")).pictures.is_empty());
        assert!(wrong.is_empty(), "read wrong (style, a tracker): {wrong:?}");
    }

    /// The same styles in a whole message whose style sheet touches its pictures (`img{border:0}`,
    /// as most newsletters' does): written on the picture, the sheet keeps one value of each
    /// property and drops the tag's `!important`, which loaded `width:1px !important;
    /// width:300px` again.
    #[test]
    fn pixels_by_their_style_under_a_style_sheet() {
        let wrong = read_wrong(|style| {
            let raw = format!(
                "From: a@shop.example\r\nTo: b@studio.example\r\nSubject: s\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
                 <html><head><style>img{{border:0}}</style></head><body><p>Hello</p>\
                 <img src=\"https://img.example/p.png\" style=\"{style}\"></body></html>"
            );
            crate::parse(raw.as_bytes()).is_some_and(|p| !p.trackers.pictures.is_empty())
        });
        assert!(wrong.is_empty(), "read wrong in a message (style, a tracker): {wrong:?}");
    }

    /// A pixel sized or hidden by a class of the message's style sheet is one too: the trackers are
    /// read once the page's styles are written on its elements.
    #[test]
    fn pixels_by_a_class() {
        let raw = "From: a@shop.example\r\nTo: b@studio.example\r\nSubject: s\r\nMIME-Version: 1.0\r\n\
                   Content-Type: text/html; charset=utf-8\r\n\r\n\
                   <html><head><style>.px{width:1px;height:1px} .ghost{visibility:hidden} .clear{opacity:0}</style></head>\
                   <body><p>Hello</p><img class=\"px\" src=\"https://t.example/a.png\"><img class=\"ghost\" src=\"https://t.example/b.png\">\
                   <img class=\"clear\" src=\"https://t.example/c.png\"><img src=\"https://img.example/logo.png\" width=\"120\" alt=\"Logo\"></body></html>";
        let p = crate::parse(raw.as_bytes()).unwrap();
        assert_eq!(p.trackers.pictures, ["https://t.example/a.png", "https://t.example/b.png", "https://t.example/c.png"]);
    }

    /// A whole message with `body` under `sheet`, as parsed.
    fn message(sheet: &str, body: &str) -> crate::Parsed {
        let raw = format!(
            "From: a@shop.example\r\nTo: b@studio.example\r\nSubject: s\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
             <html><head>{sheet}</head><body><p>Hello</p>{body}</body></html>"
        );
        crate::parse(raw.as_bytes()).unwrap()
    }

    /// The web pictures of `p` asked for once its pictures are shown (as the session asks them).
    fn loaded(p: &crate::Parsed) -> Vec<String> {
        p.images.iter().filter(|u| u.starts_with("https://") && !p.trackers.pictures.contains(u)).cloned().collect()
    }

    /// An address written with a character reference (`&#38;`, `&#x26;`, `&quest;` and `&amp;`) is
    /// one address, as a browser reads it, in the pictures and in the trackers alike: a pixel was
    /// listed twice (as written, and as decoded once the style sheet was written on it), which said
    /// two pixels for one.
    #[test]
    fn a_pixel_written_with_character_references_is_listed_once() {
        let written = ["?a=1&#38;b=2", "?a=1&#x26;b=3", "?a=1&amp;b=4", "&quest;a=1&#38;b=5"];
        let decoded = ["?a=1&b=2", "?a=1&b=3", "?a=1&b=4", "?a=1&b=5"];
        let pixels: String = written.iter().map(|q| format!("<img src=\"https://t.example/o.gif{q}\" width=\"1\" height=\"1\">")).collect();
        let listed: Vec<String> = decoded.iter().map(|q| format!("https://t.example/o.gif{q}")).collect();
        for sheet in ["", "<style>img{border:0}</style>"] {
            let p = message(sheet, &pixels);
            assert_eq!((&p.trackers.pictures, loaded(&p)), (&listed, Vec::new()), "each once, as decoded, none asked for, under {sheet:?}");
        }
    }

    /// … which used up the 50 kept: some of 49 such pixels were asked for.
    #[test]
    fn forty_nine_pixels_written_with_references_are_all_held_back() {
        let many: String = (0..49).map(|n| format!("<img src=\"https://t.example/o.gif?n={n}&#38;r=1\" width=\"1\" height=\"1\">")).collect();
        let p = message("<style>img{border:0}</style>", &many);
        assert_eq!((p.trackers.pictures.len(), loaded(&p).len()), (49, 0), "listed, asked for");
    }

    /// … and asked for the one sized `width:1px !important; width:300px` under a style sheet.
    #[test]
    fn a_pixel_sized_twice_written_with_a_reference_is_held_back() {
        let n1 = "<img src=\"https://t.example/o.gif?a=1&#38;b=2\" style=\"width:1px !important; width:300px\">";
        let p = message("<style>img{border:0}</style>", n1);
        assert_eq!((p.trackers.pictures.clone(), loaded(&p)), (vec!["https://t.example/o.gif?a=1&b=2".to_string()], Vec::new()));
    }

    /// The tokenizer's reads of addresses while `f` runs on this thread.
    fn tokenized(f: impl FnOnce()) -> usize {
        let before = crate::layout::TOKENIZED.with(std::cell::Cell::get);
        f();
        crate::layout::TOKENIZED.with(std::cell::Cell::get) - before
    }

    /// A stranger's thousands of pictures cost no more than a few reads of their addresses, in the
    /// three readings of a message (its pictures, its trackers as it came and once its style sheet
    /// is on its elements): an address with `&amp;` alone is read as it is, the same address once,
    /// a message's no more than [`MAX_DECODED`](crate::html::pictures::MAX_DECODED). Each `&` cost a
    /// tokenizer of its own, thrice: a message of 80,000 pictures took two and a half times as
    /// long to read.
    #[test]
    fn many_pictures_cost_a_few_reads_of_their_addresses() {
        let tags = |src: &dyn Fn(usize) -> String| (0..5_000).map(|n| format!("<img src=\"{}\" width=\"120\">", src(n))).collect::<String>();
        // (what, the reads, the most allowed)
        let mut reads: Vec<(String, usize, usize)> = Vec::new();
        for sheet in ["", "<style>img{border:0}</style>"] {
            let amp = tags(&|n| format!("https://img.example/{n}.png?x=1&amp;y=2"));
            reads.push((format!("`&amp;` alone, under {sheet:?}"), tokenized(|| drop(message(sheet, &amp))), 0));
            let same = tags(&|_| "https://img.example/a.png?x=1&#38;y=2".to_string());
            reads.push((format!("the same address, under {sheet:?}"), tokenized(|| drop(message(sheet, &same))), 1));
            let distinct = tags(&|n| format!("https://img.example/{n}.png?x=1&#38;y=2"));
            reads.push((
                format!("5,000 addresses, under {sheet:?}"),
                tokenized(|| drop(message(sheet, &distinct))),
                crate::html::pictures::MAX_DECODED,
            ));
        }
        // The trackers are the pictures listed that report back (each tag of a picture listed is
        // asked, so every address is read): no more reads than a message's.
        let pixels: String = (0..5_000).map(|n| format!("<img src=\"https://t.example/o.gif?n={n}&#38;r=1\" width=\"1\" height=\"1\">")).collect();
        let mut t = Trackers::default();
        reads.push(("5,000 pixels".into(), tokenized(|| t = trackers(&pixels)), crate::html::pictures::MAX_DECODED));
        assert_eq!(t.pictures.len(), crate::html::pictures::MAX_PICTURES);
        let over: Vec<_> = reads.iter().filter(|(_, n, most)| n > most).collect();
        assert!(over.is_empty(), "read too often (what, reads, most): {over:?}");
    }

    /// Found in review: a `>` in a quoted value ended the tag where the pictures and the trackers
    /// were read (`alt=">"`), before the pixel's size, and the pixel was asked for with the
    /// message's pictures. Read as a browser reads the tag, a quoted value that reads like a size
    /// (`alt=' width=500'`) isn't one either, and a size written with a character reference
    /// (`width&#58;1px`) is the size it says.
    #[test]
    fn a_pixel_is_one_whatever_its_values_hold() {
        let o = "https://t.example/o.gif";
        for tag in [
            format!(r#"<img src="{o}" alt=">" width="1" height="1">"#),
            format!(r#"<img src="{o}" title='>' width="1" height="1">"#),
            format!(r#"<img src="{o}" alt=">" style="width:1px">"#),
            format!(r#"<img src="{o}" alt=' width=500 height=500 ' width=1 height=1>"#),
            format!(r#"<img src="{o}" style="width&#58;1px;height&#58;1px">"#),
        ] {
            assert_eq!(trackers(&tag).pictures, [o], "{tag}");
            for sheet in ["", "<style>img{border:0}</style>"] {
                let p = message(sheet, &tag);
                assert_eq!((p.trackers.pictures.clone(), loaded(&p)), (vec![o.to_string()], Vec::<String>::new()), "{tag} under {sheet:?}");
            }
        }
    }

    /// Found in review: the trackers were listed up to 50 and the pictures up to 200: 50 pixels of
    /// any address first, and the next, the real one, was asked for. Each picture listed is asked
    /// whether it's a tracking picture.
    #[test]
    fn fifty_decoys_let_no_pixel_through() {
        let decoys: String = (0..50).map(|n| format!("<img src=\"https://d.example/{n}.gif\" width=\"1\" height=\"1\">")).collect();
        let real = "<img src=\"https://t.example/o.gif\" width=\"1\" height=\"1\">";
        for sheet in ["", "<style>img{border:0}</style>"] {
            let p = message(sheet, &format!("{decoys}{real}"));
            assert!(p.trackers.pictures.iter().any(|s| s == "https://t.example/o.gif"), "listed, under {sheet:?}");
            assert_eq!(loaded(&p), Vec::<String>::new(), "none asked for, under {sheet:?}");
        }
        // As many as the pictures listed: each held back, none past them asked for.
        let many: String = (0..300).map(|n| format!("<img src=\"https://d.example/{n}.gif\" width=\"1\" height=\"1\">")).collect();
        let p = message("", &many);
        assert_eq!((p.images.len(), p.trackers.pictures.len(), loaded(&p).len()), (200, 200, 0), "listed, held back, asked for");
    }

    /// Found in review: a picture behind a box was never asked whether it reports back: a counting
    /// service's (the surest sign) loaded with the message's pictures as a box's background, and
    /// so did one behind a box 1 pixel wide.
    #[test]
    fn a_tracking_picture_behind_a_box_is_one() {
        let counter = "https://x.list-manage.com/track/open.php?u=1";
        let pixel = "https://t.example/o.gif";
        for (body, src) in [
            (format!(r#"<div style="background-image:url({counter});width:1px;height:1px"></div>"#), counter),
            (format!(r#"<table><tr><td background="{counter}">Hello</td><td>there</td></tr></table>"#), counter),
            (format!(r#"<div style="background-image:url({pixel});width:2px;height:300px">x</div>"#), pixel),
            (format!(r#"<table><tr><td style="background:url('{pixel}')" height="1">x</td><td>y</td></tr></table>"#), pixel),
        ] {
            let p = message("", &body);
            assert!(p.images.iter().any(|s| s == src), "a picture of the message: {body}");
            assert_eq!((p.trackers.pictures.clone(), loaded(&p)), (vec![src.to_string()], Vec::<String>::new()), "{body}");
        }
        // A banner behind a heading is shown with the others.
        let p = message("", r#"<div style="background-image:url(https://img.example/banner.png);width:600px"><h1>News</h1></div>"#);
        assert_eq!(loaded(&p), ["https://img.example/banner.png"]);
    }

    /// Found in review: a picture in a comment (or a script, a `<textarea>`…) was asked for when the
    /// message's pictures were shown, which no browser does. The pictures are found as a browser
    /// reads the tags.
    #[test]
    fn pictures_no_browser_loads_are_never_asked_for() {
        let o = r#"<img src="https://t.example/o.gif">"#;
        for html in [
            format!("<!-- {o} --><p>x</p>"),
            format!("<!-- a -- {o} --!><p>x</p>"),
            format!("<script>var s = '{o}';</script>"),
            format!("<textarea>{o}</textarea>"),
            format!("<style>/* {o} */</style>"),
            format!("<title>{o}</title>"),
            format!("<noscript>{o}</noscript>"),
            format!("<template><p>{o}</p></template>"),
            format!("<p title='{o}'>x</p>"),
            format!("<!doctype {o}"),
            format!("<p class='{o}"),
        ] {
            assert_eq!(crate::html_images(&html), Vec::<String>::new(), "{html}");
            assert_eq!(loaded(&message("", &html)), Vec::<String>::new(), "{html}");
        }
        // As a browser reads them: after an empty comment, past a `>` in a quoted value, after a
        // script's end, `<image>`, `<img/src=…>`.
        assert_eq!(
            crate::html_images(r#"<!--><img src=a.png><p alt='>'><img src=b.png><script>x</script><IMAGE SRC=c.png><img/src=d.png>"#),
            ["a.png", "b.png", "c.png", "d.png"]
        );
    }

    #[test]
    fn odd_links_never_panic() {
        for link in [
            "",
            "https://",
            "https://%",
            "https://x.example/?%zz=1&&=&utm_",
            "https://urldefense.com/v3/__",
            "http://[::1]:80/?a",
            "https://é.example/%E2%82",
        ] {
            let _ = clean_link(link);
            let _ = link_destination(link);
            let _ = tracks_click(link);
        }
        assert_eq!(clean_link("http://[::1]:8080/a?utm_x=1&b=2"), "http://[::1]:8080/a?b=2");
        let deep = (0..10).fold("https://site.example/".to_string(), |u, _| format!("https://www.google.com/url?q={}", u.replace('&', "%26")));
        let _ = clean_link(&deep);
    }
}
