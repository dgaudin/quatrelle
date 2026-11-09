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

// ---- Move to, dragging and the notice -------------------------------------------------------

use quatrelle_engine::model::{AccountConfig, AccountRole, Folder, FolderRole, Job, Security, ServerConfig};

type Handed = std::rc::Rc<std::cell::RefCell<Vec<Job>>>;

struct Recorder(Handed);

impl crate::Backend for Recorder {
    fn submit(&mut self, job: Job) {
        self.0.borrow_mut().push(job);
    }

    fn poll(&mut self) -> Vec<MailEvent> {
        Vec::new()
    }
}

fn folder(account: &str, path: &str, role: FolderRole) -> Folder {
    Folder { account: account.into(), path: path.into(), delimiter: Some('/'), role, selectable: true, ..Default::default() }
}

fn message(account: &str, folder: &str, uid: u32, subject: &str) -> Summary {
    let key = MessageKey { account: account.into(), folder: folder.into(), validity: 1, uid };
    Summary {
        key,
        subject: subject.into(),
        message_id: format!("{uid}@studio.example"),
        received: NOW - 3600 * uid as i64,
        date: NOW - 3600 * uid as i64,
        ..Default::default()
    }
}

/// An IMAP account `a1` ("Work") with a message in its Inbox, and the Archives account `a2` with
/// that account's folder and its Inbox under it holding one archived message; the Inbox open.
fn archiving_app(name: &str) -> (MailApp, std::path::PathBuf, Handed) {
    let dir = std::env::temp_dir().join(format!("quatrelle-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let server = ServerConfig { host: "imap.studio.example".into(), port: 993, security: Security::Tls };
    let work = AccountConfig {
        id: "a1".into(),
        name: "Work".into(),
        email: "me@studio.example".into(),
        imap: server.clone(),
        smtp: server,
        ..Default::default()
    };
    let archives =
        AccountConfig { id: "a2".into(), name: "Archives".into(), email: "Archives".into(), role: Some(AccountRole::Archives), ..Default::default() };
    store.save_accounts(&[work, archives]).unwrap();
    store
        .save_folders(
            "a1",
            &[folder("a1", "INBOX", FolderRole::Inbox), folder("a1", "Projects", FolderRole::Other), folder("a1", "Reports", FolderRole::Other)],
        )
        .unwrap();
    store.save_folders("a2", &[folder("a2", "Work", FolderRole::Other), folder("a2", "Work/INBOX", FolderRole::Other)]).unwrap();
    for (account, path, subject) in [("a1", "INBOX", "Plans"), ("a2", "Work/INBOX", "Old plans")] {
        let mut st = store.load_folder(account, path).unwrap();
        st.validity = 1;
        st.messages.push(message(account, path, 1, subject));
        if (account, path) == ("a1", "INBOX") {
            // A year and more old: what archiving by age finds.
            let mut old = message(account, path, 2, "Ancient plans");
            old.received = NOW - 400 * 86_400;
            old.date = old.received;
            st.messages.push(old);
        }
        store.save_folder(account, path, &st).unwrap();
    }
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    let handed = Handed::default();
    let mut a = MailApp::new(s, Box::new(Recorder(handed.clone())), Box::new(NullServices(NOW)));
    a.session.take_jobs();
    a.session.run("folder.open", &json!({"account": "a1", "folder": "INBOX"})).unwrap();
    (a, dir, handed)
}

fn archive_jobs(handed: &Handed) -> Vec<(String, String)> {
    handed
        .borrow()
        .iter()
        .filter_map(|j| match j {
            Job::Archive { account, to_account, .. } => Some((account.clone(), to_account.clone())),
            _ => None,
        })
        .collect()
}

fn put_back_jobs(handed: &Handed) -> Vec<(String, String)> {
    handed
        .borrow()
        .iter()
        .filter_map(|j| match j {
            Job::PutBack { account, to_folder, .. } => Some((account.clone(), to_folder.clone())),
            _ => None,
        })
        .collect()
}

fn select(a: &mut MailApp, subject: &str) -> MessageKey {
    let key = a.session.list.iter().find(|m| m.subject == subject).map(|m| m.key.clone()).unwrap();
    a.session.run("list.select", &json!({"key": key.to_id()})).unwrap();
    key
}

#[test]
fn move_to_lists_the_archives_folders() {
    let (mut a, dir, handed) = archiving_app("move-to-archives");
    let ctx = egui::Context::default();
    select(&mut a, "Plans");
    a.ui.dialog = Some(Dialog::MoveTo { filter: String::new() });
    // IMAP mail: its own account's folders, then the Archives account's, under its name.
    let at = settled_text_at(&ctx, &mut a, "Projects", SIZE).expect("the account's own folder");
    assert!(settled_text_at(&ctx, &mut a, "Archives", SIZE).is_some(), "the Archives account's name");
    let shelf = settled_text_at(&ctx, &mut a, "Work", SIZE).expect("the folder archived mail of Work is kept in");
    assert!(at.y < shelf.y, "below the account's own folders");
    // Picking one archives (the worker puts it where it came from).
    click_at(&ctx, &mut a, shelf, 1, SIZE);
    assert_eq!(archive_jobs(&handed), vec![("a1".to_string(), "a2".to_string())]);
    assert!(a.ui.dialog.is_none(), "the dialog closes");
    // Archived mail: the Archives account's own folders, then the IMAP accounts' folders.
    handed.borrow_mut().clear();
    a.session.run("folder.open", &json!({"account": "a2", "folder": "Work/INBOX"})).unwrap();
    select(&mut a, "Old plans");
    a.ui.dialog = Some(Dialog::MoveTo { filter: String::new() });
    let projects = settled_text_at(&ctx, &mut a, "Projects", SIZE).expect("a folder of the server");
    assert!(settled_text_at(&ctx, &mut a, "me@studio.example", SIZE).is_some(), "under the account that has it");
    click_at(&ctx, &mut a, projects, 1, SIZE);
    assert_eq!(put_back_jobs(&handed), vec![("a1".to_string(), "Projects".to_string())]);
    let _ = std::fs::remove_dir_all(dir);
}

/// A real drag: pressed on the row of `subject`, moved over the text `onto` in steps, released
/// there (the folder pane's row, or an account's heading).
fn drag(ctx: &egui::Context, a: &mut MailApp, subject: &str, onto: &str) {
    select(a, subject);
    drag_text(ctx, a, subject, onto);
}

/// The same drag from any text on screen.
fn drag_text(ctx: &egui::Context, a: &mut MailApp, text: &str, onto: &str) {
    let from = settled_text_at(ctx, a, text, SIZE).expect("what is dragged");
    let at = settled_text_at(ctx, a, onto, SIZE).expect("the drop target");
    let press = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    for _ in 0..20 {
        frame(ctx, a, vec![]);
    }
    frame(ctx, a, vec![Event::PointerMoved(from)]);
    frame(ctx, a, vec![press(from, true)]);
    for step in 1..=8 {
        frame(ctx, a, vec![Event::PointerMoved(from + (at - from) * (step as f32 / 8.0))]);
    }
    frame(ctx, a, vec![press(at, false)]);
    frame(ctx, a, vec![]);
}

#[test]
fn dropping_on_archives_archives() {
    let ctx = egui::Context::default();
    // On a folder in the Archives account.
    let (mut a, dir, handed) = archiving_app("drop-on-archives");
    drag(&ctx, &mut a, "Plans", "Work");
    assert_eq!(archive_jobs(&handed), vec![("a1".to_string(), "a2".to_string())]);
    let _ = std::fs::remove_dir_all(dir);
    // On the account's own name.
    let (mut a, dir, handed) = archiving_app("drop-on-archives-head");
    drag(&ctx, &mut a, "Plans", "Archives");
    assert_eq!(archive_jobs(&handed), vec![("a1".to_string(), "a2".to_string())], "{:?}", handed.borrow());
    let _ = std::fs::remove_dir_all(dir);
    // Archived mail dropped on a folder of the server is put back there.
    let (mut a, dir, handed) = archiving_app("drop-put-back");
    a.session.run("folder.open", &json!({"account": "a2", "folder": "Work/INBOX"})).unwrap();
    drag(&ctx, &mut a, "Old plans", "Projects");
    assert_eq!(put_back_jobs(&handed), vec![("a1".to_string(), "Projects".to_string())]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_notice_offers_undo() {
    let (mut a, dir, handed) = archiving_app("notice-undo");
    let ctx = egui::Context::default();
    let key = select(&mut a, "Plans");
    a.session.run("mail.move", &json!({"account": "archives", "folder": "-"})).unwrap();
    a.session.take_jobs();
    let to = MessageKey { account: "a2".into(), folder: "Work/INBOX".into(), validity: 1, uid: 9 };
    a.session.apply(MailEvent::Archived { account: "a1".into(), moved: vec![(key, to.clone())], kept: Vec::new() });
    let undo = settled_text_at(&ctx, &mut a, "Undo (Ctrl+Z)", SIZE);
    assert!(undo.is_some(), "the notice has Undo: {:?}", a.ui.toast);
    assert!(settled_text_at(&ctx, &mut a, "1 messages archived on this computer", SIZE).is_some());
    // Clicked, the mail goes back up.
    click_at(&ctx, &mut a, undo.unwrap_or_default(), 1, SIZE);
    assert_eq!(put_back_jobs(&handed), vec![("a1".to_string(), "INBOX".to_string())]);
    // A notice without it (a refusal) shows none.
    a.session.apply(MailEvent::Archived { account: "a1".into(), moved: Vec::new(), kept: vec![(to, "too many at once".into())] });
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(settled_text_at(&ctx, &mut a, "Undo (Ctrl+Z)", SIZE).is_none() || a.session.undo.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_archives_account_is_named_in_the_folder_pane() {
    let (mut a, dir, _) = archiving_app("pane-name");
    let ctx = egui::Context::default();
    assert!(settled_text_at(&ctx, &mut a, "Archives", SIZE).is_some());
    assert!(settled_text_at(&ctx, &mut a, "Archives (sample)", SIZE).is_none(), "not the sample mailbox");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dragging_a_folder_onto_a_folder_moves_it() {
    let ctx = egui::Context::default();
    let (mut a, dir, _handed) = archiving_app("drag-folder");
    drag_text(&ctx, &mut a, "Projects", "Reports");
    let paths: Vec<String> = a.session.account("a1").map(|x| x.folders.iter().map(|f| f.path.clone()).collect()).unwrap_or_default();
    assert!(paths.iter().any(|p| p == "Reports/Projects"), "{paths:?}");
    let _ = std::fs::remove_dir_all(dir);
}

// ---- By age: the bar ---------------------------------------------------------------------

/// `archiving_app` (its Inbox holds an old message) with the account set to archive after a year,
/// connected, and looked through.
fn by_age_app(name: &str, ask: bool) -> (MailApp, std::path::PathBuf, Handed) {
    let (mut a, dir, handed) = archiving_app(name);
    if ask {
        a.session.run("archive.settings", &json!({"account": "a1", "afterMonths": 12})).unwrap();
    }
    a.session.apply(MailEvent::Status { account: "a1".into(), status: quatrelle_engine::model::SyncStatus::Online { at: 1 } });
    (a, dir, handed)
}

fn age_words() -> &'static str {
    "1 messages older than 12 months in Work can go to Archives."
}

#[test]
fn the_bar_offers_to_archive_by_age() {
    let ctx = egui::Context::default();
    // Off (the default): no bar.
    let (mut a, dir, _) = by_age_app("age-bar-off", false);
    assert!(settled_text_at(&ctx, &mut a, age_words(), SIZE).is_none(), "off by default");
    let _ = std::fs::remove_dir_all(dir);
    // On: the bar, its two buttons; Not now puts it away, and it stays away.
    let (mut a, dir, handed) = by_age_app("age-bar-not-now", true);
    assert!(settled_text_at(&ctx, &mut a, age_words(), SIZE).is_some(), "the bar");
    let not_now = settled_text_at(&ctx, &mut a, "Not now", SIZE).expect("Not now");
    click_at(&ctx, &mut a, not_now, 1, SIZE);
    assert!(settled_text_at(&ctx, &mut a, age_words(), SIZE).is_none(), "put away");
    a.session.apply(MailEvent::Status { account: "a1".into(), status: quatrelle_engine::model::SyncStatus::Online { at: 2 } });
    assert!(settled_text_at(&ctx, &mut a, age_words(), SIZE).is_none(), "not asked again today");
    assert!(archive_jobs(&handed).is_empty(), "nothing archived without Archive");
    let _ = std::fs::remove_dir_all(dir);
    // Archive: the job goes, and the progress line shows while it runs.
    let (mut a, dir, handed) = by_age_app("age-bar-archive", true);
    assert!(settled_text_at(&ctx, &mut a, age_words(), SIZE).is_some());
    let shelf = settled_text_at(&ctx, &mut a, age_words(), SIZE).unwrap();
    // The ribbon has an Archive button too: the one in the bar is just under its words.
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    let at = shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) if t.galley.text() == "Archive" => Some(t.pos + t.galley.rect.center().to_vec2()),
            _ => None,
        })
        .find(|p| p.y > shelf.y && p.y - shelf.y < 60.0)
        .expect("the bar's Archive button");
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert_eq!(archive_jobs(&handed), vec![("a1".to_string(), a.session.archives_id().unwrap())]);
    assert!(settled_text_at(&ctx, &mut a, "Archiving: 0 of 1", SIZE).is_some(), "the progress line");
    assert!(settled_text_at(&ctx, &mut a, age_words(), SIZE).is_none(), "the bar is gone");
    let _ = std::fs::remove_dir_all(dir);
    // Connected no more: no bar.
    let (mut a, dir, _) = by_age_app("age-bar-offline", true);
    a.session.apply(MailEvent::Status { account: "a1".into(), status: quatrelle_engine::model::SyncStatus::Offline });
    assert!(settled_text_at(&ctx, &mut a, age_words(), SIZE).is_none(), "offline");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_account_settings_offer_archiving_by_age() {
    let (mut a, dir) = app("archive-by-age-box");
    let ctx = egui::Context::default();
    let imap =
        a.session.run("account.add", &json!({"email": "zoe@studio.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let words = "Archive mail older than";
    info_texts(&ctx, &mut a, &imap);
    let at = settled_text_at(&ctx, &mut a, words, SIZE).expect("the box's words");
    assert!(a.session.account(&imap).unwrap().config.archive_after_months.is_none(), "off by default");
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert_eq!(a.session.account(&imap).unwrap().config.archive_after_months, Some(12), "ticked: a year");
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert!(a.session.account(&imap).unwrap().config.archive_after_months.is_none(), "and unticked");
    let _ = std::fs::remove_dir_all(dir);
}

mod online_archive;
