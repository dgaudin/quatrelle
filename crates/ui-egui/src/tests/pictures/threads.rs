// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a message's pictures decoded on the pictures lane, none of it in a frame, and in the
//! window a step a frame where there's no lane (the tests of upstream fe86139).

use super::*;

/// Services with threads for slow work: each piece on a thread of its own.
pub(super) struct Threads;

impl crate::Services for Threads {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
    fn background(&mut self, _: crate::Lane, work: crate::Work) -> Result<(), crate::Work> {
        std::thread::spawn(work.run);
        Ok(())
    }
    fn has_background(&self) -> bool {
        true
    }
}

/// Services that keep the work handed to them, for the test to run when it says.
pub(super) struct Held(pub(super) std::sync::Arc<std::sync::Mutex<Vec<crate::Work>>>);

impl crate::Services for Held {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
    fn background(&mut self, _: crate::Lane, work: crate::Work) -> Result<(), crate::Work> {
        self.0.lock().unwrap().push(work);
        Ok(())
    }
    fn has_background(&self) -> bool {
        true
    }
}

/// Services that say there are threads, but give the work back (one couldn't be started).
pub(super) struct Refused;

impl crate::Services for Refused {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
    fn background(&mut self, _: crate::Lane, work: crate::Work) -> Result<(), crate::Work> {
        Err(work)
    }
    fn has_background(&self) -> bool {
        true
    }
}

/// Work handed to the pictures lane isn't done in the window as well: with the lane holding it,
/// the frames decode nothing and read nothing (each picture was decoded in a frame, threads or
/// not).
#[test]
fn work_handed_to_the_lane_is_not_done_in_frames() {
    let (mut a, dir) = app("pictures-probe");
    let ctx = egui::Context::default();
    let held = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(Held(held.clone()));
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9019, pictures(36));
    a.run("list.open", json!({"key": k.to_id()}));
    pump(&ctx, &mut a, WINDOW, 60, |_| false);
    eprintln!("in the frames: {:?} (reads, decodes); {} handed to the lane", a.ui.pictures.counts(), held.lock().unwrap().len());
    assert_eq!(a.ui.pictures.counts(), (0, 0), "nothing read or decoded in a frame");
    let _ = std::fs::remove_dir_all(dir);
}

/// Work a thread gives back is done in the window, a step a frame: every picture still shows.
#[test]
fn work_given_back_is_done_in_the_window() {
    let (mut a, dir) = app("pictures-refused");
    let ctx = egui::Context::default();
    a.services = Box::new(Refused);
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9020, pictures(12));
    a.run("list.open", json!({"key": k.to_id()}));
    let (frames, _) = pump(&ctx, &mut a, WINDOW, 200, |a| a.ui.pictures.ready() == 12);
    assert!(frames < 200, "all shown ({} ready)", a.ui.pictures.ready());
    let (inline, wakes) = a.ui.pictures.steps();
    assert!(inline > 0, "stepped in the window");
    assert_eq!(wakes, 0, "nothing ran on a thread");
    let _ = std::fs::remove_dir_all(dir);
}

/// With a thread to work on, the 36 pictures are read and decoded there, none of it in a frame:
/// each comes once, from one read.
#[test]
fn pictures_are_decoded_on_a_thread() {
    let (mut a, dir) = app("pictures-thread");
    let ctx = egui::Context::default();
    a.services = Box::new(Threads);
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9013, pictures(36));
    a.run("list.open", json!({"key": k.to_id()}));
    let started = std::time::Instant::now();
    while a.ui.pictures.ready() < 36 && started.elapsed() < std::time::Duration::from_secs(10) {
        pump(&ctx, &mut a, WINDOW, 1, |_| false);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(a.ui.pictures.ready(), 36);
    assert_eq!(a.ui.pictures.counts(), (1, 36), "one read, each picture decoded once");
    let (inline, wakes) = a.ui.pictures.steps();
    assert_eq!(inline, 0, "no step of it in a frame");
    assert_eq!(wakes, 36, "a frame asked for after each picture");
    let _ = std::fs::remove_dir_all(dir);
}

/// The work handed to a thread asks for a frame after each picture it makes (not once at its
/// end), a picture opened big while it waits goes first, and work for a message left stops.
#[test]
fn work_on_a_thread_asks_for_frames_and_stops_when_left() {
    let (mut a, dir) = app("pictures-held");
    let ctx = egui::Context::default();
    let held = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(Held(held.clone()));
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9014, pictures(6));
    let left = with_files(&mut a, 9015, pictures(70));
    // Opened and left: its work, handed over, does nothing when it runs.
    a.run("list.open", json!({"key": left.to_id()}));
    pump(&ctx, &mut a, WINDOW, 2, |_| false);
    a.run("list.open", json!({"key": k.to_id()}));
    pump(&ctx, &mut a, WINDOW, 2, |_| false);
    // The picture opened big, asked while the work waits.
    a.ui.attachment_view = Some((k.clone(), 4));
    pump(&ctx, &mut a, WINDOW, 1, |_| false);
    let work: Vec<crate::Work> = std::mem::take(&mut *held.lock().unwrap());
    assert_eq!(work.len(), 2, "one piece of work for each message");
    assert!(work[0].cancelled.load(std::sync::atomic::Ordering::Relaxed), "the one left is cancelled");
    assert_eq!(a.ui.pictures.counts(), (0, 0), "nothing done in the window");
    for w in work {
        (w.run)();
    }
    assert_eq!(a.ui.pictures.counts(), (1, 7), "the six thumbnails and the one big, from one read; none of the one left");
    assert_eq!(a.ui.pictures.waiting().first(), Some(&(4, crate::reading::Want::Big)), "the picture opened big first");
    assert_eq!(a.ui.pictures.steps(), (0, 7), "a frame asked for after each");
    let (frames, _) = pump(&ctx, &mut a, WINDOW, 5, |a| a.ui.pictures.ready() == 7);
    assert!(frames < 5, "all shown ({} ready)", a.ui.pictures.ready());
    let _ = std::fs::remove_dir_all(dir);
}
