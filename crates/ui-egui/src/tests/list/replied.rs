// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a message replied to or forwarded, in the list (its arrow) and the reading pane
//! (when). With `QUATRELLE_FRAME_PNG` set to a folder, the frames are also painted there.

use super::follow_up::{Textures, frame_kept};
use super::*;

/// Replied to (on a day Exchange says), forwarded, and replied to with no day known (IMAP): the
/// row says so, and the reading pane says when, or only that it was.
#[test]
fn a_message_replied_to_or_forwarded_shows_its_arrow_and_when() {
    let (mut a, dir) = app("replied");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut tex = Textures::default();
    let pictures = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let picture = |name: &str| pictures.as_ref().map(|d| d.join(format!("{name}.png")));
    frame_kept(&ctx, &mut a, &mut tex, None);
    let when = a.session.clock.now - 26 * 3600;
    let marks = [(true, false, when), (false, true, when - 3600), (true, false, 0)];
    // As a sync leaves them in the cache.
    let first: Vec<quatrelle_engine::model::MessageKey> = a.session.list.iter().take(3).map(|m| m.key.clone()).collect();
    let (account, folder) = (first[0].account.clone(), first[0].folder.clone());
    if let Some(store) = a.session.store.as_ref() {
        let mut fs = store.load_folder(&account, &folder).unwrap();
        for (key, (answered, forwarded, acted)) in first.iter().zip(marks) {
            if let Some(m) = fs.messages.iter_mut().find(|m| m.key == *key) {
                (m.flags.answered, m.flags.forwarded, m.flags.acted) = (answered, forwarded, acted);
            }
        }
        store.save_folder(&account, &folder, &fs).unwrap();
    }
    a.session.apply(quatrelle_engine::model::Event::FolderChanged { account, folder });
    let shown_rows: Vec<Summary> = first.iter().filter_map(|k| a.session.summary(k).cloned()).collect();
    assert_eq!(shown_rows.len(), 3);
    let date = |t: i64| crate::i18n::date(&quatrelle_engine::model::dates::full_time(t, &a.session.clock));
    let said = [
        (format!("You replied to this message on {}.", date(when)), "Replied", "replied"),
        (format!("You forwarded this message on {}.", date(when - 3600)), "Forwarded", "forwarded"),
        ("You replied to this message.".to_string(), "Replied", "replied-no-day"),
    ];
    for (m, (line, word, name)) in shown_rows.iter().zip(said) {
        a.run("list.open", json!({"key": m.key.to_id()}));
        for (dark, theme) in [(Some(false), "light"), (Some(true), "dark")] {
            a.ui.dark = dark;
            for _ in 0..3 {
                frame_kept(&ctx, &mut a, &mut tex, None);
            }
            frame_kept(&ctx, &mut a, &mut tex, picture(&format!("{name}-{theme}")));
        }
        let shown = frame_kept(&ctx, &mut a, &mut tex, None);
        let spoken = format!("{}, {}, ", m.from.display(), m.subject);
        let row = shown.iter().find(|l| l.starts_with(&spoken)).cloned().unwrap_or_default();
        assert!(row.contains(word), "{row}");
        assert!(!row.contains(if word == "Replied" { "Forwarded" } else { "Replied" }), "{row}");
        assert!(shown.contains(&line), "{line:?} in {shown:?}");
    }
    // A message neither: no such line.
    let plain = a.session.list.iter().find(|m| !m.flags.answered && !m.flags.forwarded).map(|m| m.key.to_id()).unwrap();
    a.run("list.open", json!({"key": plain}));
    let shown = frame_kept(&ctx, &mut a, &mut tex, None);
    assert!(!shown.iter().any(|l| l.starts_with("You replied") || l.starts_with("You forwarded")), "{shown:?}");
    let _ = std::fs::remove_dir_all(dir);
}
