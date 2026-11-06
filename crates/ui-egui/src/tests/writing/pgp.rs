// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the message window with OpenPGP (lot 4). Without a certificate or a key nothing of
//! it shows; with a key, Encrypt and Sign; with both, the choice between S/MIME and OpenPGP
//! (Automatic first). Sending encrypted to someone without a usable key is stopped, naming them.
//! With `QUATRELLE_FRAME_PNG` set to a folder, the window is painted there, light and dark.

use super::super::list::follow_up::{Textures, frame_with};
use super::*;
use quatrelle_engine::model::job::PgpChange;
use quatrelle_engine::pgp;

/// The texts drawn once the window has settled, light and dark, each painted as `name` when
/// pictures are asked for; the light ones.
fn drawn(ctx: &egui::Context, a: &mut MailApp, tex: &mut Textures, name: &str) -> Vec<String> {
    let pictures = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let mut light = Vec::new();
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        a.ui.dark = Some(dark);
        for _ in 0..6 {
            frame_with(ctx, a, tex, vec![], None);
        }
        let png = pictures.as_ref().map(|d| d.join(format!("pgp-compose-{name}-{theme}.png")));
        let (_, shapes) = frame_with(ctx, a, tex, vec![], png);
        let texts: Vec<String> = shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect();
        if !dark {
            light = texts;
        }
    }
    a.ui.dark = Some(false);
    light
}

fn has(texts: &[String], wanted: &str) -> bool {
    texts.iter().any(|t| t.contains(wanted))
}

/// Riley's key made (the network layer's answer played).
fn make_key(a: &mut MailApp) -> pgp::Unlocked {
    let key = pgp::generate("Riley Moreno", "riley@studio.example", NOW, &mut rand_core::OsRng).unwrap();
    let changed = Some(PgpChange::Made { fingerprint: key.info.fingerprint.clone() });
    a.session.apply(MailEvent::PgpKeys { account: "sample".into(), keys: vec![key.public.clone()], changed, error: None });
    key
}

/// Riley's S/MIME certificate imported (the network layer's answer played).
fn import_certificate(a: &mut MailApp) {
    let p12 = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../smime/testdata/riley-aes.p12")).unwrap();
    let id = quatrelle_engine::smime::read_p12(&p12, "secret").unwrap();
    a.session.apply(MailEvent::Identity { account: "sample".into(), chain: id.chain, older: Vec::new(), for_reading: false, error: None });
}

/// A message to Bo (whose key is known) and Eve (who has none), on the ribbon's Options tab.
fn write_to_bo_and_eve(ctx: &egui::Context, a: &mut MailApp, tex: &mut Textures) {
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "Bo <bo@studio.example>; eve@studio.example", "subject": "Plan", "body": "The plan."}));
    for _ in 0..3 {
        frame_with(ctx, a, tex, vec![], None);
    }
    a.ui.editor.as_mut().expect("the message is drawn").tab = crate::rich_edit::ComposeTab::Options;
}

