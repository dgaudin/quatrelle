// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the rows at the top of the window, measured: the top bar at the very top, the ribbon
//! just under it (its tabs, then its buttons), in each module and on the File page, at the
//! smallest, a usual and a very wide window. The heights are those of the window as it is, within
//! two points (the fonts differ a little between systems): what changes them has to change them
//! here too.
//!
//! And the top bar as the window's title bar, where the window has none of the system's: the
//! window's buttons at its corner, its free space that moves and maximizes the window, the edges
//! that resize it, over a dialog too, and none of it with the system's title bar.

use super::*;

const SIZES: [f32; 3] = [760.0, 1320.0, 2560.0];
const MODULES: [&str; 5] = ["mail", "calendar", "people", "tasks", "notes"];

/// The rows after a few frames (the ribbon fits itself to the window over the first ones):
/// the top bar's rectangle, the ribbon panel's, and its row of tabs'.
fn rows(a: &mut MailApp, ctx: &egui::Context, width: f32) -> (egui::Rect, Option<egui::Rect>, Option<egui::Rect>) {
    for _ in 0..6 {
        frame_sized(ctx, a, vec![], egui::vec2(width, 820.0));
    }
    (a.ui.top_bar_rect.expect("the top bar"), a.ui.ribbon_panel_rect, a.ui.ribbon_tabs_rect)
}

/// The top bar's height, the ribbon panel's (its margins and border, the row of tabs, the row of
/// buttons) and the row of tabs' (its top, under the panel's border and margin), in points, as
/// measured: the same in every module and at every width.
const BAR_H: f32 = 40.0;
const RIBBON_PANEL_H: f32 = 129.0;
const TABS_H: f32 = 26.0;
const TABS_TOP_IN_PANEL: f32 = 5.0;
/// How far a measure may be from the one written here: the fonts, and so the rows' text, can
/// differ a little from one system to another (the CI's Windows and macOS).
const SLACK: f32 = 2.0;

/// Whether each measure is within [`SLACK`] of the one wanted.
fn near<const N: usize>(got: [f32; N], want: [f32; N]) -> bool {
    got.iter().zip(want).all(|(g, w)| (g - w).abs() <= SLACK)
}

