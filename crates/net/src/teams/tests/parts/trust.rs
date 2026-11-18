// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What a file's share trusts, from the security review of 1.5: the outbox read back from the
//! copy (a message with files or pictures isn't taken up again on its record's word), the item
//! OneDrive says it made (that file, or nothing is shared), and the chat's members (as Graph gives
//! them just before the share, not as the copy knew them).

use quatrelle_model::pim::teams::Outgoing;

use super::*;

/// The messages said "Not sent" without a number in the outboxes said of `chat`.
fn said_unnumbered(r: &Run, chat: &str) -> Vec<Outgoing> {
    r.outboxes(chat).into_iter().flat_map(|(items, _)| items).filter(|o| o.seq == 0 && matches!(o.state, OutState::NotSent { .. })).collect()
}

/// Review 1.5, files 1: an outbox written by hand in the copy (a file said gone up but not shared,
/// the drive's own folder for its item; a file by its path alone; a pasted picture) makes nothing
/// shared, read, uploaded or posted once Quatrelle starts again: each is said "Not sent" without a
/// number, by its parts' names (the window asks for them to be attached again), and stays in the
/// copy so, its picture too. A message of words alone goes on as it was.
#[test]
fn a_tampered_outbox_shares_nothing() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.w.work_offline(true);
    r.send_with(CHAT, "1-1", "The drive itself", vec![Part::File { path: file("report.pdf", &[7u8; 4_000]) }]);
    r.send_with(CHAT, "1-2", "A path alone", vec![Part::File { path: file("notes.txt", &[8u8; 3_000]) }]);
    r.send_with(CHAT, "1-3", "A picture alone", vec![Part::Picture { path: file("paste.png", &png(1_000)) }]);
    r.send(CHAT, "1-4", "Words alone");
    let mut op = r.ops().remove(0);
    let Some(PartRecord::File { up, .. }) = op.parts.get_mut(0) else { panic!("{op:?}") };
    *up = Some(Uploaded {
        item: "root".into(),
        guid: "0f0f0f0f-0000-4000-8000-000000000000".into(),
        url: format!("{DRIVE}/Microsoft%20Teams%20Chat%20Files/report.pdf"),
        name: "report.pdf".into(),
        shared: false,
    });
    r.w.copy.as_ref().unwrap().put_op(&op).unwrap();
    let (mut r, _) = restart(r, false);
    assert!(r.turns_until(Duration::from_secs(30), |r| r.held(CHAT, "Words alone") == 1), "{:?}", r.ops());
    r.turns(Duration::from_secs(10));
    let states: Vec<(u64, bool)> = r.ops().iter().map(|o| (o.seq, matches!(o.state, OpState::NotSent { .. }))).collect();
    assert_eq!(states, [(1, true), (2, true), (3, true)], "kept, not sent");
    let asked = (r.hits("invite"), r.hits("uploadSession"), r.hits("item"));
    assert_eq!(asked, (0, 0, 0), "nothing shared, nothing read nor sent up");
    let held = [r.held(CHAT, "Words alone"), r.held(CHAT, "The drive itself"), r.held(CHAT, "A path alone"), r.held(CHAT, "A picture alone")];
    assert_eq!(held, [1, 0, 0, 0], "the words alone went");
    let said = said_unnumbered(&r, CHAT);
    for (local, part) in [("1-1", "report.pdf"), ("1-2", "notes.txt"), ("1-3", "paste.png")] {
        let o = said.iter().find(|o| o.local == local).unwrap_or_else(|| panic!("{local} not said: {said:?}"));
        assert_eq!(o.parts.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), [part], "{local}");
    }
    assert!(r.w.copy.as_ref().unwrap().picture(3, 0).is_some(), "its picture kept with it");
    // Deleted from the window, by its local id: gone, its picture with it.
    r.w.job(Job::DiscardUnnumbered { local: "1-3".into() });
    assert_eq!(r.ops().iter().map(|o| o.seq).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(r.w.copy.as_ref().unwrap().picture(3, 0), None);
    // Nothing else is deleted by a local id: one the outbox numbers keeps its own way.
    r.send(CHAT, "1-5", "Numbered");
    r.w.job(Job::DiscardUnnumbered { local: "1-5".into() });
    assert!(r.ops().iter().any(|o| o.local == "1-5"));
}

