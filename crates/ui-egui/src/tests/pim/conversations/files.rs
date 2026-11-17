// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a file someone shared in a conversation. Its card offers "Open" (asked of the engine,
//! not again while it downloads, tried again when it failed, each reason said), and once the
//! engine hands the saved file over the window opens it with the system's app, or shows a file
//! that could run a program in its folder; the name in the notice is drawn plain.

use quatrelle_engine::model::pim::teams::{Attachment, ChatMessage, FileOutcome, FileWhy};

use super::chat::*;
use super::*;
use crate::Action;

const G: &str = "00000000-0000-0000-0000-00000000f001";

/// Services that remember the paths opened.
struct Opens(Rc<RefCell<Vec<std::path::PathBuf>>>);

impl crate::Services for Opens {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn open_path(&mut self, path: &std::path::Path) -> Result<(), String> {
        self.0.borrow_mut().push(path.to_path_buf());
        Ok(())
    }
}

/// A message `m1` of Ana Lima with the file `name` on the user's own drive.
fn with_file(name: &str) -> ChatMessage {
    let mut m = said("m1", user("u-ana", "Ana Lima"), NOW, para(vec![words("The file")]));
    m.attachments = vec![Attachment::File {
        id: G.into(),
        name: name.into(),
        url: format!("https://contoso-my.sharepoint.com/personal/riley/{name}"),
        size: Some(2048),
    }];
    m
}

fn file_app(test: &str, name: &str) -> (MailApp, std::path::PathBuf, Rc<RefCell<Vec<std::path::PathBuf>>>) {
    let (mut a, dir, _) = chat_app(test, group_row(), TEAMS_LINK, vec![with_file(name)]);
    let opened = Rc::new(RefCell::new(Vec::new()));
    a.services = Box::new(Opens(opened.clone()));
    (a, dir, opened)
}

/// The engine asked for the file and answered `outcome`.
fn answered(a: &mut MailApp, outcome: FileOutcome) {
    a.run("teams.openFile", json!({"chat": CHAT, "message": "m1", "attachment": G}));
    a.session.teams_apply(teams::Event::File { chat: CHAT.into(), message: "m1".into(), attachment: G.into(), outcome });
}

/// A file of `name` in the folder files are handed to other apps from.
fn saved_file(name: &str) -> String {
    let path = quatrelle_engine::store::scratch_dir("open").unwrap().join(name);
    std::fs::write(&path, b"x").unwrap();
    path.to_string_lossy().into_owned()
}

fn card(ctx: &egui::Context, a: &mut MailApp) -> Vec<(String, egui::Rect)> {
    settled_at(ctx, a, (1280, 800));
    pane(ctx, a, vec![], (1280, 800)).1
}

