// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Exchange's NTLMv2 sign-in (MS-NLMP): a server's challenge message read from the bytes it
//! sends (its target information, lengths and offsets), then the authenticate message answering
//! it, built from what was read. Never a panic, no field read outside the message, the target
//! information within its cap; every field of the answer made within the message it says it is.
#![no_main]

use libfuzzer_sys::fuzz_target;

/// The signature every NTLM message starts with.
const SIGNATURE: &[u8; 8] = b"NTLMSSP\0";

/// The u16 at `at`, little-endian.
fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

/// The u32 at `at`, little-endian.
fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?]))
}

/// What an authenticate message (type 3) says: its five fields as (length, offset), and whether
/// each lies inside it.
fn check_authenticate(m: &[u8]) {
    assert_eq!(m.get(..8), Some(SIGNATURE.as_slice()), "the signature");
    assert_eq!(u32_at(m, 8), Some(3), "the message type");
    // LM, NT, domain, user, workstation, then the session key (six fields from offset 12).
    for i in 0..6 {
        let at = 12 + i * 8;
        let len = usize::from(u16_at(m, at).expect("a field length"));
        let max = usize::from(u16_at(m, at + 2).expect("a field's longest"));
        let off = usize::try_from(u32_at(m, at + 4).expect("a field offset")).expect("an offset");
        assert!(len <= max, "field {i}: length {len} over its longest {max}");
        let end = off.checked_add(len).expect("a field inside the message");
        assert!(end <= m.len(), "field {i}: {off}..{end} past the {} bytes", m.len());
    }
}

fuzz_target!(|data: &[u8]| {
    let _ = quatrelle_net::ntlm::negotiate();
    let Ok(challenge) = quatrelle_net::ntlm::challenge(data) else { return };
    // What a server may put in a challenge, within the cap the reader promises.
    assert!(challenge.target_info.len() <= 4096, "{} bytes of target information", challenge.target_info.len());
    // The answer to it, for an account of both shapes: a domain and a bare address.
    for (user, password, now) in
        [("CORP\\riley", "pw", 0u64), ("zoe@contoso.example", "", u64::MAX), ("", "a longer password than most", 11_644_473_600 * 10_000_000)]
    {
        let m = quatrelle_net::ntlm::authenticate(&challenge, user, password, now, [7; 8]);
        check_authenticate(&m);
    }
});
