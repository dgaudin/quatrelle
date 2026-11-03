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
