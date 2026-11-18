// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A file's bytes sent up to an upload session of the user's OneDrive, by the upload lane alone,
//! on the fake's sessions: ranges of 5 MiB in order, without the token; a range cut taken up where
//! Graph says; Graph's answers that go back; a session gone; a file changed since; a refusal's
//! code; a cancel that cancels the session too.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::*;
use crate::teams::files::{Done, Source, Transfers, UploadFail, modified_of, send_up};

/// What the tests' lane keeps to: 16 KB a second over a minute, as Quatrelle's.
const FLOOR: (u64, Duration) = (16 << 10, Duration::from_secs(60));
const MIB: u64 = 1 << 20;

fn content(n: u64) -> Vec<u8> {
    (0..n).map(|i| (i % 253) as u8).collect()
}

/// A file of `n` bytes in a folder of the test's own (removed with it), and what the outbox would
/// keep of it.
fn file(name: &str, n: u64) -> (Scratch, PathBuf, Source) {
    let dir = Scratch::new(&format!("up-{name}"));
    let path = dir.join("big.bin");
    std::fs::write(&path, content(n)).unwrap();
    let modified = modified_of(&std::fs::metadata(&path).unwrap());
    (dir, path.clone(), Source::File { path, size: n, modified })
}

fn server() -> Server {
    serve_teams(teams_with(vec![chat(CHAT, T0, 1)]))
}

fn flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

/// The ranges the fake took: (first byte, last byte, total).
fn ranges(server: &Server) -> Vec<(u64, u64, u64)> {
    fake(server, |t| t.drive_files.ranges.iter().map(|(_, a, b, n)| (*a, *b, *n)).collect())
}

/// The bytes of the file the fake made, by its item's id.
fn made(server: &Server, item: &str) -> Vec<u8> {
    fake(server, |t| t.drive_files.files.iter().find(|f| f.id == item).map(|f| f.bytes.clone()).unwrap())
}

/// The status reads the fake had: its upload requests less the ranges put.
fn status_reads(server: &Server) -> usize {
    hits(server, "upload") as usize - ranges(server).len()
}

