// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A message's files and pasted pictures sent by the outbox, against the fake Graph, the worker
//! driven by hand: a file goes up to the user's OneDrive on a lane without the token, is shared
//! with the chat's other members, then posted as a reference; a picture goes inside its message.
//! Files gone, changed, too big, or of a chat they can't be shared in; an upload cut, a session
//! gone, a refusal, an item elsewhere, a share half made, a server's error; a name OneDrive
//! numbered; a lost answer; a restart, a Delete, a sign-out mid-way; other chats never held; and
//! what the log says.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use quatrelle_model::pim::teams::{Attachment, ChatText, FILE_MAX, Job, Op, OutPart, OutState, PICTURE_INLINE_MAX, Part, TEAMS_ACCOUNT, Unsent};
use serde_json::json;

use super::outbox::{body, restart};
use super::sync::{NOW, OTHER, Run, STEP, run};
use super::{CHAT, MINUTE, T0, TENANT, me};
use crate::fake_graph::{FakeChat, FakeMessage, FakeTeams, FakeUser, rfc3339};
use crate::teams::graph::etag_guid;
use crate::teams::store::{OpState, PartRecord, Uploaded};

/// The other members, by their object ids, as the organisation's people have them.
const ANA: &str = "00000000-0000-0000-0000-0000000000a1";
const BRUNO: &str = "00000000-0000-0000-0000-0000000000a2";
const GROUP: &str = "19:e5f6@thread.v2";
/// The user's OneDrive: the fake's items' `webDavUrl`s are under it.
const DRIVE: &str = "https://contoso-my.sharepoint.com/personal/riley";
const MIB: usize = 1 << 20;

fn ana() -> FakeUser {
    FakeUser::new(ANA, "Ana Lima")
}

/// `members`' chat, with `n` messages a minute apart from `start`, alternately Ana's and the
/// user's.
fn with_messages(mut c: FakeChat, start: i64, n: i64) -> FakeChat {
    for i in 0..n {
        let at = start + i * MINUTE;
        let from = if i % 2 == 0 { ANA } else { super::ME };
        c.messages.push(FakeMessage::new(&at.to_string(), Some(from), &rfc3339(at), &format!("<p>Message {i}</p>")));
    }
    c
}

/// A chat of the user and Ana (by her object id).
fn chat(id: &str, start: i64, n: i64) -> FakeChat {
    with_messages(FakeChat::new(id, "oneOnOne", None, vec![me(), ana()]), start, n)
}

/// A group of the user, Ana and Bruno.
fn group(id: &str) -> FakeChat {
    with_messages(FakeChat::new(id, "group", Some("Plans"), vec![me(), ana(), FakeUser::new(BRUNO, "Bruno Reis")]), T0, 3)
}

fn teams_with(chats: Vec<FakeChat>) -> FakeTeams {
    FakeTeams { chats, drive: Some(DRIVE.into()), ..FakeTeams::new(me(), TENANT) }
}

/// A folder of the test's own (its thread's), removed when the test is over.
struct Folder(PathBuf);

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

thread_local! {
    static FOLDER: Folder = {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("quatrelle-teams-parts-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Folder(dir)
    };
}

/// A file named `name` holding `bytes`, in the test's own folder: its path.
fn file(name: &str, bytes: &[u8]) -> String {
    FOLDER.with(|f| {
        let path = f.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_string_lossy().into_owned()
    })
}

/// A PNG of `n` bytes, as its first bytes say.
fn png(n: usize) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.resize(n, 1);
    bytes
}

/// `n` bytes that differ from one to the next.
fn content(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8).collect()
}

fn not_sent(reason: Unsent, code: Option<&str>) -> OpState {
    OpState::NotSent { reason, code: code.map(str::to_string) }
}

/// The state of the outbox's first record.
fn first_state(r: &Run) -> Option<OpState> {
    r.ops().first().map(|o| o.state.clone())
}

impl Run {
    /// The user sends `text` and `parts` to `chat`, the engine calling the message `local`.
    fn send_with(&mut self, chat: &str, local: &str, text: &str, parts: Vec<Part>) {
        self.w.job(Job::Op { op: Op::Send { chat: chat.into(), local: local.into(), body: body(text), mentions: Vec::new(), parts } });
    }

    /// The item of the last outbox said of `chat`, answering what was just sent.
    fn answered(&self, chat: &str) -> (u64, String, OutState) {
        let o = self.outbox_said(chat).pop().unwrap();
        (o.seq, o.local, o.state)
    }
}

/// Turns, a [`STEP`] apart and a few milliseconds of this computer's clock between them (the
/// lanes are threads), until `done`: whether it came within 30 s of this computer's clock.
fn until(r: &mut Run, done: impl Fn(&Run) -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(30);
    while Instant::now() < end {
        if done(r) {
            return true;
        }
        r.turns(STEP);
        std::thread::sleep(Duration::from_millis(5));
    }
    done(r)
}

