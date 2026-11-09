// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! EWS: Autodiscover — a server's address found from an email address, and what its answer says about a mailbox.

use quatrelle_model::addr::trust::{Trust, site, url_host};

use super::*;

/// The address in an Autodiscover answer about a mailbox.
pub(super) fn autodiscover_smtp(xml: &str) -> Option<String> {
    let doc = parse(xml);
    doc.find("AutoDiscoverSMTPAddress").map(|a| a.text.trim().to_string()).filter(|a| a.contains('@') && a.len() < 320)
}

/// The public folder hierarchy mailbox in an Autodiscover answer about the account (Exchange
/// 2013 and later: `PublicFolderInformation`'s `SmtpAddress`).
pub(super) fn autodiscover_public(xml: &str) -> Option<String> {
    let doc = parse(xml);
    doc.find("PublicFolderInformation")
        .and_then(|p| p.find("SmtpAddress"))
        .map(|a| a.text.trim().to_string())
        .filter(|a| a.contains('@') && a.len() < 320)
}

/// A mailbox Autodiscover maps to the account (Outlook opens these by itself).
pub(super) struct Alternative {
    pub(super) name: String,
    pub(super) smtp: Option<String>,
    pub(super) legacy_dn: Option<String>,
}

/// The mailboxes an Autodiscover answer maps to the account: `AlternativeMailbox` entries for
/// mailboxes it may open (`Delegate`, `TeamMailbox`), not its archive ([`archive_mailbox`]).
pub(super) fn alternative_mailboxes(xml: &str) -> Vec<Alternative> {
    let doc = parse(xml);
    let mut alts = Vec::new();
    doc.all("AlternativeMailbox", &mut alts);
    alts.iter()
        .filter(|a| a.text_of("Type").is_some_and(|t| matches!(t.trim(), "Delegate" | "TeamMailbox")))
        .map(|a| alternative(a))
        .take(MAX_SHARED)
        .collect()
}

/// One `AlternativeMailbox` entry, its texts bounded.
fn alternative(a: &El) -> Alternative {
    let text = |k: &str| a.text_of(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty() && v.len() < 500);
    Alternative { name: text("DisplayName").unwrap_or_default(), smtp: text("SmtpAddress").filter(|v| v.contains('@')), legacy_dn: text("LegacyDN") }
}

/// The account's online archive in an Autodiscover answer: the first `AlternativeMailbox` of type
/// `Archive` (Exchange 2010 SP1 and later; Outlook's "Online Archive"), with the name the server
/// gives it (shortened, without control characters; empty when it gives none) and its address
/// when it has one.
pub(super) fn archive_mailbox(xml: &str) -> Option<Alternative> {
    let doc = parse(xml);
    let mut alts = Vec::new();
    doc.all("AlternativeMailbox", &mut alts);
    let a = alts.iter().find(|a| a.text_of("Type").is_some_and(|t| t.trim() == "Archive"))?;
    let mut found = alternative(a);
    found.name = clean_name(a.text_of("DisplayName").unwrap_or_default());
    found.smtp = found.smtp.filter(|s| s.len() < 320 && !s.chars().any(|c| c.is_control() || c.is_whitespace()));
    Some(found)
}

/// The online archive an Autodiscover answer names, as [`archive_mailbox`] reads it: its name and
/// its address (the fuzz target reads it so).
pub fn archive_named(xml: &str) -> Option<(String, Option<String>)> {
    archive_mailbox(xml).map(|a| (a.name, a.smtp))
}

/// A name the server gives (a mailbox's, a folder's) as it's shown: without control characters,
/// at most [`MAX_NAME`] characters.
pub(super) fn clean_name(name: &str) -> String {
    name.trim().chars().filter(|c| !c.is_control()).take(MAX_NAME).collect::<String>().trim().to_string()
}

/// Characters kept of a name the server gives.
pub(super) const MAX_NAME: usize = 120;

/// The domain of an address, when it can name a host (letters, digits, dots and hyphens), in
/// lower case.
fn address_domain(email: &str) -> Option<String> {
    let domain = email.rsplit_once('@').map(|(_, d)| d.trim().to_ascii_lowercase()).unwrap_or_default();
    let ok = !domain.is_empty() && domain.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    ok.then_some(domain)
}

/// Where Autodiscover is asked (POX, MS-OXDSCLI): the domain's `autodiscover.` host, then the
/// domain itself.
pub(super) fn autodiscover_urls(email: &str) -> Vec<String> {
    let Some(domain) = address_domain(email) else { return Vec::new() };
    vec![format!("https://autodiscover.{domain}/autodiscover/autodiscover.xml"), format!("https://{domain}/autodiscover/autodiscover.xml")]
}

/// `base` with its host replaced by `domain`, its port and path kept: where a test asks about an
/// address on another domain (the fakes answer on 127.0.0.1 and localhost alike).
fn on_host(base: &str, domain: &str) -> String {
    let Some((scheme, rest)) = base.split_once("://") else { return base.to_string() };
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let port = authority.rsplit_once(':').filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit())).map(|(_, p)| format!(":{p}")).unwrap_or_default();
    format!("{scheme}://{domain}{port}{path}")
}

/// Whether the account's credentials may go to this host (an `EwsUrl`'s, a `RedirectAddr`'s
/// domain): `Untrusted` names its site otherwise.
pub(super) fn allowed(host: &str, trust: &Trust) -> Result<()> {
    if trust.allows(host) { Ok(()) } else { Err(NetError::Untrusted { site: site(host) }) }
}

