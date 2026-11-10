// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Calendars and address books on the account's server: CalDAV (RFC 4791) and CardDAV
//! (RFC 6352) over WebDAV (RFC 4918), found as RFC 6764 describes (an SRV record, then the
//! domain's `/.well-known/` addresses), signed in with the account's password. Nothing at all
//! goes to a site the account doesn't trust ([`Trust`]): a request there is `Untrusted`. The
//! password is never sent first: it answers a server that asks for it (a 401 offering Basic),
//! and only one on a site of the account's servers, or one the user allowed.
//!
//! A sync first sends what changed here (deletions, then edits and new items, each guarded by
//! the item's etag so a newer copy on the server is never overwritten), then reads every
//! collection whose version (`getctag`) moved: the items' etags, and the data of those that are
//! new or changed (`multiget`). The engine merges the result ([`DavSync`]).
//!
//! Server answers are hostile: sizes are capped and XML is read without entities beyond XML's
//! own.

mod answers;
mod discovery;
mod syncing;
#[cfg(test)]
mod tests;

use quatrelle_model::AccountConfig;
use quatrelle_model::addr::trust::{Trust, site, url_host};
use quatrelle_model::pim::Pim;

use crate::http::{self, Request, Response, Url};
use crate::{NetError, Result};
pub use answers::{Item, multistatus};
pub use syncing::sync;

/// Largest answer read.
const MAX_BODY: usize = 32 << 20;
/// Items fetched per `multiget`.
const BATCH: usize = 50;
const DAV: &str = "DAV:";
const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
const CARDDAV: &str = "urn:ietf:params:xml:ns:carddav";

/// What a collection holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Calendars,
    Contacts,
}

impl Kind {
    fn well_known(self) -> &'static str {
        match self {
            Kind::Calendars => "/.well-known/caldav",
            Kind::Contacts => "/.well-known/carddav",
        }
    }
    fn srv(self) -> &'static str {
        match self {
            Kind::Calendars => "_caldavs._tcp",
            Kind::Contacts => "_carddavs._tcp",
        }
    }
    fn home(self) -> &'static str {
        match self {
            Kind::Calendars => "calendar-home-set",
            Kind::Contacts => "addressbook-home-set",
        }
    }
}

/// A signed-in DAV session.
pub struct Dav {
    auth: String,
    /// The sites a request may go to at all.
    trust: Trust,
    /// The sites the password may answer: `trust`'s, less the address's own domain when no
    /// server of the account is on it and the user didn't allow it.
    password: Trust,
    /// The servers (origins) that asked for the password and had it, this session: the next
    /// requests there carry it.
    signed_in: std::sync::Mutex<std::collections::BTreeSet<String>>,
}

/// What an account's DAV session trusts: its own servers and allowed domains, and the homes
/// found and saved before (`pim`'s), where the password has already gone, unless the user
/// withdrew their site since.
pub fn trust_of(acc: &AccountConfig, pim: &Pim) -> Trust {
    let home = |h: &Option<String>| h.clone().unwrap_or_default();
    Trust::of(acc, &[&home(&pim.caldav_home), &home(&pim.carddav_home)])
}

/// Where an account's password may answer a DAV server that asks for it: the sites of its
/// servers, the domains the user allowed and the homes saved before, not the address's own
/// domain for being the address's (`Trust::of_servers`).
pub fn password_trust_of(acc: &AccountConfig, pim: &Pim) -> Trust {
    let home = |h: &Option<String>| h.clone().unwrap_or_default();
    Trust::of_servers(acc, &[&home(&pim.caldav_home), &home(&pim.carddav_home)])
}

impl Dav {
    /// A session whose password may answer any site of `trust`.
    pub fn new(user: &str, password: &str, trust: Trust) -> Dav {
        let auth = format!("Basic {}", crate::imap::base64(format!("{user}:{password}").as_bytes()));
        Dav { auth, password: trust.clone(), trust, signed_in: std::sync::Mutex::default() }
    }

