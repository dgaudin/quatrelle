// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The files kept with an appointment or a contact (a `.pst` import's, a calendar's inline
//! `ATTACH`), as chips like a message's attachments: in the appointment's window and on the
//! contact's page. Each opens with a double-click or Open, and saves with Save As; nothing opens
//! without the user's asking, never a program, and a file opened is written in the folder the
//! app opens attachments from first, as a message's are.

use egui::Rect;
use quatrelle_engine::model::pim::files::KeptFile;

use super::*;
use crate::MailApp;
use crate::services::FileAsk;

/// A chip's height, as a message's attachment's.
const CHIP_H: f32 = 42.0;

/// What the files are kept with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilesOf {
    /// An appointment, at the occurrence shown (a series' changed one may carry its own).
    Event {
        uid: String,
        occurrence: Option<i64>,
    },
    Contact {
        uid: String,
    },
}

/// One of an appointment's or a contact's files, for Open and Save As.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemFile {
    pub of: FilesOf,
    /// Its place among the item's files.
    pub index: usize,
    pub name: String,
}

impl ItemFile {
    /// The command that writes it to `path` (`open`: to be opened, so never a program), and its
    /// params.
    fn command(&self, path: &std::path::Path, open: bool) -> (&'static str, Value) {
        let path = path.to_string_lossy();
        match &self.of {
            FilesOf::Event { uid, occurrence } => {
                ("event.saveFile", json!({"uid": uid, "occurrence": occurrence, "index": self.index, "path": path, "open": open}))
            }
            FilesOf::Contact { uid } => ("contact.saveFile", json!({"uid": uid, "index": self.index, "path": path, "open": open})),
        }
    }

    /// The name it's saved under.
    fn file_name(&self) -> String {
        let name = if self.name.trim().is_empty() { "attachment" } else { self.name.as_str() };
        quatrelle_engine::mime::sanitize_filename(name)
    }
}

/// The files, under a line that counts them, wrapped into rows (scrolling past two).
pub(crate) fn strip(ui: &mut Ui, t: &Tokens, of: &FilesOf, files: &[KeptFile], acts: &mut Vec<Action>) {
    if files.is_empty() {
        return;
    }
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        icons::paint(ui.painter(), r, Icon::Attach, t.dim);
        let count = if files.len() == 1 { tl!("1 attachment").to_string() } else { tf!("{n} attachments", n = files.len()) };
        ui.label(RichText::new(count).color(t.dim).small());
    });
    ui.add_space(2.0);
    let id = egui::Id::unique(("item-files", of_id(of)));
    widgets::chip_grid(ui, id, files.len(), 220.0, 360.0, CHIP_H, 2, |ui, i, w| {
        if let Some(file) = files.get(i) {
            let item = ItemFile { of: of.clone(), index: i, name: file.name.clone() };
            chip(ui, t, item, file, w, acts);
        }
    });
}

/// An appointment's files, above its notes.
pub(crate) fn event_strip(ui: &mut Ui, t: &Tokens, f: &EventForm, acts: &mut Vec<Action>) {
    let Some(uid) = f.uid.as_ref().filter(|_| !f.files.is_empty()) else { return };
    ui.add_space(6.0);
    strip(ui, t, &FilesOf::Event { uid: uid.clone(), occurrence: f.occurrence }, &f.files, acts);
    ui.add_space(4.0);
}

/// What tells the strips apart.
fn of_id(of: &FilesOf) -> String {
    match of {
        FilesOf::Event { uid, occurrence } => format!("event-{uid}-{}", occurrence.unwrap_or_default()),
        FilesOf::Contact { uid } => format!("contact-{uid}"),
    }
}