/// This computer's clock, the worker not turned, until `done` (what a lane does on its own):
/// whether it came within 20 s.
fn wait(r: &Run, done: impl Fn(&Run) -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end && !done(r) {
        std::thread::sleep(Duration::from_millis(5));
    }
    done(r)
}

#[test]
fn a_file_goes_up_is_shared_then_sent_as_a_reference() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    let path = file("report.pdf", &[7u8; 4_000]);
    r.w.job(Job::Op {
        op: Op::Send {
            chat: CHAT.into(),
            local: "1-1".into(),
            body: body("Here it is"),
            mentions: Vec::new(),
            parts: vec![Part::File { path: path.clone() }],
        },
    });
    assert!(r.turns_until(Duration::from_secs(30), |r| r.ops().is_empty()), "{:?}", r.ops());
    assert_eq!((r.hits("uploadSession"), r.hits("item"), r.hits("invite"), r.hits("post")), (1, 1, 1, 1));
    let (ranges, invites, item) = r.fake(|t| (t.drive_files.ranges.clone(), t.drive_files.invites.clone(), t.drive_files.files[0].clone()));
    assert_eq!(ranges.iter().map(|x| (x.1, x.2, x.3)).collect::<Vec<_>>(), [(0, 3_999, 4_000)]);
    assert!(!r.fake(|t| t.last_headers["upload"].iter().any(|(k, _)| k == "authorization")));
    assert_eq!(invites[0].1, json!({"recipients": [{"objectId": ANA}], "roles": ["read"], "requireSignIn": true, "sendInvitation": false}));
    let post = r.fake(|t| t.posts[0].1.clone());
    let guid = etag_guid(&item.etag).unwrap();
    assert_eq!(post["attachments"][0]["id"], guid);
    assert_eq!((post["attachments"][0]["contentType"].as_str(), post["attachments"][0]["name"].as_str()), (Some("reference"), Some("report.pdf")));
    assert_eq!(post["body"]["content"], format!("<p>Here it is</p><attachment id=\"{guid}\"></attachment>"));
    let said = r.messages_said(CHAT).pop().unwrap();
    assert!(matches!(&said.attachments[..], [Attachment::File { name, .. }] if name == "report.pdf"));
}

/// The chat's outbox says how far a file's upload is: 12 MiB, its second range's answer held.
#[test]
fn a_big_file_shows_how_far_it_is() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| t.drive_files.hold_range = Some((1, Duration::from_secs(60))));
    let path = file("big.bin", &vec![7u8; 12 * MIB]);
    r.send_with(CHAT, "1-1", "A big one", vec![Part::File { path }]);
    let going = |r: &Run| r.outbox_said(CHAT).first().is_some_and(|o| o.parts.first().is_some_and(|p| p.sent > 0));
    assert!(until(&mut r, going), "{:?}", r.outbox_said(CHAT));
    let shown = r.outbox_said(CHAT).remove(0);
    assert_eq!(shown.state, OutState::Uploading);
    assert_eq!(shown.parts, [OutPart { name: "big.bin".into(), size: 12_582_912, picture: false, sent: 5_242_880 }]);
    assert_eq!(r.hits("post"), 0);
}

#[test]
fn a_pasted_picture_goes_inline() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    let picture = png(1_000);
    let path = file("paste.png", &picture);
    r.w.job(Job::Op {
        op: Op::Send { chat: CHAT.into(), local: "1-1".into(), body: ChatText::default(), mentions: Vec::new(), parts: vec![Part::Picture { path }] },
    });
    let seq = r.ops()[0].seq;
    assert_eq!(r.w.copy.as_ref().unwrap().picture(seq, 0), Some(picture.clone()), "kept until it goes");
    assert!(r.turns_until(Duration::from_secs(10), |r| r.ops().is_empty()), "{:?}", r.ops());
    let post = r.fake(|t| t.posts[0].1.clone());
    assert_eq!(post["hostedContents"][0]["contentType"], "image/png");
    assert_eq!(post["hostedContents"][0]["contentBytes"], crate::imap::base64(&picture));
    assert_eq!(post["body"]["content"], "<p><img src=\"../hostedContents/1/$value\"></p>");
    assert_eq!(r.hits("uploadSession"), 0);
    assert_eq!(r.w.copy.as_ref().unwrap().picture(seq, 0), None, "gone with its message");
}

