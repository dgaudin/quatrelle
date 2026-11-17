// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A file someone shared, opened: the worker asks Graph for its download address with the token,
//! the download lane fetches it without and saves it for the user alone. Hostile names, sizes,
//! redirects and servers; only a file the copy's message carries; one at a time and eight
//! waiting; short of room, offline, switched off mid-way; and what the log says.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use quatrelle_mime::sanitize_filename;
use quatrelle_model::pim::teams::{self as model, FileOutcome, FileWhy, Job};

use super::super::sync::{OTHER, Run, STEP, run_with};
use super::*;
use crate::http::Url;
use crate::teams::files::save;

/// The message carrying the files: the third of [`chat`]'s four, and when it was made.
const FILE_AT: i64 = T0 + 2 * MINUTE;
/// The file's attachment id, and the address its attachment carries.
const G: &str = "00000000-0000-0000-0000-0000000000f1";
const SHARED_AT: &str = "https://contoso.sharepoint.com/sites/team/Shared%20Documents/plan.pdf";
/// What the tests' worker keeps to: 16 KB a second over a minute, as Quatrelle's.
const FLOOR: (u64, Duration) = (16 << 10, Duration::from_secs(60));

fn file_msg() -> String {
    FILE_AT.to_string()
}

/// What a folder holds, by name.
fn listed(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

fn bytes(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8).collect()
}

/// [`chat`]'s chat whose third message carries these files (attachment id, name, address), each
/// with its bytes among the fake's shared files.
fn teams(files: &[(&str, &str, &str, &[u8])]) -> FakeTeams {
    let mut c = chat(CHAT, T0, 4);
    let mut t = teams_with(Vec::new());
    for (id, name, url, content) in files {
        c.messages[2].files.push((id.to_string(), name.to_string(), url.to_string()));
        t.shares.insert(url.to_string(), content.to_vec());
    }
    t.chats.push(c);
    t
}

/// One file `G` named `name`, of these bytes.
fn one(name: &str, content: &[u8]) -> FakeTeams {
    teams(&[(G, name, SHARED_AT, content)])
}

/// A worker signed in to `t`, its files saved in `dir`, the message in its copy.
fn started(t: FakeTeams, dir: &Path) -> Run {
    started_with(t, dir, None)
}

/// [`started`], its transfers kept to `floor` when one is given.
fn started_with(t: FakeTeams, dir: &Path, floor: Option<(u64, Duration)>) -> Run {
    let dir = dir.to_path_buf();
    let mut r = run_with(t, move |w| {
        w.hooks.files_dir = Some(dir);
        w.hooks.floor = floor;
    });
    let until = r.at + Duration::from_secs(120);
    while r.w.copy.as_ref().unwrap().message_at(CHAT, &file_msg(), FILE_AT).is_none() {
        assert!(r.at < until, "the message never came into the copy");
        r.turns(STEP);
    }
    r
}

fn ask(r: &mut Run, attachment: &str) {
    r.w.job(Job::File { chat: CHAT.into(), message: file_msg(), created: FILE_AT, attachment: attachment.into() });
}

/// Every outcome said of `attachment`, in order.
fn said(r: &Run, attachment: &str) -> Vec<FileOutcome> {
    r.events()
        .into_iter()
        .filter_map(|e| match e {
            model::Event::File { attachment: a, outcome, chat, .. } if a == attachment && chat == CHAT => Some(outcome),
            _ => None,
        })
        .collect()
}

