//! Finding where the user's calendars or address books live (RFC 6764): from each starting
//! address, the principal, then its home.

use super::{Dav, Kind, absolute};
use crate::Result;

impl Dav {
    /// The home of the user's calendars or address books, found from the account's domain.
    pub fn discover(&self, kind: Kind, email: &str, imap_host: &str) -> Result<Option<String>> {
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
        // A refused sign-in while looking (Google's servers want OAuth) means "not here".
        for start in starts {
            if let Ok(Some(h)) = self.home_from(kind, &start) {
                return Ok(Some(h));
            }
        }
        Ok(None)
    }

    /// From a starting address: the principal, then its home.
    pub fn home_from(&self, kind: Kind, start: &str) -> Result<Option<String>> {
        let (items, at) = self.propfind(start, "0", "<d:current-user-principal/>")?;
        let Some(principal) = items.iter().find_map(|i| i.hrefs.get("current-user-principal").and_then(|h| h.first())).cloned() else {
            return Ok(None);
        };
        let principal = absolute(&at, &principal)?;
        let prop = if kind == Kind::Calendars { "<c:calendar-home-set/>" } else { "<card:addressbook-home-set/>" };
        let (items, at) = self.propfind(&principal, "0", prop)?;
        let home = items.iter().find_map(|i| i.hrefs.get(kind.home()).and_then(|h| h.first())).cloned();
        home.map(|h| absolute(&at, &h)).transpose()
    }
}
