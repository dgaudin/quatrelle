// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Portable mode (issue #4): a marker file beside the executable ([`MARKERS`]) keeps everything
//! Quatrelle writes in `<exe dir>/QuatrelleData`: the account list, the window's state, the log
//! and the accounts' data files, so a copy on a USB stick leaves nothing in `%APPDATA%`,
//! Documents or the system's keychain. Passwords and sign-ins are then held in memory only and
//! asked for again at the next start, and new-mail notifications name neither sender nor subject
//! (the system keeps them). The Windows portable zip ships with `portable.txt`; the MSI doesn't.
//! The Craft apps share the scheme (PdfCraft's `portable.rs`, PhotoCraft's `app_dirs.rs`,
//! craft-libs' `craft-settings`); on macOS the marker goes beside the `.app`, since writing
//! inside a signed bundle breaks its signature.
//!
//! If that folder can't be written (a read-only medium, Program Files), portable mode stays off,
//! the per-user folders are used and the app says why.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Files beside the executable that switch on portable mode (any one; contents are ignored). The
/// first two remain from the fork this app grew out of, so an old stick keeps working.
pub const MARKERS: [&str; 3] = ["portable.txt", "Quatrelle.portable", quatrelle_engine::store::legacy::OLD_PORTABLE_MARKER];
/// The data folder created beside the executable in portable mode.
pub const DATA_DIR: &str = "QuatrelleData";

/// The outcome of looking for a portable marker.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Portable {
    /// `<exe dir>/QuatrelleData` when portable mode is on.
    pub dir: Option<PathBuf>,
    /// Why portable mode is off although a marker was found.
    pub unwritable: Option<String>,
}

/// Portable mode for this process, resolved once (the check probes the disk).
pub fn current() -> &'static Portable {
    static CURRENT: OnceLock<Portable> = OnceLock::new();
    CURRENT.get_or_init(|| {
        let exe = std::env::current_exe().ok();
        resolve(exe.as_deref().and_then(Path::parent))
    })
}

/// Look for a marker beside the executable in `exe_dir` and, when one is there, make its data
/// folder.
pub fn resolve(exe_dir: Option<&Path>) -> Portable {
    let Some(base) = exe_dir.map(|d| bundle_parent(d).unwrap_or(d)) else { return Portable::default() };
    let Some(marker) = MARKERS.iter().map(|m| base.join(m)).find(|p| p.is_file()) else { return Portable::default() };
    // A folder from the fork's days is taken over, with the account list and data files in it.
    let folder = quatrelle_engine::store::legacy::take_over(base, DATA_DIR, quatrelle_engine::store::legacy::OLD_PORTABLE_DIR);
    match writable(&folder) {
        Ok(()) => Portable { dir: Some(folder), unwritable: None },
        Err(e) => Portable {
            dir: None,
            unwritable: Some(format!(
                "portable mode is off: {} was found, but {} can't be written ({e}); Quatrelle keeps its data in your user folder instead",
                marker.display(),
                folder.display()
            )),
        },
    }
}

/// On macOS the executable is in `X.app/Contents/MacOS`: the folder the bundle is in.
fn bundle_parent(exe_dir: &Path) -> Option<&Path> {
    if !cfg!(target_os = "macos") || exe_dir.file_name()? != "MacOS" {
        return None;
    }
    let contents = exe_dir.parent()?;
    let bundle = contents.parent()?;
    (contents.file_name()? == "Contents" && bundle.extension()? == "app").then_some(bundle.parent()?)
}

