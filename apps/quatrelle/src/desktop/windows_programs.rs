// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Windows' own programs Quatrelle starts (Explorer to open a folder, rundll32 to open a file or
//! a link, PowerShell to install an update or make a video's frame), by their full path. Started
//! by its bare name, a program is looked for in Quatrelle's own folder first, where a portable
//! copy's (in Downloads, on a USB stick) could hold anyone's `explorer.exe`.

use std::path::{Path, PathBuf};

/// One of Windows' own programs (`rundll32.exe`, `WindowsPowerShell\v1.0\powershell.exe`), by
/// its full path in Windows' system folder as Windows itself answers it (`GetSystemDirectory`, not
/// `%SystemRoot%`, which whoever starts Quatrelle may set); `explorer.exe` is in the Windows folder
/// above it.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn path(name: &str) -> PathBuf {
    #[cfg(windows)]
    let answered = winsafe::GetSystemDirectory().ok().map(PathBuf::from);
    #[cfg(not(windows))]
    let answered: Option<PathBuf> = None;
    let system = answered.filter(|d| d.is_absolute()).unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32"));
    if name.eq_ignore_ascii_case("explorer.exe") {
        return system.parent().map_or_else(|| PathBuf::from(r"C:\Windows"), Path::to_path_buf).join(name);
    }
    system.join(name)
}

#[cfg(test)]
mod tests {
    /// Read as text, no source of the app starts a Windows program by its bare name outside this
    /// module: the current folder is searched first, where a portable copy's `explorer.exe` would
    /// run. A `Command::new("…exe")` or a `("…exe", …)` tuple is refused; test-only code names no
    /// `.exe` (the keychain test's `cmdkey` runs only in tests).
    #[test]
    fn windows_programs_are_never_started_by_their_bare_name() {
        let sources = [
            include_str!("../desktop.rs"),
            include_str!("appearance.rs"),
            include_str!("browsers.rs"),
            include_str!("file_dialogs.rs"),
            include_str!("keychain.rs"),
            include_str!("lanes.rs"),
            include_str!("memory.rs"),
            include_str!("opening.rs"),
            include_str!("printing.rs"),
            include_str!("profiles.rs"),
            include_str!("shares.rs"),
            include_str!("start.rs"),
            include_str!("state_writer.rs"),
            include_str!("tray.rs"),
            include_str!("window_frame.rs"),
            include_str!("../self_update/install.rs"),
            include_str!("../video.rs"),
        ];
        for src in sources {
            for line in src.lines() {
                if line.contains(r#"::path(""#) {
                    continue;
                }
                if let Some(at) = line.find(r#"Command::new(""#) {
                    let lit = line[at + r#"Command::new(""#.len()..].split('"').next().unwrap_or("");
                    let low = lit.to_ascii_lowercase();
                    assert!(!(!lit.contains('{') && (low.ends_with(".exe") || low.contains("powershell"))), "a program by its bare name: {line}");
                }
                let mut at = 0;
                while let Some(rel) = line[at..].find(r#"(""#) {
                    let open = at + rel;
                    let before = line[..open].chars().next_back();
                    let tuple_start = before.is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '.' || c == '!' || c == '>'));
                    if tuple_start {
                        let lit = line[open + 2..].split('"').next().unwrap_or("");
                        let bare = !lit.contains('{') && lit.to_ascii_lowercase().ends_with(".exe");
                        assert!(!bare, "a program by its bare name: {line}");
                    }
                    at = open + 2;
                }
            }
        }
    }

    /// Windows' programs Quatrelle starts (to open a folder, a file or a link, and to install an
    /// update) are Windows' own, by their full path where Windows says its folders are. Started by
    /// name, an `explorer.exe` beside a portable Quatrelle ran in its place.
    #[cfg(windows)]
    #[test]
    fn windows_programs_are_started_by_their_full_path() {
        use super::*;
        let system = PathBuf::from(winsafe::GetSystemDirectory().unwrap());
        for name in ["rundll32.exe", "explorer.exe", r"WindowsPowerShell\v1.0\powershell.exe"] {
            let p = path(name);
            assert!(p.is_absolute() && p.is_file(), "{}", p.display());
        }
        assert_eq!(path("rundll32.exe"), system.join("rundll32.exe"));
        assert_eq!(path("explorer.exe"), system.parent().unwrap().join("explorer.exe"));
    }
}
