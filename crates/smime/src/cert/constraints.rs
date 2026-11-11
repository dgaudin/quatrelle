// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What an authority's own certificate lets it sign (RFC 5280 §4.2.1 and §6.1): certificates at
//! all, how many authorities under it, for which addresses and names, for email; and what a
//! certificate says critically that isn't read here. An authority kept to its own domain, or to
//! web servers, can't make a certificate for anyone's address.

use x509_cert::ext::pkix::NameConstraints;
use x509_cert::ext::pkix::constraints::name::GeneralSubtree;

use super::*;

/// The extensions read here, or that say nothing a mail signature depends on (key identifiers,
/// where to ask about withdrawal, policies). Any other one marked critical is a limit that
/// can't be honoured: its certificate isn't used.
const READ: &[&str] = &[
    "2.5.29.14",          // subject key identifier
    "2.5.29.15",          // key usage
    "2.5.29.17",          // subject alternative name
    "2.5.29.18",          // issuer alternative name
    "2.5.29.19",          // basic constraints
    "2.5.29.30",          // name constraints
    "2.5.29.31",          // where its list of withdrawn certificates is
    "2.5.29.32",          // certificate policies
    "2.5.29.33",          // policy mappings
    "2.5.29.35",          // authority key identifier
    "2.5.29.36",          // policy constraints
    "2.5.29.37",          // extended key usage
    "2.5.29.46",          // freshest list of withdrawn certificates
    "2.5.29.54",          // inhibit anyPolicy
    "1.3.6.1.5.5.7.1.1",  // authority information access
    "1.3.6.1.5.5.7.1.3",  // qualified certificate statements
    "1.3.6.1.5.5.7.1.11", // subject information access
];

/// Whether `c` says something critical that isn't read here.
pub(super) fn unread_critical(c: &Certificate) -> bool {
    c.tbs_certificate.extensions.iter().flatten().any(|e| e.critical && !READ.contains(&e.extn_id.to_string().as_str()))
}

/// Whether `c`, an authority's certificate on a chain, may sign the certificate under it:
/// an authority, its key for signing certificates when it says what its key is for, for email
/// when it says what its certificates are for, with nothing critical unread.
pub(super) fn may_issue(c: &Certificate) -> bool {
    let signs = extension::<KeyUsage>(c, oids::KEY_USAGE).is_none_or(|KeyUsage(u)| u.contains(KeyUsages::KeyCertSign));
    let for_mail = extension::<ExtendedKeyUsage>(c, oids::EXT_KEY_USAGE)
        .is_none_or(|ExtendedKeyUsage(u)| u.iter().any(|o| *o == oids::EMAIL_PROTECTION || *o == oids::ANY_EXTENDED_KEY_USAGE));
    is_ca(c) && signs && for_mail && !unread_critical(c)
}

/// Whether `c` allows `below` authorities between it and the certificate that signed
/// (`pathLenConstraint`).
pub(super) fn deep_enough(c: &Certificate, below: usize) -> bool {
    extension::<BasicConstraints>(c, oids::BASIC_CONSTRAINTS).and_then(|b| b.path_len_constraint).is_none_or(|most| below <= usize::from(most))
}

/// Whether `address` is within the rfc822Name constraint `base`: a mailbox (`ana@studio.example`),
/// every mailbox at a host (`studio.example`), or at any host under a domain (`.studio.example`).
fn address_within(address: &str, base: &str) -> bool {
    let (address, base) = (address.trim().to_ascii_lowercase(), base.trim().to_ascii_lowercase());
    let Some((_, host)) = address.rsplit_once('@') else { return false };
    if base.contains('@') {
        return address == base;
    }
    match base.strip_prefix('.') {
        Some(_) => host.len() > base.len() && host.ends_with(&base),
        None => !base.is_empty() && host == base,
    }
}

/// Whether the directory name `name` is within `base`: `base`'s parts, in order, start it.
fn name_within(name: &x509_cert::name::Name, base: &x509_cert::name::Name) -> bool {
    base.0.len() <= name.0.len() && base.0.iter().zip(name.0.iter()).all(|(b, n)| matches!((b.to_der(), n.to_der()), (Ok(x), Ok(y)) if x == y))
}

/// Whether `tree` holds `leaf`: one of its addresses, or its name.
fn holds(tree: &GeneralSubtree, addresses: &[String], leaf: &Certificate) -> Option<bool> {
    match &tree.base {
        GeneralName::Rfc822Name(base) => Some(addresses.iter().any(|a| address_within(a, base.as_str()))),
        GeneralName::DirectoryName(base) => Some(name_within(&leaf.tbs_certificate.subject, base)),
        _ => None,
    }
}

/// Whether the authority `c` may stand above `leaf` by the names it's kept to (name constraints):
/// every address of `leaf` within what's permitted, when addresses are; its name too, when names
/// are; and neither within what's left out. Constraints that can't be read allow nothing.
pub(super) fn allows(c: &Certificate, leaf: &Certificate) -> bool {
    let Some(e) = c.tbs_certificate.extensions.iter().flatten().find(|e| e.extn_id == oids::NAME_CONSTRAINTS) else { return true };
    let Ok(limits) = NameConstraints::from_der(e.extn_value.as_bytes()) else { return false };
    let addresses = emails(leaf);
    let permitted = limits.permitted_subtrees.unwrap_or_default();
    let about_addresses: Vec<&GeneralSubtree> = permitted.iter().filter(|t| matches!(t.base, GeneralName::Rfc822Name(_))).collect();
    if !about_addresses.is_empty() && !addresses.iter().all(|a| about_addresses.iter().any(|t| holds(t, std::slice::from_ref(a), leaf) == Some(true)))
    {
        return false;
    }
    let about_names: Vec<&GeneralSubtree> = permitted.iter().filter(|t| matches!(t.base, GeneralName::DirectoryName(_))).collect();
    if !about_names.is_empty() && !about_names.iter().any(|t| holds(t, &addresses, leaf) == Some(true)) {
        return false;
    }
    !limits.excluded_subtrees.unwrap_or_default().iter().any(|t| holds(t, &addresses, leaf) == Some(true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_within_a_mailbox_a_host_or_a_domain() {
        for (address, base, within) in [
            ("ana@studio.example", "ana@studio.example", true),
            ("Ana@Studio.Example", "ana@studio.example", true),
            ("eve@studio.example", "ana@studio.example", false),
            ("ana@studio.example", "studio.example", true),
            ("ana@mail.studio.example", "studio.example", false),
            ("ana@notstudio.example", "studio.example", false),
            ("ana@mail.studio.example", ".studio.example", true),
            ("ana@studio.example", ".studio.example", false),
            ("ana@evilstudio.example", ".studio.example", false),
            ("not an address", "studio.example", false),
            ("ana@studio.example", "", false),
        ] {
            assert_eq!(address_within(address, base), within, "{address} within {base}");
        }
    }
}
