// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What a real Exchange refuses on a navigation shortcut's properties: a property written with
//! a type other than the one [MS-OXPROPS] gives it, or with a value that type can't hold.

use super::*;

/// The type [MS-OXPROPS] gives each shortcut property ([MS-OXOCFG] 2.2.9).
const KINDS: [(&str, &str); 13] = [
    ("0x001A", "String"),
    ("0x0E1D", "String"),
    ("0x6842", "Binary"),
    ("0x6847", "Integer"),
    ("0x6849", "Integer"),
    ("0x684A", "Integer"),
    ("0x684B", "Binary"),
    ("0x684C", "Binary"),
    ("0x684E", "Binary"),
    ("0x684F", "Binary"),
    ("0x6850", "Binary"),
    ("0x6851", "String"),
    ("0x6852", "Integer"),
];

/// Whether a shortcut property written as `kind` with `value` is one Exchange refuses: the wrong
/// type for the tag, an integer outside a signed 32-bit one, or a binary that isn't base64.
pub(in crate::fakes::fake_ews) fn refuses_wlink_property(tag: &str, kind: &str, value: &str) -> bool {
    let Some((_, wanted)) = KINDS.iter().find(|(t, _)| crate::ews::is_tag(tag, t)) else {
        return false;
    };
    if !kind.eq_ignore_ascii_case(wanted) {
        return true;
    }
    match *wanted {
        "Integer" => value.trim().parse::<i32>().is_err(),
        "Binary" => crate::ews::unb64(value.trim()).is_none(),
        _ => false,
    }
}

/// Whether any of an item's `ExtendedProperty` children is refused.
pub(in crate::fakes::fake_ews) fn refuses_wlink_item(item: &El) -> bool {
    item.children("ExtendedProperty").any(|p| {
        let uri = p.child("ExtendedFieldURI");
        match (uri.and_then(|u| u.attr("PropertyTag")), uri.and_then(|u| u.attr("PropertyType"))) {
            (Some(tag), Some(kind)) => refuses_wlink_property(tag, kind, p.text_of("Value").unwrap_or_default()),
            _ => false,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fake refuses what Exchange would: a GUID text under a CLSID type for the folder type, an
    /// integer past 32 bits, a binary that isn't base64; and takes the right ones.
    #[test]
    fn the_fake_refuses_what_exchange_refuses() {
        assert!(refuses_wlink_property("0x684F", "CLSID", "00067802-0000-0000-c000-000000000046"));
        assert!(refuses_wlink_property("0x684F", "CLSID", "AngGAAAAAADAAAAAAAAARg=="));
        assert!(!refuses_wlink_property("0x684F", "Binary", "AngGAAAAAADAAAAAAAAARg=="));
        assert!(refuses_wlink_property("0x6847", "Integer", "3000000000"));
        assert!(!refuses_wlink_property("0x6847", "Integer", "2000000000"));
        assert!(refuses_wlink_property("0x684B", "Binary", "not base64!"));
        assert!(!refuses_wlink_property("0x6851", "String", "Rooms"));
    }
}
