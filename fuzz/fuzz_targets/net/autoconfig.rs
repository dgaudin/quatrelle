// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Autoconfig: a configuration file found on the network (Mozilla's format and the ISP
//! database) read for an address, and what `discover_with` makes of hostile lookups' answers.
//! Settings found are never plain-text, a host is only a host name, and a server found through a
//! DNS answer nothing vouches for is marked; a file that offers no encrypted server is no answer.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_model::Security;
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
    // The lookups a network hands back: the same file for every address asked, mail exchangers
    // and services named by the input. The address is made up; nothing leaves this process.
    let hosts: Vec<String> = text.lines().take(64).map(|l| l.trim().chars().take(253).collect::<String>()).filter(|l| !l.is_empty()).collect();
    let mut fetch = |_: &str| Some(text.to_string());
    let mut mx = |_: &str| hosts.clone();
    let mut srv = |_: &str| hosts.first().map(|h| (h.clone(), 993));
    let found = discover_with("zoe@studio.example", &mut Lookups { fetch: &mut fetch, mx: &mut mx, srv: &mut srv });
    if let Some(found) = &found {
        check_found(found);
        // What a provider's own answer says of Microsoft 365 is Microsoft's alone.
        let _ = at_microsoft_365(&hosts, Some(found.config.imap.host.as_str()));
    }
    // The registrable part of each host, and of the input's own labels.
    for h in hosts.iter().take(16) {
        let b = base_domain(h);
        assert!(b.is_empty() || (!b.starts_with('.') && !b.ends_with('.') && b.len() <= h.len()), "{h:?} -> {b:?}");
    }
});