#[test]
fn the_rows_at_the_top_of_the_window_have_their_measured_heights() {
    let (mut a, dir) = app("title-bar-rows");
    let ctx = egui::Context::default();
    for module in MODULES {
        a.run("view.module", json!({"name": module}));
        for width in SIZES {
            let at = format!("{module} at {width} pt");
            let (bar, panel, tabs) = rows(&mut a, &ctx, width);
            // The bar is the top row: from the top edge, across the window.
            assert!(near([bar.min.x, bar.min.y, bar.max.x, bar.height()], [0.0, 0.0, width, BAR_H]), "the top bar, {at}: {bar:?}");
            // The ribbon just under it, to the window's right edge.
            let panel = panel.unwrap_or_else(|| panic!("the ribbon, {at}"));
            assert!(near([panel.min.y, panel.max.x, panel.height()], [BAR_H, width, RIBBON_PANEL_H]), "the ribbon panel, {at}: {panel:?}");
            let tabs = tabs.unwrap_or_else(|| panic!("the tabs, {at}"));
            assert!(near([tabs.min.y, tabs.height()], [BAR_H + TABS_TOP_IN_PANEL, TABS_H]), "the tabs, {at}: {tabs:?}");
        }
    }
    // The File page keeps the bar, and has no ribbon under it.
    a.run("view.module", json!({"name": "mail"}));
    a.act(crate::Action::Backstage(crate::backstage::Page::Info));
    for width in SIZES {
        let (bar, panel, tabs) = rows(&mut a, &ctx, width);
        assert!(near([bar.min.x, bar.min.y, bar.max.x, bar.height()], [0.0, 0.0, width, BAR_H]), "the top bar on the File page, {width} pt: {bar:?}");
        assert!(panel.is_none() && tabs.is_none(), "no ribbon on the File page, {width} pt");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// The labels of every node the Options page offers, after a few frames.
fn options_labels(a: &mut MailApp) -> Vec<String> {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    let mut labels = Vec::new();
    for _ in 0..4 {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 2400.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
        labels = nodes.iter().filter_map(|(_, n)| n.label().or_else(|| n.value()).map(str::to_string)).collect();
    }
    labels
}

#[test]
fn the_options_offer_the_system_title_bar_where_the_window_has_its_own() {
    let (mut a, dir) = app("title-bar-option");
    let labels = options_labels(&mut a);
    let offered = cfg!(any(windows, all(unix, not(target_os = "macos"))));
    assert_eq!(labels.iter().any(|l| l == "Use the system title bar"), offered, "the switch: {labels:?}");
    assert_eq!(labels.iter().any(|l| l == "Applies when Quatrelle starts again"), offered, "its note");
    a.run("view.systemTitleBar", json!({"on": true}));
    assert!(a.session.prefs.system_title_bar);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_window_has_no_own_title_bar_by_default() {
    let mut s = crate::NullServices(0);
    assert!(!crate::Services::own_title_bar(&s));
    crate::Services::keep_frame(&mut s);
    crate::Services::before_zoom(&mut s);
}

/// Services saying the window has no title bar of the system's, so Quatrelle's top bar is its title
/// bar; they count the times the window is told before a maximize or a restore.
struct OwnTitleBar(std::rc::Rc<std::cell::Cell<u32>>);

impl crate::Services for OwnTitleBar {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Err("not available".into())
    }
    fn own_title_bar(&self) -> bool {
        true
    }
    fn before_zoom(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

/// The sample mailbox in a window drawing its own title bar, and the count of the times the window
/// was told before a zoom.
fn own_bar_app(name: &str) -> (MailApp, std::path::PathBuf, std::rc::Rc<std::cell::Cell<u32>>) {
    let (a, dir) = app(name);
    let told = std::rc::Rc::new(std::cell::Cell::new(0));
    (MailApp::new(a.session, Box::new(NullBackend), Box::new(OwnTitleBar(told.clone()))), dir, told)
}

/// A window drawn frame after frame, a 60th of a second apart, AccessKit on; maximized or not, as
/// the system would say.
struct Window {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
    maximized: bool,
}

impl Window {
    fn new(size: egui::Vec2) -> Window {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        Window { ctx, size, time: 0.0, maximized: false }
    }

    /// A frame with `events`, the system telling the window `told` (a close asked for).
    fn frame(&mut self, a: &mut MailApp, events: Vec<Event>, told: Vec<egui::ViewportEvent>) -> egui::FullOutput {
        self.time += 1.0 / 60.0;
        let viewport = egui::ViewportInfo { maximized: Some(self.maximized), events: told, ..Default::default() };
        let input = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            events,
            time: Some(self.time),
            viewports: std::iter::once((egui::ViewportId::ROOT, viewport)).collect(),
            ..Default::default()
        };
        let mut out = self.ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        out
    }

    fn run(&mut self, a: &mut MailApp, events: Vec<Event>) -> egui::FullOutput {
        self.frame(a, events, Vec::new())
    }

    /// A few frames for the window to settle; the last one.
    fn settle(&mut self, a: &mut MailApp) -> egui::FullOutput {
        for _ in 0..4 {
            self.run(a, vec![]);
        }
        self.run(a, vec![])
    }

    /// The commands sent to the window by `times` clicks in a row at `at`, after a pause (so they
    /// aren't counted with the clicks before).
    fn click(&mut self, a: &mut MailApp, at: egui::Pos2, times: usize) -> Vec<egui::ViewportCommand> {
        self.time += 1.0;
        let mut cmds = sent(&self.run(a, vec![Event::PointerMoved(at)]));
        for _ in 0..times {
            cmds.extend(sent(&self.run(a, vec![button(at, true), button(at, false)])));
        }
        cmds.extend(sent(&self.run(a, vec![])));
        cmds
    }

    /// The commands sent by a press at `at`, a move away and the release there.
    fn drag(&mut self, a: &mut MailApp, at: egui::Pos2) -> Vec<egui::ViewportCommand> {
        self.time += 1.0;
        let to = at + egui::vec2(40.0, 30.0);
        let mut cmds = Vec::new();
        for events in [
            vec![Event::PointerMoved(at), button(at, true)],
            vec![Event::PointerMoved(at + egui::vec2(20.0, 15.0))],
            vec![Event::PointerMoved(to)],
            vec![button(to, false)],
            vec![],
        ] {
            cmds.extend(sent(&self.run(a, events)));
        }
        cmds
    }

    /// The commands sent by a press at `at` and its release, without a move.
    fn press(&mut self, a: &mut MailApp, at: egui::Pos2) -> Vec<egui::ViewportCommand> {
        self.time += 1.0;
        let mut cmds = sent(&self.run(a, vec![Event::PointerMoved(at), button(at, true)]));
        cmds.extend(sent(&self.run(a, vec![button(at, false)])));
        cmds
    }
}

fn button(at: egui::Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

/// What the frame told the window to do.
fn sent(out: &egui::FullOutput) -> Vec<egui::ViewportCommand> {
    out.viewport_output.values().flat_map(|v| v.commands.clone()).collect()
}

/// A node a screen reader is told of: its name, where it is, whether Tab reaches it.
struct Named {
    label: String,
    rect: egui::Rect,
    focusable: bool,
    role: egui::accesskit::Role,
}

fn named(out: &egui::FullOutput) -> Vec<Named> {
    let Some(tree) = out.platform_output.accesskit_update.as_ref() else { return Vec::new() };
    tree.nodes
        .iter()
        .filter_map(|(_, n)| {
            let b = n.bounds()?;
            Some(Named {
                label: n.label().unwrap_or_default().to_string(),
                rect: egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)),
                focusable: n.supports_action(egui::accesskit::Action::Focus),
                role: n.role(),
            })
        })
        .collect()
}

/// The node named `label` on the top bar (a window or pane below may have its own Close).
fn in_bar<'a>(seen: &'a [Named], label: &str) -> Option<&'a Named> {
    seen.iter().find(|n| n.label == label && n.rect.bottom() <= BAR_H + 0.5)
}

