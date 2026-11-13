// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A sync of an account's calendars and address books: what changed here goes up, then what the
//! server has comes down.

use std::collections::HashMap;

use quatrelle_model::ical::{self, CalEvent};
use quatrelle_model::pim::{BookInfo, CalendarInfo, DavSync, LOCAL, Pim, StoredCard, StoredEvent};
use quatrelle_model::vcard;

use super::discovery::starts;
use super::{BATCH, CALDAV, CARDDAV, DAV, Dav, Item, Kind, multistatus, path_of, readable, xml_escape};
use crate::{NetError, Result};

impl Dav {
    /// The collections in a home (calendars that hold events, or address books).
    pub fn collections(&self, kind: Kind, home: &str) -> Result<Vec<Item>> {
        let props = "<d:resourcetype/><d:displayname/><cs:getctag/><d:sync-token/><a:calendar-color/><c:supported-calendar-component-set/><d:current-user-privilege-set/>";
        let (items, at) = self.propfind(home, "1", props)?;
        let want = if kind == Kind::Calendars { "calendar" } else { "addressbook" };
        let mut out = Vec::new();
        for mut i in items.into_iter().filter(|i| i.kinds.iter().any(|k| k == want)) {
            if kind == Kind::Calendars && !i.comps.is_empty() && !i.comps.iter().any(|c| c == "VEVENT") {
                continue;
            }
            i.href = self.absolute(&at, &i.href)?;
            out.push(i);
        }
        Ok(out)
    }

    /// The items in a collection and their etags.
    pub fn etags(&self, collection: &str) -> Result<Vec<(String, String)>> {
        let (items, at) = self.propfind(collection, "1", "<d:getetag/><d:resourcetype/>")?;
        let mut out = Vec::new();
        for i in items {
            if i.kinds.iter().any(|k| k == "collection") {
                continue;
            }
            let Some(etag) = i.props.get("getetag") else { continue };
            out.push((self.absolute(&at, &i.href)?, etag.clone()));
        }
        Ok(out)
    }

    /// The data of items (`calendar-data` or `address-data`), by absolute href.
    pub fn multiget(&self, kind: Kind, collection: &str, hrefs: &[String]) -> Result<Vec<(String, String, String)>> {
        let mut out = Vec::new();
        for chunk in hrefs.chunks(BATCH) {
            let refs: String = chunk.iter().map(|h| format!("<d:href>{}</d:href>", xml_escape(&path_of(h)))).collect();
            let body = match kind {
                Kind::Calendars => format!(
                    "<?xml version=\"1.0\" encoding=\"utf-8\"?><c:calendar-multiget xmlns:d=\"{DAV}\" xmlns:c=\"{CALDAV}\"><d:prop><d:getetag/><c:calendar-data/></d:prop>{refs}</c:calendar-multiget>"
                ),
                Kind::Contacts => format!(
                    "<?xml version=\"1.0\" encoding=\"utf-8\"?><card:addressbook-multiget xmlns:d=\"{DAV}\" xmlns:card=\"{CARDDAV}\"><d:prop><d:getetag/><card:address-data/></d:prop>{refs}</card:addressbook-multiget>"
                ),
            };
            let (resp, at) = self.request("REPORT", collection, Some("1"), &body, &[])?;
            if resp.status != 207 {
                return Err(NetError::No(format!("REPORT answered HTTP {}", resp.status)));
            }
            let field = if kind == Kind::Calendars { "calendar-data" } else { "address-data" };
            for i in multistatus(&String::from_utf8_lossy(&resp.body)) {
                if let Some(data) = i.props.get(field) {
                    out.push((self.absolute(&at, &i.href)?, i.props.get("getetag").cloned().unwrap_or_default(), data.clone()));
                }
            }
        }
        Ok(out)
    }
}

/// The longest uid used as a file name as it is.
const NAME_KEEP: usize = 100;

/// A file name for a new item (one already on the server keeps the address it has). A short
/// uid of plain characters is the name; any other keeps its readable start and adds `~` and a
/// hash of the whole uid, so two uids alike in what's kept still get names of their own (a plain
/// name never has a `~`).
fn resource_name(uid: &str, ext: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    if !uid.is_empty() && uid.len() <= NAME_KEEP && uid.chars().all(plain) {
        return format!("{uid}.{ext}");
    }
    let safe: String = uid.chars().map(|c| if plain(c) { c } else { '-' }).take(NAME_KEEP).collect();
    format!("{safe}~{:016x}.{ext}", quatrelle_store::fnv64(uid))
}

/// Sync an account's calendars and address books with its server. `pim` is what the engine
/// last saved; nothing is written here: the result is for the engine to merge.
pub fn sync(dav: &Dav, email: &str, imap_host: &str, pim: &Pim, now: i64, local: quatrelle_model::zone::Zone) -> Result<DavSync> {
    sync_from(dav, &|kind| starts(kind, email, imap_host), pim, now, local)
}