#[test]
fn a_file_card_offers_open_and_asks_the_engine() {
    let (mut a, dir, _) = file_app("teams-file-open", "report.pdf");
    let ctx = egui::Context::default();
    let texts = card(&ctx, &mut a);
    let open = rect_of(&texts, |t| t == "Open").expect("Open on the card");
    assert!(has_text(&texts, "Open in browser"), "{texts:?}");
    assert!(open.center().y > 0.0 && rect_of(&texts, |t| t == "Open in browser").unwrap().left() > open.left(), "Open comes first");
    let acts = click_pane(&ctx, &mut a, open.center(), (1280, 800));
    let asked: Vec<_> = acts.iter().filter_map(|x| if let Action::Run("teams.openFile", p) = x { Some(p.clone()) } else { None }).collect();
    assert_eq!(asked, vec![json!({"chat": CHAT, "message": "m1", "attachment": G})]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_file_being_downloaded_says_so_and_cant_be_clicked_again() {
    let (mut a, dir, _) = file_app("teams-file-asked", "report.pdf");
    a.run("teams.openFile", json!({"chat": CHAT, "message": "m1", "attachment": G}));
    let ctx = egui::Context::default();
    let texts = card(&ctx, &mut a);
    assert!(has_text(&texts, "Downloading…"), "{texts:?}");
    assert!(!has_text(&texts, "Open"), "{texts:?}");
    assert!(has_text(&texts, "Open in browser"), "{texts:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_failed_download_says_why_and_can_be_tried_again() {
    let ctx = egui::Context::default();
    let cases = [
        (FileWhy::Refused, Some("HTTP 403 accessDenied"), "Teams refused it (HTTP 403 accessDenied)"),
        (FileWhy::TooBig, None, "It is bigger than Quatrelle downloads (100 MB): open it in the browser"),
        (FileWhy::NoRoom, None, "There isn't enough room on the disk"),
        (FileWhy::Refused, None, "Teams refused it"),
        (FileWhy::Offline, None, "Teams can't be reached"),
        (FileWhy::Stalled, None, "The download was too slow"),
        (FileWhy::Busy, None, "Too many files are being downloaded"),
        (FileWhy::Gone, None, "This file is no longer in the conversation"),
    ];
    for (why, code, said) in cases {
        let (mut a, dir, _) = file_app("teams-file-failed", "report.pdf");
        answered(&mut a, FileOutcome::Failed { why, code: code.map(str::to_string) });
        let texts = card(&ctx, &mut a);
        assert!(has_text(&texts, "This file couldn't be downloaded"), "{why:?}: {texts:?}");
        assert!(has_text(&texts, said), "{why:?}: {texts:?}");
        assert!(has_text(&texts, "Open"), "{why:?}: tried again: {texts:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn a_saved_file_is_opened_by_the_system() {
    let (mut a, dir, opened) = file_app("teams-file-saved", "report.pdf");
    let path = saved_file("ui-t11-report.pdf");
    answered(&mut a, FileOutcome::Saved { path: path.clone() });
    let ctx = egui::Context::default();
    for _ in 0..4 {
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(*opened.borrow(), vec![std::path::PathBuf::from(&path)], "opened once");
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_dir_all(dir);
}

/// H2: a program is never started from a card.
#[test]
fn a_program_is_shown_in_its_folder() {
    let (mut a, dir, opened) = file_app("teams-file-program", "setup.exe");
    let path = saved_file("ui-t11-setup.exe");
    answered(&mut a, FileOutcome::Saved { path: path.clone() });
    let ctx = egui::Context::default();
    for _ in 0..4 {
        frame(&ctx, &mut a, vec![]);
    }
    let folder = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    assert_eq!(*opened.borrow(), vec![folder], "its folder, never the file");
    assert!(
        a.session.notices.iter().any(|n| n.text.contains("This file could run a program: it is shown in its folder, not opened.")),
        "{:?}",
        a.session.notices
    );
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_dir_all(dir);
}

/// The card of a file that could run says so before it is asked for (the mail's sentence).
#[test]
fn the_card_of_a_program_says_it_is_shown_in_its_folder() {
    let ctx = egui::Context::default();
    for (name, said) in [("setup.exe", true), ("report.pdf", false), ("notes", true)] {
        let (mut a, dir, _) = file_app("teams-file-sentence", name);
        let texts = card(&ctx, &mut a);
        let sentence = "This file could run a program: it is shown in its folder, not opened.";
        assert_eq!(has_text(&texts, sentence), said, "{name}: {texts:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn a_name_in_the_notice_is_drawn_plain() {
    let name = format!("a\u{202E}b{}.exe", "x".repeat(300));
    let (mut a, dir, opened) = file_app("teams-file-notice", "setup.exe");
    let folder = quatrelle_engine::store::scratch_dir("open").unwrap();
    let path = saved_file("ui-t11-notice.exe");
    a.teams_saved(quatrelle_engine::TeamsSaved { path: path.clone(), name, open: false });
    let n = a.session.notices.last().map(|n| n.text.clone()).unwrap_or_default();
    assert!(n.contains("This file could run a program: it is shown in its folder, not opened."), "{n:?}");
    assert!(!n.contains('\u{202E}'), "{n:?}");
    assert!(n.chars().filter(|c| *c == 'x').count() <= 80, "{n:?}");
    assert_eq!(*opened.borrow(), vec![folder]);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(dir);
}

/// A saved file removed since (the engine hands it again): said, not a crash and not nothing.
#[test]
fn a_saved_file_that_is_gone_is_said() {
    let (mut a, dir, opened) = file_app("teams-file-gone", "report.pdf");
    let path = saved_file("ui-t11-gone.pdf");
    answered(&mut a, FileOutcome::Saved { path: path.clone() });
    let _ = std::fs::remove_file(&path);
    let ctx = egui::Context::default();
    for _ in 0..4 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(opened.borrow().is_empty(), "nothing handed to the system");
    assert!(a.session.notices.iter().any(|n| n.text.contains("could not be opened")), "{:?}", a.session.notices);
    let _ = std::fs::remove_dir_all(dir);
}

/// A card with Open, one downloading and one failed, light and dark.
#[test]
fn the_cards_in_light_and_dark() {
    let mut messages = Vec::new();
    for (i, name) in ["plan.docx", "figures.xlsx", "photo.png", "setup.exe"].into_iter().enumerate() {
        let mut m = with_file(name);
        m.id = format!("m{}", i + 1);
        m.created = (NOW + i as i64) * 1000;
        if let Some(Attachment::File { id, .. }) = m.attachments.first_mut() {
            *id = format!("{G}-{i}");
        }
        messages.push(m);
    }
    let (mut a, dir, _) = chat_app("teams-file-frames", group_row(), TEAMS_LINK, messages);
    a.run("teams.openFile", json!({"chat": CHAT, "message": "m2", "attachment": format!("{G}-1")}));
    a.run("teams.openFile", json!({"chat": CHAT, "message": "m3", "attachment": format!("{G}-2")}));
    a.session.teams_apply(teams::Event::File {
        chat: CHAT.into(),
        message: "m3".into(),
        attachment: format!("{G}-2"),
        outcome: FileOutcome::Failed { why: FileWhy::Offline, code: None },
    });
    let ctx = egui::Context::default();
    let pictures_dir = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let mut tex = super::super::list::follow_up::Textures::default();
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        a.ui.dark = Some(dark);
        for _ in 0..6 {
            super::super::list::follow_up::frame_with_size(&ctx, &mut a, &mut tex, vec![], None, TALL);
        }
        let png = pictures_dir.as_ref().map(|d| d.join(format!("teams-file-cards-{theme}.png")));
        let (_, shapes) = super::super::list::follow_up::frame_with_size(&ctx, &mut a, &mut tex, vec![], png, TALL);
        let texts = texts_of(&shapes);
        for w in ["Open", "Downloading…", "This file couldn't be downloaded", "Teams can't be reached"] {
            assert!(has_text(&texts, w), "{theme}: {w}: {texts:?}");
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}
