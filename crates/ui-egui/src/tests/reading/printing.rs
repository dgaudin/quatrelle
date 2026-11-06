// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Printing the message read (R26): Ctrl+P, or ⋯ › Print, hands its PDF to the system's print
//! window, nothing saved first; where there's no print window, or it fails, the PDF is saved and
//! opened in the viewer, as before, and the status bar says why. A message being written keeps
//! Ctrl+P as it was.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use super::*;
use crate::PrintOutcome;

/// What the window asked of the desktop, shared with the test.
#[derive(Default)]
struct Asked {
    /// Documents handed to the print window: their title and bytes.
    printed: Vec<(String, Vec<u8>)>,
    /// How the print window answers, once the test says.
    answer: Option<PrintOutcome>,
    /// Names suggested by Save dialogs, and the answer waiting.
    saves: Vec<String>,
    saved: Option<Vec<PathBuf>>,
    /// Files opened in the viewer.
    opened: Vec<PathBuf>,
}

/// A desktop with a print window (`print`) or without, whose Save dialog answers in `dir`.
struct Printer {
    print: bool,
    dir: PathBuf,
    asked: Rc<RefCell<Asked>>,
}

impl crate::Services for Printer {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn print(&mut self, title: &str, pdf: Vec<u8>) -> bool {
        if self.print {
            self.asked.borrow_mut().printed.push((title.to_string(), pdf));
        }
        self.print
    }
    fn take_printed(&mut self) -> Option<PrintOutcome> {
        self.asked.borrow_mut().answer.take()
    }
    fn ask_file(&mut self, ask: crate::FileAsk) -> bool {
        let crate::FileAsk::Save(name) = ask else { return false };
        let mut asked = self.asked.borrow_mut();
        asked.saved = Some(vec![self.dir.join(&name)]);
        asked.saves.push(name);
        true
    }
    fn take_file_answer(&mut self) -> Option<Vec<PathBuf>> {
        self.asked.borrow_mut().saved.take()
    }
    fn open_path(&mut self, path: &std::path::Path) -> Result<(), String> {
        self.asked.borrow_mut().opened.push(path.to_path_buf());
        Ok(())
    }
}

/// The sample with its first message read, over a desktop with a print window or without.
fn reading(name: &str, print: bool) -> (MailApp, PathBuf, Rc<RefCell<Asked>>) {
    let (mut a, dir) = app(name);
    let asked = Rc::new(RefCell::new(Asked::default()));
    let saves = dir.join("saved");
    std::fs::create_dir_all(&saves).unwrap();
    a.services = Box::new(Printer { print, dir: saves, asked: asked.clone() });
    let k = a.session.list[0].key.clone();
    a.run("list.open", json!({"key": k.to_id()}));
    (a, dir, asked)
}

fn ctrl_p(ctx: &egui::Context, a: &mut MailApp) {
    frame(ctx, a, vec![key(Key::P, Modifiers::COMMAND)]);
    frame(ctx, a, vec![]);
}

fn last_notice(a: &MailApp) -> Option<(String, bool)> {
    a.session.notices.last().map(|n| (n.text.clone(), n.error))
}

/// Ctrl+P on the message read: its PDF (with its header, as Save as PDF writes it) goes to the
/// print window, nothing asked about saving; once printed, the status bar says so.
#[test]
fn ctrl_p_hands_the_message_read_to_the_print_window() {
    let (mut a, dir, asked) = reading("print-ctrl-p", true);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    ctrl_p(&ctx, &mut a);
    {
        let asked = asked.borrow();
        assert_eq!(asked.printed.len(), 1, "one document to print");
        let (title, pdf) = &asked.printed[0];
        assert!(pdf.len() > 100 && pdf.starts_with(b"%PDF"), "a PDF: {:?}", &pdf[..pdf.len().min(8)]);
        assert!(!title.trim().is_empty());
        assert!(asked.saves.is_empty() && asked.opened.is_empty(), "nothing saved, nothing opened");
    }
    // Pressed again while the print window is open: nothing more.
    ctrl_p(&ctx, &mut a);
    assert_eq!(asked.borrow().printed.len(), 1);
    asked.borrow_mut().answer = Some(PrintOutcome::Sent);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(last_notice(&a), Some(("Sent to the printer".to_string(), false)));
    // Done: Ctrl+P prints again.
    ctrl_p(&ctx, &mut a);
    assert_eq!(asked.borrow().printed.len(), 2);
    let _ = std::fs::remove_dir_all(dir);
}

