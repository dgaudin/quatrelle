// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Finding an account's server settings from its address, the way mail clients have agreed to:
//!
//! 1. the domain's own configuration: `https://autoconfig.<domain>/mail/config-v1.1.xml` and
//!    `https://<domain>/.well-known/autoconfig/mail/config-v1.1.xml` (Mozilla's autoconfig
//!    format, the de-facto standard);
//! 2. the public ISP database (`https://autoconfig.thunderbird.net/v1.1/<domain>`);
//! 3. for the domain of its mail exchanger (MX), which covers domains hosted by Google,
//!    Microsoft, Spacemail and the like: the same database, then that provider's own
//!    configuration (`https://autoconfig.<provider>/mail/config-v1.1.xml`);
//! 4. the host an `_autodiscover._tcp` SRV record names (where Outlook looks), for the same
//!    configuration file;
//! 5. DNS SRV records (RFC 6186: `_imaps._tcp`, `_imap._tcp`, `_submissions._tcp`,
//!    `_submission._tcp`).
//!
//! The first answer wins; a POP server offered alongside is kept (`pop`) for POP accounts.
//! Steps 1 and 2 are answered over HTTPS in the domain's own name, or by the database about the
//! domain. Steps 3 to 5 start from a DNS answer nothing vouches for: servers found that way
//! off the address's own domain are marked ([`Found::unverified`]), and the user is shown them
//! and confirms before a password goes there (RFC 6186, section 6). A server without
//! encryption is never taken from a file. Only
//! the domain's own server is told the address; the others are asked about the domain, and
//! only when the user types an address into the account dialog. Everything fetched is hostile input: sizes are capped and
//! a bad file is simply skipped.
//!
//! A domain whose mail Microsoft 365 hosts (its mail exchanger is Exchange Online's, or its
//! Autodiscover record points at Microsoft) is proposed as Exchange Online through Microsoft
//! Graph rather than IMAP, which is often switched off there; IMAP settings found as well stay
//! in the answer for whoever picks IMAP anyway.

use quatrelle_model::{AccountConfig, Security, ServerConfig};

/// Largest configuration file read.
const MAX_XML: usize = 256 << 10;

/// Settings found for an address, and where they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub config: AccountConfig,
    pub source: String,
    /// The servers were named through DNS records nothing vouches for (a mail exchanger or an
    /// SRV record) and aren't on the address's own domain: they are shown, and the user says
    /// they are right before a password goes to them.
    pub unverified: bool,
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// The text of the first `<tag>…</tag>` in `xml`.
fn text_of(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml.get(start..)?.find(&format!("</{tag}>"))? + start;
    Some(unescape(xml.get(start..end)?.trim()))
}

/// Every `<tag …>…</tag>` block whose start tag contains `attr` (`type="imap"`).
fn blocks<'a>(xml: &'a str, tag: &str, attr: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut from = 0;
    let close = format!("</{tag}>");
    while let Some(i) = xml.get(from..).and_then(|r| r.find(&format!("<{tag}"))) {
        let start = from + i;
        let Some(head_end) = xml.get(start..).and_then(|r| r.find('>')).map(|e| start + e) else { break };
        let Some(end) = xml.get(head_end..).and_then(|r| r.find(&close)).map(|e| head_end + e) else { break };
        let head = xml.get(start..head_end).unwrap_or("").replace('\'', "\"");
        if head.contains(attr) {
            out.push(xml.get(head_end + 1..end).unwrap_or(""));
        }
        from = end + close.len();
    }
    out
}

fn server(block: &str, email: &str) -> Option<(ServerConfig, String)> {
    let host = text_of(block, "hostname")?;
    let port: u16 = text_of(block, "port")?.parse().ok()?;
    // A server without encryption (`plain`, or a kind nobody knows) is never taken from a file
    // found on the network: the password would travel in the clear, with nothing said.
    let security = match text_of(block, "socketType").unwrap_or_default().to_ascii_uppercase().as_str() {
        "SSL" | "TLS" => Security::Tls,
        "STARTTLS" => Security::StartTls,
        _ => return None,
    };
    let (local, domain) = email.rsplit_once('@').unwrap_or((email, ""));
    let user = text_of(block, "username").unwrap_or_else(|| "%EMAILADDRESS%".into());
    let user = user.replace("%EMAILADDRESS%", email).replace("%EMAILLOCALPART%", local).replace("%EMAILDOMAIN%", domain);
    let host = host.replace("%EMAILDOMAIN%", domain);
    let ok = !host.is_empty() && port != 0 && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'));
    ok.then_some((ServerConfig { host: host.to_ascii_lowercase(), port, security }, user))
}