/// Tests: where a worker's Autodiscover asks about an address instead of its domain's servers
/// (the fakes listen on 127.0.0.1 only), as (address, the addresses asked in turn); after a
/// redirect, the first with the new domain for its host.
#[cfg(test)]
pub(crate) static TEST_BASES: std::sync::Mutex<Vec<(String, Vec<String>)>> = std::sync::Mutex::new(Vec::new());

/// The EWS address in an Autodiscover answer (the outside one first), or the address to ask
/// about instead (a redirect).
pub(super) fn autodiscover_answer(xml: &str) -> (Option<String>, Option<String>) {
    let doc = parse(xml);
    let Some(account) = doc.find("Account") else { return (None, None) };
    if account.text_of("Action") == Some("redirectAddr") {
        return (None, account.text_of("RedirectAddr").map(|r| r.trim().to_string()));
    }
    let protocols: Vec<&El> = account.children("Protocol").collect();
    let url = ["EXPR", "EXCH", "WEB"]
        .iter()
        .find_map(|kind| protocols.iter().find(|p| p.text_of("Type") == Some(kind)).and_then(|p| p.text_of("EwsUrl")))
        .map(|u| u.trim().to_string())
        .filter(|u| u.starts_with("https://") || (cfg!(test) && (u.starts_with("http://127.0.0.1") || u.starts_with("http://localhost"))));
    (url, None)
}

/// Find a mailbox's EWS address with Autodiscover, signed in as the account. `base`: where to
/// ask instead of the usual addresses (tests); an address redirected to another domain is asked
/// about at `base` with that domain for its host. `trust`: the sites the account's credentials
/// may go to.
pub fn autodiscover(email: &str, user: &str, password: &str, base: Option<&str>, trust: &Trust) -> Result<String> {
    autodiscover_as(email, user, password, base, None, trust)
}

/// [`autodiscover`], naming itself `agent` to the servers asked.
pub fn autodiscover_as(email: &str, user: &str, password: &str, base: Option<&str>, agent: Option<&str>, trust: &Trust) -> Result<String> {
    // Where the first round asks instead of the address's domain (tests): `base`, or for a
    // worker's own Autodiscover the addresses a test set for this address.
    let first: Option<Vec<String>> = base.map(|b| vec![b.to_string()]);
    #[cfg(test)]
    let first = first.or_else(|| {
        let bases = TEST_BASES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        bases.iter().find(|(a, _)| a == email).map(|(_, b)| b.clone())
    });
    let mut address = email.to_string();
    for round in 0..3 {
        let body = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><Autodiscover xmlns=\"http://schemas.microsoft.com/exchange/autodiscover/outlook/requestschema/2006\">\
             <Request><EMailAddress>{}</EMailAddress><AcceptableResponseSchema>http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a</AcceptableResponseSchema></Request></Autodiscover>",
            esc(&address)
        );
        let urls = match &first {
            Some(f) if round == 0 => f.clone(),
            Some(f) => f.first().zip(address_domain(&address)).map(|(b, d)| vec![on_host(b, &d)]).unwrap_or_default(),
            None => autodiscover_urls(&address),
        };
        let mut redirect = None;
        let mut last = NetError::No("Autodiscover found no Exchange server for this address".into());
        // An Exchange's answer naming a site the account doesn't trust. It stands, as Outlook
        // takes it: the domain's other address isn't asked (its web server's 401 would read as a
        // wrong password, and the question would never come).
        let mut untrusted = None;
        for url in urls {
            // After a redirect, each address asked must itself be on a trusted site: a domain
            // allowed that is a public suffix itself mustn't widen what is asked.
            if round > 0 {
                match url_host(&url).map(|h| allowed(&h, trust)) {
                    Some(Ok(())) => {}
                    Some(Err(e)) => {
                        untrusted.get_or_insert(e);
                        continue;
                    }
                    None => {
                        last = NetError::Protocol("Autodiscover was sent to an address that can't be read safely".into());
                        continue;
                    }
                }
            }
            let c = match Client::new(&url, user, password, email) {
                Ok(c) => c.with_agent(agent),
                Err(e) => {
                    last = e;
                    continue;
                }
            };
            match c.post(body.clone().into_bytes()) {
                Ok(resp) if resp.ok() => match autodiscover_answer(&String::from_utf8_lossy(&resp.body)) {
                    // The server the mailbox is on: only on a site the account trusts.
                    (Some(found), _) => match url_host(&found).map(|h| allowed(&h, trust)) {
                        Some(Ok(())) => return Ok(found),
                        Some(Err(e)) => {
                            untrusted = Some(e);
                            break;
                        }
                        None => last = NetError::Protocol("Autodiscover named an address that can't be read safely".into()),
                    },
                    // Another address to ask about: only on a domain the account trusts, or
                    // the password would go there with the next question.
                    (None, Some(r)) if r != address && r.contains('@') => {
                        if let Some(Err(e)) = address_domain(&r).map(|d| allowed(&d, trust)) {
                            untrusted = Some(e);
                            break;
                        }
                        // Trusted, or no domain to ask (then there's nowhere to ask either).
                        redirect = Some(r);
                        break;
                    }
                    _ => {}
                },
                Ok(resp) => last = NetError::Protocol(format!("Autodiscover answered HTTP {}", resp.status)),
                Err(NetError::Auth(m)) => return Err(NetError::Auth(m)),
                Err(e) => last = e,
            }
        }
        match (redirect, untrusted) {
            (Some(r), _) => address = r,
            (None, Some(e)) => return Err(e),
            (None, None) => return Err(last),
        }
    }
    Err(NetError::No("Autodiscover sent Quatrelle round in circles".into()))
}