/// H17: a pasted picture's file that isn't a PNG or a JPEG, or past 4 MiB, isn't taken.
#[test]
fn a_picture_that_isnt_one_is_refused() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.w.work_offline(true);
    let refused = OutState::NotSent { reason: Unsent::Refused, code: None };
    for (local, bytes) in [("1-1", b"Not a picture at all".to_vec()), ("1-2", png(PICTURE_INLINE_MAX as usize + 1))] {
        let path = file("paste.png", &bytes);
        r.send_with(CHAT, local, "", vec![Part::Picture { path }]);
        assert_eq!(r.answered(CHAT), (0, local.to_string(), refused.clone()));
    }
    assert!(r.ops().is_empty());
    assert!(r.w.store.account_files(TEAMS_ACCOUNT, "outbox-parts").unwrap_or_default().is_empty(), "no picture kept");
}

/// H11: a folder, a path that isn't there, an empty file aren't files to send; one past 100 MiB
/// is too big. Nothing is kept, nothing asked of Teams.
#[test]
fn only_a_regular_file_of_a_size_is_taken() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    let empty = file("empty.txt", b"");
    let folder = PathBuf::from(&empty).parent().unwrap().to_string_lossy().into_owned();
    let missing = PathBuf::from(&empty).with_file_name("missing.pdf").to_string_lossy().into_owned();
    let big = file("huge.bin", b"x");
    std::fs::OpenOptions::new().write(true).open(&big).unwrap().set_len(FILE_MAX + 1).unwrap();
    let asked = r.all_hits();
    let cases = [(folder, Unsent::FileGone), (missing, Unsent::FileGone), (empty, Unsent::FileGone), (big, Unsent::FileTooBig)];
    for (i, (path, reason)) in cases.into_iter().enumerate() {
        let local = format!("1-{i}");
        r.send_with(CHAT, &local, "Attached", vec![Part::File { path }]);
        assert_eq!(r.answered(CHAT), (0, local, OutState::NotSent { reason, code: None }));
    }
    assert!(r.ops().is_empty());
    assert_eq!(r.all_hits(), asked, "no request");
}

/// H11, Review Focus 1: a file written again after it was attached isn't sent, nor once removed;
/// put back and sent again, it goes as it is then.
#[test]
fn a_file_changed_since_it_was_attached_is_not_sent() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.w.work_offline(true);
    let path = file("report.pdf", &[1u8; 4_000]);
    r.send_with(CHAT, "1-1", "The report", vec![Part::File { path: path.clone() }]);
    assert_eq!(r.ops().len(), 1, "taken");
    std::fs::write(&path, [2u8; 5_000]).unwrap();
    r.w.work_offline(false);
    let gone = not_sent(Unsent::FileGone, None);
    assert!(r.turns_until(Duration::from_secs(5), |r| first_state(r) == Some(gone.clone())), "{:?}", r.ops());
    assert_eq!(r.outbox_said(CHAT)[0].state, OutState::NotSent { reason: Unsent::FileGone, code: None });
    assert_eq!(r.hits("uploadSession"), 0);
    // Removed, then sent again: the same.
    std::fs::remove_file(&path).unwrap();
    let seq = r.ops()[0].seq;
    r.w.job(Job::Retry { seq });
    assert_eq!(first_state(&r), Some(OpState::Waiting));
    assert!(r.turns_until(Duration::from_secs(5), |r| first_state(r) == Some(gone.clone())), "{:?}", r.ops());
    assert_eq!(r.hits("uploadSession"), 0);
    // Put back and sent again: as it is now.
    std::fs::write(&path, [3u8; 6_000]).unwrap();
    r.w.job(Job::Retry { seq });
    assert!(r.turns_until(Duration::from_secs(30), |r| r.ops().is_empty()), "{:?}", r.ops());
    assert_eq!(r.fake(|t| t.drive_files.files[0].bytes.clone()), vec![3u8; 6_000]);
    assert_eq!((r.hits("uploadSession"), r.hits("post")), (1, 1));
}

/// H11: a file grown past 100 MiB since it was attached is said too big, not gone, and so again
/// after a Retry; nothing is asked of OneDrive.
#[test]
fn a_file_grown_past_the_cap_since_it_was_attached_is_too_big() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.w.work_offline(true);
    let path = file("huge.bin", &[1u8; 4_000]);
    r.send_with(CHAT, "1-1", "Growing", vec![Part::File { path: path.clone() }]);
    std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_len(FILE_MAX + 1).unwrap();
    r.w.work_offline(false);
    let big = not_sent(Unsent::FileTooBig, None);
    assert!(r.turns_until(Duration::from_secs(5), |r| first_state(r) == Some(big.clone())), "{:?}", r.ops());
    let seq = r.ops()[0].seq;
    r.w.job(Job::Retry { seq });
    assert_eq!(first_state(&r), Some(OpState::Waiting));
    assert!(r.turns_until(Duration::from_secs(5), |r| first_state(r) == Some(big.clone())), "{:?}", r.ops());
    assert_eq!(r.hits("uploadSession"), 0);
}

