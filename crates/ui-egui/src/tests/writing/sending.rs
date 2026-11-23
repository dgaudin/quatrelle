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
