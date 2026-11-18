// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What's done with the open message's files: opened (asked first when that can run something),
//! saved one by one or all at once, dragged out, handed to a Craft app; downloaded in the browser.

use super::*;

impl MailApp {
    /// An action on the open message's files (`Action::OpenAttachment` and the like), done.
    pub(super) fn attachment_act(&mut self, a: Action) {
        match a {
            Action::DragOut(i) => {
                if self.services.can_drag_out() {
                    self.ui.drag_out = self.attachment_to_temp(i, false);
                }
            }
            Action::OpenInCraft(i, cmd) => {
                let Some(app) = self.ui.craft_apps.iter().find(|(c, _)| *c == cmd).map(|(_, p)| p.clone()) else { return };
                if let Some(path) = self.attachment_to_temp(i, true)
                    && let Err(e) = self.services.open_with(&app, &path)
                {
                    self.session.notice(tf!("{app} couldn't open it: {error}", app = app.display(), error = e), true);
                }
            }
            Action::SaveAttachment(i) | Action::OpenAttachment(i) | Action::OpenAttachmentNow(i) if self.services.downloads() => {
                let name = quatrelle_engine::mime::sanitize_filename(&self.attachment_name(i));
                match self.session.attachment_bytes(i) {
                    Ok(b) => self.services.download(&name, &b),
                    Err(e) => self.session.notice(e.to_string(), true),
                }
            }
            Action::SaveAll if self.services.downloads() => {
                let files: Vec<(usize, String)> = self
                    .session
                    .reading
                    .as_ref()
                    .and_then(|r| r.parsed.as_ref())
                    .map(|p| p.attachments.iter().filter(|a| !a.inline).map(|a| (a.index, a.name.clone())).collect())
                    .unwrap_or_default();
                for (i, name) in files {
                    if let Ok(b) = self.session.attachment_bytes(i) {
                        self.services.download(&quatrelle_engine::mime::sanitize_filename(&name), &b);
                    }
                }
            }
            Action::SaveAll | Action::SaveAttachments(_) => {
                let (indices, again) = match &a {
                    Action::SaveAttachments(v) => (json!(v), Action::SaveAttachments(v.clone())),
                    _ => (Value::Null, Action::SaveAll),
                };
                if let Some(dir) = self.file_answer_one(FileAsk::Folder, again) {
                    match self.session.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy(), "indices": indices})) {
                        Ok(v) => {
                            let n = v.get("saved").and_then(Value::as_array).map_or(0, Vec::len);
                            self.session.notice(tf!("Saved {n} attachments in {folder}", n = n, folder = dir.display()), false);
                        }
                        Err(e) => self.session.notice(e.to_string(), true),
                    }
                }
            }
            Action::SaveAttachment(i) => {
                let name = self.attachment_name(i);
                if let Some(path) = self.file_answer_one(FileAsk::Save(name), Action::SaveAttachment(i)) {
                    self.run("attachment.save", json!({"index": i, "path": path.to_string_lossy()}));
                }
            }
            Action::OpenAttachment(i) => {
                let name = quatrelle_engine::mime::sanitize_filename(&self.attachment_name(i));
                // A page whose scripts would run, a document whose macros would, or a file whose
                // name says nothing, by its name and the type its sender declared: asked first.
                if let Some(risk) = quatrelle_engine::mime::open_risk(&name, &self.attachment_mime(i))
                    && let Some(key) = self.session.reading.as_ref().map(|r| r.key.clone())
                {
                    self.ui.dialog = Some(Dialog::RiskyAttachment { key, index: i, name, risk });
                    return;
                }
                self.open_attachment(i, &name);
            }
            Action::OpenAttachmentNow(i) => {
                let name = quatrelle_engine::mime::sanitize_filename(&self.attachment_name(i));
                self.open_attachment(i, &name);
            }
            _ => {}
        }
    }

    /// Write an attachment to a temporary file to hand to another app: dragged out (a copy, as
    /// saving), or to be opened (never a program, script or shortcut: the engine refuses).
    pub(crate) fn attachment_to_temp(&mut self, i: usize, open: bool) -> Option<PathBuf> {
        if !self.ui.has_files {
            return None;
        }
        let name = self.attachment_name(i);
        let Some(dir) = quatrelle_engine::store::scratch_dir("open") else {
            self.session.notice(tl!("There's no folder to put the attachment in for another app"), true);
            return None;
        };
        let path = dir.join(quatrelle_engine::mime::sanitize_filename(&name));
        match self.session.run("attachment.save", &json!({"index": i, "path": path.to_string_lossy(), "open": open})) {
            Ok(_) => Some(path),
            Err(e) => {
                self.session.notice(e.to_string(), true);
                None
            }
        }
    }

    /// Hand an attachment to the program that opens it, saying so when that fails.
    fn open_attachment(&mut self, i: usize, name: &str) {
        if let Some(path) = self.attachment_to_temp(i, true)
            && let Err(e) = self.services.open_path(&path)
        {
            self.session.notice(tf!("{name} could not be opened: {error}", name = name, error = e), true);
        }
    }

    pub(crate) fn attachment_name(&self, i: usize) -> String {
        self.session
            .reading
            .as_ref()
            .and_then(|r| r.parsed.as_ref())
            .and_then(|p| p.attachments.iter().find(|a| a.index == i))
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "attachment".into())
    }

    /// The media type the sender declared for an attachment of the message being read.
    pub(crate) fn attachment_mime(&self, i: usize) -> String {
        self.session
            .reading
            .as_ref()
            .and_then(|r| r.parsed.as_ref())
            .and_then(|p| p.attachments.iter().find(|a| a.index == i))
            .map(|a| a.mime.clone())
            .unwrap_or_default()
    }

    /// Every attachment of the message being read, selected (Ctrl+A in its attachments).
    pub(crate) fn select_all_attachments(&mut self) {
        let Some(r) = self.session.reading.as_ref() else { return };
        let all: Vec<usize> = r.parsed.as_ref().map(|p| p.attachments.iter().filter(|a| !a.inline).map(|a| a.index).collect()).unwrap_or_default();
        self.ui.attachment_selected = Some((r.key.clone(), all));
    }
}