/// H8, Review Focus 2: an upload cut once its first range went is taken up where Graph says:
/// every byte once, one message.
#[test]
fn an_upload_cut_is_resumed_where_graph_says() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| t.drive_files.cut_range = Some(1));
    let bytes = content(12 * MIB);
    let path = file("big.bin", &bytes);
    r.send_with(CHAT, "1-1", "Cut on the way", vec![Part::File { path }]);
    assert!(until(&mut r, |r| r.ops().is_empty()), "{:?}", r.ops());
    let ranges = r.fake(|t| t.drive_files.ranges.iter().map(|x| (x.1, x.2)).collect::<Vec<_>>());
    assert_eq!(ranges, [(0, 5_242_879), (5_242_880, 10_485_759), (10_485_760, 12_582_911)], "every byte once");
    assert_eq!((r.hits("uploadSession"), r.hits("post"), r.held(CHAT, "Cut on the way")), (1, 1, 1));
    assert_eq!(r.fake(|t| t.drive_files.files.len()), 1);
    assert!(r.fake(|t| t.drive_files.files[0].bytes == bytes));
}

/// H8: a session gone, the upload starts over in a new one, two in all; gone again, "Not sent"
/// (couldn't be confirmed), no third.
#[test]
fn a_session_gone_starts_over_once() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| t.drive_files.expire_after = Some(1));
    let path = file("big.bin", &vec![7u8; 6 * MIB]);
    r.send_with(CHAT, "1-1", "Once more", vec![Part::File { path: path.clone() }]);
    assert!(until(&mut r, |r| r.hits("uploadSession") == 1 && r.w.transfers.busy()));
    // The worker not turned meanwhile: the lane alone meets the session gone.
    assert!(wait(&r, |r| r.fake(|t| t.drive_files.sessions.first().is_some_and(|s| s.gone))));
    r.fake(|t| t.drive_files.expire_after = None);
    assert!(until(&mut r, |r| r.ops().is_empty()), "{:?}", r.ops());
    assert_eq!((r.hits("uploadSession"), r.hits("post")), (2, 1));

    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| t.drive_files.expire_after = Some(1));
    r.send_with(CHAT, "1-1", "Never", vec![Part::File { path }]);
    let unconfirmed = not_sent(Unsent::Unconfirmed, None);
    assert!(until(&mut r, |r| first_state(r) == Some(unconfirmed.clone())), "{:?}", r.ops());
    r.turns(Duration::from_secs(30));
    assert_eq!((r.hits("uploadSession"), r.hits("post")), (2, 0));
}

/// Final review, net I-1: OneDrive failing every range (500): each session is taken up five times
/// on the lane, three sessions in all, then the message is "Not sent" (couldn't be confirmed),
/// never begun again; the chat's next message goes once it is settled.
#[test]
fn an_upload_always_failing_on_its_lane_ends_not_sent() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| t.drive_files.refuse_ranges = Some(500));
    let path = file("report.pdf", &[7u8; 4_000]);
    r.send_with(CHAT, "1-1", "Never through", vec![Part::File { path }]);
    r.send(CHAT, "1-2", "Behind it");
    let unconfirmed = not_sent(Unsent::Unconfirmed, None);
    assert!(until(&mut r, |r| first_state(r) == Some(unconfirmed.clone())), "{:?} after {} sessions", r.ops(), r.hits("uploadSession"));
    assert!(until(&mut r, |r| r.held(CHAT, "Behind it") == 1), "{:?}", r.ops());
    r.turns(Duration::from_secs(120));
    // Each session: the range put six times, where it stands read five times between.
    assert_eq!((r.hits("uploadSession"), r.hits("upload"), r.hits("post")), (3, 33, 1));
    assert_eq!(r.outbox_said(CHAT)[0].state, OutState::NotSent { reason: Unsent::Unconfirmed, code: None });
}

/// H8, P-9: OneDrive full (507) is a refusal: "Not sent" with Graph's code, never tried again.
#[test]
fn a_refused_upload_is_not_sent_with_its_code() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| t.drive_files.full = true);
    let path = file("report.pdf", &[7u8; 4_000]);
    r.send_with(CHAT, "1-1", "Too much", vec![Part::File { path }]);
    let full = not_sent(Unsent::Refused, Some("HTTP 507 quotaLimitReached"));
    assert!(r.turns_until(Duration::from_secs(10), |r| first_state(r) == Some(full.clone())), "{:?}", r.ops());
    assert_eq!(r.outbox_said(CHAT)[0].state, OutState::NotSent { reason: Unsent::Refused, code: Some("HTTP 507 quotaLimitReached".into()) });
    r.turns(Duration::from_secs(30));
    assert_eq!((r.hits("uploadSession"), r.hits("post")), (1, 0));
}

