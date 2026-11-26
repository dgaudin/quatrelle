// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: the To-Do List and the reminders read the account's own mailbox, each message
//! once; the To-Do bar's parts share its height as dragged, kept with the preferences.

use super::*;

/// `view.todoBarShares` keeps the calendar's and People's shares of the bar's height (the tasks
/// take the rest), in percent; a share out of bounds, or two leaving the last part no room, is
/// refused and nothing changes.
#[test]
fn the_todo_bars_parts_keep_their_shares() {
    let mut s = sample_session("todo-bar-shares");
    assert_eq!(s.prefs.todo_bar_shares, None, "the parts as tall as they come, until dragged");
    let r = s.run("view.todoBarShares", &json!({"calendar": 30, "people": 15})).unwrap();
    assert_eq!((r["calendar"].as_u64(), r["people"].as_u64()), (Some(30), Some(15)));
    assert_eq!(s.prefs.todo_bar_shares, Some([30, 15]));
    for bad in [json!({"calendar": 2, "people": 15}), json!({"calendar": 60, "people": 40}), json!({"calendar": "a"})] {
        assert!(s.run("view.todoBarShares", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.prefs.todo_bar_shares, Some([30, 15]), "unchanged by a refusal");
    done(s);
}

/// Only the account's own mailbox rings and is in the To-Do List, as Outlook's come from the
/// user's own store: a message followed in a shared mailbox opened here, in a public folder or
/// in the online archive neither rings nor is listed, while the same in the account's own folder
/// does both (a shared mailbox's many overdue ones filled the bar's forty rows before the
/// account's own).
#[test]
fn a_followed_message_in_a_shared_mailbox_or_a_public_folder_doesnt_ring() {
    let mut s = sample_session("follow-up-shared-reminder");
    let now = s.clock.now;
    let paths = [("Team/Inbox", Some("team@contoso.example")), ("Public Folders/Sales", None), ("Online Archive/Inbox", None), ("Projects", None)];
    for (path, mailbox) in paths {
        s.accounts[0].folders.push(quatrelle_model::Folder {
            account: "sample".into(),
            path: path.into(),
            delimiter: Some('/'),
            selectable: true,
            mailbox: mailbox.map(str::to_string),
            archive: path.starts_with("Online Archive"),
            ..Default::default()
        });
    }
    s.mail_stamp = s.mail_stamp.wrapping_add(1); // as the Folders event that brought them does
    for (n, (path, _)) in paths.iter().enumerate() {
        let mut flags = quatrelle_model::Flags { flagged: true, ..Default::default() };
        flags.follow_up.reminder = now - 60;
        let m = Summary {
            key: MessageKey { account: "sample".into(), folder: (*path).into(), validity: 1, uid: 1 },
            message_id: format!("followed-{n}@x"),
            subject: format!("Followed in {path}"),
            flags,
            ..Default::default()
        };
        let st = quatrelle_store::FolderState { validity: 1, messages: vec![std::sync::Arc::new(m)], ..Default::default() };
        s.cache.insert(("sample".into(), (*path).into()), st);
    }
    let rings: Vec<String> = s.due_reminders(now).into_iter().filter(|r| r.message.is_some()).map(|r| r.summary).collect();
    assert!(rings.iter().any(|r| r == "Followed in Projects"), "the account's own rings: {rings:?}");
    assert!(!rings.iter().any(|r| r == "Followed in Team/Inbox"), "not the shared mailbox's: {rings:?}");
    assert!(!rings.iter().any(|r| r == "Followed in Public Folders/Sales"), "not a public folder's: {rings:?}");
    assert!(!rings.iter().any(|r| r == "Followed in Online Archive/Inbox"), "not the online archive's: {rings:?}");
    let listed: Vec<String> = s.todo("", false).iter().filter_map(|r| r.mail()).map(|m| m.subject.clone()).collect();
    assert!(listed.iter().any(|r| r == "Followed in Projects"), "the account's own is listed: {listed:?}");
    assert!(
        listed.iter().all(|r| r != "Followed in Team/Inbox" && r != "Followed in Public Folders/Sales" && r != "Followed in Online Archive/Inbox"),
        "only the account's own: {listed:?}"
    );
    done(s);
}

/// A message followed under several of Gmail's labels (Inbox, Starred, All Mail) is one message:
/// the To-Do List holds it once, as its reminder rings once, from the Inbox.
#[test]
fn a_message_under_several_labels_is_one_row_of_the_todo_list() {
    let mut s = sample_session("follow-up-labels");
    let paths = [
        ("[Gmail]/All Mail", quatrelle_model::FolderRole::Archive),
        ("[Gmail]/Starred", quatrelle_model::FolderRole::Other),
        ("INBOX", quatrelle_model::FolderRole::Inbox),
        ("[Gmail]/Important", quatrelle_model::FolderRole::Other),
    ];
    for (path, role) in paths {
        if !s.accounts[0].folders.iter().any(|f| f.path == path) {
            s.accounts[0].folders.push(quatrelle_model::Folder {
                account: "sample".into(),
                path: path.into(),
                delimiter: Some('/'),
                role,
                selectable: true,
                ..Default::default()
            });
        }
    }
    s.mail_stamp = s.mail_stamp.wrapping_add(1); // as the Folders event that brought them does
    for (n, (path, _)) in paths.iter().enumerate() {
        let m = Summary {
            key: MessageKey { account: "sample".into(), folder: (*path).into(), validity: 1, uid: 900 + n as u32 },
            message_id: "labelled@x".into(),
            subject: "Under four labels".into(),
            flags: quatrelle_model::Flags { flagged: true, ..Default::default() },
            ..Default::default()
        };
        let k = ("sample".to_string(), (*path).to_string());
        let st = s.cache.entry(k).or_insert_with(|| quatrelle_store::FolderState { validity: 1, ..Default::default() });
        st.messages.push(std::sync::Arc::new(m));
    }
    let rows: Vec<String> =
        s.todo("", false).iter().filter_map(|r| r.mail()).filter(|m| m.subject == "Under four labels").map(|m| m.key.folder.clone()).collect();
    assert_eq!(rows, vec!["INBOX".to_string()], "once, from the Inbox");
    done(s);
}

/// On an account whose folders aren't labels, two messages followed in two folders are two
/// rows even when they carry one Message-ID (a message sent to oneself, or a sender reusing
/// one): neither hides the other.
#[test]
fn two_followed_messages_with_one_message_id_stay_two_rows_off_gmail() {
    let mut s = sample_session("follow-up-same-id");
    assert!(!s.accounts[0].folders.iter().any(|f| f.path.starts_with("[Gmail]")), "not a Gmail account");
    for (n, path) in ["INBOX", "Projects"].iter().enumerate() {
        if !s.accounts[0].folders.iter().any(|f| f.path == *path) {
            s.accounts[0].folders.push(quatrelle_model::Folder {
                account: "sample".into(),
                path: (*path).into(),
                selectable: true,
                ..Default::default()
            });
        }
        let m = Summary {
            key: MessageKey { account: "sample".into(), folder: (*path).into(), validity: 1, uid: 950 + n as u32 },
            message_id: "same@x".into(),
            subject: "Same id twice".into(),
            flags: quatrelle_model::Flags { flagged: true, ..Default::default() },
            ..Default::default()
        };
        let st = s
            .cache
            .entry(("sample".to_string(), (*path).to_string()))
            .or_insert_with(|| quatrelle_store::FolderState { validity: 1, ..Default::default() });
        st.messages.push(std::sync::Arc::new(m));
    }
    s.mail_stamp = s.mail_stamp.wrapping_add(1);
    let rows = s.todo("", false).iter().filter_map(|r| r.mail()).filter(|m| m.subject == "Same id twice").count();
    assert_eq!(rows, 2, "both listed");
    done(s);
}
