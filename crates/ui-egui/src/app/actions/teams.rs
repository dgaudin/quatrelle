// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A file shared in a Teams conversation, once the engine has saved it: opened with the system's
//! app, or, when its name could run a program, its folder shown instead.
//!
//! What is hostile here: the file and its name are a stranger's. A program is never started from
//! here (the engine says which files could run: `teams_could_run`); the path is the engine's,
//! inside the user's own folder of files handed to other apps, and the system is handed it as it
//! is (the launching rules are `Services::open_path`'s). The name is said on one line, without
//! control or bidi characters, cut at [`NAME_MAX`]. A file removed since it was saved (the engine
//! hands it again at the next "Open") is said, not handed over.

use quatrelle_engine::TeamsSaved;
use quatrelle_engine::model::pim::teams::{is_bidi, one_line};

use super::*;

/// The most characters of a name said in a notice.
const NAME_MAX: usize = 80;

/// A name as a notice says it: on one line, without bidi characters, at most [`NAME_MAX`] long.
fn plain(name: &str) -> String {
    let line: String = one_line(name).chars().filter(|&c| !is_bidi(c)).collect();
    if line.chars().count() > NAME_MAX {
        let cut: String = line.chars().take(NAME_MAX - 1).collect();
        format!("{cut}…")
    } else {
        line
    }
}

impl MailApp {
    /// The files the engine saved for the window since the last frame, each handed on once.
    pub(crate) fn teams_files(&mut self) {
        while let Some(saved) = self.session.take_teams_saved() {
            self.teams_saved(saved);
        }
    }

    /// One saved file: opened by the system's app, or, when it could run a program, its folder
    /// shown with a notice saying so. A file no longer there is said (the mail's sentence), and
    /// not handed to the system.
    pub(crate) fn teams_saved(&mut self, saved: TeamsSaved) {
        let name = plain(&saved.name);
        let path = std::path::PathBuf::from(&saved.path);
        #[cfg(not(target_arch = "wasm32"))]
        if let Err(e) = std::fs::metadata(&path) {
            self.session.notice(tf!("{name} could not be opened: {error}", name = name, error = e), true);
            return;
        }
        let (target, could_run) = match (saved.open, path.parent()) {
            (true, _) => (path.clone(), false),
            (false, Some(folder)) => (folder.to_path_buf(), true),
            (false, None) => return,
        };
        match self.services.open_path(&target) {
            Ok(()) if could_run => {
                self.session.notice(tf!("{name} could run a program, so it isn't opened: it is in the folder shown", name = name), false)
            }
            Ok(()) => {}
            Err(e) => self.session.notice(tf!("{name} could not be opened: {error}", name = name, error = e), true),
        }
    }
}