/// Turns, a step apart and a few milliseconds of this computer's clock between them (the lanes
/// are threads), until an outcome of `attachment` is said: the first.
fn outcome(r: &mut Run, attachment: &str) -> FileOutcome {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(o) = said(r, attachment).into_iter().next() {
            return o;
        }
        assert!(Instant::now() < until, "nothing said of {attachment}: {:?}", r.events());
        r.turns(STEP);
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn saved(o: FileOutcome) -> PathBuf {
    match o {
        FileOutcome::Saved { path } => PathBuf::from(path),
        other => panic!("not saved: {other:?}"),
    }
}

fn failed(why: FileWhy, code: Option<&str>) -> FileOutcome {
    FileOutcome::Failed { why, code: code.map(str::to_string) }
}

#[test]
fn a_shared_file_is_saved_and_said() {
    let dir = Scratch::new("saved");
    let content = bytes(100_000);
    let mut r = started(one("plan.pdf", &content), &dir);
    ask(&mut r, G);
    let path = saved(outcome(&mut r, G));
    assert_eq!(path, dir.join("plan.pdf"));
    assert_eq!(std::fs::read(&path).unwrap(), content);
    assert_eq!((r.hits("shares"), r.hits("download")), (1, 1));
    let shares = headers(&r.server, "shares");
    assert!(shares.iter().any(|(k, v)| k == "authorization" && *v == format!("Bearer {TOKEN}")), "{shares:?}");
    assert!(!shares.iter().any(|(k, _)| k == "prefer"), "never Prefer: redeemSharingLink: {shares:?}");
    assert!(!r.w.transfers.busy());
    // Asked again later: downloaded again, beside the first.
    ask(&mut r, G);
    let until = Instant::now() + Duration::from_secs(20);
    while said(&r, G).len() < 2 {
        assert!(Instant::now() < until);
        r.turns(STEP);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(said(&r, G)[1], FileOutcome::Saved { path: dir.join("plan (2).pdf").to_string_lossy().into_owned() });
}

/// H5: the download goes without the token, and without a cookie.
#[test]
fn no_token_goes_to_a_transfer() {
    let dir = Scratch::new("no-token");
    let mut r = started(one("plan.pdf", b"bytes"), &dir);
    ask(&mut r, G);
    saved(outcome(&mut r, G));
    let download = headers(&r.server, "download");
    assert!(!download.is_empty());
    assert!(!download.iter().any(|(k, _)| k == "authorization" || k == "cookie"), "{download:?}");
}

/// H1: a name made safe by `sanitize_filename`, inside the folder, its extension kept within 180
/// characters. The copy keeps 255 characters of a name (its reader's cap): a name of 400 loses its
/// extension before it reaches the worker, and is still cut to 180.
#[test]
fn a_hostile_name_is_saved_under_a_safe_one() {
    let dir = Scratch::new("hostile");
    let long = format!("{}.pdf", "x".repeat(250));
    let longer = format!("{}.pdf", "y".repeat(400));
    let names: [(&str, &str); 9] = [
        ("00000000-0000-0000-0000-0000000000b1", "../../x.txt"),
        ("00000000-0000-0000-0000-0000000000b2", "CON.txt"),
        ("00000000-0000-0000-0000-0000000000b3", "a\u{202E}fdp.exe"),
        ("00000000-0000-0000-0000-0000000000b4", &long),
        ("00000000-0000-0000-0000-0000000000b5", &longer),
        ("00000000-0000-0000-0000-0000000000b6", ""),
        ("00000000-0000-0000-0000-0000000000b7", "evil.exe."),
        ("00000000-0000-0000-0000-0000000000b8", "evil.exe "),
        ("00000000-0000-0000-0000-0000000000b9", "re\u{7}port\u{1}.txt"),
    ];
    let urls: Vec<String> = (1..=9).map(|i| format!("https://contoso.sharepoint.com/sites/team/f{i}.bin")).collect();
    let files: Vec<(&str, &str, &str, &[u8])> = names.iter().zip(&urls).map(|((id, name), url)| (*id, *name, url.as_str(), &b"x"[..])).collect();
    let mut r = started(teams(&files), &dir);
    let mut got = Vec::new();
    for (id, _) in &names {
        ask(&mut r, id);
        let path = saved(outcome(&mut r, id));
        assert_eq!(path.parent(), Some(&*dir), "{path:?}");
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(sanitize_filename(&name), name, "its own safe name");
        assert!(name.chars().count() <= 180, "{}", name.chars().count());
        got.push(name);
    }
    assert_eq!(got[0], "x.txt");
    assert_eq!(got[1], "_CON.txt");
    assert_eq!(got[2], "afdp.exe");
    assert_eq!((got[3].chars().count(), got[3].ends_with(".pdf")), (180, true), "{}", got[3]);
    assert!(got[4].starts_with("yyy") && got[4].chars().count() == 180, "{}", got[4]);
    assert_eq!(got[5], "attachment");
    assert_eq!(got[6], "evil.exe", "a trailing dot trimmed, the extension kept");
    assert_eq!(got[7], "evil (2).exe", "the number before the extension, whatever ends the name");
    assert_eq!(got[8], "report.txt", "a control character: none in the copy's name");
    assert_eq!(listed(&dir).len(), 9, "nothing outside the folder: {:?}", listed(&dir));
}

/// H7: a name taken gets a number, a link planted at the next is passed over, and nothing is
/// written over or followed.
#[test]
fn a_name_already_there_gets_a_number_and_nothing_is_written_over() {
    let dir = Scratch::new("taken");
    let elsewhere = Scratch::new("taken-outside");
    let outside = elsewhere.join("target.pdf");
    std::fs::write(dir.join("report.pdf"), b"mine").unwrap();
    std::fs::write(&outside, b"outside").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, dir.join("report (2).pdf")).unwrap();
    let mut r = started(one("report.pdf", b"theirs"), &dir);
    ask(&mut r, G);
    let path = saved(outcome(&mut r, G));
    let want = if cfg!(unix) { "report (3).pdf" } else { "report (2).pdf" };
    assert_eq!(path, dir.join(want));
    assert_eq!(std::fs::read(&path).unwrap(), b"theirs");
    assert_eq!(std::fs::read(dir.join("report.pdf")).unwrap(), b"mine");
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside", "the link's target untouched");
}

/// H7: the file saved is readable by the user alone.
#[cfg(unix)]
#[test]
fn a_saved_file_is_the_users_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Scratch::new("mode");
    let mut r = started(one("plan.pdf", b"bytes"), &dir);
    ask(&mut r, G);
    let path = saved(outcome(&mut r, G));
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
}

