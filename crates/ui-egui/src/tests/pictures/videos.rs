// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Video attachments' thumbnails: made only once the user asked to see the file, their files
//! written and their frames decoded on the pictures lane, not in a frame, and no more of them kept than so many (the tests of upstream b580c87 and
//! 51d1cd0).

use super::*;

/// Services whose platform gives every video a frame.
struct Frames;

impl crate::Services for Frames {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
    fn notify(&mut self, _: &str, _: &str) {}
    fn video_frame(&mut self, _: &std::path::Path) -> crate::VideoFrame {
        crate::VideoFrame::Ready(png(32, 18))
    }
}

/// More videos in a message than video thumbnails are kept: those past them show their kind's
/// icon, and no frame makes a texture once the others show (one past them was decoded again
/// every frame).
#[test]
fn videos_past_those_kept_make_no_texture_each_frame() {
    let (mut a, dir) = app("many-videos");
    let ctx = egui::Context::default();
    a.services = Box::new(Frames);
    frame(&ctx, &mut a, vec![]);
    let files = (0..70).map(|i| (format!("clip {i}.mp4"), "video/mp4", vec![i as u8; 64])).collect();
    let k = with_files(&mut a, 9018, files);
    ask_for_files(&mut a, &k);
    a.run("list.open", json!({"key": k.to_id()}));
    let input = || RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, WINDOW)), ..Default::default() };
    for _ in 0..10 {
        ctx.run_ui(input(), |ui| a.ui(ui)).textures_delta.clear();
    }
    assert_eq!(a.ui.video_thumbs.len(), 64);
    for _ in 0..5 {
        let out = ctx.run_ui(input(), |ui| a.ui(ui));
        assert!(out.textures_delta.set.is_empty(), "{} textures made", out.textures_delta.set.len());
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Services that keep the pictures lane's work for the test to run when it says (the reading
/// lane's is done at once), and whose platform gives every video a frame.
struct HeldFrames(std::sync::Arc<std::sync::Mutex<Vec<crate::Work>>>);

impl crate::Services for HeldFrames {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
    fn video_frame(&mut self, _: &std::path::Path) -> crate::VideoFrame {
        crate::VideoFrame::Ready(png(32, 18))
    }
    fn background(&mut self, lane: crate::Lane, work: crate::Work) -> Result<(), crate::Work> {
        if lane != crate::Lane::Pictures {
            return Err(work);
        }
        self.0.lock().unwrap().push(work);
        Ok(())
    }
    fn has_background(&self) -> bool {
        true
    }
}

/// A video's chip has its file written out and the platform's frame decoded on the pictures
/// lane: the frames that show the message read and write nothing for it (on a real file,
/// reading a message to write its 1.7 MB video held its first frame for 12 ms), and the
/// thumbnail shows once that work is done. Work for a message left does nothing.
#[test]
fn a_video_is_written_and_decoded_off_the_window() {
    let (mut a, dir) = app("video-held");
    let ctx = egui::Context::default();
    let held = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(HeldFrames(held.clone()));
    frame(&ctx, &mut a, vec![]);
    let left = with_files(&mut a, 9031, vec![("left.mp4".into(), "video/mp4", vec![3u8; 64])]);
    let k = with_files(&mut a, 9030, vec![("clip.mp4".into(), "video/mp4", vec![7u8; 64])]);
    let take = || std::mem::take(&mut *held.lock().unwrap());
    ask_for_files(&mut a, &left);
    ask_for_files(&mut a, &k);
    // Opened and left before its work ran: cancelled, it writes nothing.
    a.run("list.open", json!({"key": left.to_id()}));
    pump(&ctx, &mut a, WINDOW, 2, |_| false);
    a.run("list.open", json!({"key": k.to_id()}));
    pump(&ctx, &mut a, WINDOW, 3, |_| false);
    assert!(a.ui.video_files.is_empty(), "no file written in a frame: {} written", a.ui.video_files.len());
    let work = take();
    assert_eq!(work.len(), 2, "the file of each, asked once");
    assert!(work[0].cancelled.load(std::sync::atomic::Ordering::Relaxed), "the one left is cancelled");
    for w in work {
        (w.run)();
    }
    pump(&ctx, &mut a, WINDOW, 2, |_| false);
    assert_eq!(a.ui.video_files.len(), 1, "the file of the one shown, and only it");
    assert!(a.ui.video_files.keys().all(|(m, _)| *m == k));
    // The platform's frame came: decoded on the lane too, not in a frame.
    pump(&ctx, &mut a, WINDOW, 2, |_| false);
    assert!(a.ui.video_thumbs.is_empty(), "nothing decoded in a frame");
    let work = take();
    assert_eq!(work.len(), 1, "the frame's decoding, asked once");
    for w in work {
        (w.run)();
    }
    pump(&ctx, &mut a, WINDOW, 2, |_| false);
    assert!(a.ui.video_thumbs.values().all(|(t, _)| t.is_some()) && a.ui.video_thumbs.len() == 1, "its thumbnail shows");
    assert!(take().is_empty(), "nothing asked again");
    let _ = std::fs::remove_dir_all(dir);
}

/// A video or a PDF from a stranger is read by decoders only when the user asks to see it:
/// showing its message writes no file, asks the platform for no frame and renders no page; a
/// click on its chip does.
#[test]
fn a_video_or_a_pdf_is_decoded_only_once_asked_for() {
    let (mut a, dir) = app("decode-on-request");
    let ctx = egui::Context::default();
    let held = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(HeldFrames(held.clone()));
    frame(&ctx, &mut a, vec![]);
    let files = vec![("clip.mp4".into(), "video/mp4", vec![7u8; 64]), ("plan.pdf".into(), "application/pdf", pdf(1))];
    let k = with_files(&mut a, 9040, files);
    a.run("list.open", json!({"key": k.to_id()}));
    pump(&ctx, &mut a, WINDOW, 6, |_| false);
    let asked = std::mem::take(&mut *held.lock().unwrap()).len();
    assert_eq!(asked, 0, "the message shown: nothing of its video or its PDF is read");
    assert!(a.ui.video_files.is_empty() && a.ui.video_frames.is_empty());
    // The user asks for each (a click on its chip).
    let indices: Vec<usize> = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.attachments.iter().map(|f| f.index).collect()).unwrap();
    for i in indices {
        a.ui.ask_thumbnail(&k, i);
    }
    pump(&ctx, &mut a, WINDOW, 3, |_| false);
    let asked = std::mem::take(&mut *held.lock().unwrap()).len();
    assert_eq!(asked, 2, "the video's file and the PDF's first page, once asked for");
    let _ = std::fs::remove_dir_all(dir);
}