/// P-9: a server's error (500) on the session, the item or the share is a try: the request is
/// made again a while later, and the message goes. A request answered counts the next one's tries
/// from none: two failed sessions don't make the item's first failure the third try.
#[test]
fn a_server_error_on_a_files_request_is_tried_again() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| {
        for (route, n) in [("uploadSession", 2), ("item", 1), ("invite", 1)] {
            t.fail_next.insert(route.into(), n);
        }
    });
    let path = file("report.pdf", &[7u8; 4_000]);
    r.send_with(CHAT, "1-1", "Again and again", vec![Part::File { path }]);
    assert!(until(&mut r, |r| r.ops().is_empty()), "{:?}", r.ops());
    assert_eq!((r.hits("uploadSession"), r.hits("item"), r.hits("invite"), r.hits("post")), (3, 2, 2, 1));
    assert_eq!(r.fake(|t| t.drive_files.files.len()), 1);
}

/// H9: an item whose `webDavUrl` isn't on the user's own OneDrive (another site, or another of
/// Microsoft's hosts) isn't shared nor sent.
#[test]
fn an_item_elsewhere_than_the_users_drive_is_refused() {
    for elsewhere in ["evil.example", "contoso.sharepoint.com"] {
        let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
        r.fake(|t| t.drive_files.item_elsewhere = Some(elsewhere.into()));
        let path = file("report.pdf", &[7u8; 4_000]);
        r.send_with(CHAT, "1-1", "From elsewhere", vec![Part::File { path }]);
        let refused = |r: &Run| matches!(first_state(r), Some(OpState::NotSent { reason: Unsent::Refused, .. }));
        assert!(r.turns_until(Duration::from_secs(30), refused), "{elsewhere}: {:?}", r.ops());
        r.turns(Duration::from_secs(10));
        assert_eq!((r.hits("item"), r.hits("invite"), r.hits("post")), (1, 0, 0), "{elsewhere}");
    }
}

/// H16: a file the outbox read back as uploaded on one of Microsoft's hosts that isn't the user's
/// own OneDrive (another site of the organisation) is never posted.
#[test]
fn a_file_kept_as_uploaded_elsewhere_is_never_posted() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.w.work_offline(true);
    let path = file("report.pdf", &[7u8; 4_000]);
    r.send_with(CHAT, "1-1", "From another site", vec![Part::File { path }]);
    let mut op = r.ops().remove(0);
    let Some(PartRecord::File { up, .. }) = op.parts.get_mut(0) else { panic!("{op:?}") };
    *up = Some(Uploaded {
        item: "01ITEM0001".into(),
        guid: "5b8c7d0e-0001-4000-8000-000000000001".into(),
        url: "https://contoso.sharepoint.com/sites/team/Shared%20Documents/report.pdf".into(),
        name: "report.pdf".into(),
        shared: true,
    });
    r.w.copy.as_ref().unwrap().put_op(&op).unwrap();
    r.w.work_offline(false);
    let refused = |r: &Run| matches!(first_state(r), Some(OpState::NotSent { reason: Unsent::Refused, .. }));
    assert!(r.turns_until(Duration::from_secs(10), refused), "{:?}", r.ops());
    assert_eq!((r.hits("uploadSession"), r.hits("post")), (0, 0));
}

/// H10: a share that fails for one of the group isn't half made: "Not sent", nothing posted; sent
/// again, the file is shared again, never uploaded again.
#[test]
fn a_partial_share_is_not_sent() {
    let mut r = run(teams_with(vec![group(GROUP)]));
    r.fake(|t| t.drive_files.fail_share = vec![BRUNO.into()]);
    let path = file("report.pdf", &[7u8; 4_000]);
    r.send_with(GROUP, "1-1", "For both", vec![Part::File { path }]);
    let unshared = |r: &Run| matches!(first_state(r), Some(OpState::NotSent { reason: Unsent::NotShared, .. }));
    assert!(r.turns_until(Duration::from_secs(30), unshared), "{:?}", r.ops());
    assert_eq!(r.hits("post"), 0);
    let asked = r.fake(|t| t.drive_files.invites[0].1["recipients"].clone());
    assert_eq!(asked, json!([{"objectId": ANA}, {"objectId": BRUNO}]));
    r.fake(|t| t.drive_files.fail_share.clear());
    let seq = r.ops()[0].seq;
    r.w.job(Job::Retry { seq });
    assert!(r.turns_until(Duration::from_secs(30), |r| r.ops().is_empty()), "{:?}", r.ops());
    assert_eq!((r.hits("uploadSession"), r.hits("invite"), r.hits("post")), (1, 2, 1));
}