/// Make `dir` and prove a file can be written in it (a folder can exist yet be read-only). The
/// probe is made new under a name nobody can tell in advance, so whatever is already there (a
/// link someone left pointing elsewhere) is never written through nor removed.
fn writable(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    quatrelle_engine::store::check_writable(dir).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("quatrelle-portable-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// No marker: off, and nothing made. Either marker: the data folder beside it, made (and
    /// what's in it kept). A folder named as a marker isn't one; a data folder that can't be made
    /// turns it off and says why.
    #[test]
    fn a_marker_beside_the_app_keeps_its_data_there() {
        let exe = temp("off");
        assert_eq!(resolve(Some(&exe)), Portable::default());
        assert!(!exe.join(DATA_DIR).exists());
        assert_eq!(resolve(None), Portable::default());
        let _ = std::fs::remove_dir_all(&exe);

        for marker in MARKERS {
            let exe = temp("on");
            std::fs::write(exe.join(marker), b"").unwrap();
            let p = resolve(Some(&exe));
            assert_eq!(p, Portable { dir: Some(exe.join(DATA_DIR)), unwritable: None }, "{marker}");
            assert!(exe.join(DATA_DIR).is_dir());
            assert_eq!(std::fs::read_dir(exe.join(DATA_DIR)).unwrap().count(), 0, "the write probe is removed");
            std::fs::write(exe.join(DATA_DIR).join("accounts.json"), b"[]").unwrap();
            assert_eq!(resolve(Some(&exe)).dir, Some(exe.join(DATA_DIR)));
            assert_eq!(std::fs::read(exe.join(DATA_DIR).join("accounts.json")).unwrap(), b"[]", "what's there is kept");
            let _ = std::fs::remove_dir_all(&exe);
        }

        // A stick from before the rename: its marker and folder are taken over, settings with them.
        let exe = temp("old");
        let (old_marker, old_dir) = (quatrelle_engine::store::legacy::OLD_PORTABLE_MARKER, quatrelle_engine::store::legacy::OLD_PORTABLE_DIR);
        std::fs::write(exe.join(old_marker), b"").unwrap();
        std::fs::create_dir_all(exe.join(old_dir)).unwrap();
        std::fs::write(exe.join(old_dir).join("accounts.json"), b"[]").unwrap();
        assert_eq!(resolve(Some(&exe)), Portable { dir: Some(exe.join(DATA_DIR)), unwritable: None });
        assert_eq!(std::fs::read(exe.join(DATA_DIR).join("accounts.json")).unwrap(), b"[]");
        assert!(!exe.join(old_dir).exists(), "renamed, not copied");
        let _ = std::fs::remove_dir_all(&exe);

        let exe = temp("dir-marker");
        std::fs::create_dir_all(exe.join("portable.txt")).unwrap();
        assert_eq!(resolve(Some(&exe)), Portable::default());
        let _ = std::fs::remove_dir_all(&exe);

        let exe = temp("unwritable");
        std::fs::write(exe.join("portable.txt"), b"").unwrap();
        std::fs::write(exe.join(DATA_DIR), b"a file in the way").unwrap();
        let p = resolve(Some(&exe));
        assert_eq!(p.dir, None);
        assert!(p.unwritable.is_some_and(|w| w.contains("portable mode is off") && w.contains("can't be written")));
        let _ = std::fs::remove_dir_all(&exe);
    }

    /// What's already in the data folder is never written through or removed: at the name the
    /// check used to write (`.write-test`), a link to a file elsewhere (a file, off Unix) stays as
    /// it was, so does the file it points at, and the probe made is gone again.
    #[test]
    fn the_write_probe_never_goes_through_what_is_at_its_name() {
        let root = temp("probe-taken");
        let data = root.join("data");
        std::fs::create_dir_all(&data).unwrap();
        let other = root.join("elsewhere.txt");
        std::fs::write(&other, b"keep me").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&other, data.join(".write-test")).unwrap();
        #[cfg(not(unix))]
        std::fs::write(data.join(".write-test"), b"someone's").unwrap();
        let listing = || {
            let mut names: Vec<_> = std::fs::read_dir(&data).unwrap().map(|e| e.unwrap().file_name()).collect();
            names.sort();
            names
        };
        let before = listing();
        writable(&data).unwrap();
        let (kept, after) = (std::fs::read(&other).unwrap(), listing());
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(kept, b"keep me", "the linked file is untouched");
        assert_eq!(after, before, "what was there stays, and the probe made is removed");
    }
}