fn bar_rect(seen: &[Named], label: &str) -> egui::Rect {
    in_bar(seen, label).map(|n| n.rect).unwrap_or_else(|| panic!("{label} on the bar: {:?}", seen.iter().map(|n| &n.label).collect::<Vec<_>>()))
}

/// The window's buttons, as a screen reader is told of them.
const CAPTIONS: [&str; 4] = ["Minimize", "Maximize", "Restore", "Close"];

/// The mark at the bar's start (Quatrelle's 20-point tile, in the accent colour), where drawn.
fn mark(out: &egui::FullOutput, accent: egui::Color32) -> Option<egui::Rect> {
    fn flat<'a>(s: &'a egui::Shape, found: &mut Vec<&'a egui::epaint::RectShape>) {
        match s {
            egui::Shape::Vec(v) => v.iter().for_each(|s| flat(s, found)),
            egui::Shape::Rect(r) => found.push(r),
            _ => {}
        }
    }
    let mut rects = Vec::new();
    out.shapes.iter().for_each(|c| flat(&c.shape, &mut rects));
    rects
        .into_iter()
        .find(|r| r.fill == accent && (r.rect.width() - 20.0).abs() < 0.5 && (r.rect.height() - 20.0).abs() < 0.5 && r.rect.bottom() <= BAR_H)
        .map(|r| r.rect)
}

/// The bar's free space: between the search box (its Advanced search button, at its end) and
/// Accounts and settings.
fn free_space(seen: &[Named]) -> egui::Pos2 {
    let (search_end, settings) = (bar_rect(seen, "Advanced search"), bar_rect(seen, "Accounts and settings"));
    assert!(settings.left() - search_end.right() > 30.0, "room between the search box and the settings: {search_end:?} {settings:?}");
    egui::pos2((search_end.right() + settings.left()) / 2.0, BAR_H / 2.0)
}