/// The best IMAP and SMTP servers of a Mozilla autoconfig file, and its POP server when it has
/// one (in `pop`). Only encrypted servers are taken (TLS before STARTTLS): a file that offers
/// none for IMAP or for SMTP is no answer.
pub fn parse_config(xml: &str, email: &str) -> Option<AccountConfig> {
    let pick = |tag: &str, kind: &str| -> Option<(ServerConfig, String)> {
        let mut all: Vec<(ServerConfig, String)> =
            blocks(xml, tag, &format!("type=\"{kind}\"")).into_iter().filter_map(|b| server(b, email)).collect();
        all.sort_by_key(|(s, _)| match s.security {
            Security::Tls => 0,
            Security::StartTls | Security::None => 1,
        });
        all.into_iter().next()
    };
    let (imap, user) = pick("incomingServer", "imap")?;
    let (smtp, _) = pick("outgoingServer", "smtp")?;
    let mut c = AccountConfig::guess(email);
    c.imap = imap;
    c.smtp = smtp;
    c.username = user;
    c.pop = pick("incomingServer", "pop3").map(|(s, _)| s);
    Some(c)
}

/// The registrable part of a host name (`aspmx.l.google.com` → `google.com`; two-letter country
/// domains keep one more label: `mx.example.co.uk` → `example.co.uk`).
pub fn base_domain(host: &str) -> String {
    let labels: Vec<&str> = host.trim_end_matches('.').split('.').filter(|l| !l.is_empty()).collect();
    let n = labels.len();
    let keep = if n >= 3
        && labels.get(n - 1).is_some_and(|t| t.len() == 2)
        && labels.get(n - 2).is_some_and(|s| ["co", "com", "net", "org", "ac", "gov", "edu", "ne", "or"].contains(s))
    {
        3
    } else {
        2
    };
    labels[n.saturating_sub(keep)..].join(".")
}

/// The lookups, injectable for tests.
pub struct Lookups<'a> {
    /// GET a URL; the body when it answered 200.
    pub fetch: &'a mut dyn FnMut(&str) -> Option<String>,
    pub mx: &'a mut dyn FnMut(&str) -> Vec<String>,
    pub srv: &'a mut dyn FnMut(&str) -> Option<(String, u16)>,
}

/// Whether Microsoft 365 hosts the domain's mail: its mail exchanger is Exchange Online's
/// (`<domain>.mail.protection.outlook.com`), or its Autodiscover record (where Outlook looks)
/// points at Microsoft. (Outlook.com's own exchangers, `*.olc.protection.outlook.com`, aren't
/// Microsoft 365's: those addresses are known by their domain.)
pub fn at_microsoft_365(mx: &[String], autodiscover: Option<&str>) -> bool {
    let microsoft = |host: &str| {
        let h = host.trim().trim_end_matches('.').to_ascii_lowercase();
        h.ends_with(".mail.protection.outlook.com")
            || h == "autodiscover.outlook.com"
            || h == "outlook.office365.com"
            || h.ends_with(".outlook.office365.com")
    };
    mx.iter().any(|h| microsoft(h)) || autodiscover.is_some_and(microsoft)
}

/// Exchange Online through Microsoft Graph, for a domain Microsoft 365 hosts; the settings found
/// otherwise (IMAP's), if any, stay for whoever picks IMAP anyway.
fn microsoft_365(email: &str, found: Option<Found>) -> Found {
    let unverified = found.as_ref().is_some_and(|f| f.unverified);
    let mut config = found.map(|f| f.config).unwrap_or_else(|| AccountConfig::guess(email));
    config.graph = Some(quatrelle_model::GRAPH_URL.to_string());
    config.oauth = Some("microsoft".into());
    // Signing in with Microsoft sends no password to the servers found; the IMAP settings kept
    // beside it keep their mark, for whoever picks IMAP.
    Found { config, source: "Microsoft 365 (Exchange Online hosts this domain's mail)".into(), unverified }
}