/// H3: the cap counted while reading, whatever the length says; nothing past what an answer says
/// of its length is written; and whatever is refused leaves no file.
#[test]
fn a_download_past_the_cap_stops_and_leaves_no_file() {
    const CAP: u64 = 1 << 20;
    let dir = Scratch::new("cap");
    let fetch = |base: &str| {
        let from = Url::parse(&format!("{base}/download/x")).unwrap();
        save(&from, "big.bin", &dir, base, &Arc::new(AtomicBool::new(false)), FLOOR, CAP)
    };
    let head = |len: Option<u64>| match len {
        Some(n) => format!("HTTP/1.1 200 OK\r\nContent-Length: {n}\r\n\r\n").into_bytes(),
        None => b"HTTP/1.0 200 OK\r\n\r\n".to_vec(),
    };
    let with = |mut h: Vec<u8>, n: u64| {
        h.extend(bytes(n as usize));
        h
    };
    // A chunk said past the cap: refused at its size line, its bytes never read (a few are
    // sent: the server must not be writing when the connection closes).
    let mut chunked = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n", CAP + 1).into_bytes();
    chunked.extend(bytes(100));
    // The answers whose body is read come first, those refused before it last.
    let (base, h) = crate::http::tests::serve(vec![
        with(head(None), CAP + 1),
        with(head(Some(CAP)), CAP),
        with(head(Some(10)), 64 << 10),
        head(Some(CAP + 1)),
        chunked,
    ]);
    assert_eq!(fetch(&base), Err((FileWhy::TooBig, None)), "no length, more than the cap");
    assert!(listed(&dir).is_empty());
    let path = fetch(&base).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), CAP, "the cap itself is taken");
    std::fs::remove_file(&path).unwrap();
    let path = fetch(&base).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes(10), "a length that says less: what it says, never a byte more");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(fetch(&base), Err((FileWhy::TooBig, None)), "a length past the cap, refused before its body");
    assert!(listed(&dir).is_empty());
    assert_eq!(fetch(&base), Err((FileWhy::TooBig, None)), "a chunk past the cap");
    assert!(listed(&dir).is_empty());
    let _ = h.join();
}

