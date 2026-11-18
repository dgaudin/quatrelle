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

/// Whether the sample's `folder` holds a message called `subject` (on disk, not only in the list
/// shown).
fn kept_in(s: &Session, folder: &str, subject: &str) -> bool {
    s.store.as_ref().unwrap().load_folder(sample::SAMPLE_ACCOUNT, folder).unwrap().messages.iter().any(|m| m.subject == subject)
}

/// Its message no longer in the list (another folder shown since in the main window): what files
/// mail away acts on it all the same, and nothing of the list on screen moves; Reply answers it.
#[test]
fn a_window_whose_message_left_the_list_still_acts_on_it() {
    let (mut s, a, _) = two("window-elsewhere");
    let subject = s.summary(&a).unwrap().subject.clone();
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let shown: Vec<MessageKey> = s.list.iter().map(|m| m.key.clone()).collect();
    let first = shown.first().cloned().expect("Sent has mail");
    s.run("list.open", &json!({"key": first.to_id()})).unwrap();
    let main = main_window(&s);
    s.run_in_window(&a, "mail.reply", &json!({})).unwrap();
    assert_eq!(s.compose.as_ref().and_then(|c| c.source.clone()), Some(a.clone()));
    s.run("compose.discard", &json!({})).unwrap();
    s.run_in_window(&a, "mail.archive", &json!({})).unwrap();
    assert!(kept_in(&s, "Archive", &subject) && !kept_in(&s, "INBOX", &subject), "archived");
    assert_eq!(s.list.iter().map(|m| m.key.clone()).collect::<Vec<_>>(), shown, "the list on screen as it was");
    assert_eq!(main_window(&s), main);
    assert!(s.windows.is_empty(), "its window closes");
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

/// The sample with its first message in a window of its own and, selected and read in the main
/// window, one of another conversation from another sender (what Ignore and Block sender take in
/// besides the message: its conversation, its sender's mail).
fn apart(name: &str) -> (Session, MessageKey, MessageKey) {
    let mut s = sample_session(name);
    let a = s.list[0].clone();
    let topic = quatrelle_model::thread::base_subject(&a.subject).to_lowercase();
    let b = s
        .list
        .iter()
        .find(|m| !m.from.email.eq_ignore_ascii_case(&a.from.email) && quatrelle_model::thread::base_subject(&m.subject).to_lowercase() != topic)
        .map(|m| m.key.clone())
        .expect("another conversation from someone else");
    s.run("message.popOut", &json!({"key": a.key.to_id()})).unwrap();
    s.run("list.open", &json!({"key": b.to_id()})).unwrap();
    (s, a.key.clone(), b)
}

/// What files mail away from a window takes the window's message, never the one selected in the
/// main window: Junk, Ignore, Block sender, and a Quick Step that answers it then deletes it.
const FILING: [(&str, &str); 4] =
    [("mail.junk", "{}"), ("mail.ignore", "{}"), ("junk.blockSender", "{}"), ("quickStep.run", r#"{"name": "Answer and bin"}"#)];

/// A Quick Step that writes a reply then deletes, as the ready-made "Reply then delete".
fn answer_and_bin(s: &mut Session) {
    s.run("quickStep.create", &json!({"name": "Answer and bin", "actions": [{"do": "delete"}], "write": "reply"})).unwrap();
}

#[test]
fn a_windows_filing_commands_take_its_message_not_the_one_selected() {
    for (id, params) in FILING {
        let (mut s, a, b) = apart("window-filing");
        answer_and_bin(&mut s);
        let b_flags = s.summary(&b).unwrap().flags.clone();
        let params: Value = serde_json::from_str(params).unwrap();
        s.run_in_window(&a, id, &params).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(s.summary(&a).is_none(), "{id}: the window's message filed away");
        assert_eq!(s.summary(&b).map(|m| m.flags.clone()), Some(b_flags), "{id}: the main window's message as it was");
        assert_eq!(main_window(&s), (vec![b.clone()], Some(b.clone())), "{id}");
        assert!(s.windows.is_empty(), "{id}: its window closes");
        if id == "quickStep.run" {
            assert_eq!(s.compose.as_ref().and_then(|c| c.source.clone()), Some(a), "the step answers the window's message");
        }
        done(s);
    }
}

#[test]
fn a_window_whose_message_left_the_list_still_files_it_and_nothing_else() {
    for (id, params) in FILING {
        let (mut s, a, _) = apart("window-filing-elsewhere");
        answer_and_bin(&mut s);
        let subject = s.summary(&a).unwrap().subject.clone();
        s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
        let shown: Vec<MessageKey> = s.list.iter().map(|m| m.key.clone()).collect();
        s.run("list.selectAll", &json!({})).unwrap();
        let main = main_window(&s);
        let params: Value = serde_json::from_str(params).unwrap();
        s.run_in_window(&a, id, &params).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(!kept_in(&s, "INBOX", &subject), "{id}: the window's message filed away");
        assert_eq!(s.list.iter().map(|m| m.key.clone()).collect::<Vec<_>>(), shown, "{id}: nothing of the list moved");
        assert_eq!(main_window(&s), main, "{id}");
        assert!(s.windows.is_empty(), "{id}: its window closes");
        if id == "quickStep.run" {
            assert_eq!(s.compose.as_ref().and_then(|c| c.source.clone()), Some(a), "the step answers the window's message");
        }
        done(s);
    }
}

/// Its message no longer in the list: what marks it marks it all the same (read, flag, category).
#[test]
fn a_window_whose_message_left_the_list_still_marks_it() {
    let (mut s, a, _) = two("window-marks-elsewhere");
    let subject = s.summary(&a).unwrap().subject.clone();
    let category = s.prefs.categories.first().map(|c| c.name.clone()).expect("a category");
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let shown: Vec<MessageKey> = s.list.iter().map(|m| m.key.clone()).collect();
    for (id, params) in
        [("mail.markUnread", json!({})), ("mail.flag", json!({"on": true})), ("mail.categorize", json!({"category": category, "on": true}))]
    {
        s.run_in_window(&a, id, &params).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    let inbox = s.store.as_ref().unwrap().load_folder(sample::SAMPLE_ACCOUNT, "INBOX").unwrap();
    let m = inbox.messages.iter().find(|m| m.subject == subject).expect("still in the Inbox");
    let c = s.prefs.categories.first().unwrap();
    assert!(!m.flags.seen && m.flags.flagged && c.marks(&m.flags), "{:?}", m.flags);
    assert_eq!(s.list.iter().map(|m| m.key.clone()).collect::<Vec<_>>(), shown);
    assert_eq!(s.windows.len(), 1, "still open: its message is still there");
    done(s);
}

/// The main window on Deleted Items, a window's message in the Inbox: Delete moves it to Deleted
/// Items; what the main window shows doesn't make it a delete for good.
#[test]
fn a_windows_delete_goes_by_where_its_message_is() {
    let (mut s, a, _) = two("window-delete-from-trash-view");
    let subject = s.summary(&a).unwrap().subject.clone();
    s.run("folder.open", &json!({"account": "sample", "folder": "Trash"})).unwrap();
    s.run_in_window(&a, "mail.delete", &json!({})).unwrap();
    assert!(kept_in(&s, "Trash", &subject) && !kept_in(&s, "INBOX", &subject), "in Deleted Items, not deleted for good");
    assert!(s.windows.is_empty());
    done(s);
}

/// Its message moved elsewhere meanwhile (deleted from the main window): the window's commands
/// find nothing to act on, and act on nothing else.
#[test]
fn a_window_whose_message_went_elsewhere_files_nothing() {
    let (mut s, a, _) = two("window-gone-elsewhere");
    let subject = s.summary(&a).unwrap().subject.clone();
    s.run("list.open", &json!({"key": a.to_id()})).unwrap();
    s.run("mail.delete", &json!({})).unwrap();
    assert_eq!(s.windows.len(), 1, "the window stays, showing what it showed");
    let shown: Vec<MessageKey> = s.list.iter().map(|m| m.key.clone()).collect();
    let main = main_window(&s);
    for id in ["mail.archive", "mail.delete", "mail.markUnread", "mail.junk"] {
        assert!(matches!(s.run_in_window(&a, id, &json!({})), Err(CmdError::Disabled(_))), "{id}");
    }
    assert_eq!(s.list.iter().map(|m| m.key.clone()).collect::<Vec<_>>(), shown);
    assert_eq!(main_window(&s), main);
    assert!(kept_in(&s, "Trash", &subject) && !kept_in(&s, "Archive", &subject) && !kept_in(&s, "Junk", &subject));
    done(s);
}

/// Each window keeps its own zoom, from the main window's when it opened; the main window keeps
/// its own.
#[test]
fn each_window_keeps_its_own_zoom() {
    let (mut s, a, b) = two("window-zoom");
    s.run("view.zoom", &json!({"percent": 120})).unwrap();
    let c = s.list.iter().map(|m| m.key.clone()).find(|k| *k != a && *k != b).unwrap();
    s.run("message.popOut", &json!({"key": c.to_id()})).unwrap();
    s.run("view.zoom", &json!({"percent": 200, "key": a.to_id()})).unwrap();
    assert_eq!((s.window_zoom(&a), s.window_zoom(&c), s.prefs.reading_zoom), (200, 120, 120));
    s.run("view.zoom", &json!({"percent": 90})).unwrap();
    assert_eq!((s.window_zoom(&a), s.window_zoom(&c), s.prefs.reading_zoom), (200, 120, 90));
    s.run("view.zoom", &json!({"percent": 1000, "key": c.to_id()})).unwrap();
    assert_eq!(s.window_zoom(&c), 400, "within the zooms offered");
    assert!(s.run("view.zoom", &json!({"percent": 150, "key": b.to_id()})).is_err(), "no window shows it");
    assert_eq!(s.prefs.reading_zoom, 90);
    done(s);
}

/// Two windows open: each acts on its own message.
#[test]
fn two_windows_each_act_on_their_own_message() {
    let (mut s, a, b) = two("two-windows");
    let c = s.list.iter().map(|m| m.key.clone()).find(|k| *k != a && *k != b).unwrap();
    s.run("message.popOut", &json!({"key": c.to_id()})).unwrap();
    s.run("list.open", &json!({"key": b.to_id()})).unwrap();
    let flags = |s: &Session, k: &MessageKey| s.summary(k).unwrap().flags.clone();
    let (fa, fb, fc) = (flags(&s, &a), flags(&s, &b), flags(&s, &c));
    s.run_in_window(&a, "mail.markUnread", &json!({})).unwrap();
    s.run_in_window(&c, "mail.flag", &json!({"on": !fc.flagged})).unwrap();
    assert_eq!((flags(&s, &a).seen, flags(&s, &a).flagged), (false, fa.flagged), "the first window's: unread only");
    assert_eq!((flags(&s, &c).seen, flags(&s, &c).flagged), (fc.seen, !fc.flagged), "the second window's: flagged only");
    assert_eq!(flags(&s, &b), fb);
    s.run_in_window(&c, "mail.delete", &json!({})).unwrap();
    assert!(s.summary(&c).is_none() && s.summary(&a).is_some());
    assert_eq!(s.windows.iter().map(|w| w.key.clone()).collect::<Vec<_>>(), vec![a.clone()], "the other window stays");
    assert_eq!(main_window(&s), (vec![b.clone()], Some(b)));
    done(s);
}

/// The main window's selection holds several messages, the window's among them: only the
/// window's message is acted on, and the selection comes back (less it, once it's gone).
#[test]
fn a_window_whose_message_is_among_several_selected_acts_on_it_alone() {
    let (mut s, a, b) = two("window-among-selected");
    s.run("list.select", &json!({"key": a.to_id(), "add": true})).unwrap();
    assert!(s.selection.contains(&a) && s.selection.contains(&b));
    let main = main_window(&s);
    let fb = s.summary(&b).unwrap().flags.clone();
    s.run_in_window(&a, "mail.markUnread", &json!({})).unwrap();
    assert!(!s.summary(&a).unwrap().flags.seen);
    assert_eq!(s.summary(&b).unwrap().flags, fb, "the other selected message as it was");
    assert_eq!(main_window(&s), main, "the selection as it was");
    s.run_in_window(&a, "mail.delete", &json!({})).unwrap();
    assert!(s.summary(&a).is_none() && s.summary(&b).is_some(), "only the window's message deleted");
    assert_eq!(main_window(&s), (vec![b.clone()], main.1), "the selection less the message gone, the message read as it was");
    assert!(s.windows.is_empty());
    done(s);
}

/// A reply being written in the reading pane is left as it is while a window's buttons are used.
#[test]
fn a_reply_written_in_the_pane_is_left_as_it_is() {
    let (mut s, a, b) = apart("window-pane-reply");
    s.run("mail.reply", &json!({})).unwrap();
    s.run("compose.update", &json!({"body": "Half a thought"})).unwrap();
    let before = s.compose.clone().expect("the reply in the pane");
    assert!(!before.window && before.source.as_ref() == Some(&b));
    for id in ["mail.markUnread", "mail.flag", "mail.archive"] {
        s.run_in_window(&a, id, &json!({})).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(s.compose.as_ref(), Some(&before), "{id}: the pane's reply untouched");
        assert!(s.composes.is_empty() && s.set_aside.is_empty(), "{id}: nothing set aside");
        assert_eq!(main_window(&s), (vec![b.clone()], Some(b.clone())), "{id}");
    }
    assert!(s.summary(&a).is_none(), "archived");
    done(s);
}

/// What the main window shows while a window's message isn't in its list: another folder, or a
/// search that doesn't find it.
const ELSEWHERE: [&str; 2] = ["another folder", "a search"];

fn show_elsewhere(s: &mut Session, how: &str, a: &MessageKey) {
    if how == "a search" {
        s.run("search.run", &json!({"query": "poster", "scope": "all"})).unwrap();
    } else {
        s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    }
    assert!(!s.list.is_empty() && !s.list.iter().any(|m| m.key == *a), "{how}: something shown, not the window's message");
}

/// Ignore from a window takes its message's whole conversation, wherever the main window is (it
/// took the window's message alone once the list didn't hold the rest of it).
#[test]
fn a_windows_ignore_takes_its_whole_conversation_wherever_the_main_window_is() {
    for how in ELSEWHERE {
        let (mut s, a, b) = two("window-ignore-conversation");
        let (sa, sb) = (s.summary(&a).unwrap().subject.clone(), s.summary(&b).unwrap().subject.clone());
        let topic = |t: &str| quatrelle_model::thread::base_subject(t).to_lowercase();
        assert_eq!(topic(&sa), topic(&sb), "the sample's first two are one conversation");
        show_elsewhere(&mut s, how, &a);
        s.run_in_window(&a, "mail.ignore", &json!({})).unwrap();
        for subject in [&sa, &sb] {
            assert!(!kept_in(&s, "INBOX", subject) && kept_in(&s, "Trash", subject), "{how}: {subject} went to Deleted Items");
        }
        assert!(s.windows.is_empty(), "{how}");
        done(s);
    }
}

/// A Quick Step of several steps run from a window does every step on its message, wherever the
/// main window is (on a search, the list made again after a step dropped the message: marked
/// read, not done).
#[test]
fn a_windows_quick_step_does_every_step_wherever_the_main_window_is() {
    for how in ELSEWHERE {
        let (mut s, a, _) = two("window-steps-elsewhere");
        s.run("quickStep.create", &json!({"name": "Done here", "actions": [{"do": "markRead"}], "complete": true})).unwrap();
        s.run_in_window(&a, "mail.markUnread", &json!({})).unwrap();
        let subject = s.summary(&a).unwrap().subject.clone();
        show_elsewhere(&mut s, how, &a);
        s.run_in_window(&a, "quickStep.run", &json!({"name": "Done here"})).unwrap();
        let inbox = s.store.as_ref().unwrap().load_folder(sample::SAMPLE_ACCOUNT, "INBOX").unwrap();
        let m = inbox.messages.iter().find(|m| m.subject == subject).expect("still in the Inbox");
        assert!(m.flags.seen && m.flags.follow_up_complete(), "{how}: read and done: {:?}", m.flags);
        done(s);
    }
}
