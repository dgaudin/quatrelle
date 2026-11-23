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
    // Ctrl+Enter (set to send at once: its question is tested below) and Alt+S, the same.
    a.session.prefs.ctrl_enter = quatrelle_engine::CtrlEnter::Send;
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

/// Closing Quatrelle while a message closed as it went is still on its way asks first, as for
/// one still open: it's in neither Drafts nor the Outbox yet.
#[test]
fn closing_quatrelle_counts_a_message_closed_as_it_goes() {
    let (mut a, dir) = app("close-closed-sending");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let id =
        a.session.run("account.add", &json!({"email": "zoe@studio.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    a.run("mail.new", json!({"account": id}));
    a.run("compose.update", json!({"to": "ann@studio.example", "subject": "On its way", "body": "See you"}));
    a.run("compose.send", json!({}));
    assert!(a.session.compose.as_ref().is_some_and(|c| c.sending), "on its way");
    // Its window closed, as closing it does.
    a.run("compose.close", json!({"save": false}));
    assert!(a.session.all_composes().next().is_none(), "off the screen");
    frame(&ctx, &mut a, vec![]);
    let close = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
        viewports: std::iter::once((egui::ViewportId::ROOT, egui::ViewportInfo { events: vec![egui::ViewportEvent::Close], ..Default::default() }))
            .collect(),
        ..Default::default()
    };
    let mut out = ctx.run_ui(close, |ui| a.ui(ui));
    out.textures_delta.clear();
    assert!(matches!(a.ui.dialog, Some(Dialog::LostOnClose { sending: 1, unsent: 0, changes: 0 })), "{:?}", a.ui.dialog);
    let _ = std::fs::remove_dir_all(dir);
}

/// The text drawn by a frame, every piece of it.
fn drawn_texts(ctx: &egui::Context, a: &mut MailApp) -> Vec<String> {
    let shapes = frame_shapes(ctx, a, vec![], egui::vec2(1280.0, 800.0));
    shapes.iter().filter_map(|s| if let egui::Shape::Text(t) = s { Some(t.galley.text().to_string()) } else { None }).collect()
}

/// A message the server never said it took (it may have gone), in French: its window says so
/// and how to find out before sending it again, never « Non envoyé » ; its row in the Outbox
/// and the reading pane over it say so too, without the worker's mark; a refused one's reading
/// pane says why without it.
#[test]
fn the_window_and_the_outbox_say_may_have_gone_never_not_sent() {
    let (mut a, dir) = app("send-may-have-gone");
    a.session.prefs.new_mail_window = false;
    a.ui.language = "fr".into();
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let id =
        a.session.run("account.add", &json!({"email": "zoe@studio.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    a.run("mail.new", json!({"account": id}));
    a.run("compose.update", json!({"to": "ann@studio.example", "subject": "Plans", "body": "See you"}));
    a.run("compose.send", json!({}));
    let sent = a.session.compose.as_ref().filter(|c| c.sending).map(|c| c.id).unwrap_or_default();
    let error = "the server didn't say whether it took the message, which may have gone: response error: incomplete response";
    a.session.apply(quatrelle_engine::model::Event::SendFailed { account: id.clone(), id: sent, error: error.into(), unconfirmed: true });
    let gone = "le serveur n’a pas dit s’il a pris le message, qui est peut-être parti : response error: incomplete response";
    // The notice, as it's drawn; then the window alone.
    let notice = a.session.notices.pop();
    a.session.notices.clear();
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(notice.as_ref().map(crate::i18n::notice), Some(format!("Il est peut-être parti : {gone}")));
    let texts = drawn_texts(&ctx, &mut a);
    assert!(texts.iter().any(|t| *t == format!("Il est peut-être parti : {gone}")), "{texts:?}");
    assert!(texts.iter().any(|t| t.starts_with("Avant de le renvoyer, demandez à son destinataire")), "{texts:?}");
    assert!(!texts.iter().any(|t| t.contains("Non envoyé") || t.contains("Not sent")), "{texts:?}");
    a.run("compose.discard", json!({}));
    // In the Outbox.
    let store = a.session.store.clone().unwrap();
    for (n, subject, mark) in
        [(7u64, "Maybe", format!("unconfirmed: {error}")), (8, "Refused", "refused: the server refused: 550 no such user".into())]
    {
        let raw = format!("From: zoe@studio.example\r\nTo: ann@studio.example\r\nSubject: {subject}\r\n\r\nhi\r\n");
        let item = quatrelle_engine::store::OutboxItem {
            id: n,
            recipients: vec!["ann@studio.example".into()],
            queued: NOW + n as i64,
            attempts: 1,
            error: Some(mark),
            ..Default::default()
        };
        store.outbox_add(&id, &item, raw.as_bytes()).unwrap();
    }
    a.run("folder.outbox", json!({}));
    let key = |a: &MailApp, subject: &str| a.session.list.iter().find(|m| m.subject == subject).map(|m| m.key.clone()).unwrap_or_default();
    let maybe = key(&a, "Maybe");
    a.session.open_message(&maybe);
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts = drawn_texts(&ctx, &mut a);
    assert!(texts.iter().any(|t| *t == format!("Peut-être parti : {gone}")), "the row: {texts:?}");
    assert!(texts.iter().any(|t| *t == format!("Il est peut-être parti : {gone}")), "the reading pane: {texts:?}");
    assert!(texts.iter().any(|t| t.starts_with("Avant de le renvoyer, demandez à son destinataire")), "{texts:?}");
    assert!(!texts.iter().any(|t| t.contains("Non envoyé") || t.contains("unconfirmed")), "{texts:?}");
    let refused = key(&a, "Refused");
    a.session.open_message(&refused);
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts = drawn_texts(&ctx, &mut a);
    let pane = texts.iter().find(|t| t.starts_with("En attente dans la boîte d’envoi. Dernière tentative :")).cloned().unwrap_or_default();
    assert_eq!(pane, "En attente dans la boîte d’envoi. Dernière tentative : le serveur a refusé : 550 no such user", "{texts:?}");
    crate::i18n::set_current(crate::i18n::Lang::EN);
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
    // (Set to send at once; the tests of its question set it back.)
    a.session.prefs.ctrl_enter = quatrelle_engine::CtrlEnter::Send;
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

/// The dialog's own button `label` (the message's own Send is another, away from it).
fn dialog_button(ctx: &egui::Context, a: &mut MailApp, label: &str) -> egui::Pos2 {
    let nodes = shown(ctx, a);
    let beside = nodes.iter().find(|n| n.0 == Role::Button && n.1 == "Don't Send").map(|n| n.2.center()).expect("the dialog's Don't Send");
    // In the dialog's row of buttons (the message's own Send is above it, on another row).
    let row = nodes.iter().filter(|n| n.0 == Role::Button && n.1 == label && !n.3 && (n.2.center().y - beside.y).abs() < 6.0);
    row.map(|n| n.2.center()).next().unwrap_or_else(|| panic!("no {label} in the dialog"))
}

/// Ctrl+Enter asks first (the default): Don't Send leaves the message as it was and nothing
/// goes; Send sends it.
#[test]
fn ctrl_enter_asks_before_sending() {
    use quatrelle_engine::CtrlEnter;
    let (mut a, dir, ctx, jobs) = writing("ctrl-enter-asks", "Plans");
    a.session.prefs.ctrl_enter = CtrlEnter::default();
    assert_eq!(a.session.prefs.ctrl_enter, CtrlEnter::Ask, "it asks unless told otherwise");
    let written = a.session.compose.clone();
    ctrl_enter(&ctx, &mut a);
    assert!(matches!(a.ui.dialog, Some(Dialog::CtrlEnter { remember: false })), "asked");
    assert_eq!(went(&jobs), 0, "not sent yet");
    assert_eq!(a.session.compose, written);
    let at = dialog_button(&ctx, &mut a, "Don't Send");
    click(&ctx, &mut a, at);
    assert!(a.ui.dialog.is_none());
    assert_eq!(went(&jobs), 0, "Don't Send: nothing goes");
    assert_eq!(a.session.compose, written, "and the message stays as it was");
    assert_eq!(a.session.prefs.ctrl_enter, CtrlEnter::Ask, "it asks again next time");
    // Asked again, Send sends it.
    ctrl_enter(&ctx, &mut a);
    let at = dialog_button(&ctx, &mut a, "Send");
    click(&ctx, &mut a, at);
    assert!(a.ui.dialog.is_none());
    assert_eq!(went(&jobs), 1, "Send: it goes");
    assert_eq!(a.session.prefs.ctrl_enter, CtrlEnter::Ask, "without keeping the answer");
    let _ = std::fs::remove_dir_all(dir);
}

/// Don't ask again ticked, then Send: it goes, the answer is kept, and the next message's
/// Ctrl+Enter sends at once. Ticked with Don't Send, it does nothing from then on.
#[test]
fn dont_ask_again_is_ticked_then_ctrl_enter_sends() {
    use quatrelle_engine::CtrlEnter;
    let (mut a, dir, ctx, jobs) = writing("ctrl-enter-keeps", "Plans");
    a.session.prefs.ctrl_enter = CtrlEnter::Ask;
    ctrl_enter(&ctx, &mut a);
    assert!(matches!(a.ui.dialog, Some(Dialog::CtrlEnter { remember: false })));
    let nodes = shown(&ctx, &mut a);
    let tick = nodes.iter().find(|n| n.1 == "Don't ask again").map(|n| n.2.center()).expect("the box to tick");
    click(&ctx, &mut a, tick);
    assert!(matches!(a.ui.dialog, Some(Dialog::CtrlEnter { remember: true })), "ticked");
    let at = dialog_button(&ctx, &mut a, "Send");
    click(&ctx, &mut a, at);
    assert_eq!(went(&jobs), 1, "sent");
    assert_eq!(a.session.prefs.ctrl_enter, CtrlEnter::Send, "the answer is kept");
    // The next message: Ctrl+Enter sends without asking.
    let account = a.session.accounts.iter().find(|x| x.config.email == "riley@contoso.example").map(|x| x.config.id.clone()).unwrap();
    a.run("mail.new", json!({"account": account}));
    a.run("compose.update", json!({"account": account, "to": "ann@studio.example", "subject": "Again", "body": "See you"}));
    for _ in 0..4 {
        frame(&ctx, &mut a, vec![]);
    }
    ctrl_enter(&ctx, &mut a);
    assert!(a.ui.dialog.is_none(), "not asked");
    assert_eq!(went(&jobs), 2, "sent at once");
    // Kept as Don't Send, it does nothing.
    a.session.prefs.ctrl_enter = CtrlEnter::Ask;
    a.run("mail.new", json!({"account": account}));
    a.run("compose.update", json!({"account": account, "to": "ann@studio.example", "subject": "Third", "body": "See you"}));
    for _ in 0..4 {
        frame(&ctx, &mut a, vec![]);
    }
    ctrl_enter(&ctx, &mut a);
    let nodes = shown(&ctx, &mut a);
    let tick = nodes.iter().find(|n| n.1 == "Don't ask again").map(|n| n.2.center()).expect("the box to tick");
    click(&ctx, &mut a, tick);
    let at = dialog_button(&ctx, &mut a, "Don't Send");
    click(&ctx, &mut a, at);
    assert_eq!(a.session.prefs.ctrl_enter, CtrlEnter::Nothing, "kept");
    assert_eq!(went(&jobs), 2, "nothing went");
    ctrl_enter(&ctx, &mut a);
    assert!(a.ui.dialog.is_none(), "not asked");
    assert_eq!(went(&jobs), 2, "and nothing goes from then on");
    assert!(a.session.compose.as_ref().is_some_and(|c| !c.sending));
    let _ = std::fs::remove_dir_all(dir);
}

/// A message in a window of its own asks the same, and Send there sends it.
#[test]
fn ctrl_enter_in_a_message_window_asks_too() {
    let (mut a, dir, ctx, jobs) = writing("ctrl-enter-window", "Plans");
    a.session.prefs.ctrl_enter = quatrelle_engine::CtrlEnter::Ask;
    a.session.prefs.new_mail_window = true;
    ctx.set_embed_viewports(false);
    let account = a.session.accounts.iter().find(|x| x.config.email == "riley@contoso.example").map(|x| x.config.id.clone()).unwrap();
    a.run("compose.discard", json!({}));
    a.run("mail.new", json!({"account": account}));
    a.run("compose.update", json!({"account": account, "to": "ann@studio.example", "subject": "Plans", "body": "See you"}));
    for _ in 0..4 {
        frame(&ctx, &mut a, vec![]);
    }
    let written = a.session.compose.clone();
    assert!(written.as_ref().is_some_and(|c| c.window), "in a window of its own");
    ctrl_enter(&ctx, &mut a);
    assert!(matches!(a.ui.dialog, Some(Dialog::CtrlEnter { .. })), "asked in its window");
    assert_eq!(went(&jobs), 0, "not sent");
    assert_eq!(a.session.compose, written);
    let _ = std::fs::remove_dir_all(dir);
}

/// Alt+S and the Send button never ask, whatever Ctrl+Enter does.
#[test]
fn alt_s_and_send_never_ask() {
    let (mut a, dir, ctx, jobs) = writing("alt-s-send", "Plans");
    a.session.prefs.ctrl_enter = quatrelle_engine::CtrlEnter::Ask;
    let m = Modifiers::ALT;
    frame(&ctx, &mut a, vec![Event::ModifiersChanged(m), key(Key::S, m)]);
    frame(&ctx, &mut a, vec![Event::ModifiersChanged(Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert!(!matches!(a.ui.dialog, Some(Dialog::CtrlEnter { .. })), "Alt+S asks nothing");
    assert_eq!(went(&jobs), 1, "Alt+S sent it");
    // The Send button, on the next message.
    let account = a.session.accounts.iter().find(|x| x.config.email == "riley@contoso.example").map(|x| x.config.id.clone()).unwrap();
    a.run("mail.new", json!({"account": account}));
    a.run("compose.update", json!({"account": account, "to": "ann@studio.example", "subject": "Again", "body": "See you"}));
    for _ in 0..4 {
        frame(&ctx, &mut a, vec![]);
    }
    let nodes = shown(&ctx, &mut a);
    let send = nodes.iter().find(|n| n.0 == Role::Button && n.1 == "Send" && !n.3).map(|n| n.2.center()).expect("Send");
    click(&ctx, &mut a, send);
    assert!(!matches!(a.ui.dialog, Some(Dialog::CtrlEnter { .. })), "Send asks nothing");
    assert_eq!(went(&jobs), 2, "Send sent it");
    let _ = std::fs::remove_dir_all(dir);
}

/// A check before sending whose findings can't be read still says there is something, not only
/// "Send anyway".
#[test]
fn a_check_before_sending_that_cant_be_shown_still_says_so() {
    let (mut a, dir) = app("send-check-unshown");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.ui.dialog = Some(Dialog::SendCheck(Vec::new()));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let nodes = shown(&ctx, &mut a);
    let said = "Something about the recipients was found to look at, but it can't be shown. Look them over before sending.";
    assert!(nodes.iter().any(|(_, name, ..)| name == said), "the dialog says there is something: {nodes:?}");
    assert!(nodes.iter().any(|(role, name, ..)| *role == Role::Button && name == "Send anyway"));
    let _ = std::fs::remove_dir_all(dir);
}