/// P-22: the download lane goes to an address on the internet only, but for the tests' servers on
/// this computer: a client of Microsoft's Graph never reaches this computer.
#[test]
fn a_download_never_reaches_this_computer_for_microsofts_graph() {
    let dir = Scratch::new("public");
    let (base, h) = crate::http::tests::serve(vec![b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec()]);
    let from = Url::parse(&format!("{base}/download/x")).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let got = save(&from, "x.bin", &dir, "https://graph.microsoft.com/v1.0", &cancel, FLOOR, 1 << 20);
    assert_eq!(got, Err((FileWhy::Refused, None)));
    assert!(listed(&dir).is_empty());
    // The server saw nothing; it is let go by a request of its own.
    assert!(save(&from, "x.bin", &dir, &base, &cancel, FLOOR, 1 << 20).is_ok());
    assert_eq!(h.join().unwrap().len(), 1);
}

/// H5: a first hop that sends the download off Microsoft's file hosts is refused before any
/// connection, its address said nowhere.
#[test]
fn a_download_redirected_elsewhere_is_refused_before_any_connection() {
    let dir = Scratch::new("elsewhere");
    let mut r = started(one("plan.pdf", b"bytes"), &dir);
    for (i, to) in ["https://evil.example/x", "http://169.254.169.254/x", "https://evilsharepoint.com/x"].into_iter().enumerate() {
        r.fake(|t| t.drive_files.share_location = Some(to.into()));
        ask(&mut r, G);
        let until = Instant::now() + Duration::from_secs(20);
        while said(&r, G).len() <= i {
            assert!(Instant::now() < until, "nothing said for {to}");
            r.turns(STEP);
        }
        assert_eq!(said(&r, G)[i], failed(FileWhy::Refused, None), "{to}");
    }
    assert_eq!(r.hits("download"), 0);
    assert!(listed(&dir).is_empty());
}

/// H5: the download's own answer redirecting again is refused: no second hop, no file.
#[test]
fn a_second_redirect_is_an_error() {
    let dir = Scratch::new("second");
    let mut r = started(one("plan.pdf", b"bytes"), &dir);
    r.fake(|t| t.drive_files.download_redirect = true);
    ask(&mut r, G);
    assert_eq!(outcome(&mut r, G), failed(FileWhy::Refused, Some("HTTP 302")));
    assert_eq!(r.hits("download"), 1);
    assert!(listed(&dir).is_empty());
}

/// H6: only an attachment the copy's message carries, of that chat, not deleted, a file.
#[test]
fn only_a_file_the_copys_message_carries_is_downloaded() {
    let dir = Scratch::new("carried");
    let mut t = one("plan.pdf", b"bytes");
    let card = format!("{}-0", file_msg());
    t.chats[0].messages[2].attachments.push(("application/vnd.microsoft.card.adaptive".into(), String::new()));
    // A deleted message carrying the file, and another chat's message carrying it.
    let deleted_at = T0 + 3 * MINUTE;
    let d = &mut t.chats[0].messages[3];
    d.files.push((G.into(), "plan.pdf".into(), SHARED_AT.into()));
    d.deleted = Some(crate::fake_graph::rfc3339(deleted_at + 1000));
    let theirs = T0 + 10 * MINUTE;
    let mut other = chat(OTHER, theirs, 1);
    other.messages[0].files.push((G.into(), "plan.pdf".into(), SHARED_AT.into()));
    t.chats.push(other);
    let mut r = started(t, &dir);
    let asks = [
        // An attachment the message doesn't have.
        (file_msg(), FILE_AT, "00000000-0000-0000-0000-0000000000ff".to_string()),
        // Another chat's message, asked as this chat's.
        (theirs.to_string(), theirs, G.to_string()),
        (deleted_at.to_string(), deleted_at, G.to_string()),
        (file_msg(), FILE_AT, card),
    ];
    for (message, created, attachment) in asks {
        r.w.job(Job::File { chat: CHAT.into(), message, created, attachment: attachment.clone() });
        assert_eq!(said(&r, &attachment).last(), Some(&failed(FileWhy::Gone, None)), "said at once: {attachment}");
    }
    r.turns(Duration::from_secs(2));
    assert_eq!(r.hits("shares"), 0);
    assert!(said(&r, G).iter().all(|o| *o == failed(FileWhy::Gone, None)), "{:?}", said(&r, G));
}

