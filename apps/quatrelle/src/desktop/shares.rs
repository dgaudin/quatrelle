// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Network shares a message links to, shown by the system's file manager. A folder is opened;
//! a file that would run code is only ever shown in its folder, on every system.

/// A network share (`smb://server/share/…`) in the desktop's file manager, which mounts it
/// (asking for a password when the server wants one): a folder opened, a file shown in its
/// folder (not run). Through the file managers' common D-Bus interface (Nautilus, Dolphin,
/// Nemo…), on a thread of its own as starting one can take a moment; without one, the system's
/// handler for `smb://` (GNOME declares none: it went nowhere).
#[cfg(all(unix, not(target_os = "macos")))]
pub(super) fn show_share(uri: String) -> Result<(), String> {
    let file = uri.rsplit('/').next().is_some_and(|last| last.contains('.'));
    std::thread::Builder::new()
        .name("open-share".into())
        .spawn(move || {
            let method = if file { "ShowItems" } else { "ShowFolders" };
            let shown = zbus::blocking::Connection::session().and_then(|bus| {
                bus.call_method(
                    Some("org.freedesktop.FileManager1"),
                    "/org/freedesktop/FileManager1",
                    Some("org.freedesktop.FileManager1"),
                    method,
                    &(vec![uri.as_str()], ""),
                )
                .map(drop)
            });
            if let Err(e) = shown {
                // A file is never handed to the system's handler, which might run it: its folder is.
                let uri = if file { folder_of(&uri) } else { uri.as_str() };
                log::info!("no file manager answered for {uri} ({e}): the system's handler opens it");
                if let Err(e) = open::that_detached(uri) {
                    log::warn!("{uri} couldn't be opened: {e}");
                }
            }
        })
        .map(drop)
        .map_err(|e| e.to_string())
}

/// The folder a share's file is in: its address without its last part.
#[cfg(any(not(windows), test))]
pub(super) fn folder_of(address: &str) -> &str {
    address.trim_end_matches('/').rsplit_once('/').map_or(address, |(folder, _)| folder)
}

/// What Explorer is told to show a file in its folder, selected and not opened: the path in
/// quotes of its own, which nothing in it may close. `None` for a path that could.
#[cfg(any(windows, test))]
fn explorer_select(path: &str) -> Option<String> {
    let path = path.trim_end_matches('\\');
    (!path.is_empty() && !path.contains('"') && !path.chars().any(char::is_control)).then(|| format!("/select,\"{path}\""))
}

/// A share's file that would run code, shown in its folder by Explorer: never run.
#[cfg(windows)]
pub(super) fn show_in_explorer(path: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let select = explorer_select(path).ok_or_else(|| "not a path Explorer can be given".to_string())?;
    std::process::Command::new("explorer.exe").raw_arg(select).spawn().map(drop).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A program on a share is shown in its folder: Explorer is told to select it, in quotes
    /// nothing in the path can close, and elsewhere the folder is what's opened.
    #[test]
    fn a_shares_program_is_shown_not_run() {
        assert_eq!(explorer_select(r"\\files-01\Tools\My setup.exe").as_deref(), Some(r#"/select,"\\files-01\Tools\My setup.exe""#));
        assert_eq!(explorer_select(r"\\files-01\Tools\setup.exe\").as_deref(), Some(r#"/select,"\\files-01\Tools\setup.exe""#));
        for bad in ["", r#"\\files-01\Tools\a" --flag "b.exe"#, "\\\\files-01\\Tools\\a\nb.exe"] {
            assert_eq!(explorer_select(bad), None, "{bad:?}");
        }
        assert_eq!(folder_of("smb://files-01/Tools/setup.exe"), "smb://files-01/Tools");
        assert_eq!(folder_of("files-01/Tools/setup.exe/"), "files-01/Tools");
    }
}
