//! Calendars and address books on the account's server: CalDAV (RFC 4791) and CardDAV
//! (RFC 6352) over WebDAV (RFC 4918), found as RFC 6764 describes (an SRV record, then the
//! domain's `/.well-known/` addresses), signed in with the account's password.
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
}

impl Dav {
    pub fn new(user: &str, password: &str) -> Dav {
        Dav { auth: format!("Basic {}", crate::imap::base64(format!("{user}:{password}").as_bytes())) }
    }

    /// One request, following redirects (WebDAV methods keep their method and body). The
    /// password goes only to the server the request was given for: from the first redirect to
    /// another origin (scheme, host or port) the following requests carry no `Authorization`
    /// or `Cookie`, and a redirect back doesn't bring them back.
    fn request(&self, method: &str, url: &str, depth: Option<&str>, body: &str, extra: &[(&str, &str)]) -> Result<(Response, String)> {
        let mut url = url.to_string();
        let first = Url::parse(&url)?;
        let mut left_origin = false;
        for _ in 0..6 {
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
            if !left_origin {
                req = req.header("Authorization", &self.auth);
            }
            if let Some(d) = depth {
                req = req.header("Depth", d);
            }
            for (k, v) in extra.iter().filter(|(k, _)| !left_origin || !http::is_credential(k)) {
                req = req.header(k, v);
            }
            let resp = http::send(&req)?;
            if matches!(resp.status, 301 | 302 | 303 | 307 | 308)
                && let Some(loc) = resp.header("Location")
            {
                let from = Url::parse(&url)?;
                let next = from.join(loc)?;
                if from.https && !next.https {
                    return Err(NetError::Tls("redirected from https to http".into()));
                }
                left_origin |= !http::same_site(&first, &next);
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
        req = req.header("Authorization", &self.auth).header(cond.0, &cond.1);
        let resp = http::send(&req)?;
        match resp.status {
            200..=299 => Ok(resp.header("ETag").unwrap_or_default().to_string()),
            401 | 403 => Err(NetError::Auth("the calendar server refused the user name or password".into())),
            412 => Err(NetError::No("changed on the server meanwhile".into())),
            s => Err(NetError::No(format!("the server didn't store it (HTTP {s})"))),
        }
    }

    pub fn delete(&self, url: &str, etag: &str) -> Result<()> {
        let mut req = Request { method: "DELETE".into(), url: url.into(), max: 1 << 20, ..Default::default() }.header("Authorization", &self.auth);
        if !etag.is_empty() {
            req = req.header("If-Match", etag);
        }
        let resp = http::send(&req)?;
        match resp.status {
            200..=299 | 404 | 410 => Ok(()),
            401 | 403 => Err(NetError::Auth("the calendar server refused the user name or password".into())),
            s => Err(NetError::No(format!("the server didn't delete it (HTTP {s})"))),
        }
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// An href (absolute, or a path) made absolute against the address it came from.
fn absolute(base: &str, href: &str) -> Result<String> {
    let u = Url::parse(base)?.join(href.trim())?;
    Ok(format!("{}{}", u.origin(), u.path))
}

/// The path of an absolute address (multiget names items by path).
fn path_of(url: &str) -> String {
    Url::parse(url).map(|u| u.path).unwrap_or_else(|_| url.to_string())
}