/// Review 1.5, files 2: the item the upload's last answer names is asked of Graph and must be the
/// file sent (a file, of its size, right in Teams' folder): the drive's own folder, an item of
/// another size, or one in another folder (another, or one deeper of the same name) is "Not sent",
/// nothing shared nor posted.
#[test]
fn an_item_that_isnt_the_file_sent_is_refused() {
    let cases: [(&str, fn(&mut FakeTeams)); 4] = [
        ("the drive's own folder", |t| t.drive_files.done_as = Some("root".into())),
        ("another size", |t| t.drive_files.item_size = Some(1)),
        ("another folder", |t| t.drive_files.item_parent = Some("/drive/root:/Documents".into())),
        ("a folder of the same name deeper", |t| t.drive_files.item_parent = Some("/drive/root:/Documents/Microsoft Teams Chat Files".into())),
    ];
    for (case, change) in cases {
        let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
        r.fake(change);
        r.send_with(CHAT, "1-1", "Is it the file", vec![Part::File { path: file("report.pdf", &[7u8; 4_000]) }]);
        let refused = |r: &Run| matches!(first_state(r), Some(OpState::NotSent { reason: Unsent::Refused, .. }));
        assert!(r.turns_until(Duration::from_secs(30), refused), "{case}: {:?}", r.ops());
        r.turns(Duration::from_secs(10));
        assert_eq!((r.hits("item"), r.hits("invite"), r.hits("post")), (1, 0, 0), "{case}");
    }
}

/// The folder an item is in, by its `parentReference`: Teams' own at the drive's root, its path
/// percent-encoded or not, or by its name when it says no path; nothing else.
#[test]
fn an_items_folder_is_read_from_its_parent() {
    use crate::teams::graph::in_upload_folder;
    for yes in [
        json!({"path": "/drive/root:/Microsoft Teams Chat Files"}),
        json!({"path": "/drives/b!x1/root:/Microsoft%20Teams%20Chat%20Files", "name": "Microsoft Teams Chat Files"}),
        json!({"path": "/drive/root:/microsoft teams chat files"}),
        json!({"name": "Microsoft Teams Chat Files"}),
    ] {
        assert!(in_upload_folder(&yes), "{yes}");
    }
    for no in [
        json!({"path": "/drive/root:"}),
        json!({"path": "/drive/root:/Documents/Microsoft Teams Chat Files"}),
        json!({"path": "/drive/root:/Microsoft Teams Chat Files/Old"}),
        json!({"path": "/drive/root:/Documents", "name": "Microsoft Teams Chat Files"}),
        json!({"path": format!("/drive/root:/{}", "a".repeat(9_000))}),
        json!({"path": 3}),
        json!({}),
    ] {
        assert!(!in_upload_folder(&no), "{no}");
    }
}

/// Review 1.5, files 3: the members are read from Graph just before the share: someone the copy
/// still knows but Teams removed from the group since isn't given the file.
#[test]
fn a_member_removed_since_isnt_shared_with() {
    let mut r = run(teams_with(vec![group(GROUP)]));
    r.fake(|t| t.chat_mut(GROUP).unwrap().members.retain(|m| m.id != BRUNO));
    r.send_with(GROUP, "1-1", "For who is there", vec![Part::File { path: file("report.pdf", &[7u8; 4_000]) }]);
    assert!(until(&mut r, |r| r.ops().is_empty()), "{:?}", r.ops());
    let asked = r.fake(|t| t.drive_files.invites.iter().map(|(_, body)| body["recipients"].clone()).collect::<Vec<_>>());
    assert_eq!(asked, [json!([{"objectId": ANA}])]);
    assert_eq!(r.held(GROUP, "For who is there"), 1);
}

/// Review 1.5, files 3: a chat Graph says it doesn't know (404) when its members are read for the
/// share is "Not sent", nothing shared nor posted.
#[test]
fn a_chat_graph_doesnt_know_shares_nothing() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| t.not_found_next.insert("members".into(), 1));
    r.send_with(CHAT, "1-1", "To nobody known", vec![Part::File { path: file("report.pdf", &[7u8; 4_000]) }]);
    let unshared = |r: &Run| matches!(first_state(r), Some(OpState::NotSent { reason: Unsent::NotShared, .. }));
    assert!(r.turns_until(Duration::from_secs(30), unshared), "{:?}", r.ops());
    r.turns(Duration::from_secs(10));
    assert_eq!((r.hits("invite"), r.hits("post")), (0, 0));
}