/// Settings reached through a DNS answer (a mail exchanger, an SRV record), which nothing
/// vouches for: anyone on the network can answer first, name a host of their own and serve a
/// configuration there under a certificate of their own. They need no confirming only when
/// every server is on the address's own site: the connection then checks a certificate in that
/// domain's name, which only the domain's owner has (found servers are always encrypted).
fn through_dns(config: AccountConfig, source: String, domain: &str) -> Found {
    use quatrelle_model::addr::trust::site;
    let own = site(domain);
    let hosts = [Some(&config.imap), Some(&config.smtp), config.pop.as_ref()];
    let within = hosts.into_iter().flatten().all(|s| s.security != Security::None && site(&s.host) == own);
    Found { config, source, unverified: !within }
}

/// Whether a name DNS gave reads as a host name (an SRV target is any bytes at all).
fn host_name(host: &str) -> bool {
    !host.is_empty() && host.len() <= 253 && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
}

/// Find settings for `email` (see the module docs for the order).
pub fn discover_with(email: &str, l: &mut Lookups<'_>) -> Option<Found> {
    let email = email.trim();
    let domain = email.rsplit_once('@').map(|(_, d)| d.trim().to_ascii_lowercase()).filter(|d| d.contains('.'))?;
    if !domain.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-')) {
        return None;
    }
    // Asked once: they tell where the mail is (Microsoft 365?), and serve the lookups below.
    let mx = (l.mx)(&domain);
    let autodiscover = (l.srv)(&format!("_autodiscover._tcp.{domain}"));
    let found = settings_for(email, &domain, l, &mx, autodiscover.as_ref().map(|(h, _)| h.as_str()));
    if at_microsoft_365(&mx, autodiscover.as_ref().map(|(h, _)| h.as_str())) {
        return Some(microsoft_365(email, found));
    }
    found
}

/// The published settings for `email` at `domain`, in the module's order, given the domain's
/// mail exchangers and the host its Autodiscover record names.
fn settings_for(email: &str, domain: &str, l: &mut Lookups<'_>, mx: &[String], autodiscover: Option<&str>) -> Option<Found> {
    let enc = crate::http::pct(email);
    let try_url = |l: &mut Lookups<'_>, url: &str| (l.fetch)(url).and_then(|x| parse_config(&x, email));
    for url in [
        format!("https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={enc}"),
        format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
    ] {
        if let Some(c) = try_url(l, &url) {
            return Some(Found { config: c, source: domain.to_string(), unverified: false });
        }
    }
    if let Some(c) = try_url(l, &format!("https://autoconfig.thunderbird.net/v1.1/{domain}")) {
        return Some(Found { config: c, source: "the ISP database".into(), unverified: false });
    }
    if let Some(mx) = mx.first() {
        let base = base_domain(mx);
        if base != domain {
            if let Some(c) = try_url(l, &format!("https://autoconfig.thunderbird.net/v1.1/{base}")) {
                return Some(through_dns(c, format!("the ISP database ({base} handles this domain's mail)"), domain));
            }
            // The mail provider's own configuration (asked about the domain, not the address).
            if let Some(c) = try_url(l, &format!("https://autoconfig.{base}/mail/config-v1.1.xml?emaildomain={domain}")) {
                return Some(through_dns(c, base, domain));
            }
        }
    }
    // Where Outlook looks: the host an `_autodiscover._tcp` record names, which providers point
    // at their configuration server.
    if let Some(host) = autodiscover
        && host_name(host)
        && let Some(c) = try_url(l, &format!("https://{host}/mail/config-v1.1.xml?emaildomain={domain}"))
    {
        return Some(through_dns(c, base_domain(host), domain));
    }
    let named = |l: &mut Lookups<'_>, service: &str, security: Security| {
        (l.srv)(&format!("{service}.{domain}")).filter(|(h, _)| host_name(h)).map(|(h, p)| ServerConfig {
            host: h.to_ascii_lowercase(),
            port: p,
            security,
        })
    };
    let imap = named(l, "_imaps._tcp", Security::Tls).or_else(|| named(l, "_imap._tcp", Security::StartTls));
    let smtp = named(l, "_submissions._tcp", Security::Tls).or_else(|| named(l, "_submission._tcp", Security::StartTls));
    if let (Some(imap), Some(smtp)) = (imap, smtp) {
        let mut c = AccountConfig::guess(email);
        c.imap = imap;
        c.smtp = smtp;
        return Some(through_dns(c, "DNS (SRV records)".into(), domain));
    }
    None
}

