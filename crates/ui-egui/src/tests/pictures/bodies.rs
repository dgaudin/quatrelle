// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the open message's body read and parsed on the reading lane, not in a frame; its
//! pictures taken out of the source read for it (the tests of upstream a75755e, 0f0a402).

use super::threads::{Held, Refused, Threads};
use super::*;

/// Each piece of work handed over (whatever its lane), run in order.
fn run_held(held: &std::sync::Arc<std::sync::Mutex<Vec<crate::Work>>>) -> usize {
    let works: Vec<crate::Work> = std::mem::take(&mut *held.lock().unwrap());
    let n = works.len();
    for w in works {
        (w.run)();
    }
    n
}

/// A message opened is read and parsed on the reading lane: the pane shows a spinner until the
/// lane is done, nothing is parsed in a frame, and its pictures are taken out of the source the
/// lane read (the cache isn't read again for them).
#[test]
fn a_body_is_read_on_the_reading_lane() {
    let (mut a, dir) = app("body-lane");
    let ctx = egui::Context::default();
    let held = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(Held(held.clone()));
    a.defer_bodies();
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9501, pictures(6));
    let before = a.session.bodies_parsed_here();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.reading.as_ref().unwrap().parsed.is_none() && a.session.body_waiting(&k), "waits for the lane");
    assert_eq!(run_held(&held), 1, "the body's work");
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.reading.as_ref().unwrap().parsed.as_ref().unwrap().attachments.len(), 6);
    assert_eq!(a.session.bodies_parsed_here(), before, "nothing parsed in a frame");
    // Its pictures: their work went to the lane too, and starts from the source read for it.
    assert_eq!(run_held(&held), 1, "the pictures' work");
    let (frames, _) = pump(&ctx, &mut a, WINDOW, 50, |a| a.ui.pictures.ready() == 6);
    assert!(frames < 50, "all shown ({} ready)", a.ui.pictures.ready());
    assert_eq!(a.ui.pictures.counts().0, 1, "one extraction");
    assert_eq!(a.ui.pictures.reads(), 0, "from the source read to show the message");
    let _ = std::fs::remove_dir_all(dir);
}

