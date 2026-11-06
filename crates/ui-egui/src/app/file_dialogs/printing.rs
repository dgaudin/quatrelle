// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Printing the message read (Ctrl+P, ⋯ › Print, File › Print) through the system's print
//! window, without waiting: its PDF, the one Save as PDF writes, is handed over, and nothing is
//! saved first. Where there's no print window (a desktop without the print portal, Windows,
//! macOS), or it fails, the PDF is saved where asked and opened in the viewer to print from
//! there, as Print did before, and the status bar says why.

use super::*;

impl MailApp {
    /// Print the open message: its PDF to the system's print window, else [`Self::print_as_pdf`].
    pub(crate) fn print(&mut self) {
        // Its print window is open already: that one prints.
        if self.ui.files.printing.is_some() {
            return;
        }
        let pdf = match self.session.message_pdf() {
            Ok(pdf) => pdf,
            Err(e) => return self.session.notice(e.to_string(), true),
        };
        if self.services.print(&self.message_file_name(), pdf) {
            self.ui.files.printing = Some(self.where_asked(Action::PrintAsPdf));
        } else {
            self.print_as_pdf();
        }
    }

    /// The open message as a PDF, saved where asked, then opened to print from the viewer.
    pub(crate) fn print_as_pdf(&mut self) {
        let Some(path) = self.file_answer_one(FileAsk::Save(format!("{}.pdf", self.message_file_name())), Action::PrintAsPdf) else { return };
        match self.session.run("message.exportPdf", &json!({"path": path.to_string_lossy()})) {
            Ok(_) => match self.services.open_path(&path) {
                Ok(()) => self.session.notice(tl!("The system's print window isn't available: opened as a PDF, print it from the viewer"), false),
                Err(e) => self.session.notice(tf!("The PDF couldn't be opened: {error}", error = e), true),
            },
            Err(e) => self.session.notice(e.to_string(), true),
        }
    }

    /// Each frame while the print window is open: once it's answered, printed is said, Cancel
    /// says nothing, and a failure falls back on the viewer; until then the window looks again
    /// shortly.
    pub(crate) fn print_answers(&mut self, ctx: &egui::Context) {
        if self.ui.files.printing.is_none() {
            return;
        }
        let Some(outcome) = self.services.take_printed() else {
            ctx.request_repaint_after(ANSWER_POLL);
            return;
        };
        let Some(fallback) = self.ui.files.printing.take() else { return };
        match outcome {
            PrintOutcome::Sent => self.session.notice(tl!("Sent to the printer"), false),
            PrintOutcome::Cancelled => {}
            PrintOutcome::Failed(why) => {
                log::warn!("the system's print window failed ({why}): the PDF opens in the viewer");
                self.act(fallback);
            }
        }
        ctx.request_repaint();
    }
}