    /// An account's session: requests go to the sites it trusts ([`trust_of`]), the password
    /// to those of [`password_trust_of`].
    pub fn for_account(user: &str, password: &str, acc: &AccountConfig, pim: &Pim) -> Dav {
        Dav { password: password_trust_of(acc, pim), ..Dav::new(user, password, trust_of(acc, pim)) }
    }

    /// Send a request to `here`, without the password unless this server asked for it before.
    /// A server that asks (a 401 offering Basic) is answered when its site may have the
    /// password, and is `Untrusted` otherwise: nothing more goes there until the user allows
    /// the site. Any other 401 is the caller's to read (no sign-in Quatrelle can make).
    fn send(&self, req: Request, here: &Url) -> Result<Response> {
        let origin = here.origin();
        let known = self.signed_in.lock().unwrap_or_else(std::sync::PoisonError::into_inner).contains(&origin);
        if known {
            return http::send(&req.header("Authorization", &self.auth));
        }
        let resp = http::send(&req)?;
        if resp.status != 401 || !offers_basic(&resp) {
            return Ok(resp);
        }
        let Some(host) = safe_host(here) else {
            return Err(NetError::Protocol("the server named an address that can't be read safely".into()));
        };
        if !self.password.allows(&host) {
            return Err(NetError::Untrusted { site: site(&host) });
        }
        let resp = http::send(&req.header("Authorization", &self.auth))?;
        if resp.status != 401 {
            let mut known = self.signed_in.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            // A session talks to a handful of servers; a cap all the same.
            if known.len() < 64 {
                known.insert(origin);
            }
        }
        Ok(resp)
    }

    /// One request, following redirects (WebDAV methods keep their method and body), signed in
    /// as [`Dav::send`] does. Nothing at all goes to a site the account doesn't trust, the
    /// first address's or a redirect's: `Untrusted` names it instead.
    fn request(&self, method: &str, url: &str, depth: Option<&str>, body: &str, extra: &[(&str, &str)]) -> Result<(Response, String)> {
        let mut url = url.to_string();
        for _ in 0..6 {
            let here = Url::parse(&url)?;
            self.reachable(&here)?;
            let mut req = if body.is_empty() {
                Request { method: method.into(), url: url.clone(), max: MAX_BODY, ..Default::default() }
            } else {
                Request {
                    method: method.into(),
                    url: url.clone(),
                    headers: vec![("Content-Type".into(), "application/xml; charset=utf-8".into())],
                    body: body.as_bytes().to_vec(),
                    max: MAX_BODY,
                    ..Default::default()
                }
            };
            if let Some(d) = depth {
                req = req.header("Depth", d);
            }
            for (k, v) in extra {
                req = req.header(k, v);
            }
            let resp = self.send(req, &here)?;
            if matches!(resp.status, 301 | 302 | 303 | 307 | 308)
                && let Some(loc) = resp.header("Location")
            {
                let next = here.join(loc)?;
                if here.https && !next.https {
                    return Err(NetError::Tls("redirected from https to http".into()));
                }
                url = format!("{}{}", next.origin(), next.path);
                continue;
            }
            if matches!(resp.status, 401 | 403) {
                return Err(NetError::Auth("the calendar server refused the user name or password".into()));
            }
            return Ok((resp, url));
        }
        Err(NetError::Protocol("too many redirects".into()))
    }

