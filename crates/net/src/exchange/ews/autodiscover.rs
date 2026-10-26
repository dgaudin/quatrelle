//! EWS: Autodiscover — a server's address found from an email address, and what its answer says about a mailbox.

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
/// mailboxes it may open (`Delegate`, `TeamMailbox`), not its archive.
pub(super) fn alternative_mailboxes(xml: &str) -> Vec<Alternative> {
    let doc = parse(xml);
    let mut alts = Vec::new();
    doc.all("AlternativeMailbox", &mut alts);
    alts.iter()
        .filter(|a| a.text_of("Type").is_some_and(|t| matches!(t.trim(), "Delegate" | "TeamMailbox")))
        .map(|a| {
            let text = |k: &str| a.text_of(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty() && v.len() < 500);
            Alternative {
                name: text("DisplayName").unwrap_or_default(),
                smtp: text("SmtpAddress").filter(|v| v.contains('@')),
                legacy_dn: text("LegacyDN"),
            }
        })
        .take(MAX_SHARED)
        .collect()
}

/// Where Autodiscover is asked (POX, MS-OXDSCLI): the domain's `autodiscover.` host, then the
/// domain itself.
pub(super) fn autodiscover_urls(email: &str) -> Vec<String> {
    let domain = email.rsplit_once('@').map(|(_, d)| d.trim().to_ascii_lowercase()).unwrap_or_default();
    if domain.is_empty() || !domain.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        return Vec::new();
    }
    vec![format!("https://autodiscover.{domain}/autodiscover/autodiscover.xml"), format!("https://{domain}/autodiscover/autodiscover.xml")]
}

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
        .filter(|u| u.starts_with("https://") || (cfg!(test) && u.starts_with("http://127.0.0.1")));
    (url, None)
}

/// Find a mailbox's EWS address with Autodiscover, signed in as the account. `base`: where to
/// ask instead of the usual addresses (tests).
pub fn autodiscover(email: &str, user: &str, password: &str, base: Option<&str>) -> Result<String> {
    autodiscover_as(email, user, password, base, None)
}

/// [`autodiscover`], naming itself `agent` to the servers asked.
pub fn autodiscover_as(email: &str, user: &str, password: &str, base: Option<&str>, agent: Option<&str>) -> Result<String> {
    let mut address = email.to_string();
    for _ in 0..3 {
        let body = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><Autodiscover xmlns=\"http://schemas.microsoft.com/exchange/autodiscover/outlook/requestschema/2006\">\
             <Request><EMailAddress>{}</EMailAddress><AcceptableResponseSchema>http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a</AcceptableResponseSchema></Request></Autodiscover>",
            esc(&address)
        );
        let urls = base.map_or_else(|| autodiscover_urls(&address), |b| vec![b.to_string()]);
        let mut redirect = None;
        let mut last = NetError::No("Autodiscover found no Exchange server for this address".into());
        for url in urls {
            let c = match Client::new(&url, user, password, email) {
                Ok(c) => c.with_agent(agent),
                Err(e) => {
                    last = e;
                    continue;
                }
            };
            match c.post(body.clone().into_bytes()) {
                Ok(resp) if resp.ok() => match autodiscover_answer(&String::from_utf8_lossy(&resp.body)) {
                    (Some(url), _) => return Ok(url),
                    (None, Some(r)) => {
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
        match redirect {
            Some(r) if r != address && r.contains('@') => address = r,
            _ => return Err(last),
        }
    }
    Err(NetError::No("Autodiscover sent Quatrelle round in circles".into()))
}
