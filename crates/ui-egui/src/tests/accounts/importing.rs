// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A data file's import in the interface: the status bar's line, bar and Stop button, the
//! account's menu, and the question when the file has changed.

use super::*;
use quatrelle_engine::model::{IMPORT_DOC, IMPORT_FILE_CHANGED, ImportJournal, Job};

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

type Handed = std::rc::Rc<std::cell::RefCell<Vec<Job>>>;

struct Recorder(Handed);

impl crate::Backend for Recorder {
    fn submit(&mut self, job: Job) {
        self.0.borrow_mut().push(job);
    }

    fn poll(&mut self) -> Vec<MailEvent> {
        Vec::new()
    }
}

/// An app importing `Archive.pst` into a new account; its id and the jobs handed out from now on.
fn importing(name: &str) -> (MailApp, std::path::PathBuf, String, Handed) {
    let (mut a, dir) = app(name);
    let handed = Handed::default();
    a.backend = Box::new(Recorder(handed.clone()));
    let id = a.session.run("account.importPst", &json!({"path": "C:/Mail/Archive.pst"})).unwrap()["account"].as_str().unwrap().to_string();
    a.session.take_jobs();
    (a, dir, id, handed)
}

fn progress(a: &mut MailApp, id: &str, done: usize, total: Option<usize>, folder: &str) {
    a.session.apply(MailEvent::Imported {
        account: id.into(),
        messages: done,
        skipped: 0,
        total,
        folder: folder.into(),
        stopped: false,
        done: false,
        error: None,
    });
}

/// A right-click at `at`, a frame after.
fn right_click_at(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2) {
    let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
    for _ in 0..20 {
        frame(ctx, a, vec![]);
    }
    frame(ctx, a, vec![Event::PointerMoved(at)]);
    frame(ctx, a, vec![press(true)]);
    frame(ctx, a, vec![press(false)]);
    frame(ctx, a, vec![]);
}

/// A few frames, so the layout has settled, and the last one's shapes.
fn settled(ctx: &egui::Context, a: &mut MailApp) -> Vec<egui::Shape> {
    for _ in 0..3 {
        frame(ctx, a, vec![]);
    }
    frame_shapes(ctx, a, vec![], SIZE)
}

/// Whether a thin bar (5 px tall, 90 wide) was drawn.
fn bar_drawn(shapes: &[egui::Shape]) -> bool {
    shapes.iter().any(|s| matches!(s, egui::Shape::Rect(r) if (r.rect.height() - 5.0).abs() < 0.01 && (r.rect.width() - 90.0).abs() < 0.01))
}

/// The line says the folder and how far, with a bar when the messages were counted; without a
/// total it is the same line without "of {total}" and without the bar.
#[test]
fn the_status_bar_follows_an_import_with_and_without_a_total() {
    let (mut a, dir, id, _) = importing("import-line");
    let ctx = egui::Context::default();
    progress(&mut a, &id, 0, None, "");
    let shapes = settled(&ctx, &mut a);
    assert!(text_at(&shapes, "Archive.pst: 0 messages").is_some());
    assert!(!bar_drawn(&shapes), "no total, no bar");
    progress(&mut a, &id, 400, None, "INBOX/Caf&AOk-");
    let shapes = settled(&ctx, &mut a);
    assert!(text_at(&shapes, "Archive.pst: Café — 400 messages").is_some());
    progress(&mut a, &id, 400, Some(3000), "INBOX/Caf&AOk-");
    let shapes = settled(&ctx, &mut a);
    assert!(text_at(&shapes, "Archive.pst: Café — 400 of 3000 messages").is_some());
    assert!(text_at(&shapes, "Stop").is_some() && bar_drawn(&shapes));
    // The repeated notice is gone: what is shown is the line.
    assert!(a.session.notices.iter().all(|n| !n.text.contains("so far")));
    // Over: the line goes.
    a.session.apply(MailEvent::Imported {
        account: id.clone(),
        messages: 3000,
        skipped: 0,
        total: Some(3000),
        folder: String::new(),
        stopped: false,
        done: true,
        error: None,
    });
    let shapes = settled(&ctx, &mut a);
    assert!(text_at(&shapes, "Stop").is_none() && !bar_drawn(&shapes));
    let _ = std::fs::remove_dir_all(dir);
}

/// The Stop button runs `account.stopImport`: the job goes out, and the line says it is stopping.
#[test]
fn the_stop_button_stops_the_import() {
    let (mut a, dir, id, handed) = importing("import-stop");
    let ctx = egui::Context::default();
    progress(&mut a, &id, 400, Some(3000), "INBOX");
    let shapes = settled(&ctx, &mut a);
    let at = text_at(&shapes, "Stop").expect("the Stop button");
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert!(handed.borrow().iter().any(|j| matches!(j, Job::StopImport { account } if *account == id)));
    assert!(a.session.import_of(&id).is_some_and(|i| i.stopping));
    let shapes = settled(&ctx, &mut a);
    assert!(text_at(&shapes, "Archive.pst: stopping the import…").is_some());
    let _ = std::fs::remove_dir_all(dir);
}

