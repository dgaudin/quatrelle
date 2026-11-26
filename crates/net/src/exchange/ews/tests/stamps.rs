// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The save stamp written on a calendar group header.

use super::*;

/// The stamp is a signed 32-bit integer ([MS-OXOCFG] 2.2.9.4): whatever the header id, it fits
/// `i32` and isn't zero; and it is the same for the same id, from any connection.
#[test]
fn a_save_stamp_fits_a_signed_32_bit_integer() {
    for n in 0..=255u8 {
        let id = [n; 16];
        let stamp = group_stamp(&id);
        assert!(i32::try_from(stamp).is_ok_and(|s| s > 0), "{n} gave {stamp}");
        assert_eq!(stamp, group_stamp(&id));
    }
    assert_ne!(group_stamp(&[1; 16]), group_stamp(&[2; 16]));
}
