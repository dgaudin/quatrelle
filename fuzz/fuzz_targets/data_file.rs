// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! An account's data file as a version before R3 may have left it, damaged any way: files whole
//! and in pieces (notes saying any length, pieces missing or too many), compressed or raw, and
//! `META` (how the file says it was written, where Compact Now's recompress stopped, where a move
//! pulled it back). Its `META` read, then compressed again (stopped after its first batch, gone
//! on with, then once more), and every file that read back before reads back the same. The input
//! is records, as `quatrelle_store::fuzz_data_file` reads them.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    assert!(quatrelle_store::fuzz_data_file(data), "a file reads back otherwise once compressed again");
});
