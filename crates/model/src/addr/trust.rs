//! Which servers an account's credentials may reach.
//!
//! A server belongs to a **site**: its registrable domain (`api.fastmail.com` → `fastmail.com`,
//! `mail.example.co.uk` → `example.co.uk`), taken from the Public Suffix List. An account trusts
//! the sites of its own servers and of the domains the user allowed; anything else is asked first.

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, Ipv6Addr};

use crate::AccountConfig;

/// Longest host name (RFC 1035 §2.3.4).
const MAX_HOST: usize = 253;

/// The site a host belongs to: its registrable domain, in lower case, without a trailing dot, in
/// the ASCII form given (no U-label/A-label conversion: two spellings are two sites, which errs
/// on the safe side). When the list gives none (a single label like `exchange01` or `localhost`,
/// an IP address, a bare suffix) the site is the normalised host itself. An unknown suffix
/// (`.local`) follows the list's `*` rule: `exch01.corp.local` → `corp.local`.
pub fn site(host: &str) -> String {
    let h = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if h.is_empty() || h.starts_with('[') || h.parse::<Ipv4Addr>().is_ok() || h.parse::<Ipv6Addr>().is_ok() {
        return h;
    }
    psl::domain_str(&h).map(str::to_string).unwrap_or(h)
}

/// The host of a URL (`https://user@Mail.Example.com:8443/path?q` → `Mail.Example.com`; an IPv6
/// address keeps its brackets). The scheme is optional, so a bare host or `host/path` reads too.
/// `None` for an empty, over-long or unreadable one (spaces, control characters).
pub fn url_host(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = match url.split_once("://") {
        Some((scheme, rest)) if !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) => rest,
        Some(_) => return None,
        None => url,
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let hostport = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(inner) = hostport.strip_prefix('[') {
        let (addr, after) = inner.split_once(']')?;
        if !(after.is_empty() || after.starts_with(':')) || addr.parse::<Ipv6Addr>().is_err() {
            return None;
        }
        format!("[{addr}]")
    } else {
        hostport.split(':').next().unwrap_or_default().to_string()
    };
    let ok = !host.is_empty() && host.len() <= MAX_HOST && !host.chars().any(|c| c.is_whitespace() || c.is_control());
    ok.then_some(host)
}

/// The sites an account sends its credentials to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Trust {
    sites: BTreeSet<String>,
}

impl Trust {
    /// The sites of the address's domain, every configured server (IMAP, SMTP, POP, and the
    /// JMAP, Graph and EWS URLs), each domain the user allowed (`trusted_domains`), and each of
    /// `extra` (URLs, or hosts, known only to the worker and the engine: the Exchange address
    /// Autodiscover found, the saved CalDAV and CardDAV homes). An empty or unreadable one is
    /// ignored.
    pub fn of(acc: &AccountConfig, extra: &[&str]) -> Trust {
        let mut t = Trust::default();
        t.add(acc.email.rsplit_once('@').map(|(_, d)| d));
        t.add(Some(&acc.imap.host));
        t.add(Some(&acc.smtp.host));
        t.add(acc.pop.as_ref().map(|p| p.host.as_str()));
        for url in [&acc.jmap, &acc.graph, &acc.ews].into_iter().flatten() {
            t.add(Some(url));
        }
        for d in &acc.trusted_domains {
            t.add(Some(d));
        }
        for e in extra {
            t.add(Some(e));
        }
        t
    }

    fn add(&mut self, host_or_url: Option<&str>) {
        if let Some(host) = host_or_url.and_then(url_host) {
            let s = site(&host);
            if !s.is_empty() {
                self.sites.insert(s);
            }
        }
    }

    /// Whether the account's credentials may go to this host.
    pub fn allows(&self, host: &str) -> bool {
        let s = site(host);
        !s.is_empty() && self.sites.contains(&s)
    }

    /// The trusted sites, in order.
    pub fn sites(&self) -> Vec<String> {
        self.sites.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Security, ServerConfig};

