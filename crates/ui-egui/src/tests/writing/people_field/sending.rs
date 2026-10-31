//! Send and the contacts field (`PeopleFields::gate_send`, `send_when_resolved`): the names still
//! being resolved, the choice dialog, a contact group expanded first, the directory's timeout, and
//! recipients left out.

use quatrelle_engine::model::{Addressee, AddresseeKind, Job};

use super::message::{SIZE, Taken, chip_named, directory_fails, field, focus_field, frame_at, handed, has_send, new_message_on, text_with, to};
use super::*;

/// Send waits for a name still being looked up, then goes once the directory has answered; a
/// name the directory doesn't know stops the send with "Not found"; a message to addresses alone
/// goes without asking the directory at all.
#[test]
fn sending_waits_for_a_pending_name_then_stops_on_one_not_found() {
    let (mut a, dir) = app("send-waits");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    new_message_on(&mut a, &gal, "zoe@studio.example; dupont");
    frame(&ctx, &mut a, vec![]);
    handed(&jobs);
    a.run("compose.send", json!({}));
    assert!(!a.session.compose.as_ref().unwrap().sending, "held for the name");
    frame(&ctx, &mut a, vec![]);
    let out = handed(&jobs);
    assert!(!has_send(&out), "nothing sent: {out:?}");
    assert!(a.session.directory_searching(&gal, "dupont"), "asked of the directory");
    frame(&ctx, &mut a, vec![]);
    assert!(!has_send(&handed(&jobs)), "still held");
    directory_answers(&mut a, &gal, "dupont", &[("Marc Dupont", "marc@contoso.example")]);
    frame(&ctx, &mut a, vec![]);
    let out = handed(&jobs);
    assert!(has_send(&out), "sent once the name was found: {out:?}");
    assert_eq!(to(&a), "Zoe Park <zoe@studio.example>; Marc Dupont <marc@contoso.example>; ");
    assert!(a.session.compose.as_ref().unwrap().sending);
    assert!(a.ui.compose_alert.is_none(), "{:?}", a.ui.compose_alert);

    // A name nobody knows stops the send.
    new_message_on(&mut a, &gal, "nobody");
    frame(&ctx, &mut a, vec![]);
    a.run("compose.send", json!({}));
    directory_answers(&mut a, &gal, "nobody", &[]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.compose_alert.as_deref(), Some("Not found: nobody"));
    assert!(!has_send(&handed(&jobs)));
    assert!(!a.session.compose.as_ref().unwrap().sending);
    a.ui.compose_alert = None;

    // Addresses alone never need the directory: the message goes, offline or not.
    new_message_on(&mut a, &gal, "zoe@studio.example");
    for acc in a.session.accounts.iter_mut().filter(|x| x.config.id == gal) {
        acc.status = quatrelle_engine::model::SyncStatus::Offline;
    }
    frame(&ctx, &mut a, vec![]);
    handed(&jobs);
    a.run("compose.send", json!({}));
    frame(&ctx, &mut a, vec![]);
    let out = handed(&jobs);
    assert!(has_send(&out), "{out:?}");
    assert!(!out.iter().any(|j| matches!(j, Job::SearchDirectory { .. })), "the directory wasn't asked: {out:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Several people fit a name at Send: the choice is asked before the message goes, and Cancel
/// (or Escape) there leaves the name as typed and the message unsent, with nothing to say.
#[test]
fn cancelling_the_choice_asked_at_send_leaves_the_message_unsent() {
    let (mut a, dir) = app("send-choice-cancel");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    new_message_on(&mut a, &gal, "dupont");
    frame(&ctx, &mut a, vec![]);
    a.run("compose.send", json!({}));
    directory_answers(&mut a, &gal, "dupont", &[("Anne Dupont", "anne@contoso.example"), ("Marc Dupont", "marc@contoso.example")]);
    assert!(settled_text_at(&ctx, &mut a, "More than one person fits “dupont”. Choose who you meant:", SIZE).is_some(), "asked before it goes");
    assert!(!has_send(&handed(&jobs)), "held for the choice");
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(settled_text_at(&ctx, &mut a, "More than one person fits “dupont”. Choose who you meant:", SIZE).is_none(), "the dialog closed");
    assert_eq!(a.ui.compose_alert, None, "nothing to say: the send was called off");
    assert!(!has_send(&handed(&jobs)), "not sent");
    assert!(!a.session.compose.as_ref().unwrap().sending);
    assert!(a.ui.people.send_wait_left(0.0).is_none(), "no send waits any more");
    let f = field(&a, "to");
    assert!(a.ui.people.pieces(f).iter().any(|p| text_with(p, "dupont", None)), "the name as typed: {:?}", a.ui.people.pieces(f));
    let _ = std::fs::remove_dir_all(dir);
}

/// A directory that never answers doesn't hold Send forever: past its timeout the send stops
/// with "The directory can't be reached".
#[test]
fn sending_gives_up_after_the_directorys_timeout() {
    let (mut a, dir) = app("send-timeout");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame_at(&ctx, &mut a, vec![], 1.0);
    new_message_on(&mut a, &gal, "dupont");
    frame_at(&ctx, &mut a, vec![], 2.0);
    a.run("compose.send", json!({}));
    frame_at(&ctx, &mut a, vec![], 30.0);
    assert!(a.ui.compose_alert.is_none(), "still waiting at 30 s");
    frame_at(&ctx, &mut a, vec![], 63.0);
    assert_eq!(a.ui.compose_alert.as_deref(), Some("The directory can't be reached"));
    assert!(!has_send(&handed(&jobs)));
    assert!(!a.session.compose.as_ref().unwrap().sending);
    let f = field(&a, "to");
    assert!(a.ui.people.pieces(f).iter().any(|p| text_with(p, "dupont", Some(PieceError::Unreached))), "{:?}", a.ui.people.pieces(f));
    let _ = std::fs::remove_dir_all(dir);
}

/// A contact group has no address: Send asks the server for its members first, and the message
/// goes to them once they're known.
#[test]
fn a_contact_group_is_expanded_before_the_message_goes() {
    let (mut a, dir) = app("send-group");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    new_message_on(&mut a, &gal, "Dupont family <group:AAMk=>");
    frame(&ctx, &mut a, vec![]);
    handed(&jobs);
    a.run("compose.send", json!({}));
    frame(&ctx, &mut a, vec![]);
    let out = handed(&jobs);
    assert!(out.iter().any(|j| matches!(j, Job::ExpandList { item_id: Some(id), .. } if id == "AAMk=")), "{out:?}");
    assert!(!has_send(&out));
    let members = vec![
        Addressee::new("Anne Dupont", "anne@contoso.example", AddresseeKind::Person),
        Addressee::new("Marc Dupont", "marc@contoso.example", AddresseeKind::Person),
    ];
    a.session.apply(MailEvent::ListExpanded {
        account: gal.clone(),
        address: String::new(),
        item_id: Some("AAMk=".into()),
        members,
        more: false,
        total: 2,
        error: None,
    });
    frame(&ctx, &mut a, vec![]);
    let f = field(&a, "to");
    let p = a.ui.people.pieces(f).to_vec();
    assert!(p.len() == 2 && chip_named(&p[0], "Anne Dupont") && chip_named(&p[1], "Marc Dupont"), "{p:?}");
    let out = handed(&jobs);
    let sent = out.iter().find_map(|j| match j {
        Job::Send { recipients, .. } => Some(recipients.clone()),
        _ => None,
    });
    assert_eq!(sent, Some(vec!["anne@contoso.example".to_string(), "marc@contoso.example".to_string()]));
    assert_eq!(to(&a), "Anne Dupont <anne@contoso.example>; Marc Dupont <marc@contoso.example>; ");
    let _ = std::fs::remove_dir_all(dir);
}

/// The directory was down when Ctrl+K asked ("The directory can't be reached"), and it is back
/// when Send asks again a minute later: the name is pending again, not stopped on its old error.
#[test]
fn an_old_unreached_name_asked_again_at_send_waits_instead_of_stopping() {
    let (mut a, dir) = app("send-retries-unreached");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    new_message_on(&mut a, &gal, "smith");
    frame(&ctx, &mut a, vec![]);
    let f = field(&a, "to");
    focus_field(&ctx, f);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::K, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    directory_fails(&mut a, &gal, "smith", "offline");
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.people.pieces(f).iter().any(|p| text_with(p, "smith", Some(PieceError::Unreached))), "{:?}", a.ui.people.pieces(f));
    // A minute later the directory is asked again at Send.
    a.session.clock.now += 61;
    handed(&jobs);
    a.run("compose.send", json!({}));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.directory_searching(&gal, "smith"), "asked again");
    assert_eq!(a.ui.compose_alert, None, "pending again, not stopped on the old error");
    assert!(!has_send(&handed(&jobs)));
    directory_answers(&mut a, &gal, "smith", &[("Jo Smith", "jo@contoso.example")]);
    frame(&ctx, &mut a, vec![]);
    assert!(has_send(&handed(&jobs)), "sent once the directory answered");
    assert_eq!(to(&a), "Jo Smith <jo@contoso.example>; ");
    let _ = std::fs::remove_dir_all(dir);
}

