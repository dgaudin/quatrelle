// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: every search the window starts goes a slice a frame, never whole. Picking "Current
//! Mailbox" from the Look in menu searched the owner's whole Exchange mailbox in one call, and
//! the window froze for minutes while its index was built.

use quatrelle_engine::SearchScope;
use quatrelle_engine::model::{AccountConfig, EmailAddress, Folder, FolderRole};

use crate::Action;

use super::*;

/// A mailbox of `per` messages, every other one downloaded with a 16 KB body (what costs the
/// most to index), every hundredth with "needle" in its subject.
fn heavy_app(name: &str, per: u32) -> (MailApp, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("quatrelle-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    store.save_accounts(&[AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
    let folder = Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, total: per, ..Default::default() };
    store.save_folders("a1", &[folder]).unwrap();
    let words: String = (0..2048).map(|i| format!("w{i:05} ")).collect();
    let mut messages = Vec::new();
    for uid in (1..=per).rev() {
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        let subject = if uid.is_multiple_of(100) { format!("needle {uid}") } else { format!("Report {uid}") };
        if uid.is_multiple_of(2) {
            let raw = format!("From: ann@studio.example\r\nSubject: {subject}\r\nContent-Type: text/plain\r\n\r\n{words}\r\n");
            store.write_body(&key, raw.as_bytes()).unwrap();
        }
        let from = EmailAddress::new("Ann Person", "ann@studio.example");
        messages.push(Summary { key, subject, from, date: NOW - 86_400 + i64::from(uid) * 10, ..Default::default() });
    }
    store.save_folder("a1", "INBOX", &FolderState { validity: 1, messages, ..Default::default() }).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    (MailApp::new(s, Box::new(NullBackend), Box::new(NullServices(NOW))), dir)
}

/// Frames until the search is done; how many it took.
fn settle(ctx: &egui::Context, a: &mut MailApp) -> usize {
    let mut frames = 0;
    while a.session.search_pending && frames < 5000 {
        frame(ctx, a, vec![]);
        frames += 1;
    }
    assert!(!a.session.search_pending, "the search must finish");
    frames
}

/// A scope picked from a Look in menu (by the box, in the Search tab, over the list) asked for a
/// whole search. It now goes a slice a frame, the window drawing between them, and finds what a
/// whole one does.
#[test]
fn a_scope_picked_from_a_menu_searches_a_slice_a_frame() {
    let (mut a, dir) = heavy_app("scope-slices", 1200);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let t = std::time::Instant::now();
    a.act(Action::Run("search.run", json!({"query": "needle", "scope": "mailbox"})));
    let first = t.elapsed();
    println!("the menu's call took {first:?}");
    assert!(a.session.search_pending, "the call leaves the rest of the index to the next frames");
    let frames = settle(&ctx, &mut a);
    println!("then {frames} frames");
    assert_eq!(a.session.list.len(), 12);
    assert_eq!(a.session.search.as_ref().map(|(_, s)| *s), Some(SearchScope::Mailbox));
    let _ = std::fs::remove_dir_all(dir);
}

/// A Search Folder made from its dialog (and one opened from the folder pane, the same search)
/// searched whole too. It now goes a slice a frame, and stays the Search Folder while the frames
/// carry it on: everywhere, as a Search Folder looks, not where the box last looked.
#[test]
fn a_search_folder_searches_a_slice_a_frame_and_stays_itself() {
    let (mut a, dir) = heavy_app("folder-slices", 1200);
    let ctx = egui::Context::default();
    a.session.prefs.search_scope = SearchScope::Current;
    frame(&ctx, &mut a, vec![]);
    a.act(Action::Submit("searchFolder.create", json!({"name": "Needles", "query": "needle"})));
    assert!(a.session.search_pending, "the call leaves the rest of the index to the next frames");
    settle(&ctx, &mut a);
    assert_eq!(a.session.open_search_folder_name(), Some("Needles"));
    assert_eq!(a.session.search.as_ref().map(|(_, s)| *s), Some(SearchScope::All));
    assert_eq!(a.session.prefs.search_scope, SearchScope::Current, "the box keeps its own scope");
    assert_eq!(a.session.list.len(), 12);
    let _ = std::fs::remove_dir_all(dir);
}
