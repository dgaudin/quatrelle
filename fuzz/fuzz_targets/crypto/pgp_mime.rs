// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A message source read for OpenPGP (RFC 3156): PGP/MIME signed or encrypted, inline PGP, the
//! keys it carries (`Autocrypt:`, `application/pgp-keys`), taken apart and put back as the reading
//! pane does; never skipped by the header's quick look when it is PGP/MIME; and written signed,
//! then encrypted, as it would be sent.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_mime::{pgp, smime};

fuzz_target!(|raw: &[u8]| {
    let carries = pgp::may_carry(raw);
    match pgp::protection(raw) {
        pgp::Protection::None => {}
        pgp::Protection::Signed { content, .. } => {
            // What the header's quick look says no to is never opened.
            assert!(carries && smime::is_protected(raw), "signed PGP/MIME told unprotected");
            let (inner, _) = smime::with_entity_protected(raw, &content);
            let _ = quatrelle_mime::parse(&inner);
        }
        pgp::Protection::Encrypted(_) => assert!(carries && smime::is_protected(raw), "encrypted PGP/MIME told unprotected"),
        pgp::Protection::Inline { body, charset } => {
            let entity = pgp::text_entity(&body, &charset);
            let _ = quatrelle_mime::parse(&smime::with_entity(raw, &entity));
        }
    }
    let _ = pgp::autocrypt(raw);
    let _ = pgp::key_parts(raw);
    // Written for sending, its signature and encryption stand-ins of the right shape.
    let mut sign = |_: &[u8]| Ok(b"-----BEGIN PGP SIGNATURE-----\n\niQ\n-----END PGP SIGNATURE-----\n".to_vec());
    let mut encrypt = |_: &[u8]| Ok(b"-----BEGIN PGP MESSAGE-----\n\nhQ\n-----END PGP MESSAGE-----\n".to_vec());
    let hide = raw.first().is_some_and(|b| b & 1 == 1);
    let _ = pgp::protect(raw, Some(&mut sign), Some(&mut encrypt), hide, "=_fz");
    let _ = pgp::protect(raw, Some(&mut sign), None, hide, "=_fz");
});