/// Review Focus 3: the same name sent twice is numbered by OneDrive, and the message carries the
/// name the file got.
#[test]
fn a_name_taken_in_onedrive_is_sent_under_the_name_it_got() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    let path = file("report.pdf", &[7u8; 4_000]);
    r.send_with(CHAT, "1-1", "First", vec![Part::File { path: path.clone() }]);
    r.send_with(CHAT, "1-2", "Second", vec![Part::File { path }]);
    assert!(r.turns_until(Duration::from_secs(60), |r| r.ops().is_empty()), "{:?}", r.ops());
    let names: Vec<Option<String>> = r.fake(|t| t.posts.iter().map(|(_, p)| p["attachments"][0]["name"].as_str().map(str::to_string)).collect());
    assert_eq!(names, [Some("report.pdf".to_string()), Some("report 1.pdf".to_string())]);
}

/// Decision 16: a post with a file whose answer is lost is checked by its text and its files'
/// names, found, and its file never uploaded again.
#[test]
fn a_post_whose_answer_is_lost_is_found_by_its_files_not_uploaded_again() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| {
        t.answer_lost.insert("post".into(), 1);
        t.lost_retry_after = Some(1);
    });
    let path = file("report.pdf", &[7u8; 4_000]);
    r.send_with(CHAT, "1-1", "Lost with its file", vec![Part::File { path }]);
    assert!(r.turns_until(Duration::from_secs(30), |r| r.ops().is_empty()), "{:?}", r.ops());
    r.turns(Duration::from_secs(20));
    let carried = r.fake(|t| t.chat(CHAT).unwrap().messages.iter().filter(|m| !m.files.is_empty()).count());
    assert_eq!(carried, 1);
    assert_eq!((r.hits("uploadSession"), r.hits("post")), (1, 1));
}

/// Decision 16: a message of the user's of the same text but without the file, made meanwhile,
/// isn't taken for the one whose post failed: it is posted again, its file not uploaded again.
#[test]
fn a_message_of_the_same_text_without_its_file_isnt_taken_for_it() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| {
        t.fail_next.insert("post".into(), 1);
    });
    let path = file("report.pdf", &[7u8; 4_000]);
    r.send_with(CHAT, "1-1", "Same words", vec![Part::File { path }]);
    assert!(r.turns_until(Duration::from_secs(30), |r| r.hits("post") == 1), "{:?}", r.ops());
    r.post(CHAT, super::ME, NOW + 1_000, "Same words");
    assert!(r.turns_until(Duration::from_secs(30), |r| r.ops().is_empty()), "{:?}", r.ops());
    assert_eq!((r.hits("uploadSession"), r.hits("post")), (1, 2), "checked, not found, posted again");
    let carried = r.fake(|t| t.chat(CHAT).unwrap().messages.iter().filter(|m| !m.files.is_empty()).count());
    assert_eq!((carried, r.held(CHAT, "Same words")), (1, 2));
}

/// Decision 16 with a picture: a post whose answer is lost is found by its text and its picture
/// (Graph lists the picture at its own address), never posted again, and its record goes.
#[test]
fn a_post_with_a_picture_whose_answer_is_lost_is_found() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| {
        t.answer_lost.insert("post".into(), 1);
        t.lost_retry_after = Some(1);
    });
    let path = file("paste.png", &png(1_000));
    r.send_with(CHAT, "1-1", "Look at this", vec![Part::Picture { path }]);
    let seq = r.ops()[0].seq;
    assert!(r.turns_until(Duration::from_secs(30), |r| r.ops().is_empty()), "{:?}", r.ops());
    r.turns(Duration::from_secs(20));
    assert_eq!((r.hits("post"), r.held(CHAT, "Look at this")), (1, 1));
    assert_eq!(r.w.copy.as_ref().unwrap().picture(seq, 0), None);
}

/// Review Focus 2: Quatrelle closed while a file goes up: started again, the upload begins anew
/// and the message is posted once.
#[test]
fn a_run_cut_off_while_uploading_starts_again_and_posts_once() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| t.hold = Some(("upload", Duration::from_secs(60))));
    let bytes = content(6 * MIB);
    let path = file("big.bin", &bytes);
    r.send_with(CHAT, "1-1", "Through a restart", vec![Part::File { path }]);
    assert!(until(&mut r, |r| r.hits("upload") >= 1), "the upload never began");
    let (mut r, _) = restart(r, false);
    assert_eq!(first_state(&r), Some(OpState::Waiting), "kept as waiting");
    assert!(until(&mut r, |r| r.ops().is_empty()), "{:?}", r.ops());
    assert_eq!((r.hits("post"), r.held(CHAT, "Through a restart")), (1, 1));
    assert!(r.fake(|t| t.drive_files.files.last().is_some_and(|f| f.bytes == bytes)), "the file's bytes once");
}

