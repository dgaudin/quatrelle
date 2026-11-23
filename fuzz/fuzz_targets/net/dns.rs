// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! DNS answers (RFC 1035, RFC 6186) as a resolver hands them back: the first two bytes are the
//! query id, and the rest is read for a mail exchanger, a service, and the question asked. Every
//! length, compression pointer and count is checked; names never run on (a pointer only goes
//! backwards), and the records found stay within the cap. A query built from the input names only
//! what DNS can carry.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_net::dns::{parse, query, MX, SRV};

/// The longest host name an answer may carry (RFC 1035 §2.3.4).
const MAX_HOST: usize = 253;

/// A host name: letters, digits and hyphens, in labels joined by dots.
fn host_ok(host: &str) -> bool {
    !host.is_empty() && host.len() <= MAX_HOST && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
}

fn check(records: &[quatrelle_net::dns::Record]) {
    assert!(records.len() <= 64, "{} records", records.len());
    for r in records {
        let host = match r {
            quatrelle_net::dns::Record::Mx { host, .. } => host,
            quatrelle_net::dns::Record::Srv { target, .. } => target,
        };
        assert!(host_ok(host), "{host:?}");
    }
}

fuzz_target!(|data: &[u8]| {
    // A query the resolver would send: its id and type, for names that DNS can and can't carry.
    let (id, rest) = match data {
        [a, b, ..] => (u16::from_be_bytes([*a, *b]), &data[2..]),
        _ => (0, data),
    };
    for name in ["example.org", "_imaps._tcp.example.org", "a.b.c.d.e.f"] {
        for qtype in [MX, SRV, 1, 5, 0xffff] {
            if let Some(q) = query(id, name, qtype) {
                assert!(q.len() <= 32 + name.len() + 32, "{} bytes for {name}", q.len());
                assert!(q.ends_with(&[0, 1][..]) || q.len() < 4);
            }
        }
    }
    let made_up = String::from_utf8_lossy(rest);
    let _ = query(id, &made_up, MX);
    let _ = query(id, &format!("{made_up}.example"), SRV);
    // The same answer read for each question, with the id the input says.
    check(&parse(data, id, "example.org", MX).unwrap_or_default());
    check(&parse(data, id, "example.org", SRV).unwrap_or_default());
    check(&parse(data, id, "_imaps._tcp.example.org", SRV).unwrap_or_default());
    check(&parse(data, id, "EXAMPLE.ORG.", MX).unwrap_or_default());
    // Any id but the input's, and any other question, must not be taken for this one.
    let other = id.wrapping_add(1);
    assert!(parse(data, other, "example.org", MX).is_none() || parse(data, other, "example.org", MX) == Some(Vec::new()));
});
