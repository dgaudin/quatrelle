//! Quatrelle's icon in the notification area: closing the window leaves Quatrelle running there,
//! the icon brings the window back, and its Exit quits as File › Exit does.

use std::cell::RefCell;
use std::rc::Rc;

use egui::ViewportCommand;

use super::*;
use crate::{Action, TrayAction, TrayIcon};

/// What the fake notification area was shown, and what's done with its icon next.
#[derive(Default)]
struct Area {
    /// The desktop has a notification area.
    hosted: bool,
    shown: Vec<TrayIcon>,
    next: Option<TrayAction>,
}

struct Hosting(Rc<RefCell<Area>>);

impl crate::Services for Hosting {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn pick_files(&mut self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
    fn save_path(&mut self, _: &str) -> Option<std::path::PathBuf> {
        None
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Err("refused".into())
    }
    fn show_tray(&mut self, icon: &TrayIcon) {
        self.0.borrow_mut().shown.push(icon.clone());
    }
    fn take_tray_action(&mut self) -> Option<TrayAction> {
        self.0.borrow_mut().next.take()
    }
    fn tray_shown(&self) -> bool {
        let area = self.0.borrow();
        area.hosted && area.shown.last().is_some_and(|i| i.shown)
    }
}

fn hosting(a: &mut MailApp, hosted: bool) -> Rc<RefCell<Area>> {
    let area = Rc::new(RefCell::new(Area { hosted, ..Default::default() }));
    a.services = Box::new(Hosting(area.clone()));
    area
}

/// One frame, the window's close button clicked in it if `close`: what the main window was told.
fn pass(ctx: &egui::Context, a: &mut MailApp, close: bool) -> Vec<ViewportCommand> {
    let events = if close { vec![egui::ViewportEvent::Close] } else { Vec::new() };
    let input = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
        viewports: std::iter::once((egui::ViewportId::ROOT, egui::ViewportInfo { events, ..Default::default() })).collect(),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    out.viewport_output.remove(&egui::ViewportId::ROOT).map(|v| v.commands).unwrap_or_default()
}

/// The first frames: the typefaces put in place, then the window drawn.
fn start(ctx: &egui::Context, a: &mut MailApp) {
    pass(ctx, a, false);
    pass(ctx, a, false);
}

/// The icon is there from the start; closing the window puts it away, with nothing asked even
/// with a message being written (nothing is lost); the icon brings it back.
#[test]
fn closing_the_window_leaves_quatrelle_in_the_notification_area() {
    let (mut a, dir) = app("tray-close");
    let area = hosting(&mut a, true);
    let ctx = egui::Context::default();
    start(&ctx, &mut a);
    let first = area.borrow().shown.last().cloned().unwrap();
    assert!(first.shown && first.new == 0 && first.tip.is_empty(), "{first:?}");
    assert_eq!((first.open.as_str(), first.quit.as_str()), ("Open Quatrelle", "Exit"));

    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "Half done"}));
    let told = pass(&ctx, &mut a, true);
    assert!(told.contains(&ViewportCommand::CancelClose) && told.contains(&ViewportCommand::Visible(false)), "{told:?}");
    assert!(a.ui.dialog.is_none(), "nothing asked: {:?}", a.ui.dialog);
    assert_eq!(a.session.all_composes().count(), 1, "still being written");

    area.borrow_mut().next = Some(TrayAction::Open);
    let told = pass(&ctx, &mut a, false);
    assert!(told.contains(&ViewportCommand::Visible(true)) && told.contains(&ViewportCommand::Focus), "{told:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The icon's Exit, and File › Exit, quit: what's being written is asked about first, the window
/// put away coming back to ask.
#[test]
fn exit_quits_from_the_icon_and_from_the_file_page() {
    let (mut a, dir) = app("tray-exit");
    let area = hosting(&mut a, true);
    let ctx = egui::Context::default();
    start(&ctx, &mut a);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "Half done"}));
    pass(&ctx, &mut a, true);

    area.borrow_mut().next = Some(TrayAction::Quit);
    pass(&ctx, &mut a, false);
    let told = pass(&ctx, &mut a, false);
    assert!(told.contains(&ViewportCommand::Close), "{told:?}");
    // The close that follows asks, as any quit does.
    let told = pass(&ctx, &mut a, true);
    assert!(matches!(a.ui.dialog, Some(Dialog::SaveOnClose(1))), "asked: {:?}", a.ui.dialog);
    assert!(told.contains(&ViewportCommand::Visible(true)), "back to ask: {told:?}");

    // Cancelled: the next close of the window puts it away again.
    a.ui.dialog = None;
    pass(&ctx, &mut a, false);
    let told = pass(&ctx, &mut a, true);
    assert!(told.contains(&ViewportCommand::Visible(false)), "{told:?}");

    // File › Exit.
    a.act(Action::Quit);
    pass(&ctx, &mut a, false);
    pass(&ctx, &mut a, true);
    assert!(matches!(a.ui.dialog, Some(Dialog::SaveOnClose(1))), "asked: {:?}", a.ui.dialog);
    let _ = std::fs::remove_dir_all(dir);
}

/// Without a notification area, or with the option off, closing the window closes Quatrelle as
/// it always did; with the option off, the icon is there only for new mail.
#[test]
fn without_a_notification_area_closing_quits() {
    for (hosted, keep) in [(false, true), (true, false)] {
        let (mut a, dir) = app("tray-none");
        let area = hosting(&mut a, hosted);
        a.run("view.keepRunning", json!({"on": keep}));
        let ctx = egui::Context::default();
        start(&ctx, &mut a);
        if !keep {
            assert!(area.borrow().shown.iter().all(|i| !i.shown), "no icon without new mail");
        }
        let told = pass(&ctx, &mut a, true);
        assert!(!told.contains(&ViewportCommand::CancelClose) && !told.contains(&ViewportCommand::Visible(false)), "{hosted} {keep}: {told:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
