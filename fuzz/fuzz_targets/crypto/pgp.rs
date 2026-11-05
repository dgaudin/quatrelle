// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! OpenPGP as it arrives: armor and packets read as keys (public, secret, Autocrypt's), a message
//! decrypted with a test key and its signature checked, signatures detached and cleartext, inline
//! PGP found in a text; a secret key that opens, extended (not backed up: its S2K hashes 16 MiB
//! a part, too slow for a fuzzer).
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_pgp as pgp;

/// The crate's GnuPG test key (Ana, Curve25519), unlocked once.
fn ana() -> Option<&'static pgp::Unlocked> {
    static KEY: std::sync::OnceLock<Option<pgp::Unlocked>> = std::sync::OnceLock::new();
    KEY.get_or_init(|| pgp::read_secret_key(include_bytes!("../../../crates/pgp/testdata/ana-secret.asc"), "secret").ok()).as_ref()
}

/// Numbers that only have to be numbers: what's signed here is never kept.
struct Counter(u64);

impl rand_core::RngCore for Counter {
    fn next_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        self.0
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for b in dest {
            *b = (self.next_u64() >> 56) as u8;
        }
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl rand_core::CryptoRng for Counter {}

fuzz_target!(|input: &[u8]| {
    if let Some(ana) = ana() {
        let _ = pgp::decrypt(input, &[&ana.secret], &[&ana.public, input]);
        let _ = pgp::open(input, &[&ana.public, input]);
        let _ = pgp::verify(b"Content-Type: text/plain\r\n\r\nx\r\n", input, &[&ana.public, input]);
        let _ = pgp::verify_cleartext(input, &[&ana.public]);
    }
    let _ = pgp::recipients(input);
    let _ = pgp::is_encrypted(input);
    let _ = pgp::inline(input);
    if let Ok(keys) = pgp::public_keys(input) {
        for k in keys.iter().take(4) {
            let _ = pgp::info(k);
            let _ = pgp::minimal(k, "ana@posteo.example");
        }
    }
    let _ = pgp::info(input);
    // A secret key someone imports may later be extended here.
    if let Ok(k) = pgp::read_secret_key(input, "secret") {
        let mut rng = Counter(input.len() as u64);
        if let Ok(x) = pgp::extend(&k, 1_800_000_000, &mut rng) {
            let _ = pgp::info(&x.public);
        }
    }
    let text = String::from_utf8_lossy(input);
    let _ = pgp::autocrypt::parse(&text);
});
