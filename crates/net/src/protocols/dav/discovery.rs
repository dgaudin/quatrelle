//! Finding where the user's calendars or address books live (RFC 6764): from each starting
//! address, the principal, then its home.

use super::{Dav, Kind};
use crate::{NetError, Result};

impl Dav {
    /// The home of the user's calendars or address books, found from the account's domain.
    pub fn discover(&self, kind: Kind, email: &str, imap_host: &str) -> Result<Option<String>> {
        self.discover_from(kind, &starts(kind, email, imap_host))
    }

    /// The home found from the first of `starts` that leads to one. A start that leads to a
    /// site the account doesn't trust (a redirect, an href, an SRV target) is skipped, nothing
    /// sent there: the address's website commonly sends everything to a web host, and another
    /// start finding the home must not ask about it. Only when none finds one is the first such
    /// site `Untrusted`.
    pub(super) fn discover_from(&self, kind: Kind, starts: &[String]) -> Result<Option<String>> {
        let mut noted: Option<String> = None;
        for start in starts {
            match self.home_from(kind, start) {
                Ok(Some(h)) => return Ok(Some(h)),
                Err(NetError::Untrusted { site }) => {
                    noted.get_or_insert(site);
                }
                // A refused sign-in while looking (Google's servers want OAuth) means "not here".
                Ok(None) | Err(_) => {}
            }
        }
        match noted {
            Some(site) => Err(NetError::Untrusted { site }),
            None => Ok(None),
        }
    }

    /// From a starting address: the principal, then its home.
    pub fn home_from(&self, kind: Kind, start: &str) -> Result<Option<String>> {
        let (items, at) = self.propfind(start, "0", "<d:current-user-principal/>")?;
        let Some(principal) = items.iter().find_map(|i| i.hrefs.get("current-user-principal").and_then(|h| h.first())).cloned() else {
            return Ok(None);
        };
        let principal = self.absolute(&at, &principal)?;
        let prop = if kind == Kind::Calendars { "<c:calendar-home-set/>" } else { "<card:addressbook-home-set/>" };
        let (items, at) = self.propfind(&principal, "0", prop)?;
        let home = items.iter().find_map(|i| i.hrefs.get(kind.home()).and_then(|h| h.first())).cloned();
        home.map(|h| self.absolute(&at, &h)).transpose()
    }
}

/// Where to look, in order: the SRV record's target, then the domain's, `caldav.`'s (or
/// `carddav.`'s), the IMAP host's sibling's and the IMAP host's `/.well-known/` address.
pub(super) fn starts(kind: Kind, email: &str, imap_host: &str) -> Vec<String> {
    let domain = email.rsplit_once('@').map(|(_, d)| d.trim().to_ascii_lowercase()).unwrap_or_default();
    let mut starts: Vec<String> = Vec::new();
    if !domain.is_empty()
        && let Some((host, port)) = crate::dns::srv(&format!("{}.{domain}", kind.srv()))
    {
        starts.push(format!("https://{host}:{port}{}", kind.well_known()));
    }
    let service = if kind == Kind::Calendars { "caldav" } else { "carddav" };
    for host in [domain.clone(), format!("{service}.{domain}"), imap_host.replacen("imap.", &format!("{service}."), 1), imap_host.to_string()] {
        if !host.is_empty() && !host.contains(['/', ' ']) {
            let u = format!("https://{host}{}", kind.well_known());
            if !starts.contains(&u) {
                starts.push(u);
            }
        }
    }
    starts
}