/// The account's menu offers to stop the import while it runs, to go on with it when it isn't
/// finished, and neither otherwise.
#[test]
fn the_accounts_menu_offers_stop_and_go_on() {
    let (mut a, dir, id, handed) = importing("import-menu");
    let ctx = egui::Context::default();
    progress(&mut a, &id, 10, Some(3000), "INBOX");
    let shapes = settled(&ctx, &mut a);
    let at = text_at(&shapes, "Archive.pst").expect("the account in the folder pane");
    right_click_at(&ctx, &mut a, at);
    let shapes = settled(&ctx, &mut a);
    assert!(text_at(&shapes, "Stop the import").is_some() && text_at(&shapes, "Go on with the import").is_none());
    let stop = text_at(&shapes, "Stop the import").unwrap();
    click_at(&ctx, &mut a, stop, 1, SIZE);
    assert!(handed.borrow().iter().any(|j| matches!(j, Job::StopImport { .. })));
    // Stopped, with a journal that is not finished: go on.
    let store = a.session.store.clone().unwrap();
    let file = dir.join("Archive.pst");
    std::fs::write(&file, b"x").unwrap();
    let journal = ImportJournal { path: file.to_string_lossy().into_owned(), messages: 10, total: Some(3000), ..Default::default() };
    store.save_account_doc(&id, IMPORT_DOC, &journal).unwrap();
    a.session.apply(MailEvent::Imported {
        account: id.clone(),
        messages: 10,
        skipped: 0,
        total: Some(3000),
        folder: String::new(),
        stopped: true,
        done: true,
        error: None,
    });
    let shapes = settled(&ctx, &mut a);
    let at = text_at(&shapes, "Archive.pst").expect("the account in the folder pane");
    right_click_at(&ctx, &mut a, at);
    let shapes = settled(&ctx, &mut a);
    assert!(text_at(&shapes, "Stop the import").is_none() && text_at(&shapes, "Go on with the import").is_some());
    handed.borrow_mut().clear();
    let on = text_at(&shapes, "Go on with the import").unwrap();
    click_at(&ctx, &mut a, on, 1, SIZE);
    assert!(handed.borrow().iter().any(|j| matches!(j, Job::ResumeImport { account, allow_changed: false, .. } if *account == id)));
    assert!(a.session.import_of(&id).is_some());
    let _ = std::fs::remove_dir_all(dir);
}

/// A file that changed since the import began is asked about: Yes goes on bringing in what is
/// new, No leaves the account as it is.
#[test]
fn a_changed_file_is_asked_about() {
    let (mut a, dir, id, handed) = importing("import-changed");
    let ctx = egui::Context::default();
    let store = a.session.store.clone().unwrap();
    let journal = ImportJournal { path: "C:/Mail/Archive.pst".into(), messages: 10, total: Some(3000), ..Default::default() };
    store.save_account_doc(&id, IMPORT_DOC, &journal).unwrap();
    a.session.apply(MailEvent::Imported {
        account: id.clone(),
        messages: 10,
        skipped: 0,
        total: None,
        folder: String::new(),
        stopped: false,
        done: true,
        error: Some(IMPORT_FILE_CHANGED.into()),
    });
    a.logic(&ctx);
    frame(&ctx, &mut a, vec![]);
    assert!(matches!(&a.ui.dialog, Some(Dialog::ImportChanged { account }) if *account == id));
    let shapes = settled(&ctx, &mut a);
    assert!(settled_text_at(&ctx, &mut a, "The file has changed", SIZE).is_some());
    let no = settled_text_at(&ctx, &mut a, "No", SIZE).expect("the No button");
    click_at(&ctx, &mut a, no, 1, SIZE);
    assert!(a.ui.dialog.is_none() && handed.borrow().iter().all(|j| !matches!(j, Job::ResumeImport { .. })));
    assert!(a.session.account(&id).is_some() && a.session.unfinished_import(&id).is_some());
    // Asked again (the user clicked Go on), and Yes goes on.
    a.ui.dialog = Some(Dialog::ImportChanged { account: id.clone() });
    let yes = settled_text_at(&ctx, &mut a, "Yes", SIZE).expect("the Yes button");
    click_at(&ctx, &mut a, yes, 1, SIZE);
    assert!(handed.borrow().iter().any(|j| matches!(j, Job::ResumeImport { allow_changed: true, .. })));
    let _ = std::fs::remove_dir_all(dir);
}