/// Final review, net I-2: "Work offline" while a file goes up stops the upload at once (its
/// session cancelled, nothing more asked); the message waits, said as waiting; back online, it
/// goes up again from its start and is posted once.
#[test]
fn working_offline_stops_an_upload_that_goes_again_after() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| t.hold = Some(("upload", Duration::from_secs(60))));
    let bytes = content(6 * MIB);
    let path = file("big.bin", &bytes);
    r.send_with(CHAT, "1-1", "Through an evening offline", vec![Part::File { path }]);
    assert!(until(&mut r, |r| r.hits("upload") >= 1), "the upload never began");
    r.w.work_offline(true);
    assert!(!r.w.transfers.busy(), "stopped at once");
    assert_eq!((first_state(&r), r.outbox_said(CHAT)[0].state.clone()), (Some(OpState::Waiting), OutState::Waiting));
    assert!(wait(&r, |r| r.fake(|t| t.drive_files.sessions.iter().all(|s| s.gone))), "its session cancelled");
    r.turns(Duration::from_secs(60));
    assert_eq!((r.hits("uploadSession"), r.hits("post")), (1, 0), "nothing asked offline");
    r.w.work_offline(false);
    assert!(until(&mut r, |r| r.ops().is_empty()), "{:?}", r.ops());
    assert_eq!((r.hits("uploadSession"), r.hits("post"), r.held(CHAT, "Through an evening offline")), (2, 1, 1));
    assert!(r.fake(|t| t.drive_files.files.last().is_some_and(|f| f.bytes == bytes)), "the file's bytes once");
}

#[test]
fn deleting_a_message_being_uploaded_cancels_it() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| t.hold = Some(("upload", Duration::from_secs(60))));
    let path = file("big.bin", &vec![7u8; 6 * MIB]);
    r.send_with(CHAT, "1-1", "Never mind", vec![Part::File { path }]);
    assert!(until(&mut r, |r| r.hits("upload") >= 1), "the upload never began");
    let seq = r.ops()[0].seq;
    r.w.job(Job::Discard { seq });
    assert!(r.ops().is_empty(), "taken: it was waiting");
    assert!(r.outbox_said(CHAT).is_empty());
    assert!(!r.w.transfers.busy());
    assert!(wait(&r, |r| r.fake(|t| t.drive_files.sessions.iter().all(|s| s.gone))), "the fake holds no session");
    r.turns(Duration::from_secs(10));
    assert_eq!(r.hits("post"), 0);
}

/// A 12 MiB upload held never holds another chat's messages nor the reads; the chat's next
/// message waits behind it.
#[test]
fn uploads_dont_hold_other_chats() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4), chat(OTHER, T0, 4)]));
    r.w.job(Job::Open { chat: Some(CHAT.into()) });
    r.fake(|t| t.hold = Some(("upload", Duration::from_secs(60))));
    let path = file("big.bin", &vec![7u8; 12 * MIB]);
    r.send_with(CHAT, "1-1", "Big", vec![Part::File { path }]);
    assert!(until(&mut r, |r| r.hits("upload") >= 1), "the upload never began");
    let reads = r.reads(CHAT);
    r.send(OTHER, "1-2", "Elsewhere");
    r.send(CHAT, "1-3", "Behind the big one");
    assert!(until(&mut r, |r| r.held(OTHER, "Elsewhere") == 1), "{:?}", r.ops());
    r.turns(Duration::from_secs(5));
    assert!(r.reads(CHAT) > reads, "the open chat is read meanwhile");
    assert_eq!((r.hits("post"), r.held(CHAT, "Behind the big one")), (1, 0));
    let waiting: Vec<OutState> = r.outbox_said(CHAT).into_iter().map(|o| o.state).collect();
    assert_eq!(waiting, [OutState::Uploading, OutState::Waiting]);
}

/// Decision 8: a chat of more than 100 others takes no file.
#[test]
fn a_chat_of_too_many_people_takes_no_file() {
    let people = (1..=101).map(|i| FakeUser::new(&format!("00000000-0000-0000-0000-{i:012x}"), &format!("Person {i}")));
    let crowd = with_messages(FakeChat::new(GROUP, "group", Some("Everyone"), std::iter::once(me()).chain(people).collect()), T0, 2);
    let mut r = run(teams_with(vec![crowd]));
    assert!(r.turns_until(Duration::from_secs(30), |r| r.record(GROUP).chat.members.len() == 102), "the members never came");
    let path = file("report.pdf", &[7u8; 4_000]);
    r.send_with(GROUP, "1-1", "For everyone", vec![Part::File { path }]);
    assert_eq!(r.answered(GROUP), (0, "1-1".to_string(), OutState::NotSent { reason: Unsent::NotShared, code: None }));
    assert!(r.ops().is_empty());
    assert_eq!(r.hits("uploadSession"), 0);
}