/// Moving down the list faster than the lane reads hands it one piece of work, which reads the
/// message landed on (not each one passed); a reply then acts on that message, read at once.
#[test]
fn the_lane_reads_only_the_latest() {
    let (mut a, dir) = app("body-latest");
    let ctx = egui::Context::default();
    let held = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(Held(held.clone()));
    a.defer_bodies();
    frame(&ctx, &mut a, vec![]);
    let first = a.session.list[0].key.clone();
    a.run("list.open", json!({"key": first.to_id()}));
    frame(&ctx, &mut a, vec![]);
    for _ in 0..4 {
        a.run("list.next", json!({}));
        frame(&ctx, &mut a, vec![]);
    }
    let landed = a.session.list[4].key.clone();
    assert_eq!(a.session.reading.as_ref().unwrap().key, landed);
    assert_eq!(held.lock().unwrap().len(), 1, "one piece of work handed over");
    run_held(&held);
    frame(&ctx, &mut a, vec![]);
    let r = a.session.reading.as_ref().unwrap();
    assert_eq!(r.parsed.as_ref().unwrap().subject, a.session.summary(&landed).unwrap().subject);
    // Another, and a reply before the lane is done.
    a.run("list.next", json!({}));
    frame(&ctx, &mut a, vec![]);
    a.session.prefs.new_mail_window = false;
    a.run("mail.reply", json!({}));
    let next = a.session.list[5].key.clone();
    assert!(a.session.compose.as_ref().is_some_and(|c| c.subject.contains(&a.session.summary(&next).unwrap().subject)));
    run_held(&held);
    frame(&ctx, &mut a, vec![]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Work no thread takes is done in the window: the message shows in the frame it was opened in.
#[test]
fn a_body_no_lane_takes_is_read_in_the_window() {
    let (mut a, dir) = app("body-refused");
    let ctx = egui::Context::default();
    a.services = Box::new(Refused);
    a.defer_bodies();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list[0].key.clone();
    a.run("list.open", json!({"key": k.to_id()}));
    assert!(a.session.body_waiting(&k));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.reading.as_ref().unwrap().parsed.is_some() && !a.session.body_waiting(&k));
    let _ = std::fs::remove_dir_all(dir);
}

/// On a thread of its own (as in the app), holding Down over big messages parses none of them
/// in a frame, and the one landed on shows.
#[test]
fn holding_down_over_big_messages_parses_none_in_a_frame() {
    let (mut a, dir) = app("body-threads");
    let ctx = egui::Context::default();
    a.services = Box::new(Threads);
    a.defer_bodies();
    frame(&ctx, &mut a, vec![]);
    let big = "A long line of text that goes on. ".repeat(30_000);
    let keys: Vec<MessageKey> =
        (0..20u32).map(|i| with_files(&mut a, 9600 + i, vec![(format!("notes {i}.txt"), "text/plain", big.clone().into_bytes())])).collect();
    let before = a.session.bodies_parsed_here();
    a.run("list.open", json!({"key": a.session.list[0].key.to_id()}));
    for _ in 1..keys.len() {
        frame(&ctx, &mut a, vec![]);
        a.run("list.next", json!({}));
    }
    let landed = a.session.reading.as_ref().unwrap().key.clone();
    let shown = |a: &MailApp| a.session.reading.as_ref().is_some_and(|r| r.parsed.is_some());
    for _ in 0..1000 {
        if shown(&a) {
            break;
        }
        frame(&ctx, &mut a, vec![]);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(shown(&a), "shown");
    assert_eq!(a.session.reading.as_ref().unwrap().key, landed);
    assert_eq!(a.session.bodies_parsed_here(), before, "none parsed in a frame");
    let _ = std::fs::remove_dir_all(dir);
}

/// While the lane reads it, the reading pane shows the message's subject, where its header puts
/// it, and no "Downloading" (it isn't); then the message.
#[test]
fn the_pane_shows_the_subject_while_the_lane_reads() {
    let (mut a, dir) = app("body-waiting");
    let ctx = egui::Context::default();
    let held = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(Held(held.clone()));
    a.defer_bodies();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list[0].key.clone();
    let subject = a.session.summary(&k).unwrap().subject.clone();
    a.run("list.open", json!({"key": k.to_id()}));
    let texts = |shapes: Vec<egui::Shape>| -> Vec<String> {
        shapes.into_iter().filter_map(|s| if let egui::Shape::Text(t) = s { Some(t.galley.text().to_string()) } else { None }).collect()
    };
    let drawn = texts(frame_shapes(&ctx, &mut a, vec![], WINDOW));
    assert!(a.session.body_waiting(&k));
    assert!(drawn.contains(&subject), "{drawn:?}");
    assert!(!drawn.iter().any(|t| t.contains("Downloading")), "{drawn:?}");
    run_held(&held);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.reading.as_ref().unwrap().parsed.is_some());
    let _ = std::fs::remove_dir_all(dir);
}

/// Services with a thread for each piece of work, the reading lane's kept to be joined.
struct Joined(std::sync::Arc<std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>>);

impl crate::Services for Joined {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
    fn background(&mut self, lane: crate::Lane, work: crate::Work) -> Result<(), crate::Work> {
        let h = std::thread::spawn(work.run);
        if matches!(lane, crate::Lane::Reading) {
            self.0.lock().unwrap().push(h);
        }
        Ok(())
    }
    fn has_background(&self) -> bool {
        true
    }
}

/// Two threads of the reading lane at once (the desktop's lane starts another when one is caught
/// on cancelled work; here each piece has its own), the older one finishing last: the newer body
/// stays (from upstream eb56351). It was overwritten, and the pane waited for a body that never
/// came, blank.
#[test]
fn an_older_body_finishing_last_is_let_go() {
    let (mut a, dir) = app("body-order");
    let ctx = egui::Context::default();
    let handles = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(Joined(handles.clone()));
    a.defer_bodies();
    frame(&ctx, &mut a, vec![]);
    // Long formatted mail: slow to parse (a quarter of a second in release).
    let html = (0..12_000).map(|i| format!("<p>Line {i} of a <b>long</b> newsletter, <i>formatted</i>.</p>")).collect::<String>();
    let k = with_files_and_html(&mut a, 9650, Vec::new(), Some(html), Vec::new());
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    // Its thread has taken the work and reads the big one.
    for _ in 0..5000 {
        if !a.ui.bodies.waiting_to_run() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    a.run("list.next", json!({}));
    let next = a.session.reading.as_ref().unwrap().key.clone();
    assert_ne!(next, k);
    frame(&ctx, &mut a, vec![]);
    let mut hs: Vec<_> = std::mem::take(&mut *handles.lock().unwrap());
    assert_eq!(hs.len(), 2, "two threads at work");
    let newer = hs.pop().unwrap();
    let older = hs.pop().unwrap();
    newer.join().unwrap();
    let overlapped = !older.is_finished();
    older.join().unwrap();
    assert!(overlapped, "the older one still at work when the newer one was done");
    frame(&ctx, &mut a, vec![]);
    let r = a.session.reading.as_ref().unwrap();
    assert_eq!(r.key, next);
    assert!(r.parsed.is_some() && !a.session.body_waiting(&next), "the newer body is shown");
    let _ = std::fs::remove_dir_all(dir);
}

/// The arc's target: holding Down over 200 big messages on threads of their own, every step
/// (the command and its frame) takes under 16 ms, marked read as they're shown (the fork marks
/// them, and since R10 that's cheap on a server account). On the sample's local account the mark
/// is written by the window: told, not held to the target. A timing test: run in release (`cargo
/// test --release -p quatrelle-ui-egui holding_down_frames -- --ignored --nocapture`).
#[test]
#[ignore = "a bench: run it with --release, --ignored and --nocapture"]
fn holding_down_frames_stay_under_16_ms() {
    let local = hold_down(false);
    let server = hold_down(true);
    assert!(server < std::time::Duration::from_millis(16), "worst step {server:?} (local account: {local:?})");
}

/// Down held over 200 messages of about 1 MB, each marked read as it's shown: the slowest step.
fn hold_down(server: bool) -> std::time::Duration {
    let (mut a, dir) = app(if server { "body-frames-server" } else { "body-frames" });
    if server {
        on_a_server(&mut a);
    }
    let ctx = egui::Context::default();
    a.services = Box::new(Threads);
    a.defer_bodies();
    frame(&ctx, &mut a, vec![]);
    let big = "A long line of text that goes on. ".repeat(30_000);
    for i in 0..200u32 {
        with_files(&mut a, 9700 + i, vec![(format!("notes {i}.txt"), "text/plain", big.clone().into_bytes())]);
    }
    assert!(a.session.prefs.mark_read_on_open);
    a.run("list.open", json!({"key": a.session.list[0].key.to_id()}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let (mut worst, mut total) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
    for _ in 1..200 {
        let t = std::time::Instant::now();
        a.run("list.next", json!({}));
        frame(&ctx, &mut a, vec![]);
        worst = worst.max(t.elapsed());
        total += t.elapsed();
        // A key held down repeats about every 30 ms.
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    let unread = a.session.list.iter().filter(|m| m.key.uid >= 9700 && !m.flags.seen).count();
    println!(
        "{}: worst step {:.1} ms, mean {:.1} ms, {unread} of 200 left unread",
        if server { "server account" } else { "local account" },
        worst.as_secs_f64() * 1e3,
        total.as_secs_f64() * 1e3 / 199.0
    );
    let _ = std::fs::remove_dir_all(dir);
    worst
}
