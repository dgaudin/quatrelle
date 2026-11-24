// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The main window's frame: whether Quatrelle's top bar is the window's title bar
//! ([`quatrelle_ui_egui::Services::own_title_bar`]), decided once before the window opens, which
//! then opens without the system's.

use serde_json::Value;

/// Whether Quatrelle's top bar is the window's title bar, as Outlook's is: on Windows, Linux and
/// the BSDs, where the desktop's title bar sat above it, unless File › Options asked for the
/// system's (`prefs.systemTitleBar` in the settings saved last time). Settings missing or
/// unreadable ask for nothing. A Mac keeps its own.
pub(crate) fn own_title_bar(saved_ui: Option<&Value>) -> bool {
    cfg!(any(windows, all(unix, not(target_os = "macos"))))
        && !saved_ui.and_then(|v| v.get("prefs")?.get("systemTitleBar")?.as_bool()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where the window draws its own title bar.
    const OWN: bool = cfg!(any(windows, all(unix, not(target_os = "macos"))));

    /// The top bar is the window's title bar on Windows, Linux and the BSDs, unless the settings
    /// ask for the system's; a Mac keeps its own.
    #[test]
    fn the_top_bar_is_the_title_bar_but_on_a_mac() {
        assert_eq!(own_title_bar(None), OWN);
        assert_eq!(own_title_bar(Some(&serde_json::json!({"prefs": {"systemTitleBar": false}}))), OWN);
        assert!(!own_title_bar(Some(&serde_json::json!({"prefs": {"systemTitleBar": true}}))));
        // Settings from before the option, or with it typed otherwise: Quatrelle's own bar.
        assert_eq!(own_title_bar(Some(&serde_json::json!({"prefs": {"conversations": true}}))), OWN);
        assert_eq!(own_title_bar(Some(&serde_json::json!({"prefs": {"systemTitleBar": "yes"}}))), OWN);
        assert_eq!(own_title_bar(Some(&serde_json::json!({"prefs": [true]}))), OWN);
        // What the window was opened with is what the top bar is told.
        use quatrelle_ui_egui::Services;
        assert!(crate::Desktop { own_title_bar: true, ..Default::default() }.own_title_bar());
        assert!(!crate::Desktop::default().own_title_bar(), "the system's bar unless the window opened without it");
    }

    /// The settings file is read as the window opens: missing, damaged or not settings at all, it
    /// gives Quatrelle's own bar and never stops the start; one that asks for the system's bar
    /// gives that.
    #[test]
    fn a_missing_or_damaged_settings_file_gives_the_own_title_bar() {
        let dir = std::env::temp_dir().join(format!("quatrelle-window-frame-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let at_start = |name: &str, content: Option<&[u8]>| {
            let path = dir.join(name);
            if let Some(c) = content {
                let _ = std::fs::write(&path, c);
            }
            own_title_bar(crate::saved_ui(Some(&path)).as_ref())
        };
        assert_eq!(own_title_bar(crate::saved_ui(None).as_ref()), OWN, "no data folder");
        assert_eq!(at_start("missing.json", None), OWN, "no ui.json yet");
        assert_eq!(at_start("damaged.json", Some(b"{\"prefs\": {\"systemTitleBar\": tr")), OWN, "cut short");
        assert_eq!(at_start("binary.json", Some(&[0xff, 0xfe, 0x00, 0x7b])), OWN, "not text");
        assert_eq!(at_start("empty.json", Some(b"")), OWN, "empty");
        assert_eq!(at_start("list.json", Some(b"[1, 2]")), OWN, "not settings");
        assert!(!at_start("system.json", Some(br#"{"prefs": {"systemTitleBar": true}, "dark": false}"#)), "asked for the system's");
        assert_eq!(at_start("own.json", Some(br#"{"prefs": {"systemTitleBar": false}}"#)), OWN, "asked for the own one");
        // A folder where the file should be: unreadable.
        let _ = std::fs::create_dir_all(dir.join("folder.json"));
        assert_eq!(at_start("folder.json", None), OWN, "a folder");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
