// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The system's file dialogs never hold the window: asked, they answer on a later frame, and
//! what asked completes then (GNOME killed a Quatrelle whose window stopped answering while its
//! Attach dialog was open, 03/11).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use super::*;
use crate::{Action, FileAsk};

/// The dialogs asked for, and the answer the next frame gets (as the user clicks it).
#[derive(Default)]
struct Desk {
    asked: Vec<FileAsk>,
    answer: Option<Vec<PathBuf>>,
}

/// A desktop whose file dialogs stay open until the test answers them, as the system's do.
struct Dialogs(Rc<RefCell<Desk>>);

impl crate::Services for Dialogs {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Err("refused".into())
    }
    fn ask_file(&mut self, ask: FileAsk) -> bool {
        self.0.borrow_mut().asked.push(ask);
        true
    }
    fn take_file_answer(&mut self) -> Option<Vec<PathBuf>> {
        self.0.borrow_mut().answer.take()
    }
}

fn attached(a: &MailApp, id: u64) -> Vec<String> {
    a.session.all_composes().find(|c| c.id == id).map(|c| c.attachments.iter().map(|x| x.name.clone()).collect()).unwrap_or_default()
}

/// Attach: the dialog opens and the window keeps going; a second Attach meanwhile opens nothing
/// more; the files picked go to the message that asked, though another is written since, and
/// to that one only; Cancel attaches nothing.
#[test]
fn attach_answers_later_to_the_message_that_asked() {
    let (mut a, dir) = app("file-dialogs-attach");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let desk = Rc::new(RefCell::new(Desk::default()));
    a.services = Box::new(Dialogs(desk.clone()));
    let file = dir.join("plan.txt");
    std::fs::write(&file, "the plan").unwrap();

    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "zoe@studio.example", "subject": "Plans"}));
    let first = a.session.compose.as_ref().map(|c| c.id).unwrap();
    a.act(Action::AttachFiles);
    assert_eq!(desk.borrow().asked, [FileAsk::Attach], "the dialog opened");
    // The window goes on drawing while it's open; Attach again opens no second dialog.
    let before = ctx.cumulative_pass_nr();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(ctx.cumulative_pass_nr() >= before + 3);
    a.act(Action::AttachFiles);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(desk.borrow().asked.len(), 1, "one dialog at a time");
    assert!(attached(&a, first).is_empty(), "nothing yet");

    // Another message is written meanwhile; the answer comes.
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "max@studio.example", "subject": "Other"}));
    let second = a.session.compose.as_ref().map(|c| c.id).unwrap();
    assert_ne!(first, second);
    desk.borrow_mut().answer = Some(vec![file.clone()]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(attached(&a, first), ["plan.txt"], "to the message that asked");
    assert!(attached(&a, second).is_empty(), "and to that one only");

    // Cancel: nothing attached, and the next Attach opens a dialog again.
    a.act(Action::ForCompose(second, Box::new(Action::AttachFiles)));
    assert_eq!(desk.borrow().asked.len(), 2);
    desk.borrow_mut().answer = Some(Vec::new());
    frame(&ctx, &mut a, vec![]);
    assert!(attached(&a, second).is_empty(), "cancelled");
    assert_eq!(attached(&a, first), ["plan.txt"]);
    a.act(Action::AttachFiles);
    assert_eq!(desk.borrow().asked.len(), 3, "free to ask again");
    let _ = std::fs::remove_dir_all(dir);
}

/// Save as: where to is asked, the window goes on, and the file is written once it's said.
#[test]
fn save_as_answers_later() {
    let (mut a, dir) = app("file-dialogs-save");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let desk = Rc::new(RefCell::new(Desk::default()));
    a.services = Box::new(Dialogs(desk.clone()));
    a.act(Action::ExportRules);
    assert_eq!(desk.borrow().asked, [FileAsk::Save("Quatrelle rules.json".into())]);
    frame(&ctx, &mut a, vec![]);
    let to = dir.join("rules.json");
    assert!(!to.exists());
    desk.borrow_mut().answer = Some(vec![to.clone()]);
    frame(&ctx, &mut a, vec![]);
    assert!(to.is_file(), "saved where it was said");
    let _ = std::fs::remove_dir_all(dir);
}

/// Where the File page's tile named `label` is, in a frame drawn with AccessKit on.
fn tile_at(ctx: &egui::Context, a: &mut MailApp, label: &str) -> Option<egui::Pos2> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 1600.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let nodes = out.platform_output.accesskit_update?.nodes;
    let r = nodes.iter().find(|(_, n)| n.label() == Some(label)).and_then(|(_, n)| n.bounds())?;
    Some(egui::pos2(((r.x0 + r.x1) / 2.0) as f32, ((r.y0 + r.y1) / 2.0) as f32))
}

/// A click at `at`, in the tall window `tile_at` draws.
fn click_at(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2) {
    let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    for events in [vec![Event::PointerMoved(at)], vec![press(true)], vec![press(false)], vec![]] {
        let input =
            RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 1600.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
    }
}

/// Exporting to an Outlook data file: Open & Export has two entries. Export asks the system for
/// Save as (a new file), whose window asks whether to replace a file picked there; Add to file
/// asks for the Open window, which doesn't, as the export adds to a file already there and
/// replaces nothing in it. A file already there is said to be added to, whichever way it came.
#[test]
fn adding_to_a_data_file_opens_it_rather_than_asking_to_replace_it() {
    let (mut a, dir) = app("file-dialogs-pst");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let desk = Rc::new(RefCell::new(Desk::default()));
    a.services = Box::new(Dialogs(desk.clone()));
    let old = dir.join("Old.pst");
    std::fs::write(&old, b"a data file already there").unwrap();
    let notice = |a: &MailApp| a.session.notices.last().map(|n| n.text.clone()).unwrap_or_default();

    // Add to file: the Open window, nothing to replace.
    a.act(Action::Backstage(crate::backstage::Page::Open));
    let at = tile_at(&ctx, &mut a, "Add to file").expect("Open & Export has Add to file");
    click_at(&ctx, &mut a, at);
    assert_eq!(desk.borrow().asked, [FileAsk::AddToDataFile], "the Open window, not Save as");
    desk.borrow_mut().answer = Some(vec![old.clone()]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(notice(&a), "Adding to Old.pst… (what it already has stays)");
    assert!(a.ui.backstage.is_none(), "the File page closed on the export");

    // Export: Save as, under the account's address; a new file is exported to.
    a.act(Action::Backstage(crate::backstage::Page::Open));
    let at = tile_at(&ctx, &mut a, "Export").expect("Open & Export has Export");
    click_at(&ctx, &mut a, at);
    let asked = desk.borrow().asked.clone();
    assert!(matches!(asked.as_slice(), [FileAsk::AddToDataFile, FileAsk::Save(name)] if name.ends_with(".pst")), "{asked:?}");
    desk.borrow_mut().answer = Some(vec![dir.join("New.pst")]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(notice(&a), "Exporting to New.pst…");

    // A file already there picked in Save as (its window has just asked to replace it): added to,
    // and said so.
    a.act(Action::ExportDataFile(a.session.accounts[0].config.id.clone()));
    desk.borrow_mut().answer = Some(vec![old]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(desk.borrow().asked.len(), 3);
    assert_eq!(notice(&a), "Adding to Old.pst… (what it already has stays)");
    let _ = std::fs::remove_dir_all(dir);
}
