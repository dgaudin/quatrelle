// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
use quatrelle_model::{AccountConfig, Flags, MessageKey, Summary};

use crate::Store;

mod compaction;
mod folders;
mod kept;
mod names;
mod opening;
mod recompress;

fn temp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("quatrelle-packed-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// Whether the disk has told the room a file takes (`on_disk`): ZFS (FreeBSD's runner) counts a
/// file's blocks only once its writes have reached the disk, and says one block of 512 bytes for
/// megabytes until then, so room given back can't be seen there. Linux always tells.
fn room_told(on_disk: u64) -> bool {
    let told = on_disk > 64 * 1024;
    assert!(told || !cfg!(target_os = "linux"), "the disk says {on_disk} bytes");
    told
}

fn account(id: &str, email: &str) -> AccountConfig {
    AccountConfig { id: id.into(), email: email.into(), ..Default::default() }
}

/// Bytes that don't compress (xorshift).
fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

/// Base64 (as attachments travel in mail) in lines of 76.
fn base64(bytes: &[u8]) -> Vec<u8> {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    for (i, c) in bytes.chunks(3).enumerate() {
        let n = c.iter().enumerate().fold(0u32, |n, (j, b)| n | (u32::from(*b) << (16 - 8 * j)));
        for j in 0..4 {
            out.push(if j <= c.len() { ABC[((n >> (18 - 6 * j)) & 63) as usize] } else { b'=' });
        }
        if i % 19 == 18 {
            out.extend_from_slice(b"\r\n");
        }
    }
    out
}