/// Review 1.5, tour 1 (files 1): a message read back with a file not shared yet stays in the copy,
/// "Not sent", its words kept from one start to the next; each start refuses it again (said
/// without its number, its file to be attached again), and a Retry never takes it up.
#[test]
fn a_message_read_back_with_files_is_kept_unsent_through_restarts() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.w.work_offline(true);
    r.send_with(CHAT, "1-1", "Kept through restarts", vec![Part::File { path: file("report.pdf", &[7u8; 4_000]) }]);
    let seq = r.ops()[0].seq;
    for round in 0..2 {
        r = restart(r, false).0;
        r.turns(Duration::from_secs(10));
        let said = said_unnumbered(&r, CHAT);
        let o = said.iter().find(|o| o.local == "1-1").unwrap_or_else(|| panic!("round {round}: {:?}", r.events()));
        assert_eq!((o.body.plain(), o.parts.len()), ("Kept through restarts".to_string(), 1), "round {round}");
        assert!(matches!(first_state(&r), Some(OpState::NotSent { .. })), "round {round}: kept in the copy, not sent: {:?}", r.ops());
        r.w.job(Job::Retry { seq });
        r.turns(Duration::from_secs(10));
        assert_eq!((r.hits("uploadSession"), r.hits("invite"), r.held(CHAT, "Kept through restarts")), (0, 0, 0), "round {round}");
    }
}

/// Review 1.5, tour 1 (files 1): the copy refusing to write a message read back with its file (left
/// waiting on the disk, the write failing) sends and shares nothing of it, whatever comes after: a
/// message sent behind it goes, a Retry of it doesn't.
#[test]
fn a_message_read_back_with_files_goes_nowhere_when_the_disk_fails() {
    let store = quatrelle_store::Store::failing();
    let mut r = super::super::sync::run_on(teams_with(vec![chat(CHAT, T0, 4)]), store.clone());
    r.w.work_offline(true);
    r.send_with(CHAT, "1-1", "Read back", vec![Part::File { path: file("report.pdf", &[7u8; 4_000]) }]);
    let seq = r.ops()[0].seq;
    store.fail(None, Some(&format!("{seq:020}.json")));
    let (mut r, _) = restart(r, false);
    assert_eq!(first_state(&r), Some(OpState::Waiting), "the disk kept it waiting");
    r.send(CHAT, "1-2", "Words after it");
    r.w.job(Job::Retry { seq });
    assert!(r.turns_until(Duration::from_secs(30), |r| r.held(CHAT, "Words after it") == 1), "{:?}", r.ops());
    r.turns(Duration::from_secs(20));
    assert_eq!((r.hits("uploadSession"), r.hits("invite"), r.held(CHAT, "Read back")), (0, 0, 0));
}

/// Review 1.5, tour 1 (files 3): members read for a share that goes more than a minute later (the
/// limiter, a Retry-After, the network away, a computer asleep) are read again first: someone
/// removed meanwhile isn't given the file.
#[test]
fn members_read_more_than_a_minute_before_the_share_are_read_again() {
    let mut r = run(teams_with(vec![group(GROUP)]));
    let before = r.hits("members");
    r.send_with(GROUP, "1-1", "For who is there now", vec![Part::File { path: file("report.pdf", &[7u8; 4_000]) }]);
    assert!(until(&mut r, |r| r.hits("members") > before), "the members were never read for the share");
    assert_eq!(r.hits("invite"), 0, "read, not shared yet");
    r.fake(|t| t.chat_mut(GROUP).unwrap().members.retain(|m| m.id != BRUNO));
    r.at += Duration::from_secs(90);
    assert!(until(&mut r, |r| r.ops().is_empty()), "{:?}", r.ops());
    let asked = r.fake(|t| t.drive_files.invites.iter().map(|(_, body)| body["recipients"].clone()).collect::<Vec<_>>());
    assert_eq!(asked, [json!([{"objectId": ANA}])]);
    assert_eq!(r.hits("members"), before + 2, "read again before the share");
}