fn moves(cmds: &[egui::ViewportCommand]) -> bool {
    cmds.iter().any(|c| matches!(c, egui::ViewportCommand::StartDrag))
}

fn zooms(cmds: &[egui::ViewportCommand]) -> bool {
    cmds.iter().any(|c| matches!(c, egui::ViewportCommand::Maximized(_)))
}

fn resizes(cmds: &[egui::ViewportCommand]) -> bool {
    cmds.iter().any(|c| matches!(c, egui::ViewportCommand::BeginResize(_)))
}

/// The window's edges and corners, each with the way it resizes the window.
fn edges(size: egui::Vec2) -> [(egui::Pos2, egui::ResizeDirection); 8] {
    use egui::ResizeDirection::*;
    let (w, h) = (size.x, size.y);
    [
        (egui::pos2(w / 2.0, 2.0), North),
        (egui::pos2(w / 2.0, h - 2.0), South),
        (egui::pos2(2.0, h / 2.0), West),
        (egui::pos2(w - 2.0, h / 2.0), East),
        (egui::pos2(2.0, 2.0), NorthWest),
        (egui::pos2(w - 2.0, 2.0), NorthEast),
        (egui::pos2(2.0, h - 2.0), SouthWest),
        (egui::pos2(w - 2.0, h - 2.0), SouthEast),
    ]
}

/// The window's own buttons, as the system's: 46 × 40 each at the window's top right corner,
/// named to a screen reader, out of the Tab order, clear of the bar's own controls, each doing
/// what the system's does. Close asks about a message being written first, as the system's Close
/// does, and Maximize is Restore while the window is maximized. The File page keeps them.
#[test]
fn the_window_buttons_sit_in_the_corner_and_do_what_the_systems_do() {
    let (mut a, dir, _) = own_bar_app("own-title-bar-buttons");
    let size = egui::vec2(1280.0, 800.0);
    let mut w = Window::new(size);
    let seen = named(&w.settle(&mut a));
    let (min, max, close) = (bar_rect(&seen, "Minimize"), bar_rect(&seen, "Maximize"), bar_rect(&seen, "Close"));
    for (r, name) in [(min, "Minimize"), (max, "Maximize"), (close, "Close")] {
        assert!((r.width() - 46.0).abs() < 0.5 && (r.height() - BAR_H).abs() < 0.5 && r.top().abs() < 0.5, "{name}: {r:?}");
    }
    assert!((close.right() - size.x).abs() < 0.5, "flush with the window's corner: {close:?}");
    assert!((min.right() - max.left()).abs() < 0.5 && (max.right() - close.left()).abs() < 0.5, "side by side: {min:?} {max:?} {close:?}");
    assert!(seen.iter().filter(|n| CAPTIONS.contains(&n.label.as_str())).all(|n| !n.focusable), "out of the Tab order");
    for control in ["Accounts and settings", "Switch to dark mode", "Advanced search"] {
        assert!(bar_rect(&seen, control).right() <= min.left() + 0.5, "{control} keeps clear of the window's buttons");
    }
    // Tab goes through the window's controls, never to its buttons.
    for _ in 0..6 {
        let out = w.run(&mut a, vec![key(Key::Tab, Modifiers::NONE)]);
        let focus = out.platform_output.accesskit_update.as_ref().and_then(|t| t.nodes.iter().find(|(id, _)| *id == t.focus));
        let focus = focus.and_then(|(_, n)| n.label().map(str::to_string)).unwrap_or_default();
        assert!(!CAPTIONS.contains(&focus.as_str()), "the focus on {focus}");
    }
    // Each does what the system's does.
    let cmds = w.click(&mut a, min.center(), 1);
    assert!(cmds.contains(&egui::ViewportCommand::Minimized(true)), "Minimize: {cmds:?}");
    let cmds = w.click(&mut a, max.center(), 1);
    assert!(cmds.contains(&egui::ViewportCommand::Maximized(true)), "Maximize: {cmds:?}");
    // Maximized: Restore in its place, which restores; the window's corner is Close's.
    w.maximized = true;
    let seen = named(&w.settle(&mut a));
    assert!(in_bar(&seen, "Maximize").is_none(), "Restore, not Maximize");
    assert_eq!(bar_rect(&seen, "Restore"), max);
    let cmds = w.click(&mut a, max.center(), 1);
    assert!(cmds.contains(&egui::ViewportCommand::Maximized(false)), "Restore: {cmds:?}");
    let cmds = w.click(&mut a, egui::pos2(size.x - 1.0, 1.0), 1);
    assert!(cmds.contains(&egui::ViewportCommand::Close), "the corner of a maximized window closes it: {cmds:?}");
    w.maximized = false;
    // Close, with a message being written: the window is asked to close as by the system's
    // button, and Quatrelle asks first.
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "Half done"}));
    w.settle(&mut a);
    let cmds = w.click(&mut a, close.center(), 1);
    assert!(cmds.contains(&egui::ViewportCommand::Close), "Close: {cmds:?}");
    let out = w.frame(&mut a, vec![], vec![egui::ViewportEvent::Close]);
    assert!(matches!(a.ui.dialog, Some(Dialog::SaveOnClose(1))), "asked about the message first");
    assert!(sent(&out).contains(&egui::ViewportCommand::CancelClose), "and the window stays meanwhile");
    a.ui.dialog = None;
    a.run("compose.discard", json!({}));
    // The File page keeps them.
    a.run("ui.file", json!({}));
    let seen = named(&w.settle(&mut a));
    assert_eq!(bar_rect(&seen, "Close"), close, "the File page's title bar");
    assert_eq!(bar_rect(&seen, "Minimize"), min);
    let _ = std::fs::remove_dir_all(dir);
}