/// [`sync`], a missing home looked for from `starts`' addresses.
pub(super) fn sync_from(dav: &Dav, starts: &dyn Fn(Kind) -> Vec<String>, pim: &Pim, now: i64, local: quatrelle_model::zone::Zone) -> Result<DavSync> {
    let mut out = DavSync::default();
    // Where the collections live (looked up once; again a day after finding none, or at once
    // when the site the last look stopped at has been allowed since).
    let allowed = pim.dav_untrusted.as_deref().is_some_and(|s| dav.trust.allows(s));
    let retry = allowed || pim.dav_checked.is_none_or(|t| now - t > 86_400);
    // A look that stops at a site the account doesn't trust finds nothing, as one that finds no
    // server does, and is noted (to ask about); the other kind, and what has a home, sync all the
    // same.
    let find = |kind: Kind, out: &mut DavSync| -> Result<Option<String>> {
        out.looked = true;
        match dav.discover_from(kind, &starts(kind)) {
            Err(NetError::Untrusted { site }) => {
                out.untrusted.get_or_insert(site);
                Ok(None)
            }
            r => r,
        }
    };
    out.caldav_home = match &pim.caldav_home {
        Some(h) if !h.is_empty() => Some(h.clone()),
        _ if retry => find(Kind::Calendars, &mut out)?,
        _ => None,
    };
    out.carddav_home = match &pim.carddav_home {
        Some(h) if !h.is_empty() => Some(h.clone()),
        _ if retry => find(Kind::Contacts, &mut out)?,
        _ => None,
    };
    out.checked = now;

    // 1. Deletions made here.
    for t in &pim.deleted {
        // Saved with an address that can't be read safely: it can never be sent, and would
        // stay for good.
        if !readable(&t.href) {
            log::info!("calendars and contacts: a deletion whose address can't be read is dropped");
            out.deleted.push(t.href.clone());
            continue;
        }
        match dav.delete(&t.href, &t.etag) {
            Ok(()) => out.deleted.push(t.href.clone()),
            // The account's own problems, not the item's: kept with the item's errors, they'd
            // come back at each sync (a tombstone would stay for good).
            Err(e @ (NetError::Auth(_) | NetError::Untrusted { .. })) => return Err(e),
            Err(e) => out.errors.push(e.to_string()),
        }
    }

    // 2. Changes made here, per item (an event with its changed instances).
    let mut uids: Vec<&str> = pim.events.iter().filter(|e| e.dirty && e.calendar != LOCAL).map(|e| e.event.uid.as_str()).collect();
    uids.sort_unstable();
    uids.dedup();
    for uid in uids {
        let parts: Vec<&StoredEvent> = pim.events.iter().filter(|e| e.event.uid == uid).collect();
        let Some(master) = parts.iter().find(|e| e.event.recurrence_id.is_none()).or_else(|| parts.first()) else { continue };
        let url = if master.href.is_empty() {
            format!("{}/{}", master.calendar.trim_end_matches('/'), resource_name(uid, "ics"))
        } else {
            master.href.clone()
        };
        let events: Vec<CalEvent> = parts.iter().map(|e| e.event.clone()).collect();
        let data = ical::write(&events, None, now);
        let etag = (!master.href.is_empty()).then_some(master.etag.as_str());
        match dav.put(&url, "text/calendar; charset=utf-8", &data, etag) {
            Ok(new) => {
                for p in parts {
                    out.pushed_events.push(StoredEvent { href: url.clone(), etag: new.clone(), dirty: false, ..(*p).clone() });
                }
            }
            Err(e @ (NetError::Auth(_) | NetError::Untrusted { .. })) => return Err(e),
            Err(e) => out.errors.push(format!("{}: {e}", master.event.summary)),
        }
    }
    for c in pim.cards.iter().filter(|c| c.dirty && c.book != LOCAL) {
        let url = if c.href.is_empty() { format!("{}/{}", c.book.trim_end_matches('/'), resource_name(&c.card.uid, "vcf")) } else { c.href.clone() };
        let etag = (!c.href.is_empty()).then_some(c.etag.as_str());
        match dav.put(&url, "text/vcard; charset=utf-8", &vcard::write(&c.card), etag) {
            Ok(new) => out.pushed_cards.push(StoredCard { href: url, etag: new, dirty: false, ..c.clone() }),
            Err(e @ (NetError::Auth(_) | NetError::Untrusted { .. })) => return Err(e),
            Err(e) => out.errors.push(format!("{}: {e}", c.card.display())),
        }
    }

    // 3. What the server has.
    if let Some(home) = &out.caldav_home {
        let mut calendars = Vec::new();
        for (n, c) in dav.collections(Kind::Calendars, home)?.into_iter().enumerate() {
            let ctag = c.props.get("getctag").or_else(|| c.props.get("sync-token")).cloned().unwrap_or_default();
            let known = pim.calendars.iter().find(|k| k.id == c.href);
            let info = CalendarInfo {
                id: c.href.clone(),
                name: c.props.get("displayname").cloned().unwrap_or_else(|| "Calendar".into()),
                color: known.map_or(u8::try_from(n % 8).unwrap_or(0), |k| k.color),
                read_only: c.writable == Some(false),
                ctag: ctag.clone(),
                group: String::new(),
                main: false,
                not_own: false,
            };
            let pushed_here = out.pushed_events.iter().any(|e| e.calendar == c.href);
            if !ctag.is_empty() && known.is_some_and(|k| k.ctag == ctag) && !pushed_here {
                out.unchanged_calendars.push(c.href.clone());
            } else {
                let have: HashMap<&str, &str> =
                    pim.events.iter().filter(|e| e.calendar == c.href).map(|e| (e.href.as_str(), e.etag.as_str())).collect();
                let listed = dav.etags(&c.href)?;
                let fetch: Vec<String> = listed.iter().filter(|(h, t)| have.get(h.as_str()) != Some(&t.as_str())).map(|(h, _)| h.clone()).collect();
                // Unchanged: the engine keeps what it has.
                out.kept.extend(listed.iter().filter(|(h, _)| !fetch.contains(h)).map(|(h, _)| h.clone()));
                for (href, etag, data) in dav.multiget(Kind::Calendars, &c.href, &fetch)? {
                    for event in ical::parse_in(&data, local).events {
                        out.events.push(StoredEvent {
                            calendar: c.href.clone(),
                            href: href.clone(),
                            etag: etag.clone(),
                            event,
                            dirty: false,
                            notify: false,
                            respond: None,
                            changed_too: false,
                        });
                    }
                }
                out.synced_calendars.push(c.href.clone());
            }
            calendars.push(info);
        }
        out.calendars = Some(calendars);
    }
    if let Some(home) = &out.carddav_home {
        let mut books = Vec::new();
        for c in dav.collections(Kind::Contacts, home)? {
            let ctag = c.props.get("getctag").or_else(|| c.props.get("sync-token")).cloned().unwrap_or_default();
            let known = pim.books.iter().find(|k| k.id == c.href);
            let info = BookInfo {
                id: c.href.clone(),
                name: c.props.get("displayname").cloned().unwrap_or_else(|| "Contacts".into()),
                read_only: c.writable == Some(false),
                ctag: ctag.clone(),
            };
            let pushed_here = out.pushed_cards.iter().any(|x| x.book == c.href);
            if !ctag.is_empty() && known.is_some_and(|k| k.ctag == ctag) && !pushed_here {
                out.unchanged_books.push(c.href.clone());
            } else {
                let have: HashMap<&str, &str> = pim.cards.iter().filter(|x| x.book == c.href).map(|x| (x.href.as_str(), x.etag.as_str())).collect();
                let listed = dav.etags(&c.href)?;
                let fetch: Vec<String> = listed.iter().filter(|(h, t)| have.get(h.as_str()) != Some(&t.as_str())).map(|(h, _)| h.clone()).collect();
                out.kept.extend(listed.iter().filter(|(h, _)| !fetch.contains(h)).map(|(h, _)| h.clone()));
                for (href, etag, data) in dav.multiget(Kind::Contacts, &c.href, &fetch)? {
                    for card in vcard::parse(&data) {
                        out.cards.push(StoredCard { book: c.href.clone(), href: href.clone(), etag: etag.clone(), card, dirty: false });
                    }
                }
                out.synced_books.push(c.href.clone());
            }
            books.push(info);
        }
        out.books = Some(books);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Item names are one per uid, short, and safe in an address.
    #[test]
    fn item_names_dont_collide() {
        assert_eq!(resource_name("plain-uid_1.2", "ics"), "plain-uid_1.2.ics", "a plain uid is its own name");
        let odd = resource_name("a/b c@x", "ics");
        assert!(odd.starts_with("a-b-c-x~") && odd.ends_with(".ics"), "{odd}");
        assert_ne!(resource_name("a/b", "ics"), resource_name("a b", "ics"), "changed characters don't collide");
        assert_ne!(resource_name("a-b", "ics"), resource_name("a b", "ics"));
        let long = "z".repeat(200);
        let (one, two) = (resource_name(&format!("{long}1"), "ics"), resource_name(&format!("{long}2"), "ics"));
        assert_ne!(one, two, "long uids alike at the start don't collide");
        assert_ne!(resource_name(&"y".repeat(101), "ics"), resource_name(&"y".repeat(102), "ics"));
        assert_eq!(one, resource_name(&format!("{long}1"), "ics"), "stable");
        for uid in ["", "é", "../..", "a\r\nb", &"€".repeat(500), &long] {
            let n = resource_name(uid, "vcf");
            assert!(n.len() <= NAME_KEEP + 21 && n.chars().all(|c| c.is_ascii_alphanumeric() || "-_.~".contains(c)), "{uid:?}: {n}");
        }
    }
}