/// P-17: a file no one could open (no other member known), or not everyone (someone from outside
/// without an address), isn't taken; a picture alone needs no one.
#[test]
fn a_file_for_people_it_cant_be_shared_with_isnt_taken() {
    let mut stranger = FakeUser::new(BRUNO, "Bruno Reis");
    stranger.mail = String::new();
    let mut t = teams_with(vec![chat(CHAT, T0, 4), with_messages(FakeChat::new(GROUP, "group", Some("Plans"), vec![me(), ana(), stranger]), T0, 2)]);
    t.guests.push(BRUNO.into());
    let mut r = run(t);
    assert!(r.turns_until(Duration::from_secs(30), |r| r.record(GROUP).chat.members.len() == 3), "the members never came");
    r.w.work_offline(true);
    let mut alone = r.record(CHAT);
    alone.chat.members.clear();
    r.w.copy.as_ref().unwrap().put_chat(&alone).unwrap();
    let path = file("report.pdf", &[7u8; 4_000]);
    for (chat, local) in [(CHAT, "1-1"), (GROUP, "1-2")] {
        r.send_with(chat, local, "For whom?", vec![Part::File { path: path.clone() }]);
        assert_eq!(r.answered(chat), (0, local.to_string(), OutState::NotSent { reason: Unsent::NotShared, code: None }), "{chat}");
    }
    let picture = file("paste.png", &png(100));
    r.send_with(CHAT, "1-3", "", vec![Part::Picture { path: picture }]);
    assert_eq!(r.ops().len(), 1, "a picture taken");
}

#[test]
fn signing_out_cancels_uploads_and_forgets_pictures() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| t.hold = Some(("upload", Duration::from_secs(60))));
    let path = file("big.bin", &vec![7u8; 6 * MIB]);
    r.send_with(CHAT, "1-1", "Big", vec![Part::File { path }]);
    assert!(until(&mut r, |r| r.hits("upload") >= 1), "the upload never began");
    let picture = file("paste.png", &png(100));
    r.send_with(CHAT, "1-2", "", vec![Part::Picture { path: picture }]);
    let seq = r.ops()[1].seq;
    assert!(r.w.copy.as_ref().unwrap().picture(seq, 0).is_some(), "kept behind the upload");
    r.w.job(Job::SignOut { delete_copy: false });
    assert!(!r.w.transfers.busy(), "the upload stopped at once");
    let kept = r.w.store.account_files(TEAMS_ACCOUNT, "outbox-parts").unwrap_or_default();
    assert!(kept.is_empty(), "no picture left: {kept:?}");
    assert!(wait(&r, |r| r.fake(|t| t.drive_files.sessions.iter().all(|s| s.gone))), "the fake holds no session");
    assert_eq!(r.hits("post"), 0);
}

/// H18: the log says a file by counts, sizes, statuses and a host's last two labels: never its
/// name, its path, an address or the chat.
#[test]
fn logs_carry_no_name_path_or_address_of_a_file() {
    const HIDDEN: &str = "19:quokka5520@thread.v2";
    let mut r = run(teams_with(vec![chat(HIDDEN, T0, 4)]));
    let tap = crate::fakes::logs::tap(|l| l.starts_with("teams: ") || l.starts_with("Teams copy: "));
    let path = file("wallaby bilby.pdf", &[7u8; 4_000]);
    let picture = file("numbat.png", &png(500));
    r.send_with(HIDDEN, "1-1", "Totals", vec![Part::File { path: path.clone() }, Part::Picture { path: picture }]);
    assert!(r.turns_until(Duration::from_secs(30), |r| r.ops().is_empty()), "{:?}", r.ops());
    r.fake(|t| t.drive_files.item_elsewhere = Some("wombat.example".into()));
    r.send_with(HIDDEN, "1-2", "Again", vec![Part::File { path }]);
    assert!(r.turns_until(Duration::from_secs(30), |r| matches!(first_state(r), Some(OpState::NotSent { .. }))), "{:?}", r.ops());
    let lines = tap.lines();
    assert!(lines.iter().any(|l| l.contains("4000 bytes")), "a size said: {lines:?}");
    let port = r.server.base.rsplit(':').next().unwrap().split('/').next().unwrap().to_string();
    for line in &lines {
        let lower = line.to_lowercase();
        for private in [
            "wallaby",
            "bilby",
            "numbat",
            "wombat",
            "quokka",
            "riley",
            "contoso-my",
            "quatrelle-teams-parts",
            "/upload/",
            "@thread.v2",
            port.as_str(),
        ] {
            assert!(!lower.contains(private), "{private:?} in the log: {line}");
        }
    }
}
