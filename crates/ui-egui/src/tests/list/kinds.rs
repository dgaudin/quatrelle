// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: what a message is or what was done with it, in the list: one icon in the table's
//! icon column and in the two-line view, its tip, and what a screen reader hears. With
//! `QUATRELLE_FRAME_PNG` set to a folder, the frames are also painted there.

use super::follow_up::{Textures, frame_kept};
use super::*;
use crate::icons::Icon;
use crate::panes::list::icon_column;
use quatrelle_engine::model::{ListIcon, MessageKind as K, Smime};

/// One row of the list: what the cache says of it.
struct Case {
    name: &'static str,
    kind: K,
    smime: Smime,
    answered: bool,
    forwarded: bool,
    all: bool,
}

const fn case(name: &'static str, kind: K, smime: Smime, answered: bool, forwarded: bool, all: bool) -> Case {
    Case { name, kind, smime, answered, forwarded, all }
}

const CASES: [Case; 7] = [
    case("not-delivered", K::NotDelivered, Smime::None, false, false, false),
    case("invitation-forwarded", K::MeetingRequest, Smime::None, false, true, false),
    case("encrypted-replied", K::Plain, Smime::Encrypted, true, false, false),
    case("signed", K::Plain, Smime::Signed, false, false, false),
    case("replied-all", K::Plain, Smime::None, true, false, true),
    case("meeting-canceled", K::MeetingCanceled, Smime::None, false, false, false),
    case("ordinary", K::Plain, Smime::None, false, false, false),
];

/// The first rows of the list set as `CASES` say, as a sync leaves them in the cache.
fn seeded(a: &mut MailApp) -> Vec<Summary> {
    let first: Vec<quatrelle_engine::model::MessageKey> = a.session.list.iter().take(CASES.len()).map(|m| m.key.clone()).collect();
    let (account, folder) = (first[0].account.clone(), first[0].folder.clone());
    if let Some(store) = a.session.store.as_ref() {
        let mut fs = store.load_folder(&account, &folder).unwrap();
        for (key, c) in first.iter().zip(&CASES) {
            if let Some(m) = fs.messages.iter_mut().find(|m| m.key == *key) {
                (m.kind, m.smime, m.flags.answered, m.flags.forwarded) = (c.kind, c.smime, c.answered, c.forwarded);
                if c.all {
                    m.flags.keywords.push(quatrelle_engine::model::REPLIED_ALL.to_string());
                }
            }
        }
        store.save_folder(&account, &folder, &fs).unwrap();
    }
    a.session.apply(quatrelle_engine::model::Event::FolderChanged { account, folder });
    let rows: Vec<Summary> = first.iter().filter_map(|k| a.session.summary(k).cloned()).collect();
    assert_eq!(rows.len(), CASES.len());
    rows
}

/// What a row says to a screen reader, without who, the subject and when.
fn tail(shown: &[String], m: &Summary) -> String {
    let spoken = format!("{}, {}, ", m.from.display(), m.subject);
    let row = shown.iter().find(|l| l.starts_with(&spoken)).cloned().unwrap_or_default();
    // Who, the subject and the time come first; the rest are the marks.
    row.split(", ").skip(spoken.matches(", ").count() + 1).collect::<Vec<_>>().join(", ")
}

/// Every kind of row says what it is aloud, and an ordinary one says nothing of the sort.
#[test]
fn every_row_says_what_it_is() {
    let (mut a, dir) = app("kinds-said");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut tex = Textures::default();
    let pictures = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    a.run("view.listLayout", json!({"value": "table"}));
    frame_kept(&ctx, &mut a, &mut tex, None);
    let rows = seeded(&mut a);
    let words: [&[&str]; 7] = [
        &["Not delivered"],
        &["Meeting request", "Forwarded"],
        &["Replied", "Encrypted"],
        &["Signed"],
        &["Replied to all"],
        &["Meeting canceled"],
        &[],
    ];
    for dark in [false, true] {
        a.ui.dark = Some(dark);
        for _ in 0..3 {
            frame_kept(&ctx, &mut a, &mut tex, None);
        }
        let png = pictures.as_ref().map(|d| d.join(format!("kinds-table-{}.png", if dark { "dark" } else { "light" })));
        let shown = frame_kept(&ctx, &mut a, &mut tex, png);
        for ((m, c), said) in rows.iter().zip(&CASES).zip(words) {
            let marks = tail(&shown, m);
            for w in said {
                assert!(marks.contains(w), "{}: {w} in {marks:?}", c.name);
            }
            if said.is_empty() {
                for w in ["Not delivered", "Meeting", "Replied", "Forwarded", "Encrypted", "Signed"] {
                    assert!(!marks.contains(w), "{}: {w} in {marks:?}", c.name);
                }
            }
        }
    }
    // The tip under the pointer says everything, in the order of what matters.
    assert_eq!(icon_column::said(&a.session, &rows[1]), "Meeting request · Forwarded");
    let _ = std::fs::remove_dir_all(dir);
}

/// The icon each row shows and its colour: red only for mail not delivered and a meeting
/// canceled; nothing for an ordinary message.
#[test]
fn one_icon_and_its_colour() {
    let (mut a, dir) = app("kinds-icon");
    let rows = seeded(&mut a);
    for dark in [false, true] {
        let t = crate::theme::Tokens::for_dark(dark);
        let want = [
            Some((Icon::NotDelivered, t.danger)),
            Some((Icon::Calendar, t.dim)),
            Some((Icon::Reply, t.dim)),
            Some((Icon::Seal, t.dim)),
            Some((Icon::ReplyAll, t.dim)),
            Some((Icon::MeetingCanceled, t.danger)),
            None,
        ];
        for (m, want) in rows.iter().zip(want) {
            let got = a.session.list_icon(m, true).map(|i| (icon_column::icon_of(i), icon_column::color_of(i, &t)));
            assert_eq!(got, want, "{}", m.subject);
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// The two-line view has its own marks for encrypted and signed mail, so its one icon never
/// doubles them; the frame draws, the pointer on a row included.
#[test]
fn the_two_line_view_never_doubles_the_smime_mark() {
    let (mut a, dir) = app("kinds-two-lines");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut tex = Textures::default();
    let pictures = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    a.run("view.listLayout", json!({"value": "cards"}));
    frame_kept(&ctx, &mut a, &mut tex, None);
    let rows = seeded(&mut a);
    assert_eq!(a.session.list_icon(&rows[3], false), None);
    assert_eq!(a.session.list_icon(&rows[2], false), Some(ListIcon::Replied));
    for dark in [false, true] {
        a.ui.dark = Some(dark);
        for _ in 0..3 {
            frame_kept(&ctx, &mut a, &mut tex, None);
        }
        let png = pictures.as_ref().map(|d| d.join(format!("kinds-cards-{}.png", if dark { "dark" } else { "light" })));
        frame_kept(&ctx, &mut a, &mut tex, png);
    }
    // The pointer on a row: its marks go, and nothing breaks.
    let at = egui::pos2(300.0, 200.0);
    let hover = vec![egui::Event::PointerMoved(at)];
    let (shown, _) = super::follow_up::frame_with(&ctx, &mut a, &mut tex, hover, None);
    assert!(!shown.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}
