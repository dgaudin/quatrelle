// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The cache, opened as the app opens it: the account folders of an older layout come into data
//! files, and what can't be opened leaves the app without a cache rather than without a window.

use std::path::{Path, PathBuf};

use quatrelle_engine::store::{Store, StoreError};

/// What the opening gave.
pub(crate) struct Opened {
    pub store: Option<Store>,
    /// The pending move's outcome as it came in, or the error of the data files that kept the
    /// older layout (told once).
    pub moved: Option<Result<PathBuf, String>>,
}

/// Why Quatrelle must stop on these data.
pub(crate) enum Stop {
    Names { path: String, why: String },
}

/// The cache opened as the app opens it. `moved`: what the pending move of the data files gave
/// (`apply_pending_move`, done by `main` before). `Err`: Quatrelle must stop on these data.
pub(crate) fn open_store(root: Option<&Path>, files: Option<&Path>, moved: Option<Result<PathBuf, String>>) -> Result<Opened, Stop> {
    let Some(r) = root else { return Ok(Opened { store: None, moved }) };
    let mut moved = moved;
    let opened = match files {
        Some(f) => match Store::open_packed(r, f) {
            // What says which data file is which account's can't be read: with the list taken for
            // none, every account's file would be forgotten and its mail downloaded again.
            Err(StoreError::NamesUnreadable { path, source }) => return Err(Stop::Names { path, why: source.kind().to_string() }),
            // Folders from before that couldn't come into data files stay as they are.
            Err(e) => {
                moved = Some(Err(e.to_string()));
                Store::open(r)
            }
            Ok(s) => Ok(s),
        },
        None => Store::open(r),
    };
    let store = match opened {
        Ok(s) => Some(s),
        Err(e) => {
            crate::say(&format!("the mail cache can't be opened ({e}); running without one"));
            None
        }
    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use quatrelle_engine::model::AccountConfig;
    use std::collections::BTreeMap;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("quatrelle-opening-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Every path under `dir` and, for a file, its bytes.
    fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p.clone());
                    out.insert(p, None);
                } else {
                    out.insert(p.clone(), std::fs::read(&p).ok());
                }
            }
        }
        out
    }

    /// A root with one account and its data file, as an earlier run left them.
    fn with_an_account(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let t = temp(tag);
        let (app, files) = (t.join("app"), t.join("Files"));
        let s = Store::open_packed(&app, &files).unwrap();
        let account = AccountConfig { id: "a1".into(), email: "zoe@studio.example".into(), ..Default::default() };
        s.save_accounts(&[account]).unwrap();
        s.save_account_doc("a1", "pim.json", &serde_json::json!({"notes": []})).unwrap();
        drop(s);
        (t, app, files)
    }

    /// What `open_store` stopped on, as (path, why).
    fn stopped(r: Result<Opened, Stop>) -> Option<(String, String)> {
        match r {
            Err(Stop::Names { path, why }) => Some((path, why)),
            Ok(_) => None,
        }
    }

    /// A list of which data file is which account's that can't be read (here a folder in its
    /// place) stops Quatrelle: taken for none, every account's file was forgotten, a second one
    /// made beside it, and the mail downloaded again under the root, with "your mail couldn't be
    /// moved" said. Nothing is made, in the app's folder or the data files'.
    #[test]
    fn an_unreadable_list_of_data_files_stops_quatrelle_on_these_data() {
        let (t, app, files) = with_an_account("names-unreadable");
        std::fs::remove_file(app.join("datafiles.json")).unwrap();
        std::fs::create_dir(app.join("datafiles.json")).unwrap();
        let before = (snapshot(&app), snapshot(&files));
        let (path, why) = stopped(open_store(Some(&app), Some(&files), None)).expect("stops on these data");
        assert!(path.ends_with("datafiles.json"), "{path}");
        assert!(!why.is_empty(), "the kind of error is said");
        assert_eq!((snapshot(&app), snapshot(&files)), before, "nothing made, nothing written, nothing set aside");
        let _ = std::fs::remove_dir_all(t);
    }

    /// The same with the list damaged and the data files' folder not there (a drive not plugged
    /// in): what stops the opening is that folder, and nothing is made.
    #[test]
    fn a_damaged_list_with_the_data_files_folder_away_stops_quatrelle_too() {
        let (t, app, files) = with_an_account("folder-away");
        std::fs::write(app.join("datafiles.json"), b"{\"a1\": ").unwrap();
        std::fs::rename(&files, t.join("Elsewhere")).unwrap();
        let before = snapshot(&t);
        let (path, _) = stopped(open_store(Some(&app), Some(&files), None)).expect("stops on these data");
        assert_eq!(path, files.display().to_string());
        assert_eq!(snapshot(&t), before, "nothing made, nothing written, nothing set aside");
        let _ = std::fs::remove_dir_all(t);
    }

    /// Guard: folders that can't come into data files (the data files' folder is a file) keep the
    /// older layout and say so, as before.
    #[test]
    fn folders_that_cant_come_into_data_files_still_open_the_old_way() {
        let t = temp("old-way");
        let (app, files) = (t.join("app"), t.join("not-a-folder"));
        std::fs::write(&files, b"x").unwrap();
        let moved = Some(Ok(PathBuf::from("elsewhere")));
        let Ok(opened) = open_store(Some(&app), Some(&files), moved) else { panic!("stopped") };
        assert!(opened.store.is_some(), "the old layout opens");
        assert!(matches!(opened.moved, Some(Err(_))), "the notice stays");
        // Opened as it came in when nothing went wrong.
        let (app, files) = (t.join("app2"), t.join("Files2"));
        let Ok(opened) = open_store(Some(&app), Some(&files), Some(Ok(PathBuf::from("to")))) else { panic!("stopped") };
        assert!(opened.store.is_some());
        assert_eq!(opened.moved, Some(Ok(PathBuf::from("to"))));
        let _ = std::fs::remove_dir_all(t);
    }

    /// The box names the list, says nothing was changed and what to do; not what it says for a
    /// move or a window.
    #[test]
    fn the_box_for_an_unreadable_list_names_the_file() {
        let m = cant_read_names("/x/datafiles.json", "permission denied");
        assert!(m.contains("/x/datafiles.json") && m.contains("permission denied"), "{m}");
        assert!(m.contains("Nothing has been changed") && m.contains("start Quatrelle again"), "{m}");
        assert!(!m.to_lowercase().contains("moved") && !m.to_lowercase().contains("graphics") && !m.contains("  "), "{m}");
    }
}
