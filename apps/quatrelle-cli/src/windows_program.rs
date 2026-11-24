// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Windows' own programs the CLI starts, by their full path: started by its bare name, a
//! program is looked for in the current folder first, where a portable copy's `rundll32.exe`
//! could hold anyone's. A copy of the app's `windows_programs.rs` — the engine is sans-I/O and
//! `net` is the wrong layer for one path, so the CLI keeps its own ten lines.

use std::path::PathBuf;

/// One of Windows' own programs, by its full path in Windows' system folder as Windows itself
/// answers it (`GetSystemDirectory`, not `%SystemRoot%`, which whoever starts the CLI may set).
pub(crate) fn path(name: &str) -> PathBuf {
    let answered = winsafe::GetSystemDirectory().ok().map(PathBuf::from);
    let system = answered.filter(|d| d.is_absolute()).unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32"));
    if name.eq_ignore_ascii_case("explorer.exe") {
        return system.parent().map_or_else(|| PathBuf::from(r"C:\Windows"), |p| p.to_path_buf()).join(name);
    }
    system.join(name)
}