/// Every word of a lane, taken until its last for that upload (or 20 s).
fn words(t: &mut Transfers) -> Vec<Done> {
    let until = Instant::now() + Duration::from_secs(20);
    let mut said = Vec::new();
    loop {
        while let Some(d) = t.take() {
            let last = !matches!(d, Done::Sent { .. });
            said.push(d);
            if last {
                return said;
            }
        }
        assert!(Instant::now() < until, "the upload never ended: {said:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// 12 MiB in three ranges of 5, 5 and 2 MiB, in order, each said as it goes, the last making the
/// file; the session's address without the token.
#[test]
fn a_file_goes_up_in_ranges_of_five_mebibytes() {
    let server = server();
    let (_dir, path, source) = file("ranges", 12 * MIB);
    let session = client(&server).upload_session("big.bin").unwrap();
    let mut t = Transfers::default();
    assert!(t.upload(7, 1, session, source, &server.base, FLOOR));
    assert!(t.busy());
    let said = words(&mut t);
    let item = match said.last() {
        Some(Done::Uploaded { item, .. }) => item.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        said,
        [
            Done::Sent { seq: 7, part: 1, sent: 5_242_880 },
            Done::Sent { seq: 7, part: 1, sent: 10_485_760 },
            Done::Uploaded { seq: 7, part: 1, item: item.clone() },
        ]
    );
    assert_eq!(CHUNK, 5_242_880);
    assert_eq!(ranges(&server), [(0, 5_242_879, 12_582_912), (5_242_880, 10_485_759, 12_582_912), (10_485_760, 12_582_911, 12_582_912)]);
    assert_eq!(made(&server, &item), std::fs::read(&path).unwrap());
    assert!(!headers(&server, "upload").iter().any(|(k, _)| k == "authorization"));
    assert!(!t.busy());
}

/// A range cut after OneDrive took it: where it stands is read once, and the upload goes on from
/// there, every byte sent once.
#[test]
fn a_range_cut_is_taken_up_where_graph_says() {
    let server = server();
    let (_dir, path, source) = file("cut", 12 * MIB);
    let session = client(&server).upload_session("big.bin").unwrap();
    let cut = |sent: u64| {
        if sent == CHUNK as u64 {
            fake(&server, |t| t.cut_after.insert("upload".into(), 1));
        }
    };
    let item = send_up(&session, &source, &server.base, &flag(), FLOOR, &cut).unwrap();
    assert_eq!(status_reads(&server), 1);
    assert_eq!(ranges(&server), [(0, 5_242_879, 12_582_912), (5_242_880, 10_485_759, 12_582_912), (10_485_760, 12_582_911, 12_582_912)]);
    assert_eq!(made(&server, &item), std::fs::read(&path).unwrap());
}

/// H9: a status that goes back to the start once 5 MiB went ends the upload as the network's,
/// after one read: never sent again from the start, never read for ever.
#[test]
fn ranges_that_go_backwards_end_the_upload() {
    let server = server();
    let (_dir, _, source) = file("back", 12 * MIB);
    let session = client(&server).upload_session("big.bin").unwrap();
    fake(&server, |t| t.drive_files.status_back = true);
    let cut = |sent: u64| {
        if sent == CHUNK as u64 {
            fake(&server, |t| t.cut_after.insert("upload".into(), 1));
        }
    };
    assert_eq!(send_up(&session, &source, &server.base, &flag(), FLOOR, &cut), Err(UploadFail::Network));
    assert!((1..=64).contains(&status_reads(&server)), "{}", status_reads(&server));
    assert_eq!(ranges(&server).len(), 2, "nothing sent again");
}

/// Final review, net I-1: a range OneDrive answers 429 with a `Retry-After` of 2 s is waited whole,
/// on the lane's own thread, before the session is asked anything again; then the upload goes on
/// from where it stands, every byte once.
#[test]
fn a_busy_range_waits_its_retry_after_whole() {
    let server = server();
    let (_dir, path, source) = file("busy", 6 * MIB);
    let session = client(&server).upload_session("big.bin").unwrap();
    fake(&server, |t| t.drive_files.busy_ranges = Some((1, 2)));
    let started = Instant::now();
    let item = send_up(&session, &source, &server.base, &flag(), FLOOR, &|_| {}).unwrap();
    assert!(started.elapsed() >= Duration::from_secs(2), "waited {:?}", started.elapsed());
    assert_eq!(ranges(&server), [(0, 5_242_879, 6_291_456), (5_242_880, 6_291_455, 6_291_456)], "every byte once");
    assert_eq!(made(&server, &item), std::fs::read(&path).unwrap());
}

#[test]
fn a_session_gone_is_said_so() {
    let server = server();
    let (_dir, _, source) = file("gone", 12 * MIB);
    let session = client(&server).upload_session("big.bin").unwrap();
    fake(&server, |t| t.drive_files.expire_after = Some(1));
    assert_eq!(send_up(&session, &source, &server.base, &flag(), FLOOR, &|_| {}), Err(UploadFail::SessionGone));
}

/// H11: the file is sent only as it was when attached: another size, a later time, gone, a folder
/// are refused before a byte goes.
#[test]
fn a_file_changed_or_gone_isnt_sent() {
    let server = server();
    let session = client(&server).upload_session("big.bin").unwrap();
    let (_dir, path, Source::File { size, modified, .. }) = file("changed", MIB);
    let send = |source: Source| send_up(&session, &source, &server.base, &flag(), FLOOR, &|_| {});
    assert_eq!(send(Source::File { path: path.clone(), size: size + 1, modified }), Err(UploadFail::Gone), "another size");
    assert_eq!(send(Source::File { path: path.clone(), size, modified: modified + 1000 }), Err(UploadFail::Gone), "a later time");
    let folder = path.parent().unwrap().to_path_buf();
    assert_eq!(send(Source::File { path: folder, size, modified }), Err(UploadFail::Gone), "a folder");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(send(Source::File { path, size, modified }), Err(UploadFail::Gone), "removed");
    assert_eq!(hits(&server, "upload"), 0);
}

#[test]
fn a_refusal_keeps_its_code() {
    let server = server();
    let (_dir, _, source) = file("refused", 6 * MIB);
    let session = client(&server).upload_session("big.bin").unwrap();
    fake(&server, |t| t.drive_files.refuse_ranges = Some(403));
    assert_eq!(send_up(&session, &source, &server.base, &flag(), FLOOR, &|_| {}), Err(UploadFail::Refused(Some("HTTP 403 accessDenied".into()))));
}

/// H19: an upload cancelled cancels its session (best effort, on a connection of its own), and the
/// lane's late word is never heard.
#[test]
fn a_cancelled_upload_cancels_its_session() {
    let server = server();
    let (_dir, _, source) = file("cancel", 12 * MIB);
    let session = client(&server).upload_session("big.bin").unwrap();
    let cancel = flag();
    let stop = |sent: u64| {
        if sent == CHUNK as u64 {
            cancel.store(true, Ordering::SeqCst);
        }
    };
    assert_eq!(send_up(&session, &source, &server.base, &cancel, FLOOR, &stop), Err(UploadFail::Cancelled));
    assert_eq!(ranges(&server).len(), 1);
    assert!(fake(&server, |t| t.drive_files.sessions.iter().all(|s| s.gone)), "the fake holds no session");
    assert_eq!(hits(&server, "upload"), 2, "a range, then the cancel");

    // Through the lane: cancelled, it is let go at once, and nothing of it is heard after.
    let (_dir, _, source) = file("cancel-lane", 12 * MIB);
    let session = client(&server).upload_session("big.bin").unwrap();
    let mut t = Transfers::default();
    assert!(t.upload(9, 0, session, source.clone(), &server.base, FLOOR));
    let session = client(&server).upload_session("big.bin").unwrap();
    assert!(!t.upload(9, 1, session, source, &server.base, FLOOR), "one upload of a message at a time");
    t.cancel_upload(9);
    assert!(!t.busy());
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(t.take(), None);
}

/// P-22: the upload lane goes to an address on the internet only, but for the tests' servers on
/// this computer: a client of Microsoft's Graph never reaches this computer.
#[test]
fn an_upload_never_reaches_this_computer_for_microsofts_graph() {
    let server = server();
    let (_dir, _, source) = file("public", MIB);
    let session = client(&server).upload_session("big.bin").unwrap();
    let got = send_up(&session, &source, "https://graph.microsoft.com/v1.0", &flag(), FLOOR, &|_| {});
    assert_eq!(got, Err(UploadFail::Refused(None)));
    assert_eq!(hits(&server, "upload"), 0);
}
