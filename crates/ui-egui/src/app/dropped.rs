// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
//! Files dropped on the main window where nothing is being written: a new message with them
//! attached, as Outlook starts one (from upstream 865a712); message files, `.eml` and Outlook's
//! `.msg`, go into the folder open instead, as Outlook files them (from upstream 467901df). Files
//! dropped on a message being
//! written are its own (`reading::compose`), and files copied in a file manager are attached as
//! they're pasted into one.

use crate::*;

/// Most files taken from one drop.
const MAX_FILES: usize = 100;

impl MailApp {
    /// The main window's files from the desktop, with no message written in it: while they're
    /// held over it, it says what letting go does; let go, a new message with them attached.
    pub(crate) fn files_dropped_on_window(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        let writing_here = self.session.compose.as_ref().is_some_and(|c| compose_inline(c, ctx));
        // Somewhere else to put them first: the message written here, the File page, a dialog,
        // no account yet.
        if writing_here
            || self.session.module != Module::Mail
            || self.ui.backstage.is_some()
            || self.ui.dialog.is_some()
            || self.session.accounts.is_empty()
        {
            return;
        }
        let (held, dropped) = ctx.input(|i| (i.raw.hovered_files.clone(), i.raw.dropped_files.clone()));
        // Message files go into the folder open, as Outlook files them; others are attached.
        let open = match &self.session.view {
            quatrelle_engine::View::Folder { account, folder } => {
                self.session.folder(account, folder).map(|f| (f.path.clone(), crate::i18n::folder(f)))
            }
            _ => None,
        };
        let message = |p: &std::path::Path| quatrelle_engine::cmd::is_message_file(&p.to_string_lossy());
        let filing = open.as_ref().filter(|_| !held.is_empty() && held.iter().all(|f| f.path.as_deref().is_some_and(message)));
        if !held.is_empty() {
            let says = match filing {
                Some((_, name)) => tf!("Drop to add them to {folder}", folder = name),
                None => tl!("Drop to attach them to a new message").to_string(),
            };
            let rect = ctx.content_rect();
            let t = self.ui.tokens;
            let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("files-held-over")));
            p.rect_stroke(rect.shrink(3.0), 6.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
            let g = p.layout_no_wrap(says, theme::regular(15.0), t.text);
            let card = egui::Rect::from_center_size(rect.center(), g.size() + vec2(32.0, 20.0));
            p.rect(card, 8.0, t.surface, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
            p.galley(card.center() - g.size() / 2.0, g, t.text);
        }
        let files: Vec<String> = dropped
            .iter()
            .filter(|f| f.path().is_absolute() && f.path().is_file())
            .take(MAX_FILES)
            .map(|f| f.path().to_string_lossy().to_string())
            .collect();
        let (messages, files): (Vec<String>, Vec<String>) =
            files.into_iter().partition(|f| open.is_some() && quatrelle_engine::cmd::is_message_file(f));
        if !messages.is_empty() {
            acts.push(Action::Run("file.addToFolder", json!({"paths": messages})));
        }
        // Folders aren't attached: none, no message.
        if files.is_empty() {
            return;
        }
        acts.push(Action::Run("mail.new", json!({})));
        for f in files {
            acts.push(Action::Run("compose.attach", json!({"path": f})));
        }
    }

    /// What message files dropped on a folder did, said; and an Outlook message file opened with
    /// Quatrelle, shown once it's read.
    pub(crate) fn files_filed(&mut self) {
        for f in self.session.take_filed() {
            let folder = self.session.folder(&f.account, &f.folder).map(crate::i18n::folder).unwrap_or_else(|| f.folder.clone());
            if f.added > 0 {
                self.session.notice(tf!("Added to {folder}: {n}", folder = folder, n = f.added), false);
            }
            for (file, why) in f.failed.iter().take(3) {
                self.session.notice(tf!("Couldn't add {file} to {folder}: {error}", file = file, folder = folder, error = why), true);
            }
        }
        match self.session.opened_file.take() {
            Some(Ok(v)) => {
                self.ui.backstage = None;
                self.ui.dialog = Some(Dialog::preview(&v, &self.session.clock));
            }
            Some(Err(e)) => self.session.notice(e.to_string(), true),
            None => {}
        }
    }
}
