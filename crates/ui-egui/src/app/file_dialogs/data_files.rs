// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! An account's mail exported to an Outlook data file, through the system's dialogs: a new
//! file by Save as, or one already there by Open.
//!
//! An export to a file already there adds to it, nothing in it replaced. Save as can't be told
//! so: the system's window (the file chooser portal, GTK, macOS's and Windows' panels) always
//! asks whether to replace a file picked there, and the window's toolkit has no way to turn the
//! question off. So adding to a file has an entry of its own, which opens the file the Open way
//! (nothing is asked), and the notice says when a file was added to, whichever way it came.

use crate::*;

impl MailApp {
    /// Export `account`'s mail to an Outlook data file: a new one (Save as, named after the
    /// account's address, as Outlook names a data file), or with `existing` one already there
    /// (Open).
    pub(crate) fn export_data_file(&mut self, account: String, existing: bool) {
        let path = if existing {
            self.file_answer_one(FileAsk::AddToDataFile, Action::AddToDataFile(account.clone()))
        } else {
            let name = self
                .session
                .account(&account)
                .map(|a| if a.config.email.trim().is_empty() { a.config.name.clone() } else { a.config.email.clone() })
                .unwrap_or_default();
            let name = quatrelle_engine::mime::sanitize_filename(if name.trim().is_empty() { "Mail" } else { name.trim() });
            self.file_answer_one(FileAsk::Save(format!("{name}.pst")), Action::ExportDataFile(account.clone()))
        };
        let Some(path) = path else { return };
        // Looked at before the export begins writing: a file there is added to (also one picked
        // in Save as, whose window has just asked whether to replace it).
        let there = path.is_file();
        match self.session.run("account.exportPst", &json!({"account": account, "path": path.to_string_lossy()})) {
            Ok(_) => {
                self.ui.backstage = None;
                let file = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                let say = if there {
                    tf!("Adding to {file}… (what it already has stays)", file = file)
                } else {
                    tf!("Exporting to {file}…", file = file)
                };
                self.session.notice(say, false);
            }
            Err(e) => self.session.notice(e.to_string(), true),
        }
    }
}