/// Find settings for `email` over the network.
pub fn discover(email: &str) -> Option<Found> {
    let mut fetch = |url: &str| {
        crate::http::get(url, MAX_XML)
            .ok()
            .filter(|r| r.ok() && r.header("content-type").is_none_or(|t| t.contains("xml") || t.contains("text")))
            .map(|r| String::from_utf8_lossy(&r.body).to_string())
    };
    let mut mx = crate::dns::mx;
    let mut srv = crate::dns::srv;
    discover_with(email, &mut Lookups { fetch: &mut fetch, mx: &mut mx, srv: &mut srv })
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0"?>
<clientConfig version="1.1">
  <emailProvider id="example.org">
    <incomingServer type="pop3"><hostname>pop.example.org</hostname><port>995</port><socketType>SSL</socketType></incomingServer>
    <incomingServer type="imap"><hostname>plain.example.org</hostname><port>143</port><socketType>plain</socketType><username>%EMAILADDRESS%</username></incomingServer>
    <incomingServer type='imap'>
      <hostname>imap.example.org</hostname>
      <port>993</port>
      <socketType>SSL</socketType>
      <username>%EMAILLOCALPART%</username>
    </incomingServer>
    <outgoingServer type="smtp"><hostname>smtp.%EMAILDOMAIN%</hostname><port>587</port><socketType>STARTTLS</socketType></outgoingServer>
  </emailProvider>
</clientConfig>"#;

    #[test]
    fn reads_mozilla_autoconfig() {
        let c = parse_config(XML, "zoe@example.org").unwrap();
        assert_eq!(c.imap, ServerConfig { host: "imap.example.org".into(), port: 993, security: Security::Tls }, "encrypted IMAP wins");
        assert_eq!(c.smtp, ServerConfig { host: "smtp.example.org".into(), port: 587, security: Security::StartTls });
        assert_eq!(c.username, "zoe");
        assert!(parse_config("<html>nope</html>", "a@b.c").is_none());
        assert!(
            parse_config(&XML.replace("<port>993</port>", "<port>x</port>").replace("<port>143</port>", "<port>0</port>"), "z@example.org").is_none()
        );
        let evil = XML.replace("imap.example.org", "imap.example.org/../x");
        assert!(parse_config(&evil, "z@example.org").is_none(), "bad host names are skipped, and the one in the clear is no fallback");
    }

    /// Live: real DNS and the real ISP database (only domain names are sent). Run by hand with
    /// `cargo test -p quatrelle-net -- --ignored live_`.
    #[test]
    #[ignore]
    fn live_discovery() {
        assert!(!crate::dns::mx("gmail.com").is_empty(), "MX lookup through the system resolver");
        let f = discover("someone@gmail.com").unwrap();
        assert_eq!(f.config.imap.host, "imap.gmail.com");
        println!("gmail.com: {} ({}); smtp {}:{}", f.config.imap.host, f.source, f.config.smtp.host, f.config.smtp.port);
    }

    /// A domain hosted by a provider that publishes its own configuration (not in the ISP
    /// database) and points Outlook at it with an `_autodiscover._tcp` record.
    #[test]
    fn hosted_domains_find_their_provider() {
        const HOSTED: &str = r#"<clientConfig version="1.1"><emailProvider id="mailhost.example">
            <incomingServer type="imap"><hostname>mail.mailhost.example</hostname><port>993</port><socketType>SSL</socketType><username>%EMAILADDRESS%</username></incomingServer>
            <incomingServer type="pop3"><hostname>mail.mailhost.example</hostname><port>995</port><socketType>SSL</socketType><username>%EMAILADDRESS%</username></incomingServer>
            <outgoingServer type="smtp"><hostname>mail.mailhost.example</hostname><port>587</port><socketType>STARTTLS</socketType></outgoingServer>
            <outgoingServer type="smtp"><hostname>mail.mailhost.example</hostname><port>465</port><socketType>SSL</socketType></outgoingServer>
        </emailProvider></clientConfig>"#;
        // Through the mail exchanger's provider.
        let mut asked = Vec::new();
        let mut fetch = |url: &str| {
            asked.push(url.to_string());
            url.starts_with("https://autoconfig.mailhost.example/").then(|| HOSTED.to_string())
        };
        let mut mx = |_: &str| vec!["mx1.mailhost.example".to_string()];
        let mut srv = |_: &str| None;
        let f = discover_with("zoe@studio.example", &mut Lookups { fetch: &mut fetch, mx: &mut mx, srv: &mut srv }).unwrap();
        assert_eq!(f.source, "mailhost.example");
        assert_eq!((f.config.imap.host.as_str(), f.config.smtp.port, f.config.smtp.security), ("mail.mailhost.example", 465, Security::Tls));
        assert_eq!(f.config.pop, Some(ServerConfig { host: "mail.mailhost.example".into(), port: 995, security: Security::Tls }));
        assert_eq!(f.config.username, "zoe@studio.example");
        assert!(
            asked.iter().filter(|u| !u.contains("autoconfig.studio.example")).all(|u| !u.contains("zoe")),
            "only the domain's own server is told the address"
        );
        // Through the `_autodiscover._tcp` record, when the mail exchanger says nothing.
        let mut fetch = |url: &str| url.starts_with("https://autoconfig.mailhost.example/").then(|| HOSTED.to_string());
        let mut no_mx = |_: &str| Vec::new();
        let mut srv = |name: &str| (name == "_autodiscover._tcp.studio.example").then(|| ("autoconfig.mailhost.example".to_string(), 443));
        let f = discover_with("zoe@studio.example", &mut Lookups { fetch: &mut fetch, mx: &mut no_mx, srv: &mut srv }).unwrap();
        assert_eq!((f.source.as_str(), f.config.pop.map(|p| p.port)), ("mailhost.example", Some(995)));
        // A record naming something that isn't a host name is ignored.
        let mut fetch = |_: &str| None;
        let mut srv = |name: &str| name.starts_with("_autodiscover").then(|| ("evil/../x".to_string(), 443));
        assert!(discover_with("zoe@studio.example", &mut Lookups { fetch: &mut fetch, mx: &mut no_mx, srv: &mut srv }).is_none());
    }

    /// A domain whose mail Microsoft 365 hosts is proposed as Exchange Online through Graph: by
    /// its mail exchanger, or by Outlook's Autodiscover record alone; IMAP settings found as
    /// well stay for whoever picks IMAP. Any other exchanger (Outlook.com's own among them)
    /// proposes nothing of the kind.
    #[test]
    fn microsoft_365_domains_are_proposed_through_graph() {
        let mut none = |_: &str| None;
        let mut microsoft_mx = |_: &str| vec!["contoso-example.mail.protection.outlook.com.".to_string()];
        let mut no_srv = |_: &str| None;
        let f = discover_with("zoe@contoso.example", &mut Lookups { fetch: &mut none, mx: &mut microsoft_mx, srv: &mut no_srv }).unwrap();
        assert!(f.source.contains("Microsoft 365"), "{}", f.source);
        assert_eq!((f.config.graph.as_deref(), f.config.oauth.as_deref()), (Some(quatrelle_model::GRAPH_URL), Some("microsoft")));
        assert_eq!(f.config.email, "zoe@contoso.example");
        // By the Autodiscover record alone.
        let mut no_mx = |_: &str| Vec::new();
        let mut srv = |name: &str| (name == "_autodiscover._tcp.contoso.example").then(|| ("autodiscover.outlook.com".to_string(), 443));
        let f = discover_with("zoe@contoso.example", &mut Lookups { fetch: &mut none, mx: &mut no_mx, srv: &mut srv }).unwrap();
        assert!(f.config.graph.is_some());
        // The domain's own IMAP settings stay in the answer, for whoever picks IMAP.
        let mut own = |url: &str| url.starts_with("https://autoconfig.contoso.example/").then(|| XML.replace("example.org", "contoso.example"));
        let f = discover_with("zoe@contoso.example", &mut Lookups { fetch: &mut own, mx: &mut microsoft_mx, srv: &mut no_srv }).unwrap();
        assert!(f.config.graph.is_some());
        assert_eq!(f.config.imap.host, "imap.contoso.example");
        // Any other exchanger: as before.
        let mut google_mx = |_: &str| vec!["aspmx.l.google.com".to_string()];
        assert!(discover_with("zoe@contoso.example", &mut Lookups { fetch: &mut none, mx: &mut google_mx, srv: &mut no_srv }).is_none());
        assert!(!at_microsoft_365(&["outlook-com.olc.protection.outlook.com".into()], None), "Outlook.com's own, not Microsoft 365's");
        assert!(!at_microsoft_365(&["mx.contoso.example".into()], Some("autoconfig.contoso.example")));
        assert!(at_microsoft_365(&[], Some("OUTLOOK.office365.com")));
    }

    /// The DNS answers that lead off the address's domain come from anyone on the network: a
    /// mail exchanger or an SRV record naming a host of theirs, which serves a configuration
    /// under a certificate of its own. What is found that way is marked, for the user to see
    /// and confirm; what the domain itself says over HTTPS, or the ISP database about the
    /// domain, is not.
    #[test]
    fn servers_named_through_dns_alone_are_to_be_confirmed() {
        let theirs = XML.replace("example.org", "attacker.example");
        let mut no_mx = |_: &str| Vec::new();
        let mut no_srv = |_: &str| None;
        // A mail exchanger on another site, then that site's own configuration.
        let mut fetch = |url: &str| url.starts_with("https://autoconfig.attacker.example/").then(|| theirs.clone());
        let mut mx = |_: &str| vec!["mx.attacker.example".to_string()];
        let f = discover_with("zoe@studio.example", &mut Lookups { fetch: &mut fetch, mx: &mut mx, srv: &mut no_srv }).unwrap();
        assert_eq!(f.config.imap.host, "imap.attacker.example");
        assert!(f.unverified, "through the mail exchanger");
        // The ISP database asked about the exchanger's site is no better: the exchanger was DNS's.
        let mut fetch = |url: &str| (url == "https://autoconfig.thunderbird.net/v1.1/attacker.example").then(|| theirs.clone());
        let f = discover_with("zoe@studio.example", &mut Lookups { fetch: &mut fetch, mx: &mut mx, srv: &mut no_srv }).unwrap();
        assert!(f.unverified, "through the mail exchanger and the ISP database");
        // An Autodiscover record naming their host.
        let mut fetch = |url: &str| url.starts_with("https://cfg.attacker.example/").then(|| theirs.clone());
        let mut srv = |name: &str| name.starts_with("_autodiscover.").then(|| ("cfg.attacker.example".to_string(), 443));
        let f = discover_with("zoe@studio.example", &mut Lookups { fetch: &mut fetch, mx: &mut no_mx, srv: &mut srv }).unwrap();
        assert!(f.unverified, "through the Autodiscover record");
        // SRV records naming their servers.
        let mut none = |_: &str| None;
        let mut srv = |name: &str| match name {
            "_imaps._tcp.studio.example" => Some(("imap.attacker.example".to_string(), 993)),
            "_submissions._tcp.studio.example" => Some(("smtp.attacker.example".to_string(), 465)),
            _ => None,
        };
        let f = discover_with("zoe@studio.example", &mut Lookups { fetch: &mut none, mx: &mut no_mx, srv: &mut srv }).unwrap();
        assert!(f.unverified, "through SRV records");
        // One of the two elsewhere is enough.
        let mut srv = |name: &str| match name {
            "_imaps._tcp.studio.example" => Some(("imap.studio.example".to_string(), 993)),
            "_submissions._tcp.studio.example" => Some(("smtp.attacker.example".to_string(), 465)),
            _ => None,
        };
        assert!(discover_with("zoe@studio.example", &mut Lookups { fetch: &mut none, mx: &mut no_mx, srv: &mut srv }).unwrap().unverified);

        // SRV records naming the domain's own servers: their certificate will be the domain's.
        let mut srv = |name: &str| match name {
            "_imaps._tcp.studio.example" => Some(("imap.studio.example".to_string(), 993)),
            "_submission._tcp.studio.example" => Some(("smtp.studio.example".to_string(), 587)),
            _ => None,
        };
        assert!(!discover_with("zoe@studio.example", &mut Lookups { fetch: &mut none, mx: &mut no_mx, srv: &mut srv }).unwrap().unverified);
        // The domain's own configuration, over HTTPS in its name, wherever it sends the mail.
        let mut fetch = |url: &str| url.starts_with("https://autoconfig.studio.example/").then(|| theirs.clone());
        let f = discover_with("zoe@studio.example", &mut Lookups { fetch: &mut fetch, mx: &mut mx, srv: &mut no_srv }).unwrap();
        assert!(!f.unverified && f.config.imap.host == "imap.attacker.example");
        // The ISP database about the domain itself.
        let mut fetch = |url: &str| (url == "https://autoconfig.thunderbird.net/v1.1/studio.example").then(|| theirs.clone());
        assert!(!discover_with("zoe@studio.example", &mut Lookups { fetch: &mut fetch, mx: &mut mx, srv: &mut no_srv }).unwrap().unverified);
        // Microsoft 365 by its exchanger: the IMAP settings kept beside it keep their mark.
        let mut fetch = |url: &str| url.starts_with("https://autoconfig.outlook.com/").then(|| theirs.clone());
        let mut microsoft = |_: &str| vec!["studio-example.mail.protection.outlook.com".to_string()];
        let f = discover_with("zoe@studio.example", &mut Lookups { fetch: &mut fetch, mx: &mut microsoft, srv: &mut no_srv }).unwrap();
        assert!(f.config.graph.is_some() && f.config.imap.host == "imap.attacker.example" && f.unverified);
        // An SRV target that is no host name is no server.
        let mut srv = |name: &str| match name {
            "_imaps._tcp.studio.example" => Some(("imap.studio.example/x".to_string(), 993)),
            "_submissions._tcp.studio.example" => Some(("smtp.studio.example".to_string(), 465)),
            _ => None,
        };
        assert!(discover_with("zoe@studio.example", &mut Lookups { fetch: &mut none, mx: &mut no_mx, srv: &mut srv }).is_none());
    }

    /// A configuration that offers a server without encryption only is not taken: found
    /// settings never send a password in the clear. (Such a server can still be typed in.)
    #[test]
    fn a_server_without_encryption_is_never_found() {
        let plain = XML.replace("<socketType>SSL</socketType>", "<socketType>plain</socketType>");
        assert!(parse_config(&plain, "zoe@example.org").is_none(), "IMAP in the clear only");
        let smtp = XML.replace("<socketType>STARTTLS</socketType>", "<socketType>plain</socketType>");
        assert!(parse_config(&smtp, "zoe@example.org").is_none(), "SMTP in the clear only");
        let unknown = XML.replace("<socketType>STARTTLS</socketType>", "<socketType>whatever</socketType>");
        assert!(parse_config(&unknown, "zoe@example.org").is_none(), "a kind of connection nobody knows");
        let c = parse_config(XML, "zoe@example.org").unwrap();
        assert!(c.imap.security == Security::Tls && c.pop.is_some_and(|p| p.security == Security::Tls), "the encrypted ones are");
    }

    #[test]
    fn lookups_in_order() {
        let mut asked: Vec<String> = Vec::new();
        let mut fetch = |url: &str| {
            asked.push(url.to_string());
            (url == "https://autoconfig.thunderbird.net/v1.1/google.com").then(|| XML.to_string())
        };
        let mut mx = |d: &str| if d == "studio.example" { vec!["aspmx.l.google.com".to_string()] } else { Vec::new() };
        let mut srv = |_: &str| None;
        let f = discover_with("zoe@studio.example", &mut Lookups { fetch: &mut fetch, mx: &mut mx, srv: &mut srv }).unwrap();
        assert!(f.source.contains("google.com"), "{}", f.source);
        assert_eq!(
            asked,
            vec![
                "https://autoconfig.studio.example/mail/config-v1.1.xml?emailaddress=zoe%40studio.example".to_string(),
                "https://studio.example/.well-known/autoconfig/mail/config-v1.1.xml".into(),
                "https://autoconfig.thunderbird.net/v1.1/studio.example".into(),
                "https://autoconfig.thunderbird.net/v1.1/google.com".into(),
            ]
        );

        // SRV when nothing else answers.
        let mut none = |_: &str| None;
        let mut no_mx = |_: &str| Vec::new();
        let mut srv = |name: &str| match name {
            "_imaps._tcp.x.example" => Some(("mail.x.example".to_string(), 993)),
            "_submission._tcp.x.example" => Some(("mail.x.example".to_string(), 587)),
            _ => None,
        };
        let f = discover_with("a@x.example", &mut Lookups { fetch: &mut none, mx: &mut no_mx, srv: &mut srv }).unwrap();
        assert_eq!((f.config.imap.port, f.config.imap.security, f.config.smtp.security), (993, Security::Tls, Security::StartTls));
        let mut srv_none = |_: &str| None;
        assert!(discover_with("nobody", &mut Lookups { fetch: &mut none, mx: &mut no_mx, srv: &mut srv_none }).is_none());
        assert_eq!(base_domain("aspmx.l.google.com"), "google.com");
        assert_eq!(base_domain("mx1.mail.example.co.uk."), "example.co.uk");
    }
}
