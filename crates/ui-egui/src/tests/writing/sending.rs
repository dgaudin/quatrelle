// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a message written from an account that can't send (an Outlook data file, with no
//! account set up) says so in a bar over its form, Send says why instead of "Message sent", and
//! the message stays open; an account added from the bar takes it, and Send is on again.

use egui::accesskit::Role;

use super::*;

/// What the frame shows, as a screen reader is told it: each node's role, name, place and
/// whether it's off.
fn shown(ctx: &egui::Context, a: &mut MailApp) -> Vec<(Role, String, egui::Rect, bool)> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let tree = out.platform_output.accesskit_update.expect("AccessKit is on");
    tree.nodes
        .iter()
        .filter_map(|(_, n)| {
            let name = n.label().or(n.value())?.to_string();
            let b = n.bounds()?;
            let rect = egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32));
            Some((n.role(), name, rect, n.is_disabled()))
        })
        .collect()
}

/// The message being written, To's names as Send's check closes them (`a; `).
fn as_written(a: &MailApp) -> Option<quatrelle_engine::Compose> {
    a.session.compose.as_ref().map(as_written_c)
}

fn as_written_c(c: &quatrelle_engine::Compose) -> quatrelle_engine::Compose {
    quatrelle_engine::Compose { to: c.to.trim_end_matches([';', ' ']).to_string(), ..c.clone() }
}

fn click(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2) {
    let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(ctx, a, vec![Event::PointerMoved(at)]);
    frame(ctx, a, vec![button(true)]);
    frame(ctx, a, vec![button(false)]);
    frame(ctx, a, vec![]);
}

#[test]
fn send_from_a_data_file_says_to_add_an_account() {
    let (mut a, dir) = app("send-data-file");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    // Only a data file to write from (the sample mailbox, which can send, set aside).
    let file = a.session.run("account.importPst", &json!({"path": r"C:\Mail\Archive.pst"})).unwrap()["account"].as_str().unwrap().to_string();
    a.session.accounts.retain(|x| x.config.id == file);
    a.run("mail.new", json!({"account": file}));
    a.run("compose.update", json!({"to": "ann@example.org", "subject": "Plans", "body": "See you"}));
    let (what, todo) = crate::reading::send_block_lines(quatrelle_engine::SendBlock::DataFile);
    let why = crate::reading::send_block_text(quatrelle_engine::SendBlock::DataFile);
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let written = a.session.compose.clone().expect("a message is being written");
    assert_eq!(written.account, file);
    // The bar: what's wrong, what to do under it, and a way to do it; above the form.
    let nodes = shown(&ctx, &mut a);
    let bar = nodes.iter().find(|(_, name, ..)| name == what).expect("the bar says why it can't be sent");
    let note = nodes.iter().find(|(_, name, ..)| Some(name.as_str()) == todo).expect("and what to do");
    let link = nodes.iter().find(|(_, name, ..)| name == "Add an account").expect("Add an account in the bar");
    let send = nodes.iter().find(|(role, name, ..)| *role == Role::Button && name == "Send").expect("Send");
    assert!(send.3, "Send is off");
    assert!(bar.2.bottom() <= note.2.top() && (note.2.center().y - link.2.center().y).abs() < 4.0, "{bar:?} {note:?} {link:?}");
    assert!(bar.2.bottom() < send.2.top(), "over the form");
    // Clicked anyway: why, over the message; not sent, and the message stays as it was.
    click(&ctx, &mut a, send.2.center());
    assert_eq!(a.ui.compose_alert.as_deref(), Some(why.as_str()));
    assert_eq!(as_written(&a), Some(as_written_c(&written)));
    assert!(!a.session.notices.iter().any(|n| n.text.contains("sent")), "{:?}", a.session.notices);
    // Ctrl+Enter and Alt+S, the same.
    for (k, m) in [(Key::Enter, Modifiers::COMMAND), (Key::S, Modifiers::ALT)] {
        a.ui.compose_alert = None;
        frame(&ctx, &mut a, vec![Event::ModifiersChanged(m), key(k, m)]);
        frame(&ctx, &mut a, vec![Event::ModifiersChanged(Modifiers::NONE)]);
        assert_eq!(a.ui.compose_alert.as_deref(), Some(why.as_str()), "{k:?}");
        assert_eq!(as_written(&a), Some(as_written_c(&written)));
    }
    // The alert offers to add an account too.
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    let nodes = shown(&ctx, &mut a);
    assert!(nodes.iter().any(|(role, name, ..)| *role == Role::Button && name == "Add an account"), "in the alert");
    a.ui.compose_alert = None;
    frame(&ctx, &mut a, vec![]);
    let nodes = shown(&ctx, &mut a);
    let add = nodes.iter().find(|(_, name, ..)| name == "Add an account").map(|n| n.2.center()).unwrap();
    click(&ctx, &mut a, add);
    assert!(matches!(a.ui.dialog, Some(Dialog::AddAccount(_))), "the account form opens");
    assert!(a.session.compose.is_some(), "still open");
    // The account added (as the form adds it): the open message goes From it, the bar goes and
    // Send is on.
    a.ui.dialog = None;
    a.run("account.add", json!({"email": "zoe@example.org", "password": "pw"}));
    a.session.take_jobs();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let zoe = a.session.accounts.iter().find(|x| x.config.email == "zoe@example.org").map(|x| x.config.id.clone()).unwrap();
    assert_eq!(a.session.compose.as_ref().map(|c| c.account.as_str()), Some(zoe.as_str()));
    let nodes = shown(&ctx, &mut a);
    assert!(!nodes.iter().any(|(_, name, ..)| name == what || name == "Add an account"), "the bar is gone");
    let send = nodes.iter().find(|(role, name, ..)| *role == Role::Button && name == "Send").expect("Send");
    assert!(!send.3, "Send is on");
    let _ = std::fs::remove_dir_all(dir);
}