/// ⋯ › Print in the reading pane's header does as Ctrl+P; Cancel in the print window says
/// nothing and saves nothing.
#[test]
fn more_actions_print_asks_the_print_window_and_cancel_says_nothing() {
    let (mut a, dir, asked) = reading("print-more", true);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let before: Vec<egui::Rect> = nodes(&ctx, &mut a).into_iter().filter(|(l, _)| l == "Print").map(|(_, r)| r).collect();
    let more = nodes(&ctx, &mut a).into_iter().find(|(l, _)| l == "More actions").map(|(_, r)| r.center()).expect("⋯");
    press(&ctx, &mut a, more);
    let item = nodes(&ctx, &mut a).into_iter().find(|(l, r)| l == "Print" && !before.contains(r)).map(|(_, r)| r.center()).expect("the menu's Print");
    press(&ctx, &mut a, item);
    assert_eq!(asked.borrow().printed.len(), 1, "to the print window");
    assert!(asked.borrow().printed[0].1.starts_with(b"%PDF"));
    let notices = a.session.notices.len();
    asked.borrow_mut().answer = Some(PrintOutcome::Cancelled);
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(a.session.notices.len(), notices, "Cancel says nothing: {:?}", last_notice(&a));
    assert!(asked.borrow().saves.is_empty() && asked.borrow().opened.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// A message being written keeps Ctrl+P as it was: nothing printed (nor asked to save), the
/// draft as it was; and nor does it print from the Calendar.
#[test]
fn ctrl_p_in_a_draft_or_elsewhere_prints_nothing() {
    let (mut a, dir, asked) = reading("print-draft", true);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.reply", json!({}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let body = a.session.compose.as_ref().map(|c| c.body.clone()).expect("a reply being written");
    ctrl_p(&ctx, &mut a);
    assert!(asked.borrow().printed.is_empty() && asked.borrow().saves.is_empty(), "nothing printed from a draft");
    assert_eq!(a.session.compose.as_ref().map(|c| c.body.clone()), Some(body), "the draft as it was");
    a.run("compose.close", json!({"save": false}));
    a.run("view.module", json!({"name": "calendar"}));
    frame(&ctx, &mut a, vec![]);
    ctrl_p(&ctx, &mut a);
    assert!(asked.borrow().printed.is_empty() && asked.borrow().saves.is_empty(), "nothing printed from the Calendar");
    let _ = std::fs::remove_dir_all(dir);
}

/// Where there's no print window (another desktop, Windows, macOS), Ctrl+P does what Print did
/// before: the PDF saved where asked and opened in the viewer, and the status bar says why.
#[test]
fn without_a_print_window_the_pdf_opens_in_the_viewer() {
    let (mut a, dir, asked) = reading("print-none", false);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    ctrl_p(&ctx, &mut a);
    fallen_back(&a, &asked);
    let _ = std::fs::remove_dir_all(dir);
}

/// A print window that fails (the portal there but answering with an error) falls back the
/// same way, on the message it was asked for.
#[test]
fn a_print_window_that_fails_falls_back_on_the_viewer() {
    let (mut a, dir, asked) = reading("print-failed", true);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    ctrl_p(&ctx, &mut a);
    assert_eq!(asked.borrow().printed.len(), 1);
    assert!(asked.borrow().saves.is_empty());
    asked.borrow_mut().answer = Some(PrintOutcome::Failed("org.freedesktop.DBus.Error.ServiceUnknown".into()));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    fallen_back(&a, &asked);
    let _ = std::fs::remove_dir_all(dir);
}

/// The PDF saved where the Save dialog said, opened in the viewer, and why said.
fn fallen_back(a: &MailApp, asked: &Rc<RefCell<Asked>>) {
    let asked = asked.borrow();
    assert_eq!(asked.saves.len(), 1, "asked where to save");
    assert!(asked.saves[0].ends_with(".pdf"), "{:?}", asked.saves);
    assert_eq!(asked.opened.len(), 1, "opened in the viewer");
    let pdf = std::fs::read(&asked.opened[0]).expect("the PDF saved");
    assert!(pdf.starts_with(b"%PDF"));
    assert_eq!(last_notice(a), Some(("The system's print window isn't available: opened as a PDF, print it from the viewer".to_string(), false)));
}

/// What a screen reader is told of: each node's label and where it is.
fn nodes(ctx: &egui::Context, a: &mut MailApp) -> Vec<(String, egui::Rect)> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let tree = out.platform_output.accesskit_update.expect("AccessKit is on");
    tree.nodes
        .iter()
        .filter_map(|(_, n)| {
            let b = n.bounds()?;
            let label = n.label().or(n.value()).unwrap_or_default().to_string();
            Some((label, egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32))))
        })
        .collect()
}

fn press(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2) {
    let b = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(ctx, a, vec![Event::PointerMoved(at)]);
    frame(ctx, a, vec![b(true)]);
    frame(ctx, a, vec![b(false)]);
    frame(ctx, a, vec![]);
}
