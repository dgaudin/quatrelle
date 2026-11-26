// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The X11 name of Quatrelle's windows (`WM_CLASS`, instance then class). egui gives a window
//! the class only, its instance empty, and GNOME matches an empty instance to any desktop entry
//! whose `StartupWMClass` is empty (a Remmina flatpak's): the dock then draws that app's icon
//! for Quatrelle, and groups its windows there. Each window of this process gets both, the app
//! id, as the desktop entry's `StartupWMClass` says; windows opened later too (a message's own,
//! a reply's), looked for every few seconds on a connection and a thread of their own.

use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode};
use x11rb::wrapper::ConnectionExt as _;

/// How often the windows are looked at.
const EVERY: Duration = Duration::from_secs(2);
/// Most windows of the desktop looked at, and the longest name read.
const MAX_WINDOWS: u32 = 4096;
const MAX_NAME: u32 = 512;

/// `WM_CLASS` as Quatrelle's windows carry it: the instance, then the class, each ended by a
/// zero byte (ICCCM 4.1.2.5).
pub(crate) fn wanted(app_id: &str) -> Vec<u8> {
    format!("{app_id}\0{app_id}\0").into_bytes()
}

/// Whether a window's `WM_CLASS` is to be written again: any other than `wanted`.
pub(crate) fn needs_name(current: &[u8], wanted: &[u8]) -> bool {
    current != wanted
}

/// Name every window of this process from now on, on a thread of its own (X11 only: under
/// Wayland the app id is the window's name already).
pub(crate) fn name_windows(app_id: &'static str) {
    let spawned = std::thread::Builder::new().name("wm-class".into()).spawn(move || {
        let Ok((conn, screen)) = x11rb::connect(None) else { return };
        let Some(root) = conn.setup().roots.get(screen).map(|s| s.root) else { return };
        let want = wanted(app_id);
        loop {
            if let Err(e) = name_once(&conn, root, &want) {
                log::info!("window names: {e}");
                return;
            }
            std::thread::sleep(EVERY);
        }
    });
    if let Err(e) = spawned {
        log::warn!("window names: {e}");
    }
}

/// One look: the desktop's windows (`_NET_CLIENT_LIST`) whose `_NET_WM_PID` is this process's
/// get `want` as their `WM_CLASS` where theirs differs.
fn name_once(conn: &impl Connection, root: u32, want: &[u8]) -> Result<(), String> {
    let err = |e: &dyn std::fmt::Display| e.to_string();
    let atom =
        |name: &str| -> Result<u32, String> { Ok(conn.intern_atom(false, name.as_bytes()).map_err(|e| err(&e))?.reply().map_err(|e| err(&e))?.atom) };
    let (clients, pid_atom) = (atom("_NET_CLIENT_LIST")?, atom("_NET_WM_PID")?);
    let list = conn.get_property(false, root, clients, AtomEnum::WINDOW, 0, MAX_WINDOWS).map_err(|e| err(&e))?.reply().map_err(|e| err(&e))?;
    let me = std::process::id();
    for w in list.value32().into_iter().flatten() {
        let Ok(pid) =
            conn.get_property(false, w, pid_atom, AtomEnum::CARDINAL, 0, 1).map_err(|e| err(&e)).and_then(|c| c.reply().map_err(|e| err(&e)))
        else {
            continue;
        };
        if pid.value32().and_then(|mut v| v.next()) != Some(me) {
            continue;
        }
        let Ok(class) = conn
            .get_property(false, w, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, MAX_NAME)
            .map_err(|e| err(&e))
            .and_then(|c| c.reply().map_err(|e| err(&e)))
        else {
            continue;
        };
        if needs_name(&class.value, want) {
            conn.change_property8(PropMode::REPLACE, w, AtomEnum::WM_CLASS, AtomEnum::STRING, want).map_err(|e| err(&e))?;
        }
    }
    conn.flush().map_err(|e| err(&e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window gets the app id as its instance and its class: egui's, with an empty instance,
    /// is written again; one already right is left alone.
    #[test]
    fn a_window_is_named_by_the_app_id_twice() {
        let want = wanted("ai.storyteller.quatrelle");
        assert_eq!(want, b"ai.storyteller.quatrelle\0ai.storyteller.quatrelle\0");
        assert!(needs_name(b"\0ai.storyteller.quatrelle\0", &want), "egui's empty instance");
        assert!(needs_name(b"", &want), "none at all");
        assert!(!needs_name(&want, &want), "already right");
    }
}