/// A file as a message's attachment chip shows one: its kind's icon, its name and size, and a ▾
/// menu (Open, Save As…), the same on a right-click. A double-click opens it.
fn chip(ui: &mut Ui, t: &Tokens, item: ItemFile, file: &KeptFile, w: f32, acts: &mut Vec<Action>) {
    // One without a name is shown by the name it's saved under, as a message's is.
    let name = if file.name.trim().is_empty() { "attachment".to_string() } else { file.name.clone() };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, CHIP_H), Sense::click());
    let more_rect = Rect::from_min_max(egui::pos2(rect.right() - 30.0, rect.top()), rect.max).shrink(2.0);
    let more = ui.interact(more_rect, resp.id.with("more"), Sense::click()).on_hover_text(tl!("More options"));
    widgets::named(ui, &more, tl!("More options"), true, None);
    let on_more = more.contains_pointer();
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect(rect, 4.0, t.surface, egui::Stroke::new(1.0, t.faint.gamma_multiply(0.55)), egui::StrokeKind::Inside);
        if resp.hovered() && !on_more {
            let body = Rect::from_min_max(rect.min, egui::pos2(more_rect.left() - 2.0, rect.bottom())).shrink(2.0);
            p.rect_filled(body, 3.0, t.hover_row);
        }
        if on_more || egui::Popup::is_id_open(ui.ctx(), more.id.with("menu")) {
            p.rect_filled(more_rect, 3.0, t.hover_row);
        }
        icons::paint(p, icons::centered(more_rect, 13.0), Icon::ChevronDown, if on_more { t.text } else { t.dim });
        let r = Rect::from_min_size(egui::pos2(rect.left() + 6.0, rect.center().y - 15.0), vec2(30.0, 30.0));
        p.rect_filled(r, 4.0, t.accent_soft);
        let image = quatrelle_engine::mime::kind_of(&file.mime, &file.name) == quatrelle_engine::mime::AttachmentKind::Image;
        icons::paint(p, icons::centered(r, 17.0), if image { Icon::Image } else { Icon::File }, t.accent);
        let text_x = r.right() + 8.0;
        let text_w = more_rect.left() - 4.0 - text_x;
        let shown = widgets::one_line(ui, &name, theme::regular(12.5), t.text, text_w);
        let size = widgets::one_line(ui, &widgets::human_size(file.size), theme::regular(11.5), t.dim, text_w);
        p.galley(egui::pos2(text_x, rect.center().y - shown.size().y - 1.0), shown, t.text);
        p.galley(egui::pos2(text_x, rect.center().y + 1.0), size, t.dim);
    }
    let menu = |ui: &mut Ui, acts: &mut Vec<Action>| {
        if ui.button(tl!("Open")).clicked() {
            acts.push(Action::OpenItemFile(item.clone()));
            ui.close();
        }
        ui.separator();
        if ui.button(tl!("Save As…")).clicked() {
            acts.push(Action::SaveItemFile(item.clone()));
            ui.close();
        }
    };
    egui::Popup::menu(&more).id(more.id.with("menu")).show(|ui| menu(ui, acts));
    if resp.double_clicked() && !on_more {
        acts.push(Action::OpenItemFile(item.clone()));
    }
    resp.context_menu(|ui| menu(ui, acts));
    if !on_more {
        resp.on_hover_text(name);
    }
}

impl MailApp {
    /// Open (`open`) or save one of an appointment's or a contact's files: saved where the user
    /// says; opened from the folder attachments are opened from (never a program: the engine
    /// refuses); handed over as a download in the browser.
    pub(crate) fn item_file(&mut self, item: ItemFile, open: bool) {
        let name = item.file_name();
        if self.services.downloads() {
            let got = match &item.of {
                FilesOf::Event { uid, occurrence } => self.session.event_file(uid, *occurrence, item.index),
                FilesOf::Contact { uid } => self.session.card_file(uid, item.index),
            };
            match got {
                Ok((_, data)) => self.services.download(&name, &data),
                Err(e) => self.session.notice(e.to_string(), true),
            }
            return;
        }
        if !open {
            if let Some(path) = self.file_answer_one(FileAsk::Save(name), Action::SaveItemFile(item.clone())) {
                let (id, params) = item.command(&path, false);
                self.run(id, params);
            }
            return;
        }
        if !self.ui.has_files {
            return;
        }
        let Some(dir) = quatrelle_engine::store::scratch_dir("open") else {
            self.session.notice(tl!("There's no folder to put the attachment in for another app"), true);
            return;
        };
        let path = dir.join(&name);
        let (id, params) = item.command(&path, true);
        match self.session.run(id, &params) {
            Ok(_) => {
                if let Err(e) = self.services.open_path(&path) {
                    self.session.notice(tf!("{name} could not be opened: {error}", name = name, error = e), true);
                }
            }
            Err(e) => self.session.notice(e.to_string(), true),
        }
    }
}
