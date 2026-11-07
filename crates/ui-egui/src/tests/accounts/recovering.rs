// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: Recover deleted items. The window opens from the folder menu's and the list's link,
//! lists what the server says, asks before a purge (Enter purges, Escape keeps), and is greyed out
//! for a server that keeps nothing.

use egui::{Key, Modifiers};
use quatrelle_engine::model::{Event as MailEvent, Job, Recoverable, SyncStatus};

use super::*;

fn exchange(a: &mut MailApp) -> String {
    let id = a
        .session
        .run("account.add", &json!({"email": "riley@contoso.example", "ews": "mail.contoso.example", "username": "CONTOSO\\riley", "password": "pw"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    a.session.apply(MailEvent::Status { account: id.clone(), status: SyncStatus::Online { at: 1 } });
    a.session.take_jobs();
    id
}

fn press(ctx: &egui::Context, a: &mut MailApp, k: Key) {
    frame(ctx, a, vec![key(k, Modifiers::NONE)]);
}

#[test]
fn the_window_opens_lists_and_asks_before_purging() {
    let (mut a, dir) = app("recover-window");
    let ctx = egui::Context::default();
    let id = exchange(&mut a);
    assert!(crate::dialogs::recover_blocked(&a.session, &id).is_none());
    // What the folder menu and the list's link do.
    let mut acts = Vec::new();
    crate::dialogs::open_recovery(&id, &mut acts);
    for act in acts {
        a.act(act);
    }
    assert!(matches!(&a.ui.dialog, Some(Dialog::Recover(f)) if f.account == id));
    let ask = match a.session.take_jobs().as_slice() {
        [Job::ListRecoverable { ask, offset: 0, .. }] => *ask,
        other => panic!("{other:?}"),
    };
    frame(&ctx, &mut a, vec![]);
    let item = |n: i64| Recoverable {
        id: format!("i{n}"),
        subject: format!("Budget {n}"),
        from: "Zoe Park".into(),
        deleted: 1_700_000_000 + n,
        size: 2048,
        ..Default::default()
    };
    a.session.apply(MailEvent::Recoverable { account: id.clone(), ask, offset: 0, items: vec![item(1), item(2)], more: false, error: None });
    frame(&ctx, &mut a, vec![]);
    let shapes = frame_shapes(&ctx, &mut a, vec![], egui::vec2(1280.0, 800.0));
    let drawn: Vec<String> =
        shapes.iter().filter_map(|s| if let egui::Shape::Text(t) = s { Some(t.galley.text().to_string()) } else { None }).collect();
    assert!(text_at(&shapes, "Budget 2").is_some() && text_at(&shapes, "Budget 1").is_some(), "the items are listed: {drawn:?}");
    // Enter alone purges nothing.
    press(&ctx, &mut a, Key::Enter);
    assert!(a.session.take_jobs().is_empty());
    // The question: Escape keeps everything and stays in the list.
    if let Some(Dialog::Recover(f)) = &mut a.ui.dialog {
        f.confirming = Some(vec!["i1".into()]);
    }
    frame(&ctx, &mut a, vec![]);
    press(&ctx, &mut a, Key::Escape);
    assert!(matches!(&a.ui.dialog, Some(Dialog::Recover(f)) if f.confirming.is_none()), "back to the list: {:?}", a.ui.dialog);
    assert!(a.session.take_jobs().is_empty() && a.session.recovering.is_some());
    // Enter purges.
    if let Some(Dialog::Recover(f)) = &mut a.ui.dialog {
        f.confirming = Some(vec!["i1".into()]);
    }
    frame(&ctx, &mut a, vec![]);
    press(&ctx, &mut a, Key::Enter);
    assert!(a.session.recovering.as_ref().is_some_and(|r| r.working), "the purge was asked for");
    // Escape from the list closes it, and the session lets go of the list.
    a.session.apply(MailEvent::RecoverableDone { account: id.clone(), ask, done: vec!["i1".into()], purge: true, error: None });
    frame(&ctx, &mut a, vec![]);
    press(&ctx, &mut a, Key::Escape);
    assert!(a.ui.dialog.is_none() && a.session.recovering.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_server_that_keeps_nothing_has_the_entry_greyed_out() {
    let (mut a, dir) = app("recover-greyed");
    // The sample mailbox has no Exchange server; neither does IMAP.
    let why = crate::dialogs::recover_blocked(&a.session, "sample");
    assert!(why.is_some_and(|w| w.contains("keep deleted items")), "{why:?}");
    // Offline, an Exchange account says to connect.
    let id = exchange(&mut a);
    a.session.apply(MailEvent::Status { account: id.clone(), status: SyncStatus::Offline });
    assert!(crate::dialogs::recover_blocked(&a.session, &id).is_some_and(|w| w.contains("Connect first")));
    let _ = std::fs::remove_dir_all(dir);
}
