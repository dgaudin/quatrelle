// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The save stamp written on a calendar group header.

use super::*;

/// The stamp is a signed 32-bit integer ([MS-OXOCFG] 2.2.9.4): whatever the random bytes, it
/// fits `i32` and isn't zero.
#[test]
fn a_save_stamp_fits_a_signed_32_bit_integer() {
    for bytes in [[0, 0, 0, 0x90], [0xff; 4], [0; 4], [1, 0, 0, 0], [0xff, 0xff, 0xff, 0x7f]] {
        let stamp = stamp_from(bytes);
        assert!(i32::try_from(stamp).is_ok_and(|s| s > 0), "{bytes:?} gave {stamp}");
    }
}