/// Several messages can wait at once: one sent meanwhile (addresses alone) doesn't lose the
/// others' waits, and each goes exactly once when its own names settle.
#[test]
fn two_messages_waiting_each_go_once_when_their_names_settle() {
    let (mut a, dir) = app("two-waits");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    new_message_on(&mut a, &gal, "dupont");
    let first = a.session.compose.as_ref().unwrap().id;
    frame(&ctx, &mut a, vec![]);
    a.run("compose.send", json!({}));
    new_message_on(&mut a, &gal, "zoe@studio.example");
    let second = a.session.compose.as_ref().unwrap().id;
    frame(&ctx, &mut a, vec![]);
    handed(&jobs);
    a.run("compose.send", json!({}));
    frame(&ctx, &mut a, vec![]);
    let sent_ids = |jobs: &[Job]| {
        jobs.iter()
            .filter_map(|j| match j {
                Job::Send { id, .. } => Some(*id),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(sent_ids(&handed(&jobs)), [second], "addresses alone go at once");
    new_message_on(&mut a, &gal, "smith");
    let third = a.session.compose.as_ref().unwrap().id;
    frame(&ctx, &mut a, vec![]);
    a.run("compose.send", json!({}));
    frame(&ctx, &mut a, vec![]);
    assert!(sent_ids(&handed(&jobs)).is_empty(), "both wait");
    directory_answers(&mut a, &gal, "dupont", &[("Marc Dupont", "marc@contoso.example")]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(sent_ids(&handed(&jobs)), [first], "the first goes when its name is found");
    directory_answers(&mut a, &gal, "smith", &[("Jo Smith", "jo@contoso.example")]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(sent_ids(&handed(&jobs)), [third], "the third too, once");
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(sent_ids(&handed(&jobs)).is_empty(), "nothing goes twice");
    assert_eq!(a.ui.compose_alert, None);
    let _ = std::fs::remove_dir_all(dir);
}

/// While the choice dialog is open, the directory's timeout doesn't run: a slow choice never reads
/// "The directory can't be reached", and the message goes once the person is picked.
#[test]
fn the_timeout_waits_while_the_choice_dialog_is_open() {
    let (mut a, dir) = app("send-choice-slow");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame_at(&ctx, &mut a, vec![], 1.0);
    new_message_on(&mut a, &gal, "dupont");
    frame_at(&ctx, &mut a, vec![], 2.0);
    a.run("compose.send", json!({}));
    directory_answers(&mut a, &gal, "dupont", &[("Anne Dupont", "anne@contoso.example"), ("Marc Dupont", "marc@contoso.example")]);
    frame_at(&ctx, &mut a, vec![], 3.0);
    frame_at(&ctx, &mut a, vec![], 70.0);
    frame_at(&ctx, &mut a, vec![], 130.0);
    assert_eq!(a.ui.compose_alert, None, "the clock waits for the choice");
    assert!(settled_text_at(&ctx, &mut a, "More than one person fits “dupont”. Choose who you meant:", SIZE).is_some(), "still asked");
    let marc = settled_text_at(&ctx, &mut a, "Marc Dupont", SIZE).expect("Marc offered");
    click_at(&ctx, &mut a, marc, 2, SIZE);
    frame(&ctx, &mut a, vec![]);
    assert!(has_send(&handed(&jobs)), "sent once chosen");
    assert_eq!(to(&a), "Marc Dupont <marc@contoso.example>; ");
    let _ = std::fs::remove_dir_all(dir);
}

/// A stop that isn't the user's doing is said: the choice answered "as typed" by a command (not by
/// the dialog's Cancel) leaves a name nothing waits for, and the send stops with a word.
#[test]
fn a_name_left_unsettled_by_something_else_stops_the_send_with_a_word() {
    let (mut a, dir) = app("send-unsettled");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    new_message_on(&mut a, &gal, "dupont");
    frame(&ctx, &mut a, vec![]);
    let f = field(&a, "to");
    a.run("compose.send", json!({}));
    directory_answers(&mut a, &gal, "dupont", &[("Anne Dupont", "anne@contoso.example"), ("Marc Dupont", "marc@contoso.example")]);
    frame(&ctx, &mut a, vec![]);
    a.run("people.choose", json!({"window": f.window, "field": "to", "token": "dupont"}));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.compose_alert.as_deref(), Some("Some names need checking before sending"));
    assert!(!has_send(&handed(&jobs)));
    assert!(a.ui.people.send_wait_left(0.0).is_none(), "the wait is off");
    let _ = std::fs::remove_dir_all(dir);
}

/// A contact group with more members than the field holds, expanded at Send: the message doesn't
/// go to part of the list; it stops and says so.
#[test]
fn a_contact_group_with_more_members_than_the_field_holds_stops_the_send() {
    let (mut a, dir) = app("send-group-partial");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    new_message_on(&mut a, &gal, "Dupont family <group:AAMk=>");
    frame(&ctx, &mut a, vec![]);
    handed(&jobs);
    a.run("compose.send", json!({}));
    frame(&ctx, &mut a, vec![]);
    assert!(handed(&jobs).iter().any(|j| matches!(j, Job::ExpandList { .. })));
    let members = vec![
        Addressee::new("Anne Dupont", "anne@contoso.example", AddresseeKind::Person),
        Addressee::new("Marc Dupont", "marc@contoso.example", AddresseeKind::Person),
    ];
    a.session.apply(MailEvent::ListExpanded {
        account: gal.clone(),
        address: String::new(),
        item_id: Some("AAMk=".into()),
        members,
        more: true,
        total: 700,
        error: None,
    });
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.compose_alert.as_deref(), Some("Some recipients were left out: send to fewer at once, or to the list itself"));
    assert!(!has_send(&handed(&jobs)), "not sent to part of the list");
    assert!(!a.session.compose.as_ref().unwrap().sending);
    assert!(a.ui.people.send_wait_left(0.0).is_none(), "the wait is off");
    let _ = std::fs::remove_dir_all(dir);
}

/// A field fed more entries than it holds (a long list pasted) keeps the first 500 and says so;
/// Send doesn't go to those 500 alone.
#[test]
fn a_field_fed_more_entries_than_it_holds_stops_the_send() {
    let (mut a, dir) = app("send-over-cap");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    new_message_on(&mut a, &gal, "");
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let many: Vec<String> = (0..503).map(|i| format!("p{i}@x.example")).collect();
    frame(&ctx, &mut a, vec![Event::Paste(many.join("; "))]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let f = field(&a, "to");
    assert_eq!(a.ui.people.pieces(f).len(), 500);
    assert_eq!(a.ui.people.left_out(f), Some(3));
    handed(&jobs);
    a.run("compose.send", json!({}));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.compose_alert.as_deref(), Some("Some recipients were left out: send to fewer at once, or to the list itself"));
    assert!(!has_send(&handed(&jobs)), "not sent to the first 500 alone");
    assert!(!a.session.compose.as_ref().unwrap().sending);
    let _ = std::fs::remove_dir_all(dir);
}

/// A field that lost recipients stays stopped until the loss is undone: a group expanded to 100 of
/// 700, then another address typed and closed — the message doesn't go to 101 of 700.
#[test]
fn a_group_expanded_in_part_keeps_stopping_the_send_after_another_entry() {
    let (mut a, dir) = app("send-partial-stays");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    new_message_on(&mut a, &gal, "Dupont family <group:AAMk=>");
    frame(&ctx, &mut a, vec![]);
    handed(&jobs);
    a.run("compose.send", json!({}));
    frame(&ctx, &mut a, vec![]);
    let members: Vec<Addressee> =
        (0..100).map(|i| Addressee::new(format!("Member {i}"), format!("m{i}@contoso.example"), AddresseeKind::Person)).collect();
    a.session.apply(MailEvent::ListExpanded {
        account: gal.clone(),
        address: String::new(),
        item_id: Some("AAMk=".into()),
        members,
        more: true,
        total: 700,
        error: None,
    });
    frame(&ctx, &mut a, vec![]);
    let alert = "Some recipients were left out: send to fewer at once, or to the list itself";
    assert_eq!(a.ui.compose_alert.as_deref(), Some(alert));
    a.ui.compose_alert = None;
    let f = field(&a, "to");
    assert_eq!(a.ui.people.pieces(f).len(), 100);
    // Another address typed and closed: 100 of 700 is still not the list. (The alert's modal
    // layer holds the keyboard off the window for a frame after it closes.)
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    focus_field(&ctx, f);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("zoe@studio.example;".into())]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.people.pieces(f).len(), 101);
    assert!(settled_text_at(&ctx, &mut a, "and 600 more", SIZE).is_some(), "the note stays while the loss does");
    handed(&jobs);
    a.run("compose.send", json!({}));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.compose_alert.as_deref(), Some(alert));
    assert!(!has_send(&handed(&jobs)), "not sent to 101 of 700");
    let _ = std::fs::remove_dir_all(dir);
}