/// One download at a time and eight waiting: the tenth asked is refused as busy, and a file asked
/// twice is downloaded once.
#[test]
fn one_download_at_a_time_and_eight_waiting_at_most() {
    let dir = Scratch::new("queue");
    let ids: Vec<String> = (0..10).map(|i| format!("00000000-0000-0000-0000-0000000000c{i}")).collect();
    let urls: Vec<String> = (0..10).map(|i| format!("https://contoso.sharepoint.com/sites/team/q{i}.bin")).collect();
    let names: Vec<String> = (0..10).map(|i| format!("q{i}.bin")).collect();
    let files: Vec<(&str, &str, &str, &[u8])> = (0..10).map(|i| (ids[i].as_str(), names[i].as_str(), urls[i].as_str(), &b"q"[..])).collect();
    let mut t = teams(&files);
    t.hold = Some(("download", Duration::from_secs(2)));
    let mut r = started(t, &dir);
    ask(&mut r, &ids[0]);
    ask(&mut r, &ids[0]);
    let until = Instant::now() + Duration::from_secs(20);
    while r.hits("download") == 0 {
        assert!(Instant::now() < until);
        r.turns(STEP);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(r.hits("shares"), 1, "asked twice, its address asked once");
    assert!(r.w.transfers.busy());
    for id in &ids[1..] {
        ask(&mut r, id);
    }
    assert_eq!(said(&r, &ids[9]), [failed(FileWhy::Busy, None)]);
    assert!(ids[..9].iter().all(|id| said(&r, id).is_empty()));
    for id in &ids[..9] {
        saved(outcome(&mut r, id));
    }
    assert_eq!(r.hits("shares"), 9);
    assert_eq!(listed(&dir).len(), 9);
    assert!(ids[..9].iter().all(|id| said(&r, id).len() == 1), "each said once");
}

#[test]
fn a_refused_or_missing_file_says_its_code() {
    let dir = Scratch::new("code");
    let mut r = started(one("plan.pdf", b"bytes"), &dir);
    r.fake(|t| {
        t.not_found_next.insert("shares".into(), 1);
    });
    ask(&mut r, G);
    assert_eq!(outcome(&mut r, G), failed(FileWhy::Refused, Some("HTTP 404 NotFound")));
    assert_eq!(r.hits("download"), 0);
}

#[test]
fn short_of_room_nothing_is_downloaded() {
    const GB: u64 = 1 << 30;
    let dir = Scratch::new("room");
    let mut r = started(one("plan.pdf", b"bytes"), &dir);
    r.w.backfill.probe = Some((GB, 100 * GB));
    ask(&mut r, G);
    assert_eq!(outcome(&mut r, G), failed(FileWhy::NoRoom, None));
    assert_eq!(r.hits("shares"), 0);
    assert!(listed(&dir).is_empty());
}

#[test]
fn offline_or_signed_out_nothing_is_asked() {
    let dir = Scratch::new("offline");
    let mut r = started(one("plan.pdf", b"bytes"), &dir);
    r.w.work_offline(true);
    ask(&mut r, G);
    assert_eq!(said(&r, G), [failed(FileWhy::Offline, None)], "said at once");
    r.w.work_offline(false);
    r.w.job(Job::SignOut { delete_copy: false });
    ask(&mut r, G);
    assert_eq!(said(&r, G), [failed(FileWhy::Offline, None), failed(FileWhy::Offline, None)]);
    r.turns(Duration::from_secs(2));
    assert_eq!(r.hits("shares"), 0);
}

/// H4: a download that stalls mid-body is given up at the floor, "Stalled", and the part written
/// removed.
#[test]
fn a_download_slower_than_the_floor_is_given_up() {
    let dir = Scratch::new("slow");
    let mut t = one("plan.pdf", &bytes(100_000));
    t.drive_files.stall_after = Some(1000);
    let mut r = started_with(t, &dir, Some((10_000, Duration::from_millis(500))));
    ask(&mut r, G);
    assert_eq!(outcome(&mut r, G), failed(FileWhy::Stalled, None));
    assert!(listed(&dir).is_empty());
}

/// H19: Teams switched off mid-download: said stopped within a second, and the part already
/// written removed.
#[test]
fn stopping_teams_stops_a_transfer_and_leaves_no_file() {
    let dir = Scratch::new("stop");
    let mut t = one("plan.pdf", &bytes(100_000));
    t.drive_files.stall_after = Some(1000);
    let mut r = started(t, &dir);
    ask(&mut r, G);
    let until = Instant::now() + Duration::from_secs(20);
    while listed(&dir).is_empty() || std::fs::metadata(dir.join("plan.pdf")).map_or(0, |m| m.len()) < 1000 {
        assert!(Instant::now() < until, "the download never began");
        r.turns(STEP);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(said(&r, G).is_empty() && r.w.transfers.busy());
    let stopped = Instant::now();
    r.w.job(Job::Stop);
    assert_eq!(said(&r, G), [failed(FileWhy::Offline, None)]);
    assert!(!r.w.transfers.busy());
    assert!(stopped.elapsed() < Duration::from_secs(1));
    while !listed(&dir).is_empty() {
        assert!(stopped.elapsed() < Duration::from_secs(1), "the part written is still there: {:?}", listed(&dir));
        std::thread::sleep(Duration::from_millis(10));
    }
    // The lane's late word, if any, is dropped: nothing more is said.
    std::thread::sleep(Duration::from_millis(300));
    r.w.turn(r.t0 + r.at);
    assert_eq!(said(&r, G).len(), 1);
}

/// H18: the log says a file by its chat's keyed hash, sizes, statuses and a host's last two
/// labels: never its name, a path or an address.
#[test]
fn logs_carry_no_name_path_or_address_of_a_file() {
    let dir = Scratch::new("numbat-logs");
    let url = "https://contoso.sharepoint.com/sites/bilby/Shared%20Documents/quokka.pdf";
    let other = "00000000-0000-0000-0000-0000000000d2";
    let mut r = started(
        teams(&[
            (G, "wallaby quokka.pdf", url, &bytes(5000)),
            (other, "wallaby two.pdf", "https://contoso.sharepoint.com/sites/bilby/two.pdf", b"2"),
        ]),
        &dir,
    );
    let tap = crate::fakes::logs::tap(|l| l.starts_with("teams: "));
    ask(&mut r, G);
    saved(outcome(&mut r, G));
    r.fake(|t| {
        t.not_found_next.insert("shares".into(), 1);
    });
    ask(&mut r, other);
    assert!(matches!(outcome(&mut r, other), FileOutcome::Failed { .. }));
    r.fake(|t| t.drive_files.share_location = Some("https://wombat.example/x".into()));
    ask(&mut r, other);
    let until = Instant::now() + Duration::from_secs(20);
    while said(&r, other).len() < 2 {
        assert!(Instant::now() < until);
        r.turns(STEP);
    }
    let lines = tap.lines();
    assert!(lines.iter().any(|l| l.contains("a file") && l.contains("HTTP 404")), "a failure said by its status: {lines:?}");
    assert!(lines.iter().any(|l| l.contains("5000 bytes")), "a size said: {lines:?}");
    let base = r.server.base.clone();
    let port = base.rsplit(':').next().unwrap().split('/').next().unwrap().to_string();
    for line in &lines {
        let lower = line.to_lowercase();
        for private in ["wallaby", "quokka", "bilby", "numbat", "wombat", "u!", "/download/", "@thread.v2", "quatrelle-teams-files", port.as_str()] {
            assert!(!lower.contains(private), "{private:?} in the log: {line}");
        }
    }
}