/// From the sample mailbox, Send is on and nothing about adding an account shows.
#[test]
fn send_from_an_account_that_can_shows_nothing_more() {
    let (mut a, dir) = app("send-sample");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.run("mail.new", json!({}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let nodes = shown(&ctx, &mut a);
    let send = nodes.iter().find(|(role, name, ..)| *role == Role::Button && name == "Send").expect("Send");
    assert!(!send.3);
    assert!(!nodes.iter().any(|(_, name, ..)| name.contains("Add an account")));
    let _ = std::fs::remove_dir_all(dir);
}

/// A backend that keeps the jobs handed to it, so that a test sees what went out.
#[derive(Default)]
struct Kept(std::rc::Rc<std::cell::RefCell<Vec<quatrelle_engine::model::Job>>>);

impl crate::Backend for Kept {
    fn submit(&mut self, job: quatrelle_engine::model::Job) {
        self.0.borrow_mut().push(job);
    }

    fn poll(&mut self) -> Vec<MailEvent> {
        Vec::new()
    }
}

type Sent = std::rc::Rc<std::cell::RefCell<Vec<quatrelle_engine::model::Job>>>;

/// How many messages went out so far.
fn went(jobs: &Sent) -> usize {
    jobs.borrow().iter().filter(|j| matches!(j, quatrelle_engine::model::Job::Send { .. })).count()
}

/// The app with a message being written beside the list, and what it hands its backend.
fn writing(name: &str, subject: &str) -> (MailApp, std::path::PathBuf, egui::Context, Sent) {
    let (mut a, dir) = app(name);
    let kept = Kept::default();
    let jobs = kept.0.clone();
    a.backend = Box::new(kept);
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    // An account that sends through its server: the backend is handed a `Job::Send`.
    let account = with_exchange(&mut a);
    a.run("mail.new", json!({"account": account}));
    a.run("compose.update", json!({"account": account, "to": "ann@studio.example", "subject": subject, "body": "See you"}));
    for _ in 0..4 {
        frame(&ctx, &mut a, vec![]);
    }
    (a, dir, ctx, jobs)
}

/// Ctrl+Enter pressed and let go.
fn ctrl_enter(ctx: &egui::Context, a: &mut MailApp) {
    let m = Modifiers::COMMAND;
    frame(ctx, a, vec![Event::ModifiersChanged(m), key(Key::Enter, m)]);
    frame(ctx, a, vec![Event::ModifiersChanged(Modifiers::NONE)]);
    frame(ctx, a, vec![]);
}

/// Ctrl+Enter sent the message being written from wherever the keyboard was: with a search
/// typed, in the Calendar or Contacts, with a dialog open, a message half written went. It goes
/// only from the message (or with the keyboard nowhere), in Mail, with no dialog over it. (The
/// To-Do bar has no box to type in: only the search box can hold the keyboard beside the message.)
#[test]
fn ctrl_enter_from_elsewhere_sends_nothing() {
    // The search box has the keyboard.
    let (mut a, dir, ctx, jobs) = writing("ctrl-enter-search", "Plans");
    a.ui.focus_search = true;
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.search_focused, "the search box has the keyboard");
    ctrl_enter(&ctx, &mut a);
    assert_eq!(went(&jobs), 0, "not from the search box");
    assert!(a.session.compose.as_ref().is_some_and(|c| !c.sending), "the message is as it was");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ctrl_enter_in_another_module_sends_nothing() {
    use quatrelle_engine::Module;
    for module in [Module::Calendar, Module::People, Module::Tasks, Module::Notes] {
        let (mut a, dir, ctx, jobs) = writing("ctrl-enter-module", "Plans");
        a.session.module = module;
        for _ in 0..2 {
            frame(&ctx, &mut a, vec![]);
        }
        ctrl_enter(&ctx, &mut a);
        assert_eq!(went(&jobs), 0, "not from {module:?}");
        assert!(a.session.compose.as_ref().is_some_and(|c| !c.sending), "{module:?}: the message is as it was");
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn ctrl_enter_with_a_dialog_open_sends_nothing() {
    let (mut a, dir, ctx, jobs) = writing("ctrl-enter-dialog", "Plans");
    a.ui.dialog = Some(Dialog::Rules);
    frame(&ctx, &mut a, vec![]);
    ctrl_enter(&ctx, &mut a);
    assert_eq!(went(&jobs), 0, "not with a dialog open");
    assert!(a.session.compose.as_ref().is_some_and(|c| !c.sending));
    let _ = std::fs::remove_dir_all(dir);
}

/// Sent as the button sends it: a message with no subject asks first (the engine's own words,
/// `allowEmptySubject` and all, came up as a notice).
#[test]
fn ctrl_enter_with_no_subject_asks_as_send_does() {
    let (mut a, dir, ctx, jobs) = writing("ctrl-enter-no-subject", "");
    ctrl_enter(&ctx, &mut a);
    assert!(matches!(a.ui.dialog, Some(Dialog::EmptySubject)), "asked about the subject");
    assert_eq!(went(&jobs), 0, "not sent yet");
    assert!(a.session.compose.as_ref().is_some_and(|c| !c.sending));
    let _ = std::fs::remove_dir_all(dir);
}

/// The keyboard in the message, a subject filled in: it goes, as ever.
#[test]
fn ctrl_enter_in_the_message_still_sends() {
    let (mut a, dir, ctx, jobs) = writing("ctrl-enter-sends", "Plans");
    ctrl_enter(&ctx, &mut a);
    assert_eq!(went(&jobs), 1, "sent once");
    assert!(a.ui.dialog.is_none());
    let _ = std::fs::remove_dir_all(dir);
}
