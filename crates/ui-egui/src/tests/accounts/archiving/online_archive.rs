// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The online archive on screen: a mailbox of the account, read-only (greyed commands, no drops).

use super::*;

const TOP: &str = "Online Archive";
const PROJECTS: &str = "Online Archive/2019/Projects";
const ADDRESS: &str = "archive@contoso.example";

fn archived(path: &str, total: u32) -> Folder {
    Folder { mailbox: Some(ADDRESS.into()), archive: true, total, ..folder("a1", path, FolderRole::Other) }
}

/// An Exchange account with an Inbox holding "Plans", a shared mailbox, and an online archive
/// (a nameless one: the top folder has the default name) with a Projects folder.
fn exchange_app(name: &str) -> (MailApp, std::path::PathBuf, Handed) {
    let dir = std::env::temp_dir().join(format!("quatrelle-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let config = AccountConfig {
        id: "a1".into(),
        name: "Riley".into(),
        email: "riley@contoso.example".into(),
        username: "riley".into(),
        ews: Some("https://mail.contoso.example/EWS/Exchange.asmx".into()),
        ..Default::default()
    };
    store.save_accounts(&[config]).unwrap();
    let shared = Folder { mailbox: Some("zoe@contoso.example".into()), ..folder("a1", "Zoe Park", FolderRole::Other) };
    store
        .save_folders(
            "a1",
            &[
                folder("a1", "INBOX", FolderRole::Inbox),
                folder("a1", "Sent Items", FolderRole::Sent),
                folder("a1", "Deleted Items", FolderRole::Trash),
                shared,
                archived(TOP, 0),
                archived("Online Archive/2019", 0),
                archived(PROJECTS, 1),
            ],
        )
        .unwrap();
    let mut st = store.load_folder("a1", "INBOX").unwrap();
    st.validity = 1;
    st.messages.push(message("a1", "INBOX", 1, "Plans"));
    store.save_folder("a1", "INBOX", &st).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    let handed = Handed::default();
    let mut a = MailApp::new(s, Box::new(Recorder(handed.clone())), Box::new(NullServices(NOW)));
    a.session.take_jobs();
    a.session.run("folder.open", &json!({"account": "a1", "folder": "INBOX"})).unwrap();
    (a, dir, handed)
}

/// The archive's Projects folder opened, with one message on view and selected.
fn open_projects(a: &mut MailApp) {
    a.session.run("folder.open", &json!({"account": "a1", "folder": PROJECTS})).unwrap();
    let mut st = quatrelle_engine::store::FolderState { validity: 1, ..Default::default() };
    st.messages.push(message("a1", PROJECTS, 1, "Old plans"));
    a.session.store.as_ref().unwrap().save_folder("a1", PROJECTS, &st).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "a1".into(), folder: PROJECTS.into() });
    let key = message("a1", PROJECTS, 1, "").key;
    a.session.run("list.select", &json!({"key": key.to_id()})).unwrap();
}

fn texts(shapes: &[egui::Shape]) -> Vec<String> {
    shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect()
}

/// How `text` is drawn (its colour and how faded), each time it is drawn.
fn colours(shapes: &[egui::Shape], text: &str) -> Vec<String> {
    shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) if t.galley.text() == text => Some(format!(
                "{:?} {:?} {:?} {:?}",
                t.galley.job.sections.first().map(|x| x.format.color),
                t.fallback_color,
                t.override_text_color,
                t.opacity_factor
            )),
            _ => None,
        })
        .collect()
}

fn settle(ctx: &egui::Context, a: &mut MailApp) -> Vec<egui::Shape> {
    let mut last = Vec::new();
    for _ in 0..6 {
        last = frame_shapes(ctx, a, vec![], SIZE);
    }
    last
}

fn right_click(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2) {
    let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
    for _ in 0..20 {
        frame(ctx, a, vec![]);
    }
    frame(ctx, a, vec![Event::PointerMoved(at)]);
    frame(ctx, a, vec![press(true)]);
    frame(ctx, a, vec![press(false)]);
}