/// The bar's free space moves the window (a press and a move) and maximizes it (a double-click);
/// its controls keep their clicks and move nothing: the search box, the bar's buttons, the
/// window's buttons and the mark.
#[test]
fn the_bars_free_space_moves_and_maximizes_the_window_and_its_controls_dont() {
    let (mut a, dir, _) = own_bar_app("own-title-bar-drag");
    let mut w = Window::new(egui::vec2(1280.0, 800.0));
    let out = w.settle(&mut a);
    let seen = named(&out);
    let free = free_space(&seen);
    let cmds = w.click(&mut a, free, 2);
    assert!(cmds.contains(&egui::ViewportCommand::Maximized(true)), "a double-click maximizes: {cmds:?}");
    let cmds = w.drag(&mut a, free);
    assert!(moves(&cmds), "a press and a move drag the window: {cmds:?}");
    let search = seen.iter().find(|n| n.role == egui::accesskit::Role::TextInput && n.rect.bottom() <= BAR_H).map(|n| n.rect.center());
    let search = search.unwrap_or_else(|| panic!("the search box"));
    let mark = mark(&out, a.ui.tokens.accent).unwrap_or_else(|| panic!("the mark")).center();
    let controls = [
        ("the search box", search),
        ("Send / Receive", bar_rect(&seen, "Send / Receive all folders (F9)").center()),
        ("Accounts and settings", bar_rect(&seen, "Accounts and settings").center()),
        ("the mark", mark),
        ("Minimize", bar_rect(&seen, "Minimize").center()),
    ];
    for (what, at) in controls {
        let cmds = w.drag(&mut a, at);
        assert!(!moves(&cmds), "a press and a move on {what} don't drag the window: {cmds:?}");
        let cmds = w.click(&mut a, at, 2);
        assert!(!zooms(&cmds), "a double-click on {what} doesn't maximize it: {cmds:?}");
        a.ui.backstage = None;
        a.ui.dialog = None;
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// The window's edges and corners resize it, each the way it faces, over whatever is drawn
/// there; not while it's maximized.
#[test]
fn the_windows_edges_resize_it_unless_it_is_maximized() {
    let (mut a, dir, _) = own_bar_app("own-title-bar-resize");
    let size = egui::vec2(1280.0, 800.0);
    let mut w = Window::new(size);
    w.settle(&mut a);
    for (at, way) in edges(size) {
        let cmds = w.press(&mut a, at);
        let ways: Vec<_> = cmds.iter().filter_map(|c| if let egui::ViewportCommand::BeginResize(d) = c { Some(*d) } else { None }).collect();
        assert_eq!(ways, [way], "at {at:?}: {cmds:?}");
        assert!(!moves(&cmds) && !cmds.contains(&egui::ViewportCommand::Close), "at {at:?}, only a resize: {cmds:?}");
    }
    w.maximized = true;
    w.settle(&mut a);
    for (at, _) in edges(size) {
        let cmds = w.press(&mut a, at);
        assert!(!resizes(&cmds), "maximized, at {at:?}: {cmds:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// A dialog holding the window (Add account): nothing under it takes a click, so the bar's drag
/// and the window's buttons are laid again above it. A screen reader is told of one Minimize,
/// Maximize and Close, and Tab stays in the dialog. In a window so short that the dialog reaches
/// into the bar, the dialog keeps its clicks there.
#[test]
fn over_a_dialog_the_title_bar_still_moves_and_closes_the_window() {
    let (mut a, dir, _) = own_bar_app("own-title-bar-dialog");
    let size = egui::vec2(1280.0, 800.0);
    let mut w = Window::new(size);
    let seen = named(&w.settle(&mut a));
    let (min, close, free) = (bar_rect(&seen, "Minimize"), bar_rect(&seen, "Close"), free_space(&seen));
    a.ui.dialog = Some(Dialog::add_account());
    let seen = named(&w.settle(&mut a));
    for name in ["Minimize", "Maximize", "Close"] {
        let count = seen.iter().filter(|n| n.label == name && n.rect.bottom() <= BAR_H + 0.5).count();
        assert_eq!(count, 1, "{name} told once to a screen reader");
    }
    let cmds = w.click(&mut a, min.center(), 1);
    assert!(cmds.contains(&egui::ViewportCommand::Minimized(true)), "minimized over the dialog: {cmds:?}");
    let cmds = w.click(&mut a, close.center(), 1);
    assert!(cmds.contains(&egui::ViewportCommand::Close), "closed over the dialog: {cmds:?}");
    let cmds = w.drag(&mut a, free);
    assert!(moves(&cmds), "moved over the dialog: {cmds:?}");
    let cmds = w.click(&mut a, free, 2);
    assert!(zooms(&cmds), "maximized over the dialog: {cmds:?}");
    assert!(matches!(a.ui.dialog, Some(Dialog::AddAccount(_))), "the dialog stays");
    for _ in 0..8 {
        let out = w.run(&mut a, vec![key(Key::Tab, Modifiers::NONE)]);
        let focus = out.platform_output.accesskit_update.as_ref().and_then(|t| t.nodes.iter().find(|(id, _)| *id == t.focus));
        let focus = focus.and_then(|(_, n)| n.label().map(str::to_string)).unwrap_or_default();
        assert!(!CAPTIONS.contains(&focus.as_str()), "the focus on {focus}");
        assert!(sent(&out).iter().all(|c| !matches!(c, egui::ViewportCommand::Minimized(_) | egui::ViewportCommand::Close)));
    }
    // A window so short the dialog reaches into the bar: the bar isn't laid over the dialog, whose
    // place there stays its own.
    let short = egui::vec2(1280.0, 150.0);
    w.size = short;
    a.ui.dialog = Some(Dialog::SaveOnClose(1));
    w.settle(&mut a);
    let modal = w.ctx.memory(|m| m.top_modal_layer()).and_then(|l| w.ctx.memory(|m| m.area_rect(l.id)));
    assert!(modal.is_some_and(|r| r.top() < BAR_H), "the dialog reaches into the bar: {modal:?}");
    let cmds = w.click(&mut a, egui::pos2(short.x - 23.0, BAR_H / 2.0), 1);
    assert!(!cmds.contains(&egui::ViewportCommand::Close), "{cmds:?}");
    a.ui.dialog = None;
    let _ = std::fs::remove_dir_all(dir);
}

/// In Arabic (right to left), the mark and the window's buttons keep apart, and the bar's
/// controls stay in the room the buttons leave; on the File page too.
#[test]
fn in_arabic_the_mark_and_the_window_buttons_keep_apart() {
    let (mut a, dir, _) = own_bar_app("own-title-bar-arabic");
    a.ui.language = "ar".into();
    let size = egui::vec2(1280.0, 800.0);
    let mut w = Window::new(size);
    for file_page in [false, true] {
        if file_page {
            a.run("ui.file", json!({}));
        }
        let out = w.settle(&mut a);
        let seen = named(&out);
        let buttons: Vec<egui::Rect> = seen
            .iter()
            .filter(|n| n.rect.top().abs() < 0.5 && (n.rect.width() - 46.0).abs() < 0.5 && (n.rect.height() - BAR_H).abs() < 0.5)
            .map(|n| n.rect)
            .collect();
        assert_eq!(buttons.len(), 3, "the window's three buttons (File page: {file_page})");
        let start = buttons.iter().map(|b| b.left()).fold(f32::INFINITY, f32::min);
        let mark = mark(&out, a.ui.tokens.accent).unwrap_or_else(|| panic!("the mark (File page: {file_page})"));
        assert!(mark.right() <= start, "the mark clear of the buttons: {mark:?} {buttons:?}");
        for n in seen.iter().filter(|n| n.rect.bottom() <= BAR_H + 0.5 && !buttons.contains(&n.rect) && n.rect.width() < size.x / 2.0) {
            assert!(n.rect.right() <= start + 0.5, "{} clear of the buttons: {:?}", n.label, n.rect);
        }
    }
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let _ = std::fs::remove_dir_all(dir);
}

/// The search box's middle, from where to look (its menu at its start) to Advanced search at its
/// end.
fn search_middle(seen: &[Named]) -> f32 {
    let scope = seen.iter().find(|n| n.label.ends_with('▾') && n.rect.bottom() <= BAR_H).map(|n| n.rect);
    let scope = scope.unwrap_or_else(|| panic!("the search box's menu"));
    (scope.left() + bar_rect(seen, "Advanced search").right()) / 2.0
}

/// The search box stays where the system's title bar has it, in the middle of the window, not in
/// the middle of what the window's buttons leave of the bar. In the smallest window, in languages
/// with long words and with Undo on the bar, it gives way rather than push the bar's controls
/// into the window's buttons (it did by six points in Russian).
#[test]
fn the_search_box_stays_in_the_middle_of_the_window_and_clear_of_the_controls() {
    let (mut own, dir, _) = own_bar_app("own-title-bar-search");
    let (mut system, dir2) = app("system-title-bar-search");
    for width in [1000.0, 1280.0, 2560.0] {
        let size = egui::vec2(width, 800.0);
        let (mine, theirs) = (named(&Window::new(size).settle(&mut own)), named(&Window::new(size).settle(&mut system)));
        let (mine, theirs) = (search_middle(&mine), search_middle(&theirs));
        assert!((mine - theirs).abs() < 1.0, "{width} pt: the box's middle at {mine}, {theirs} under the system's title bar");
    }
    let k = own.session.list[0].key.clone();
    own.run("list.open", json!({"key": k.to_id()}));
    own.run("mail.flag", json!({}));
    assert!(!own.session.undo.is_empty(), "Undo on the bar");
    for lang in ["en", "fr", "de", "ru"] {
        own.ui.language = lang.into();
        let seen = named(&Window::new(egui::vec2(760.0, 800.0)).settle(&mut own));
        let controls: Vec<&Named> = seen.iter().filter(|n| n.rect.bottom() <= BAR_H + 0.5 && !n.label.is_empty() && n.rect.width() < 380.0).collect();
        assert!(controls.len() >= 9, "{lang}: {:?}", controls.iter().map(|n| &n.label).collect::<Vec<_>>());
        for (i, a) in controls.iter().enumerate() {
            for b in &controls[i + 1..] {
                let overlap = a.rect.intersect(b.rect);
                assert!(overlap.width() <= 0.5 || overlap.height() <= 0.5, "{lang}: {} over {}: {:?} {:?}", a.label, b.label, a.rect, b.rect);
            }
        }
    }
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
}

/// Quatrelle's Maximize, Restore and double-click on the bar tell the window first, in the frame
/// that asks for them (Windows animates them so); Minimize doesn't, and each tells it once.
#[test]
fn the_window_is_told_before_its_own_title_bar_zooms_it() {
    let (mut a, dir, told) = own_bar_app("own-title-bar-zoom");
    let mut w = Window::new(egui::vec2(1280.0, 800.0));
    let seen = named(&w.settle(&mut a));
    let (min, max, free) = (bar_rect(&seen, "Minimize"), bar_rect(&seen, "Maximize"), free_space(&seen));
    w.click(&mut a, min.center(), 1);
    assert_eq!(told.get(), 0, "Minimize says nothing");
    w.click(&mut a, max.center(), 1);
    assert_eq!(told.get(), 1, "Maximize says so, once");
    w.click(&mut a, free, 2);
    assert_eq!(told.get(), 2, "so does a double-click on the bar");
    w.maximized = true;
    w.settle(&mut a);
    w.click(&mut a, max.center(), 1);
    assert_eq!(told.get(), 3, "so does Restore");
    let _ = std::fs::remove_dir_all(dir);
}

/// The window's own title bar keeps the rows where they were: the bar at the top, 40 points, and
/// the ribbon's tabs under it, on the File page too.
#[test]
fn the_rows_stay_where_they_were_under_the_windows_own_title_bar() {
    let (mut a, dir, _) = own_bar_app("own-title-bar-rows");
    let ctx = egui::Context::default();
    for width in SIZES {
        let (bar, panel, tabs) = rows(&mut a, &ctx, width);
        assert!(near([bar.min.x, bar.min.y, bar.max.x, bar.height()], [0.0, 0.0, width, BAR_H]), "the top bar, {width} pt: {bar:?}");
        let panel = panel.unwrap_or_else(|| panic!("the ribbon, {width} pt"));
        assert!(near([panel.min.y, panel.height()], [BAR_H, RIBBON_PANEL_H]), "the ribbon panel, {width} pt: {panel:?}");
        let tabs = tabs.unwrap_or_else(|| panic!("the tabs, {width} pt"));
        assert!(near([tabs.min.y, tabs.height()], [BAR_H + TABS_TOP_IN_PANEL, TABS_H]), "the tabs, {width} pt: {tabs:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// With the system's title bar (the option, a Mac, the browser), the top bar is as it was: no
/// window buttons, and neither its free space nor the window's edges do anything.
#[test]
fn with_the_systems_title_bar_the_top_bar_draws_none_of_it() {
    let (mut a, dir) = app("system-title-bar");
    let size = egui::vec2(1280.0, 800.0);
    let mut w = Window::new(size);
    let seen = named(&w.settle(&mut a));
    for name in CAPTIONS {
        assert!(in_bar(&seen, name).is_none(), "no {name} on the bar");
    }
    let free = free_space(&seen);
    let cmds = w.click(&mut a, free, 2);
    assert!(!zooms(&cmds), "{cmds:?}");
    let cmds = w.drag(&mut a, free);
    assert!(!moves(&cmds), "{cmds:?}");
    for (at, _) in edges(size) {
        let cmds = w.press(&mut a, at);
        assert!(!resizes(&cmds), "at {at:?}: {cmds:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}
