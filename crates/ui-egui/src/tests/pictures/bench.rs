// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A bench, ignored (`cargo test --release -p quatrelle-ui-egui pictures::bench -- --ignored
//! --nocapture`): the slowest frame of a message of about 10 MB, 36 photos in its HTML and
//! photos, a PDF and a video attached, from opening it until its pictures are all shown; then
//! the same with the PDF previewed in the reading pane. Once in the window (no threads, as in
//! the browser), once with a lane: a thread doing the work handed to it in order, as the
//! desktop's (`apps/quatrelle/src/desktop/lanes.rs`).

use super::*;
use quatrelle_engine::mime::InlinePicture;
use std::time::{Duration, Instant};

/// A photo-like JPEG: a gradient under some noise (`seed` varies it).
fn photo(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    let img = image::RgbImage::from_fn(w, h, |i, j| {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        let n = (x % 16) as u8;
        image::Rgb([((i * 255) / w.max(1)) as u8 / 2 + n, ((j * 255) / h.max(1)) as u8 / 2 + n, (seed % 200) as u8 + n / 2])
    });
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).encode_image(&img).unwrap();
    out
}

/// What [`settle`] saw.
struct Settled {
    /// The first frame, and the slowest after it.
    first: Duration,
    slowest: Duration,
    /// When the last picture came.
    last: Duration,
    frames: usize,
    made: usize,
}

/// Frames until no picture has been made for a second.
fn settle(ctx: &egui::Context, a: &mut MailApp) -> Settled {
    let started = Instant::now();
    let (mut first, mut slowest, mut last, mut frames, mut made) = (Duration::ZERO, Duration::ZERO, Duration::ZERO, 0, 0);
    while started.elapsed() < Duration::from_secs(180) && started.elapsed().saturating_sub(last) < Duration::from_secs(1) {
        let t = Instant::now();
        let n = textures_made(ctx, a, WINDOW);
        if frames == 0 {
            first = t.elapsed();
        } else {
            slowest = slowest.max(t.elapsed());
        }
        frames += 1;
        made += n;
        if n > 0 {
            last = started.elapsed();
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Settled { first, slowest, last, frames, made }
}

impl std::fmt::Display for Settled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ms = |d: Duration| d.as_secs_f64() * 1e3;
        write!(
            f,
            "first frame {:.1} ms, slowest after it {:.1} ms, last picture at {:.0} ms, {} frames, {} pictures",
            ms(self.first),
            ms(self.slowest),
            ms(self.last),
            self.frames,
            self.made
        )
    }
}

/// The message: 36 photos in its HTML, three photos, a PDF and a video attached.
fn big_message(a: &mut MailApp, uid: u32) -> (MessageKey, usize) {
    let inline: Vec<InlinePicture> = (0..36)
        .map(|i| InlinePicture {
            cid: format!("photo{i}@bench"),
            name: format!("photo{i}.jpg"),
            mime: "image/jpeg".into(),
            data: photo(1024, 768, i),
        })
        .collect();
    let html = format!(
        "<p>The site visit, in pictures.</p>{}",
        (0..36).map(|i| format!("<p><img src=\"cid:photo{i}@bench\" width=\"640\" height=\"480\"></p>")).collect::<String>()
    );
    let mut files: Vec<(String, &str, Vec<u8>)> = (0..3).map(|i| (format!("plan {i}.jpg"), "image/jpeg", photo(2400, 1800, 100 + i))).collect();
    files.push(("report.pdf".into(), "application/pdf", pdf(10)));
    let mut x = 7u32;
    files.push((
        "walkthrough.mp4".into(),
        "video/mp4",
        (0..1_500_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect(),
    ));
    let k = with_files_and_html(a, uid, files, Some(html), inline);
    let size = a.session.store.as_ref().and_then(|s| s.read_body(&k)).map_or(0, |r| r.len());
    let files =
        a.session.store.as_ref().and_then(|s| s.read_body(&k)).and_then(|r| quatrelle_engine::mime::parse(&r)).map_or(0, |p| p.attachments.len());
    eprintln!("{files} files listed");
    (k, size)
}

/// Services with one thread for the work handed to them, in order.
struct OneLane(std::sync::mpsc::Sender<crate::Work>);

impl crate::Services for OneLane {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
    fn background(&mut self, _: crate::Lane, work: crate::Work) -> Result<(), crate::Work> {
        self.0.send(work).map_err(|e| e.0)
    }
    fn has_background(&self) -> bool {
        true
    }
}

#[test]
#[ignore = "a bench: run it with --release, --ignored and --nocapture"]
fn slowest_frame_of_a_message_with_many_pictures() {
    bench(false);
    bench(true);
}

fn bench(lane: bool) {
    let (mut a, dir) = app(if lane { "bench-pictures-lane" } else { "bench-pictures" });
    let ctx = egui::Context::default();
    if lane {
        let (tx, rx) = std::sync::mpsc::channel::<crate::Work>();
        std::thread::spawn(move || {
            for w in rx {
                if !w.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                    (w.run)();
                }
            }
        });
        a.services = Box::new(OneLane(tx));
        // The open message read and parsed on the lane too, as in the app.
        a.defer_bodies();
    }
    eprintln!("{}", if lane { "On a lane:" } else { "In the window:" });
    frame(&ctx, &mut a, vec![]);
    let (k, size) = big_message(&mut a, 9900);
    let store = a.session.store.clone().unwrap();
    let reads = store.reads();
    let t = Instant::now();
    a.run("list.open", json!({"key": k.to_id()}));
    let opened = t.elapsed();
    let shown = settle(&ctx, &mut a);
    eprintln!(
        "message {:.1} MB: opened in {:.1} ms, then {shown}, {} reads of the source",
        size as f64 / 1e6,
        opened.as_secs_f64() * 1e3,
        store.reads() - reads
    );
    let pdf = a
        .session
        .reading
        .as_ref()
        .and_then(|r| r.parsed.as_ref())
        .and_then(|p| p.attachments.iter().find(|f| f.name == "report.pdf"))
        .map(|f| f.index);
    a.ui.attachment_view = pdf.map(|i| (k.clone(), i));
    let reads = store.reads();
    let shown = settle(&ctx, &mut a);
    eprintln!("its PDF previewed: {shown}, {} reads of the source", store.reads() - reads);
    let _ = std::fs::remove_dir_all(dir);
}
