// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: File › Info › Security's OpenPGP card (lot 3b). Without a key it offers one (made
//! or imported) and nothing else; with one, its fingerprint and end, a backup until one is saved,
//! « Extend » in the month before it runs out, and the keys of the people the account writes to;
//! the backup's passphrase is typed twice. With `QUATRELLE_FRAME_PNG` set to a folder, the card
//! is painted there, light and dark.

use super::super::list::follow_up::{Textures, frame_with_size};
use super::*;
use quatrelle_engine::model::job::PgpChange;
use quatrelle_engine::pgp;

/// Tall enough for the File page's Info to be drawn whole.
const PAGE: (usize, usize) = (1280, 2400);

/// The texts drawn once the page has settled, light and dark, each painted as `name` when
/// pictures are asked for; the light ones.
fn drawn(ctx: &egui::Context, a: &mut MailApp, tex: &mut Textures, name: &str) -> Vec<String> {
    let pictures = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let mut light = Vec::new();
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        a.ui.dark = Some(dark);
        for _ in 0..8 {
            frame_with_size(ctx, a, tex, vec![], None, PAGE);
        }
        let png = pictures.as_ref().map(|d| d.join(format!("pgp-card-{name}-{theme}.png")));
        let (_, shapes) = frame_with_size(ctx, a, tex, vec![], png, PAGE);
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
    light
}

fn has(texts: &[String], wanted: &str) -> bool {
    texts.iter().any(|t| t.contains(wanted))
}

#[test]
fn the_openpgp_card_offers_a_key_then_its_backup() {
    let (mut a, dir) = app("pgp-card");
    let ctx = egui::Context::default();
    let mut tex = Textures::default();
    a.act(crate::Action::Backstage(crate::backstage::Page::Info));
    // No key: one is offered, nothing else.
    let t = drawn(&ctx, &mut a, &mut tex, "none");
    assert!(has(&t, "No OpenPGP key") && has(&t, "Create an OpenPGP key") && has(&t, "Import a key…"), "{t:?}");
    assert!(!has(&t, "Send my public key with my messages") && !has(&t, "Save a backup…"), "{t:?}");
    // Made (the network layer's answer played): its fingerprint and end, and its backup asked for.
    let key = pgp::generate("Riley Moreno", "riley@studio.example", NOW, &mut rand_core::OsRng).unwrap();
    let changed = Some(PgpChange::Made { fingerprint: key.info.fingerprint.clone() });
    a.session.apply(MailEvent::PgpKeys { account: "sample".into(), keys: vec![key.public.clone()], changed, error: None });
    let t = drawn(&ctx, &mut a, &mut tex, "made");
    assert!(has(&t, &key.info.spaced()), "the fingerprint, spaced: {t:?}");
    let end = crate::i18n::date(&quatrelle_engine::model::dates::day_label(key.info.expires.unwrap().div_euclid(86_400)));
    assert!(has(&t, &format!("Valid until {end}")), "{end} {t:?}");
    assert!(has(&t, "Save a backup…") && has(&t, "No backup of this key is saved"), "{t:?}");
    assert!(has(&t, "Send my public key with my messages") && !has(&t, "Create an OpenPGP key"), "{t:?}");
    assert!(!has(&t, "Extend"), "not yet: {t:?}");
    // Someone's key, learnt from their mail.
    let bo = pgp::generate("Bo", "bo@studio.example", NOW, &mut rand_core::OsRng).unwrap();
    a.session.run("pgp.importPublic", &json!({"data": bo.public})).unwrap();
    let t = drawn(&ctx, &mut a, &mut tex, "people");
    assert!(has(&t, "Keys of the people you write to: 1") && has(&t, "bo@studio.example"), "{t:?}");
    assert!(has(&t, &format!("{} · imported by you", bo.info.spaced())), "{t:?}");
    // Backed up: the line goes.
    a.session.apply(MailEvent::PgpBackedUp { account: "sample".into(), fingerprint: key.info.fingerprint.clone(), error: None });
    let t = drawn(&ctx, &mut a, &mut tex, "backed-up");
    assert!(!has(&t, "No backup of this key is saved") && has(&t, "Save a backup…"), "{t:?}");
    // In the month before it runs out: « Extend ».
    let later = key.info.expires.unwrap() - 20 * 86_400;
    a.services = Box::new(crate::NullServices(later));
    a.session.clock.now = later;
    let t = drawn(&ctx, &mut a, &mut tex, "running-out");
    assert!(has(&t, "Extend") && has(&t, &format!("This key runs out on {end}")), "{t:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The backup's passphrase is typed twice: saved only when both are the same, never empty.
#[test]
fn a_backup_asks_its_passphrase_twice() {
    let (mut a, dir) = app("pgp-backup");
    let ctx = egui::Context::default();
    let mut tex = Textures::default();
    let path = dir.join("riley-openpgp-backup.asc").display().to_string();
    let dialog = |pass: &str, again: &str| crate::Dialog::PgpBackup {
        account: "sample".into(),
        fingerprint: "AB".into(),
        path: path.clone(),
        passphrase: quatrelle_engine::model::Secret(pass.into()),
        again: quatrelle_engine::model::Secret(again.into()),
    };
    a.ui.dialog = Some(dialog("correct horse", "correct hose"));
    let t = drawn(&ctx, &mut a, &mut tex, "backup-differ");
    assert!(has(&t, "Save a backup of your OpenPGP key") && has(&t, "The two passphrases aren't the same."), "{t:?}");
    assert!(has(&t, "riley-openpgp-backup.asc"), "{t:?}");
    a.ui.dialog = Some(dialog("correct horse", "correct horse"));
    let t = drawn(&ctx, &mut a, &mut tex, "backup-same");
    assert!(!has(&t, "The two passphrases aren't the same."), "{t:?}");
    // Its Debug never says the passphrase.
    assert!(!format!("{:?}", dialog("correct horse", "correct horse")).contains("horse"));
    let _ = std::fs::remove_dir_all(dir);
}
