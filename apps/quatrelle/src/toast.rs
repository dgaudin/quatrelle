//! Windows notifications under Quatrelle's own name and icon.
//!
//! A toast is shown for an application user model id (AUMID). Without one, notify-rust uses
//! PowerShell's, so the toast says "Windows PowerShell". An unpackaged app gives Windows its
//! name and icon under `HKCU\Software\Classes\AppUserModelId\<id>`; the Start menu shortcut the
//! installer makes carries the same id. Writing that per-user key needs no rights; it's done
//! when the first notification is shown (the icon file lives in Quatrelle's data folder).

/// Quatrelle's id: also the Linux app id and the macOS bundle id.
pub const APP_ID: &str = "ai.storyteller.quatrelle";

/// The icon shown on Quatrelle's notifications.
const ICON: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/ai.storyteller.quatrelle.png");

/// Whether toasts can use [`APP_ID`]: registered once, when the first one is shown.
pub fn registered() -> bool {
    static DONE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DONE.get_or_init(|| match try_register(quatrelle_engine::store::default_root().as_deref()) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("notifications will show under PowerShell's name: {e}");
            false
        }
    })
}

fn try_register(data: Option<&std::path::Path>) -> Result<(), String> {
    let key = windows_registry::CURRENT_USER.create(format!(r"Software\Classes\AppUserModelId\{APP_ID}")).map_err(|e| e.to_string())?;
    key.set_string("DisplayName", "Quatrelle").map_err(|e| e.to_string())?;
    if let Some(dir) = data {
        let icon = dir.join("notification-icon.png");
        if std::fs::read(&icon).ok().as_deref() != Some(ICON) {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            std::fs::write(&icon, ICON).map_err(|e| e.to_string())?;
        }
        key.set_string("IconUri", icon.to_string_lossy()).map_err(|e| e.to_string())?;
    }
    Ok(())
}
