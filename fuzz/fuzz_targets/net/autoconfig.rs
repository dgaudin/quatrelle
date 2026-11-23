// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Autoconfig: a configuration file found on the network (Mozilla's format and the ISP
//! database) read for an address, and what `discover_with` makes of hostile lookups' answers.
//! Settings found are never plain-text, a host is only a host name, and a server found through a
//! DNS answer nothing vouches for is marked; a file that offers no encrypted server is no answer.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_model::Security;
use quatrelle_model::addr::trust::site;
use quatrelle_net::autoconfig::{at_microsoft_365, base_domain, discover_with, parse_config, Lookups};

/// A host name, as the account settings may carry one.
fn host_ok(host: &str) -> bool {
    !host.is_empty() && host.len() <= 253 && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
}

/// What a found configuration promises: servers encrypted, on host names, with ports.
fn check_found(found: &quatrelle_net::autoconfig::Found) {
    let c = &found.config;
    assert_ne!(c.imap.security, Security::None, "{:?}", c.imap);
    assert_ne!(c.smtp.security, Security::None, "{:?}", c.smtp);
    for s in [&c.imap, &c.smtp].into_iter().chain(c.pop.as_ref()) {
        assert!(host_ok(&s.host), "{:?}", s.host);
        assert_ne!(s.port, 0, "{s:?}");
        assert_ne!(s.security, Security::None, "{s:?}");
    }
    assert!(!found.source.is_empty() && found.source.len() <= 512);
}

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    for email in ["zoe@studio.example", "ana@contoso.example", "a@b.c", "zoe@studio.example.<>"] {
        if let Some(c) = parse_config(&text, email) {
            assert_ne!(c.imap.security, Security::None);
            assert_ne!(c.smtp.security, Security::None);
            assert!(host_ok(&c.imap.host) && host_ok(&c.smtp.host));
            assert!(c.pop.as_ref().is_none_or(|p| p.security != Security::None && host_ok(&p.host)));
            assert!(
                !c.username.contains("%EMAILADDRESS%") && !c.username.contains("%EMAILLOCALPART%") && !c.username.contains("%EMAILDOMAIN%"),
                "{:?} for {email}",
                c.username
            );
        }
    }
    // The lookups a network hands back: mail exchangers and services named by the input, and
    // the file answered only for the families the first byte selects (1: the domain's own
    // HTTPS, 2: the ISP database about the domain, 4: the provider's own configuration,
    // 8: the Autodiscover host). The address is made up; nothing leaves this process.
    let selector = data.first().copied().unwrap_or(0);
    let hosts: Vec<String> = text.lines().take(64).map(|l| l.trim().chars().take(253).collect::<String>()).filter(|l| !l.is_empty()).collect();
    let family = |url: &str| -> u8 {
        if url.starts_with("https://autoconfig.studio.example/") || url.starts_with("https://studio.example/.well-known/") {
            1
        } else if url.starts_with("https://autoconfig.thunderbird.net/") {
            2
        } else if url.starts_with("https://autoconfig.") {
            4
        } else {
            8
        }
    };
    let mut fetch = |url: &str| (selector & family(url) != 0).then(|| text.to_string());
    let mut mx = |_: &str| hosts.clone();
    let mut srv = |_: &str| hosts.first().map(|h| (h.clone(), 993));
    let found = discover_with("zoe@studio.example", &mut Lookups { fetch: &mut fetch, mx: &mut mx, srv: &mut srv });
    if let Some(found) = &found {
        check_found(found);
        // A server reached through a DNS answer (the mail exchanger, a service, Autodiscover)
        // is marked unless it is on the address's own site; what the domain itself serves over
        // HTTPS, or the ISP database about the domain, is not. Microsoft 365 keeps the mark of
        // the IMAP settings it keeps beside it.
        let own_site = site("studio.example");
        let off_site = [Some(&found.config.imap), Some(&found.config.smtp), found.config.pop.as_ref()]
            .into_iter()
            .flatten()
            .any(|s| site(&s.host) != own_site);
        let direct = (selector & 1 != 0 && found.source == "studio.example") || (selector & 2 != 0 && found.source == "the ISP database");
        if found.source.contains("Microsoft 365") {
            if selector & 1 == 0 && site(&found.config.imap.host) != own_site {
                assert!(found.unverified, "{found:?}");
            }
        } else if !direct && off_site {
            assert!(found.unverified, "{found:?}");
        }
        // What a provider's own answer says of Microsoft 365 is Microsoft's alone.
        let _ = at_microsoft_365(&hosts, Some(found.config.imap.host.as_str()));
    }
    // The registrable part of each host, and of the input's own labels.
    for h in hosts.iter().take(16) {
        let b = base_domain(h);
        assert!(b.is_empty() || (!b.starts_with('.') && !b.ends_with('.') && b.len() <= h.len()), "{h:?} -> {b:?}");
    }
});
