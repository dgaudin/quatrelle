// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The system's file dialogs, asked without waiting: the window keeps drawing, and answering
//! the desktop, while one is open (a window that stops answering is "not responding" to GNOME,
//! which offers to kill it); what asked runs again with the answer once it's given.

use std::path::PathBuf;

use crate::*;

/// The file dialog open, and what asked for it.
#[derive(Default)]
pub(crate) struct FileDialogs {
    /// What runs again once the open dialog is answered, in the place that asked (the message
    /// being written, the message read).
    asked: Option<Action>,
    /// The answer, while that runs again.
    answer: Option<Vec<PathBuf>>,
}

/// How often, at most, the window looks for the answer while a dialog is open.
const ANSWER_POLL: std::time::Duration = std::time::Duration::from_millis(100);

impl MailApp {
    /// What file dialog `ask` answered, when `again` (what's running) runs with its answer.
    /// Otherwise the dialog opens, unless one is open already (nothing more opens then), and
    /// `again` runs once it's answered: `None` meanwhile, as on Cancel.
    pub(crate) fn file_answer(&mut self, ask: FileAsk, again: Action) -> Option<Vec<PathBuf>> {
        if let Some(answer) = self.ui.files.answer.take() {
            return Some(answer);
        }
        if self.ui.files.asked.is_none() && self.services.ask_file(ask) {
            self.ui.files.asked = Some(self.where_asked(again));
        }
        None
    }

    /// The one file or folder a dialog answered ([`Self::file_answer`]).
    pub(crate) fn file_answer_one(&mut self, ask: FileAsk, again: Action) -> Option<PathBuf> {
        self.file_answer(ask, again)?.into_iter().next()
    }

    /// `a` done where it was asked when it runs again: to the message being written then, or
    /// the message read then, whichever is shown by the time the answer comes.
    fn where_asked(&self, a: Action) -> Action {
        match a {
            Action::AttachFiles | Action::InsertPicture => match &self.session.compose {
                Some(c) => Action::ForCompose(c.id, Box::new(a)),
                None => a,
            },
            Action::SaveAttachment(_) | Action::SaveAttachments(_) | Action::SaveAll | Action::SaveMessage | Action::ExportPdf | Action::Print => {
                match &self.session.reading {
                    Some(r) => Action::InWindow(r.key.clone(), Box::new(a)),
                    None => a,
                }
            }
            a => a,
        }
    }

    /// Each frame while a dialog is open: its answer, once given, completes what asked (nothing,
    /// on Cancel); until then the window looks again shortly.
    pub(crate) fn file_dialog_answers(&mut self, ctx: &egui::Context) {
        if self.ui.files.asked.is_none() {
            return;
        }
        let Some(answer) = self.services.take_file_answer() else {
            ctx.request_repaint_after(ANSWER_POLL);
            return;
        };
        let Some(again) = self.ui.files.asked.take() else { return };
        if answer.is_empty() {
            return;
        }
        self.ui.files.answer = Some(answer);
        self.act(again);
        // Not taken (the message that asked is gone): never left for something else to take.
        self.ui.files.answer = None;
        ctx.request_repaint();
    }
}
