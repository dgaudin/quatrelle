// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The File page's box « Archive on this computer », for IMAP accounts only.

use super::*;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// The File page's text, drawn for the account `id`.
fn info_texts(ctx: &egui::Context, a: &mut MailApp, id: &str) -> Vec<String> {
    a.act(crate::Action::Backstage(crate::backstage::Page::Info));
    if let Some(b) = a.ui.backstage.as_mut() {
        b.account = Some(id.to_string());
    }
    let mut seen = Vec::new();
    for _ in 0..3 {
        seen = frame_shapes(ctx, a, vec![], SIZE)
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect();
    }
    seen
}

#[test]
fn the_account_settings_offer_archiving_on_this_computer() {
    let (mut a, dir) = app("archive-box");
    let ctx = egui::Context::default();
    let imap =
        a.session.run("account.add", &json!({"email": "zoe@studio.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let ews = a
        .session
        .run("account.add", &json!({"email": "riley@contoso.example", "ews": "mail.contoso.example", "username": "CONTOSO\\riley", "password": "pw"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let pop = a
        .session
        .run("account.add", &json!({"email": "ana@studio.example", "pop": true, "popHost": "pop.studio.example", "popPort": 995, "password": "pw"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let local = a.session.accounts.first().map(|x| x.config.id.clone()).unwrap();
    let words = "Archive on this computer";
    // An IMAP account has it; Exchange, POP and an account on this computer do not.
    assert!(info_texts(&ctx, &mut a, &imap).iter().any(|t| t == words), "IMAP");
    for other in [&ews, &pop, &local] {
        assert!(!info_texts(&ctx, &mut a, other).iter().any(|t| t == words), "{other}");
    }
    // Ticked: the account keeps it.
    info_texts(&ctx, &mut a, &imap);
    let at = settled_text_at(&ctx, &mut a, words, SIZE).expect("the box's words");
    assert!(!a.session.account(&imap).unwrap().config.archive_local);
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert!(a.session.account(&imap).unwrap().config.archive_local, "ticked");
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert!(!a.session.account(&imap).unwrap().config.archive_local, "and unticked");
    let _ = std::fs::remove_dir_all(dir);
}
