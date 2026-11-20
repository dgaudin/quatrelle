// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What a Teams conversation asks of the window. A file shared in it, once the engine has saved
//! it: opened with the system's app, or, when its name could run a program or its kind is
//! unknown, its folder shown instead. What its box asks for ([`TeamsAct`]): files picked in the
//! system's file dialog, or a screenshot pasted, to go with the message being written.
//!
//! What is hostile here: the file and its name are a stranger's. A program is never started from
//! here (the engine says which files could run: `teams_could_run`); the path is the engine's,
//! inside the user's own folder of files handed to other apps, and the system is handed it as it
//! is (the launching rules are `Services::open_path`'s). The name is said on one line, without
//! control or bidi characters, cut at [`NAME_MAX`]. A file removed since it was saved (the engine
//! hands it again at the next "Open") isn't handed over: the engine is told (`teams.fileGone`),
//! which downloads it again when an Open handed it, else it is said. The files picked and the
//! screenshot pasted are the user's: looked at, or written to a file of the user's own, by the box
//! (`writing/attach.rs`).

use quatrelle_engine::model::pim::teams::{is_bidi, one_line};
use quatrelle_engine::{TeamsOpen, TeamsSaved, teams_open};

use super::*;
use crate::conversations::TeamsAct;

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
    /// What a conversation's box asked for: the system's file dialog (asked without waiting: its
    /// answer comes back here), whose files go with `chat`'s message; or a screenshot pasted in it,
    /// the picture egui handed over (the clipboard isn't read again). Nothing in the browser,
    /// which has no files.
    pub(crate) fn teams_act(&mut self, a: TeamsAct) {
        if cfg!(target_arch = "wasm32") {
            return;
        }
        match a {
            TeamsAct::Attach { chat } => {
                let again = Action::Teams(TeamsAct::Attach { chat: chat.clone() });
                if let Some(paths) = self.file_answer(FileAsk::Attach, again) {
                    conversations::add_files(conversations::draft(&mut self.ui.conversations, &chat), paths);
                }
            }
            TeamsAct::Paste { chat, png } => conversations::add_picture(conversations::draft(&mut self.ui.conversations, &chat), &png),
        }
    }

    /// The files the engine saved for the window since the last frame, each handed on once.
    pub(crate) fn teams_files(&mut self) {
        while let Some(saved) = self.session.take_teams_saved() {
            self.teams_saved(saved);
        }
    }

    /// One saved file: opened by the system's app, or, when it could run a program, its folder
    /// shown with a notice saying so. A file no longer there isn't handed to the system: the engine
    /// is told, and asks it again when an Open handed it (the card says it downloads); else it is
    /// said (the mail's sentence).
    pub(crate) fn teams_saved(&mut self, saved: TeamsSaved) {
        let name = plain(&saved.name);
        let path = std::path::PathBuf::from(&saved.path);
        #[cfg(not(target_arch = "wasm32"))]
        if let Err(e) = std::fs::metadata(&path) {
            let again = self.session.run("teams.fileGone", &json!({"path": saved.path}));
            if !again.is_ok_and(|v| v["asked"] == true) {
                self.session.notice(tf!("{name} could not be opened: {error}", name = name, error = e), true);
            }
            return;
        }
        let (target, opens) = match (saved.open, path.parent()) {
            (true, _) => (path.clone(), TeamsOpen::Opened),
            (false, Some(folder)) => (folder.to_path_buf(), teams_open(&saved.name)),
            (false, None) => return,
        };
        match self.services.open_path(&target) {
            Ok(()) if opens != TeamsOpen::Opened => {
                // The mail's sentence for a file that could run, this one's for a kind Quatrelle
                // doesn't know; the name said apart before it.
                let said = match opens {
                    TeamsOpen::Unknown => tl!("Quatrelle doesn't know this kind of file: it is shown in its folder, not opened."),
                    _ => tl!("This file could run a program: it is shown in its folder, not opened."),
                };
                self.session.notice(format!("{name}: {said}"), false)
            }
            Ok(()) => {}
            Err(e) => self.session.notice(tf!("{name} could not be opened: {error}", name = name, error = e), true),
        }
    }
}
