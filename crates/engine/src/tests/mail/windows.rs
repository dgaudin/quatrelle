// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a message's own window acts on its message (its Reply answers it, its Delete
//! deletes it, its tags mark it), whatever the main window has selected and shows, which stays
//! as it was, as Outlook's message window acts on its item.

use super::*;

/// The sample with its first message in a window of its own and its second selected and read in
/// the main window.
fn two(name: &str) -> (Session, MessageKey, MessageKey) {
    let mut s = sample_session(name);
    let a = s.list[0].key.clone();
    let b = s.list[1].key.clone();
    s.run("message.popOut", &json!({"key": a.to_id()})).unwrap();
    s.run("list.open", &json!({"key": b.to_id()})).unwrap();
    (s, a, b)
}

/// What the main window has selected, and the message its reading pane shows.
fn main_window(s: &Session) -> (Vec<MessageKey>, Option<MessageKey>) {
    (s.selection.clone(), s.reading.as_ref().map(|r| r.key.clone()))
}

#[test]
fn a_windows_reply_answers_its_own_message() {
    for id in ["mail.reply", "mail.replyAll", "mail.forward"] {
        let (mut s, a, b) = two("window-reply");
        s.run_in_window(&a, id, &json!({})).unwrap();
        let c = s.compose.as_ref().expect("the answer");
        assert_eq!(c.source.as_ref(), Some(&a), "{id} answers the window's message");
        assert!(c.window, "{id}: in a window of its own, as Outlook answers from an open message");
        assert_eq!(main_window(&s), (vec![b.clone()], Some(b)), "{id}: the main window as it was");
        assert_eq!(s.windows.len(), 1, "{id}: the message's window stays open");
        done(s);
    }
}

#[test]
fn a_windows_delete_deletes_its_message_and_closes_it() {
    let (mut s, a, b) = two("window-delete");
    let subject = s.summary(&a).unwrap().subject.clone();
    s.run_in_window(&a, "mail.delete", &json!({})).unwrap();
    assert!(s.summary(&a).is_none(), "gone from the list");
    let trash = s.store.as_ref().unwrap().load_folder(sample::SAMPLE_ACCOUNT, "Trash").unwrap();
    assert!(trash.messages.iter().any(|m| m.subject == subject), "in Deleted Items");
    assert_eq!(main_window(&s), (vec![b.clone()], Some(b)), "the main window as it was");
    assert!(s.windows.is_empty(), "its window closes, its message gone");
    done(s);
}

#[test]
fn a_windows_tags_mark_only_its_message() {
    let (mut s, a, b) = two("window-tags");
    let b_flags = s.summary(&b).unwrap().flags.clone();
    s.run_in_window(&a, "mail.markUnread", &json!({})).unwrap();
    s.run_in_window(&a, "mail.flag", &json!({"on": true})).unwrap();
    let flags = &s.summary(&a).unwrap().flags;
    assert!(!flags.seen && flags.flagged, "{flags:?}");
    assert_eq!(s.summary(&b).unwrap().flags, b_flags, "the main window's message as it was");
    assert_eq!(main_window(&s), (vec![b.clone()], Some(b)));
    assert_eq!(s.windows.len(), 1, "still open: its message is still there");
    done(s);
}

/// The main window on the same message: what's done to it is done as there (deleted, the next
/// one shows there).
#[test]
fn a_window_on_the_message_read_deletes_as_the_main_window_does() {
    let mut s = sample_session("window-same");
    let a = s.list[0].key.clone();
    let next = s.list[1].key.clone();
    s.run("message.popOut", &json!({"key": a.to_id()})).unwrap();
    s.run("list.open", &json!({"key": a.to_id()})).unwrap();
    s.run_in_window(&a, "mail.delete", &json!({})).unwrap();
    assert_eq!(main_window(&s), (vec![next.clone()], Some(next)));
    assert!(s.windows.is_empty());
    done(s);
}

/// Its message no longer in the list (another folder opened since): what acts on the list's mail
/// can't reach it, and nothing else is filed away in its place; Reply still answers it.
#[test]
fn a_window_whose_message_left_the_list_files_nothing_else_away() {
    let (mut s, a, _) = two("window-elsewhere");
    s.run("folder.open", &json!({"account": "sample", "folder": "Archive"})).unwrap();
    let shown: Vec<MessageKey> = s.list.iter().map(|m| m.key.clone()).collect();
    let first = shown.first().cloned().expect("the Archive has mail");
    s.run("list.open", &json!({"key": first.to_id()})).unwrap();
    let main = main_window(&s);
    assert!(matches!(s.run_in_window(&a, "mail.archive", &json!({})), Err(CmdError::Disabled(_))));
    assert!(matches!(s.run_in_window(&a, "mail.delete", &json!({})), Err(CmdError::Disabled(_))));
    assert_eq!(s.list.iter().map(|m| m.key.clone()).collect::<Vec<_>>(), shown, "nothing moved");
    assert_eq!(main_window(&s), main);
    s.run_in_window(&a, "mail.reply", &json!({})).unwrap();
    assert_eq!(s.compose.as_ref().and_then(|c| c.source.clone()), Some(a));
    assert_eq!(main_window(&s), main);
    done(s);
}

/// While a window's ribbon is drawn, its message stands in as the one selected and read (what's
/// offered is worked out for it); the main window's come back after. No window, no stand-in.
#[test]
fn a_windows_message_stands_in_and_the_main_windows_comes_back() {
    let (mut s, a, b) = two("window-scope");
    let enabled = |s: &Session, id: &str| s.registry().get(id).is_some_and(|c| (c.enabled)(s).is_none());
    let scope = s.enter_window(&a).expect("its window is open");
    assert_eq!(main_window(&s), (vec![a.clone()], Some(a.clone())));
    assert!(enabled(&s, "mail.reply") && enabled(&s, "mail.archive"));
    s.leave_window(scope);
    assert_eq!(main_window(&s), (vec![b.clone()], Some(b.clone())));
    assert!(s.enter_window(&b).is_none(), "no window shows it");
    assert!(s.run_in_window(&b, "mail.reply", &json!({})).is_err());
    assert!(s.compose.is_none());
    done(s);
}

/// The main window's message still being got ready on another thread: a window's command
/// doesn't drop it (the pane would stay empty).
#[test]
fn a_windows_command_leaves_the_main_windows_message_coming() {
    let mut s = sample_session("window-deferred");
    s.defer_bodies(true);
    let a = s.list[0].key.clone();
    let b = s.list[1].key.clone();
    s.run("message.popOut", &json!({"key": a.to_id()})).unwrap();
    s.run("list.open", &json!({"key": b.to_id()})).unwrap();
    assert!(s.body_waiting(&b));
    s.run_in_window(&a, "mail.flag", &json!({"on": true})).unwrap();
    let shown = s.reading.as_ref().is_some_and(|r| r.key == b && r.parsed.is_some());
    assert!(shown || s.body_waiting(&b), "the main window's message is still coming");
    done(s);
}