#[test]
fn the_message_window_offers_the_scheme_only_with_both() {
    let (mut a, dir) = app("pgp-compose");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    let mut tex = Textures::default();
    let bo = pgp::generate("Bo", "bo@studio.example", NOW, &mut rand_core::OsRng).unwrap();
    a.session.run("pgp.importPublic", &json!({"data": bo.public})).unwrap();
    // Neither a certificate nor a key: no Encrypt group at all.
    write_to_bo_and_eve(&ctx, &mut a, &mut tex);
    let t = drawn(&ctx, &mut a, &mut tex, "nothing");
    assert!(has(&t, "Read receipt") && !has(&t, "Encrypt") && !has(&t, "Automatic"), "{t:?}");
    a.run("compose.discard", json!({}));
    // A key: Encrypt and Sign, no choice.
    make_key(&mut a);
    write_to_bo_and_eve(&ctx, &mut a, &mut tex);
    let t = drawn(&ctx, &mut a, &mut tex, "key");
    assert!(has(&t, "Encrypt") && has(&t, "Sign") && !has(&t, "Automatic"), "{t:?}");
    a.run("compose.discard", json!({}));
    // Both: the choice, Automatic first; chosen, it says which.
    import_certificate(&mut a);
    write_to_bo_and_eve(&ctx, &mut a, &mut tex);
    let t = drawn(&ctx, &mut a, &mut tex, "both");
    assert!(has(&t, "Encrypt") && has(&t, "Automatic"), "{t:?}");
    a.run("compose.security", json!({"scheme": "pgp"}));
    let t = drawn(&ctx, &mut a, &mut tex, "both-openpgp");
    assert!(has(&t, "OpenPGP") && !has(&t, "Automatic"), "{t:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Encrypted with OpenPGP to Eve, who has no key: Send says it can't, naming her, and nothing
/// is asked of the engine.
#[test]
fn sending_encrypted_without_a_key_names_who_lacks_one() {
    let (mut a, dir) = app("pgp-compose-refused");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    let mut tex = Textures::default();
    make_key(&mut a);
    let bo = pgp::generate("Bo", "bo@studio.example", NOW, &mut rand_core::OsRng).unwrap();
    a.session.run("pgp.importPublic", &json!({"data": bo.public})).unwrap();
    write_to_bo_and_eve(&ctx, &mut a, &mut tex);
    a.run("compose.security", json!({"encrypt": true}));
    frame_with(&ctx, &mut a, &mut tex, vec![], None);
    let alt_s = Event::Key { key: egui::Key::S, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::ALT };
    frame_with(&ctx, &mut a, &mut tex, vec![alt_s], None);
    let alert = a.ui.compose_alert.clone().expect("said over the message");
    assert!(alert.contains("eve@studio.example") && !alert.contains("bo@"), "{alert}");
    assert!(a.session.compose.as_ref().is_some_and(|c| !c.sending), "not sent");
    assert!(!a.session.take_jobs().iter().any(|j| matches!(j, quatrelle_engine::model::Job::Pgp { .. })));
    let t = drawn(&ctx, &mut a, &mut tex, "refused");
    assert!(has(&t, "eve@studio.example"), "{t:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// With a key, the message window says under the subject whether everyone in it can receive it
/// encrypted, naming who can't (in red while Encrypt is on: it won't go); and offers the key as
/// an attachment. The option that has encryption follow the recipients is in File › Options.
#[test]
fn the_line_says_who_cant_receive_it_encrypted() {
    let (mut a, dir) = app("pgp-compose-line");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    let mut tex = Textures::default();
    let bo = pgp::generate("Bo", "bo@studio.example", NOW, &mut rand_core::OsRng).unwrap();
    a.session.run("pgp.importPublic", &json!({"data": bo.public})).unwrap();
    // Without a key of the account's: no line.
    write_to_bo_and_eve(&ctx, &mut a, &mut tex);
    let t = drawn(&ctx, &mut a, &mut tex, "line-without-key");
    assert!(!has(&t, "It can't be encrypted") && !has(&t, "Attach my public key"), "{t:?}");
    a.run("compose.discard", json!({}));
    make_key(&mut a);
    write_to_bo_and_eve(&ctx, &mut a, &mut tex);
    let t = drawn(&ctx, &mut a, &mut tex, "line-off");
    assert!(has(&t, "It can't be encrypted for eve@studio.example: no usable OpenPGP key"), "{t:?}");
    assert!(has(&t, "Attach my public key"), "{t:?}");
    a.run("compose.security", json!({"encrypt": true}));
    let t = drawn(&ctx, &mut a, &mut tex, "line-on");
    assert!(has(&t, "It can't be encrypted for eve@studio.example"), "{t:?}");
    a.run("compose.update", json!({"to": "Bo <bo@studio.example>"}));
    let t = drawn(&ctx, &mut a, &mut tex, "line-everyone");
    assert!(has(&t, "Every recipient can receive it encrypted (OpenPGP)") && !has(&t, "It can't be encrypted"), "{t:?}");
    // The option, off as it starts.
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    let mut texts = Vec::new();
    for _ in 0..6 {
        texts = super::super::list::follow_up::frame_with_size(&ctx, &mut a, &mut tex, vec![], None, (1280, 4000))
            .1
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect();
    }
    assert!(has(&texts, "Encrypt by itself when everyone can receive it"), "{texts:?}");
    assert!(!a.session.prefs.encrypt_when_possible);
    let _ = std::fs::remove_dir_all(dir);
}