    fn propfind(&self, url: &str, depth: &str, props: &str) -> Result<(Vec<Item>, String)> {
        let body = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><d:propfind xmlns:d=\"{DAV}\" xmlns:c=\"{CALDAV}\" xmlns:card=\"{CARDDAV}\" xmlns:cs=\"http://calendarserver.org/ns/\" xmlns:a=\"http://apple.com/ns/ical/\"><d:prop>{props}</d:prop></d:propfind>"
        );
        let (resp, at) = self.request("PROPFIND", url, Some(depth), &body, &[])?;
        if resp.status != 207 {
            return Err(NetError::No(format!("PROPFIND answered HTTP {}", resp.status)));
        }
        Ok((multistatus(&String::from_utf8_lossy(&resp.body)), at))
    }

    /// Store an item: a new one (`etag` `None`) only if nothing is there, else only if it is
    /// unchanged on the server. Returns the new etag (empty when the server doesn't say).
    pub fn put(&self, url: &str, content_type: &str, data: &str, etag: Option<&str>) -> Result<String> {
        let here = Url::parse(url)?;
        self.reachable(&here)?;
        let cond = match etag {
            Some(e) if !e.is_empty() => ("If-Match", e.to_string()),
            _ => ("If-None-Match", "*".to_string()),
        };
        let mut req = Request {
            method: "PUT".into(),
            url: url.into(),
            headers: vec![("Content-Type".into(), content_type.into())],
            body: data.as_bytes().to_vec(),
            max: 1 << 20,
            ..Default::default()
        };
        req = req.header(cond.0, &cond.1);
        let resp = self.send(req, &here)?;
        match resp.status {
            200..=299 => Ok(resp.header("ETag").unwrap_or_default().to_string()),
            401 | 403 => Err(NetError::Auth("the calendar server refused the user name or password".into())),
            412 => Err(NetError::No("changed on the server meanwhile".into())),
            s => Err(NetError::No(format!("the server didn't store it (HTTP {s})"))),
        }
    }

    pub fn delete(&self, url: &str, etag: &str) -> Result<()> {
        let here = Url::parse(url)?;
        self.reachable(&here)?;
        let mut req = Request { method: "DELETE".into(), url: url.into(), max: 1 << 20, ..Default::default() };
        if !etag.is_empty() {
            req = req.header("If-Match", etag);
        }
        let resp = self.send(req, &here)?;
        match resp.status {
            200..=299 | 404 | 410 => Ok(()),
            401 | 403 => Err(NetError::Auth("the calendar server refused the user name or password".into())),
            s => Err(NetError::No(format!("the server didn't delete it (HTTP {s})"))),
        }
    }
}

/// Whether a 401 asks for the one sign-in a DAV session has: a `WWW-Authenticate` naming Basic.
fn offers_basic(resp: &Response) -> bool {
    resp.headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case("WWW-Authenticate")).any(|(_, v)| {
        // One header may list several challenges, each starting with its scheme.
        v.split(',').any(|c| c.trim_start().get(..5).is_some_and(|s| s.eq_ignore_ascii_case("basic")))
    })
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

impl Dav {
    /// Whether the password, or anything at all, may go to this address: its host's site is
    /// one the account trusts. `Untrusted` names the site otherwise.
    fn reachable(&self, url: &Url) -> Result<()> {
        // A host two parsers could read differently (`evil.example\.good.example`, whose site
        // reads `good.example`) goes nowhere, and is no domain to ask about either.
        let Some(host) = safe_host(url) else {
            return Err(NetError::Protocol("the server named an address that can't be read safely".into()));
        };
        if self.trust.allows(&host) { Ok(()) } else { Err(NetError::Untrusted { site: site(&host) }) }
    }

    /// An href (absolute, or a path) made absolute against the address it came from. The
    /// server's answer is no reason to go elsewhere: an href on a site the account doesn't
    /// trust is `Untrusted`, and one from https down to plain http is refused.
    fn absolute(&self, base: &str, href: &str) -> Result<String> {
        let from = Url::parse(base)?;
        let u = from.join(href.trim())?;
        if from.https && !u.https {
            return Err(NetError::Tls("an address the server gave goes from https down to http".into()));
        }
        self.reachable(&u)?;
        Ok(format!("{}{}", u.origin(), u.path))
    }
}

/// The host of an address as the trust writes it, when it reads one way only. (`Url` keeps an
/// IPv6 address without its brackets; the trust, and `site`, write it with them, as a URL does.)
fn safe_host(url: &Url) -> Option<String> {
    url_host(&if url.host.contains(':') { format!("[{}]", url.host) } else { url.host.clone() })
}

/// Whether an address saved here can be sent at all: a web address whose host reads one way only.
fn readable(url: &str) -> bool {
    Url::parse(url).is_ok_and(|u| safe_host(&u).is_some())
}

/// The path of an absolute address (multiget names items by path).
fn path_of(url: &str) -> String {
    Url::parse(url).map(|u| u.path).unwrap_or_else(|_| url.to_string())
}
