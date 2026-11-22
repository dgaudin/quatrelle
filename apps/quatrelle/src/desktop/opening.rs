// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The cache, opened as the app opens it: the account folders of an older layout come into data
//! files, and what can't be opened leaves the app without a cache rather than without a window.

use std::path::{Path, PathBuf};

use quatrelle_engine::store::Store;

/// What the opening gave.
pub(crate) struct Opened {
    pub store: Option<Store>,
    /// The pending move's outcome as it came in, or the error of the data files that kept the
    /// older layout (told once).
    pub moved: Option<Result<PathBuf, String>>,
}

/// Why Quatrelle must stop on these data.
#[allow(dead_code)]
pub(crate) enum Stop {
    Names { path: String, why: String },
}

/// The cache opened as the app opens it. `moved`: what the pending move of the data files gave
/// (`apply_pending_move`, done by `main` before). `Err`: Quatrelle must stop on these data.
pub(crate) fn open_store(root: Option<&Path>, files: Option<&Path>, moved: Option<Result<PathBuf, String>>) -> Result<Opened, Stop> {
    let mut moved = moved;
    let store = root.and_then(|r| {
        let opened = match files {
            Some(f) => Store::open_packed(r, f).or_else(|e| {
                // Folders from before that couldn't come into data files stay as they are.
                moved = Some(Err(e.to_string()));
                Store::open(r)
            }),
            None => Store::open(r),
        };
        match opened {
            Ok(s) => Some(s),
            Err(e) => {
                crate::say(&format!("the mail cache can't be opened ({e}); running without one"));
                None
            }
        }
    });
    Ok(Opened { store, moved })
}

/// What the message box says when the list of data files can't be read: `path` is the list, or the
/// folder or list that making it again needed, and `why` the kind of error.
pub(crate) fn cant_read_names(path: &str, why: &str) -> String {
    format!(
        "Quatrelle couldn't read what tells it which data file is which account's:\n{path}\n({why})\n\n\
         Nothing has been changed. Once that file or folder can be read again, or is back where it was (a drive plugged \
         in again, a share mounted), start Quatrelle again.\n\nThe details are in quatrelle.log, in Quatrelle's data folder."
    )
}

/// The box shown, the log flushed, then the exit: never returns.
pub(crate) fn stop(why: &Stop) -> ! {
    match why {
        Stop::Names { path, why } => {
            log::error!("the list of data files can't be read ({why}): {path}");
            log::logger().flush();
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("Quatrelle")
                .set_description(cant_read_names(path, why))
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
        }
    }
    std::process::exit(1)
}