    #[test]
    fn sites_are_registrable_domains() {
        for (host, want) in [
            ("api.fastmail.com", "fastmail.com"),
            ("mail.example.co.uk", "example.co.uk"),
            ("MAIL.Example.COM.", "example.com"),
            ("exch01.corp.local", "corp.local"),
            ("exchange01", "exchange01"),
            ("localhost", "localhost"),
            ("127.0.0.1", "127.0.0.1"),
            ("[::1]", "[::1]"),
            ("xn--bcher-kva.example", "xn--bcher-kva.example"),
            ("XN--Bcher-KVA.Example", "xn--bcher-kva.example"),
            // Shared hosting: each customer is a site of its own.
            ("alice.github.io", "alice.github.io"),
            ("github.io", "github.io"),
            ("co.uk", "co.uk"),
            ("", ""),
        ] {
            assert_eq!(site(host), want, "{host}");
        }
    }

    #[test]
    fn a_url_gives_its_host() {
        for (url, want) in [
            ("https://caldav.icloud.com/123/", Some("caldav.icloud.com")),
            ("https://user:pw@Mail.Example.com:8443/EWS/Exchange.asmx?x=1#y", Some("Mail.Example.com")),
            ("http://[::1]:8080/a", Some("[::1]")),
            ("example.org", Some("example.org")),
            ("example.org/path", Some("example.org")),
            ("https://", None),
            ("", None),
            ("   ", None),
            ("https://bad host/", None),
            ("http://[::1/", None),
            ("http://[nope]/", None),
            ("://x.example/", None),
        ] {
            assert_eq!(url_host(url).as_deref(), want, "{url}");
        }
        assert!(url_host(&format!("https://{}.example/", "a".repeat(300))).is_none(), "capped");
    }

    #[test]
    fn an_account_trusts_its_servers_and_what_it_was_given() {
        let server = |host: &str| ServerConfig { host: host.into(), port: 993, security: Security::Tls };
        let acc = AccountConfig {
            email: "zoe@example.org".into(),
            imap: server("imap.mail.me.com"),
            smtp: server("smtp.mail.me.com"),
            trusted_domains: vec!["icloud.com".into()],
            ..Default::default()
        };
        let t = Trust::of(&acc, &["https://caldav.icloud.com/123/", "", "https://", "  "]);
        assert!(t.allows("p01-caldav.icloud.com") && t.allows("smtp.mail.me.com") && t.allows("MAIL.example.org."));
        assert!(!t.allows("evil.example") && !t.allows("") && !t.allows("example.org.evil.example"));
        assert_eq!(t.sites(), ["example.org", "icloud.com", "me.com"]);
    }

    #[test]
    fn every_kind_of_server_counts() {
        let acc = AccountConfig {
            email: "zoe@corp.example".into(),
            pop: Some(ServerConfig { host: "pop.pop-host.example".into(), port: 995, security: Security::Tls }),
            jmap: Some("https://api.jmap-host.example/session".into()),
            graph: Some("https://graph.microsoft.com/v1.0".into()),
            ews: Some("https://exch01.corp.local/EWS/Exchange.asmx".into()),
            ..Default::default()
        };
        let t = Trust::of(&acc, &[]);
        assert_eq!(t.sites(), ["corp.example", "corp.local", "jmap-host.example", "microsoft.com", "pop-host.example"]);
        // Nothing configured: nothing trusted, not even the empty host.
        let none = Trust::of(&AccountConfig::default(), &[]);
        assert!(none.sites().is_empty() && !none.allows("") && !none.allows("example.org"));
    }

    #[test]
    fn trusted_domains_are_saved_only_when_there_are_some() {
        let mut acc = AccountConfig::guess("zoe@example.org");
        assert!(!serde_json::to_string(&acc).unwrap().contains("trusted_domains"));
        acc.trusted_domains.push("icloud.com".into());
        let json = serde_json::to_string(&acc).unwrap();
        assert!(json.contains(r#""trusted_domains":["icloud.com"]"#));
        assert_eq!(serde_json::from_str::<AccountConfig>(&json).unwrap().trusted_domains, ["icloud.com"]);
        // A file written before the field existed still loads.
        let old = json.replace(r#","trusted_domains":["icloud.com"]"#, "");
        assert!(serde_json::from_str::<AccountConfig>(&old).unwrap().trusted_domains.is_empty());
    }
}