/// The archive is one more mailbox of the account, named « Online Archive » when the server
/// gave it no name; it isn't a shared mailbox: none of the shared mailbox's menu (closing it,
/// adding another, making a folder in it).
#[test]
fn the_tree_draws_the_archive_as_a_mailbox_of_the_account() {
    let ctx = egui::Context::default();
    // A shared mailbox's menu, for contrast.
    let (mut a, dir, _) = exchange_app("oa-tree");
    settle(&ctx, &mut a);
    let shared = settled_text_at(&ctx, &mut a, "Zoe Park", SIZE).expect("the shared mailbox");
    right_click(&ctx, &mut a, shared);
    let menu = texts(&settle(&ctx, &mut a));
    assert!(menu.iter().any(|t| t == "Add shared mailbox…"), "{menu:?}");
    let _ = std::fs::remove_dir_all(&dir);
    // The archive's.
    let (mut a, dir, _) = exchange_app("oa-tree-2");
    assert!(texts(&settle(&ctx, &mut a)).iter().any(|t| t == "Projects"), "its folders show under it");
    let head = settled_text_at(&ctx, &mut a, TOP, SIZE).expect("the archive's heading");
    right_click(&ctx, &mut a, head);
    let menu = texts(&settle(&ctx, &mut a));
    assert!(menu.iter().any(|t| t == "Send / Receive"), "{menu:?}");
    for shared_only in ["Add shared mailbox…", "Close this mailbox", "Opened by the server for your access to it", "New folder…"] {
        assert!(!menu.iter().any(|t| t == shared_only), "the archive isn't a shared mailbox: {shared_only}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// With a folder of the archive open, the commands that move or delete are greyed in the ribbon
/// and in a message's right-click menu; the ones that only read or mark stay as they are.
#[test]
fn the_commands_that_change_mail_are_greyed_in_the_archive() {
    let ctx = egui::Context::default();
    let ribbon = ["Archive", "Report junk", "Move to", "Flag"];
    let menu = ["Move to…", "Archive", "Report junk", "Ignore", "Reply", "Mark as unread"];
    let mut seen = Vec::new();
    for archive in [false, true] {
        let (mut a, dir, _) = exchange_app(if archive { "oa-grey-archive" } else { "oa-grey-inbox" });
        if archive {
            open_projects(&mut a);
        } else {
            let key = message("a1", "INBOX", 1, "").key;
            a.session.run("list.select", &json!({"key": key.to_id()})).unwrap();
        }
        let shapes = settle(&ctx, &mut a);
        let bar: Vec<Vec<String>> = ribbon.iter().map(|l| colours(&shapes, l)).collect();
        let row = settled_text_at(&ctx, &mut a, if archive { "Old plans" } else { "Plans" }, SIZE).expect("the message");
        right_click(&ctx, &mut a, row);
        let shapes = settle(&ctx, &mut a);
        let items: Vec<Vec<String>> = menu.iter().map(|l| colours(&shapes, l)).collect();
        assert!(items.iter().all(|c| !c.is_empty()), "the menu is open: {:?}", texts(&shapes));
        seen.push((bar, items));
        let _ = std::fs::remove_dir_all(dir);
    }
    let ((bar_in, menu_in), (bar_oa, menu_oa)) = (&seen[0], &seen[1]);
    for (i, l) in ribbon.iter().enumerate() {
        let same = bar_in[i] == bar_oa[i];
        assert_eq!(same, *l == "Flag", "ribbon {l}: {:?} then {:?}", bar_in[i], bar_oa[i]);
    }
    for (i, l) in menu.iter().enumerate() {
        let same = menu_in[i] == menu_oa[i];
        assert_eq!(same, matches!(*l, "Reply" | "Mark as unread"), "menu {l}: {:?} then {:?}", menu_in[i], menu_oa[i]);
    }
}

/// Mail dragged onto a folder of the archive isn't taken: nothing is asked of the engine, so no
/// refusal comes up either; onto a folder of the mailbox it is.
#[test]
fn a_drop_on_an_archive_folder_is_not_taken() {
    let ctx = egui::Context::default();
    let (mut a, dir, handed) = exchange_app("oa-drop");
    settle(&ctx, &mut a);
    let before = a.session.notices.len();
    drag(&ctx, &mut a, "Plans", "Projects");
    assert_eq!(a.session.notices.len(), before, "{:?}", a.session.notices.last());
    assert!(!handed.borrow().iter().any(|j| matches!(j, Job::Apply { .. })), "{:?}", handed.borrow());
    // Control: a folder of the mailbox takes it.
    drag(&ctx, &mut a, "Plans", "Sent Items");
    assert!(handed.borrow().iter().any(|j| matches!(j, Job::Apply { .. })), "a drop on Sent Items moves it: {:?}", handed.borrow());
    let _ = std::fs::remove_dir_all(dir);
}

/// A refused change is said, once, in a notice (the other commands that can't be done are only
/// logged): from the Delete key and from a command run by the window.
#[test]
fn a_refusal_is_said_in_a_notice() {
    let ctx = egui::Context::default();
    let (mut a, dir, handed) = exchange_app("oa-notice");
    open_projects(&mut a);
    settle(&ctx, &mut a);
    for how in 0..3 {
        a.session.notices.clear();
        match how {
            0 => a.act(crate::Action::DeleteMail { for_good: false }),
            1 => a.act(crate::Action::DeleteMail { for_good: true }),
            _ => a.run("mail.archive", json!({})),
        }
        let said: Vec<(&str, bool)> = a.session.notices.iter().map(|n| (n.text.as_str(), n.error)).collect();
        assert_eq!(said, [(quatrelle_engine::ARCHIVE_READ_ONLY, true)], "{how}");
    }
    assert!(a.ui.dialog.is_none(), "no question about deleting for good what can't be deleted");
    assert!(!handed.borrow().iter().any(|j| matches!(j, Job::Apply { .. })));
    let _ = std::fs::remove_dir_all(dir);
}

/// The Move to dialog lists the folders mail may be moved to: not the archive's, which the
/// engine refuses at the click.
#[test]
fn the_move_dialog_doesnt_offer_the_archive() {
    let ctx = egui::Context::default();
    let (mut a, dir, _) = exchange_app("oa-move-dialog");
    let key = message("a1", "INBOX", 1, "").key;
    a.session.run("list.select", &json!({"key": key.to_id()})).unwrap();
    a.ui.dialog = Some(crate::Dialog::MoveTo { filter: String::new() });
    let shapes = settle(&ctx, &mut a);
    let listed: Vec<String> = texts(&shapes).into_iter().filter(|t| t.starts_with(' ')).collect();
    assert!(listed.iter().any(|t| t.trim() == "Sent Items") || texts(&shapes).iter().any(|t| t == "Sent Items"), "{:?}", texts(&shapes));
    assert!(!listed.iter().any(|t| t.trim() == "Projects" || t.trim() == "2019"), "the archive's folders are offered: {listed:?}");
    let _ = std::fs::remove_dir_all(dir);
}