/// Likewise for a paste past the cap: removing a chip doesn't undo the three left out.
#[test]
fn a_paste_past_the_cap_keeps_stopping_the_send_after_a_chip_is_removed() {
    let (mut a, dir) = app("send-over-cap-stays");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    new_message_on(&mut a, &gal, "");
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let many: Vec<String> = (0..503).map(|i| format!("p{i}@x.example")).collect();
    frame(&ctx, &mut a, vec![Event::Paste(many.join("; "))]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let f = field(&a, "to");
    assert_eq!((a.ui.people.pieces(f).len(), a.ui.people.left_out(f)), (500, Some(3)));
    // Backspace twice: the last chip selected, then removed.
    frame(&ctx, &mut a, vec![key(Key::Backspace, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![key(Key::Backspace, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.people.pieces(f).len(), 499);
    assert_eq!(a.ui.people.left_out(f), Some(3), "the three left out are still left out");
    assert!(settled_text_at(&ctx, &mut a, "and 3 more were left out", SIZE).is_some(), "still said");
    handed(&jobs);
    a.run("compose.send", json!({}));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.compose_alert.as_deref(), Some("Some recipients were left out: send to fewer at once, or to the list itself"));
    assert!(!has_send(&handed(&jobs)), "not sent to 499 of 503");
    let _ = std::fs::remove_dir_all(dir);
}
