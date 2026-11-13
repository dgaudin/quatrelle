// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the pictures of a message read, its files taken out once and decoded once each,
//! within a budget (the tests of upstream 321bef2, and ours).

use super::*;
use quatrelle_engine::mime::InlinePicture;

/// A message's thumbnails, the pictures its HTML shows by `cid:` and a PDF's first page come
/// from one read of it, a picture a frame (each read the whole message again, all of them in
/// the frame that first showed it).
#[test]
fn a_messages_pictures_come_from_one_read() {
    let (mut a, dir) = app("one-read");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let inline: Vec<InlinePicture> = (0..12)
        .map(|i| InlinePicture { cid: format!("pic{i}@x"), name: format!("pic{i}.png"), mime: "image/png".into(), data: png(20 + i, 20) })
        .collect();
    let html = format!("<p>Pictures</p>{}", (0..12).map(|i| format!("<img src=\"cid:pic{i}@x\" width=\"20\" height=\"20\">")).collect::<String>());
    let mut files = pictures(12);
    files.push(("report.pdf".into(), "application/pdf", pdf(2)));
    let k = with_files_and_html(&mut a, 9100, files, Some(html), inline);
    // The reading pane's icons are made once, with a message like it shown first.
    let warm = with_files_and_html(
        &mut a,
        9101,
        vec![("photo.png".into(), "image/png", png(9, 9)), ("plan.pdf".into(), "application/pdf", pdf(1))],
        Some("<p>Hello</p>".into()),
        Vec::new(),
    );
    a.run("list.open", json!({"key": warm.to_id()}));
    for _ in 0..10 {
        textures_made(&ctx, &mut a, WINDOW);
    }
    let store = a.session.store.clone().unwrap();
    ask_for_files(&mut a, &k);
    a.run("list.open", json!({"key": k.to_id()}));
    let reads = store.reads();
    let (mut most, mut made) = (0, 0);
    for _ in 0..80 {
        let n = textures_made(&ctx, &mut a, WINDOW);
        most = most.max(n);
        made += n;
    }
    eprintln!("{made} pictures made, at most {most} in a frame, {} reads", store.reads() - reads);
    assert_eq!(made, 25, "the twelve thumbnails, the twelve pictures in its text and the PDF's page");
    assert_eq!(store.reads() - reads, 1, "one read of the message for all of them");
    assert_eq!(most, 1, "a picture a frame");
    let _ = std::fs::remove_dir_all(dir);
}

/// A message with more than 64 pictures cleared every thumbnail each time one more was made,
/// so it read, parsed and decoded all of them again every frame: each is decoded once, from
/// one read of the message, a picture a frame (none of it in one long frame).
#[test]
fn seventy_pictures_decode_once_each() {
    let (mut a, dir) = app("seventy-pictures");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9001, pictures(70));
    a.run("list.open", json!({"key": k.to_id()}));
    let (frames, most) = pump(&ctx, &mut a, WINDOW, 80, |a| a.ui.pictures.ready() == 70);
    assert!(frames < 80, "all 70 thumbnails in {frames} frames ({} ready)", a.ui.pictures.ready());
    assert_eq!(most, 1, "a picture a frame");
    assert_eq!(a.ui.pictures.counts(), (1, 70), "one read, each picture decoded once");
    pump(&ctx, &mut a, WINDOW, 10, |_| false);
    assert_eq!(a.ui.pictures.counts(), (1, 70), "and not again");
    assert_eq!(a.ui.pictures.working(), 0);
    assert_eq!(a.ui.pictures.steps().1, 0, "no threads here: done in the window");
    let _ = std::fs::remove_dir_all(dir);
}

/// The 36 thumbnails, a picture opened big and a PDF's ten pages all come from one read of the
/// message (each was a read and a parse of the whole message, per picture and per page).
#[test]
fn one_extraction_per_message() {
    let (mut a, dir) = app("one-extraction");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let mut files = pictures(36);
    files.push(("report.pdf".into(), "application/pdf", pdf(10)));
    let k = with_files(&mut a, 9002, files);
    ask_for_files(&mut a, &k);
    a.run("list.open", json!({"key": k.to_id()}));
    let (frames, most) = pump(&ctx, &mut a, WINDOW, 60, |a| a.ui.pictures.ready() == 37);
    assert!(frames < 60, "36 thumbnails and the PDF's first page ({} ready)", a.ui.pictures.ready());
    assert_eq!(most, 1);
    // A picture shown at the pane's size.
    a.ui.attachment_view = Some((k.clone(), 3));
    let (frames, _) = pump(&ctx, &mut a, WINDOW, 10, |a| a.ui.pictures.ready() == 38);
    assert!(frames < 10, "the picture big");
    // The PDF's pages, all on a tall screen.
    a.ui.attachment_view = Some((k.clone(), 36));
    let (frames, _) = pump(&ctx, &mut a, egui::vec2(1280.0, 8000.0), 30, |a| a.ui.pictures.ready() == 48);
    assert!(frames < 30, "its ten pages ({} ready)", a.ui.pictures.ready());
    assert_eq!(a.ui.pictures.pages(&k, 36), Some(10));
    assert_eq!(a.ui.pictures.counts(), (1, 48), "one read of the message for all of them");
    let _ = std::fs::remove_dir_all(dir);
}

/// A 60-page PDF draws the pages on screen only, and what it keeps stays within the budget
/// even when every page shows (those that don't fit come smaller, or not at all).
#[test]
fn a_long_pdf_stays_within_the_budget() {
    let (mut a, dir) = app("long-pdf");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9003, vec![("manual.pdf".into(), "application/pdf", pdf(60))]);
    a.run("list.open", json!({"key": k.to_id()}));
    a.ui.attachment_view = Some((k.clone(), 0));
    pump(&ctx, &mut a, WINDOW, 40, |_| false);
    assert_eq!(a.ui.pictures.pages(&k, 0), Some(60));
    let (_, decodes) = a.ui.pictures.counts();
    assert!(decodes <= 6, "the chip and the pages on screen only: {decodes}");
    // Every page on screen, with room for a few.
    let budget = 1 << 20;
    a.ui.pictures.set_budget(budget);
    for _ in 0..150 {
        pump(&ctx, &mut a, egui::vec2(1280.0, 24_000.0), 1, |_| false);
        assert!(a.ui.pictures.bytes() <= budget, "{} kept", a.ui.pictures.bytes());
    }
    assert!(a.ui.pictures.ready() > 3, "the pages that fit ({})", a.ui.pictures.ready());
    let _ = std::fs::remove_dir_all(dir);
}

/// A picture bigger than a frame's uploads (2048 pixels square, 16 MiB) is still made, alone.
#[test]
fn a_single_big_upload() {
    let (mut a, dir) = app("big-upload");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9004, vec![("poster.png".into(), "image/png", png(2048, 2048))]);
    a.run("list.open", json!({"key": k.to_id()}));
    a.ui.attachment_view = Some((k.clone(), 0));
    let (frames, _) = pump(&ctx, &mut a, WINDOW, 20, |a| a.ui.pictures.ready() == 2);
    assert!(frames < 20, "the thumbnail and the picture big ({} ready)", a.ui.pictures.ready());
    assert!(a.ui.pictures.bytes() >= 16 << 20, "{}", a.ui.pictures.bytes());
    let _ = std::fs::remove_dir_all(dir);
}

/// A picture that breaks the decoder fails, with the rest of its message's, and the next
/// message's pictures show.
#[test]
fn a_panicking_decode_leaves_later_work_alone() {
    let (mut a, dir) = app("panicking-decode");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let bad = with_files(&mut a, 9005, pictures(5));
    let good = with_files(&mut a, 9006, pictures(4));
    a.ui.pictures.panic_at = Some(2);
    a.run("list.open", json!({"key": bad.to_id()}));
    // (Once it started: the message was read.)
    let (frames, _) = pump(&ctx, &mut a, WINDOW, 20, |a| a.ui.pictures.counts().0 == 1 && a.ui.pictures.working() == 0);
    assert!(frames < 20, "its work ends");
    assert_eq!(a.ui.pictures.ready(), 2, "the pictures before it");
    assert_eq!(a.ui.pictures.counts(), (1, 2), "and none after it decoded");
    a.ui.pictures.panic_at = None;
    a.run("list.open", json!({"key": good.to_id()}));
    let before = a.ui.pictures.ready();
    let (frames, _) = pump(&ctx, &mut a, WINDOW, 20, |a| a.ui.pictures.ready() >= before + 4);
    assert!(frames < 20, "the next message's four ({} ready)", a.ui.pictures.ready());
    let _ = std::fs::remove_dir_all(dir);
}

/// Moving on to another message stops the work on the last one's pictures.
#[test]
fn pictures_of_a_message_left_are_cancelled() {
    let (mut a, dir) = app("pictures-left");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let many = with_files(&mut a, 9007, pictures(70));
    let next = with_files(&mut a, 9008, pictures(2));
    a.run("list.open", json!({"key": many.to_id()}));
    pump(&ctx, &mut a, WINDOW, 5, |_| false);
    a.run("list.open", json!({"key": next.to_id()}));
    pump(&ctx, &mut a, WINDOW, 30, |_| false);
    let (reads, decodes) = a.ui.pictures.counts();
    assert_eq!(reads, 2);
    assert!(decodes < 10, "the first message's stopped: {decodes} decoded");
    assert_eq!(a.ui.pictures.working(), 0);
    let _ = std::fs::remove_dir_all(dir);
}

/// A window resized a pixel a frame with a PDF's pages shown: the pages are made again only
/// at each 100 pixels, what's kept for the message stays small, and the pages show at the
/// width it ends at (each width was a new page kept, until the message's 200 failed them all).
#[test]
fn resizing_keeps_a_pdf_showing() {
    let (mut a, dir) = app("pdf-resized");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9009, vec![("plan.pdf".into(), "application/pdf", pdf(3))]);
    a.run("list.open", json!({"key": k.to_id()}));
    a.ui.attachment_view = Some((k.clone(), 0));
    pump(&ctx, &mut a, egui::vec2(1000.0, 800.0), 20, |_| false);
    let settled = a.ui.pictures.counts().1;
    for w in 0..300 {
        pump(&ctx, &mut a, egui::vec2(1000.0 + w as f32, 800.0), 1, |_| false);
        assert!(a.ui.pictures.of(&k).0 <= 12, "pictures kept at {w}: {:?}", a.ui.pictures.of(&k));
    }
    pump(&ctx, &mut a, egui::vec2(900.0, 800.0), 20, |_| false);
    let decodes = a.ui.pictures.counts().1 - settled;
    assert!(decodes <= 20, "the pages made again at each 100 pixels only: {decodes}");
    let (kept, shown) = a.ui.pictures.of(&k);
    assert!(shown >= 3 && kept <= 12, "the three pages show at the last width: {shown} shown, {kept} kept");
    let _ = std::fs::remove_dir_all(dir);
}

/// Past 200 pictures the chips show their kind's icon, but one opened still shows.
#[test]
fn a_picture_past_the_chips_opens() {
    let (mut a, dir) = app("many-chips");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9010, pictures(205));
    a.run("list.open", json!({"key": k.to_id()}));
    pump(&ctx, &mut a, WINDOW, 5, |_| false);
    a.ui.attachment_view = Some((k.clone(), 204));
    pump(&ctx, &mut a, WINDOW, 10, |_| false);
    assert!(matches!(a.ui.pictures.get(&k, 204, crate::reading::Want::Big), crate::reading::Got::Ready(..)), "the last picture big");
    let _ = std::fs::remove_dir_all(dir);
}

/// Pictures left out of a read for room are read again, not failed: with room for two files
/// taken out, all six show.
#[test]
fn pictures_left_out_for_room_show() {
    let (mut a, dir) = app("sources-room");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let files = pictures(6);
    let most = files.iter().map(|f| f.2.len()).max().unwrap();
    let k = with_files(&mut a, 9011, files);
    a.ui.pictures.set_sources_max(2 * most);
    a.run("list.open", json!({"key": k.to_id()}));
    let (frames, _) = pump(&ctx, &mut a, WINDOW, 40, |a| a.ui.pictures.ready() == 6);
    assert!(frames < 40, "all six ({} ready)", a.ui.pictures.ready());
    let (reads, decodes) = a.ui.pictures.counts();
    assert!(decodes == 6 && reads <= 3, "two at a time, each decoded once: {reads} reads, {decodes} decodes");
    let _ = std::fs::remove_dir_all(dir);
}

/// More web pictures shown at once than are kept: each is decoded once and kept while shown,
/// not decoded again every frame (all were let go at the 49th, then made again each frame).
#[test]
fn many_web_pictures_are_kept_while_shown() {
    let (mut a, dir) = app("web-pictures");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let urls: Vec<String> = (0..250).map(|i| format!("https://cdn.shop.example/{i}.png")).collect();
    let imgs: String = urls.iter().map(|u| format!("<img src=\"{u}\" width=\"4\" height=\"4\">")).collect();
    let raw = format!("From: Shop <news@shop.example>\r\nSubject: Sale\r\nContent-Type: text/html\r\n\r\n<p>Sale</p>{imgs}");
    let k = with_raw(&mut a, 9112, raw.as_bytes());
    a.run("list.open", json!({"key": k.to_id()}));
    a.run("message.showPictures", json!({}));
    for u in &urls {
        a.session.apply(MailEvent::Fetched { url: u.clone(), data: Some(png(4, 4)), error: None });
    }
    let tall = egui::vec2(1280.0, 8000.0);
    for _ in 0..3 {
        textures_made(&ctx, &mut a, tall);
    }
    let again: usize = (0..3).map(|_| textures_made(&ctx, &mut a, tall)).sum();
    eprintln!("{} kept, {again} made again in three frames", a.ui.shown_pictures.len());
    assert_eq!(a.ui.shown_pictures.len(), 250, "all of them kept while shown");
    assert_eq!(again, 0, "none made again");
    let _ = std::fs::remove_dir_all(dir);
}

/// One read looks for 256 files at most: the pictures past them (an HTML message's 300) are
/// looked for by a later read, not taken as not in the message.
#[test]
fn more_pictures_than_one_read_takes_show() {
    use crate::reading::Want;
    let (mut a, dir) = app("past-one-read");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9013, pictures(300));
    let store = a.session.store.clone();
    for _ in 0..700 {
        if a.ui.pictures.ready() == 300 {
            break;
        }
        for i in 0..300 {
            let _ = a.ui.pictures.get(&k, i, Want::Body(900));
        }
        a.ui.pictures.end_frame(&ctx, store.as_ref(), None, &|_| None, &a.ui.fonts, &mut crate::NullServices(NOW));
    }
    assert_eq!(a.ui.pictures.ready(), 300, "all of them");
    assert_eq!(a.ui.pictures.counts(), (2, 300), "two reads, each picture decoded once");
    let _ = std::fs::remove_dir_all(dir);
}

/// A picture that can't be shown is tried again when its message is opened again, after its
/// work ended (it was only when the work was still running).
#[test]
fn a_failed_picture_is_tried_again_when_opened_again() {
    let (mut a, dir) = app("failed-again");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let broken = with_files(&mut a, 9014, vec![("broken.png".into(), "image/png", b"not a picture at all".to_vec())]);
    let other = with_files(&mut a, 9015, pictures(1));
    a.run("list.open", json!({"key": broken.to_id()}));
    pump(&ctx, &mut a, WINDOW, 10, |_| false);
    assert_eq!((a.ui.pictures.counts(), a.ui.pictures.working()), ((1, 1), 0), "tried, and its work ended");
    a.run("list.open", json!({"key": other.to_id()}));
    pump(&ctx, &mut a, WINDOW, 10, |_| false);
    assert_eq!(a.ui.pictures.counts(), (2, 2));
    a.run("list.open", json!({"key": broken.to_id()}));
    pump(&ctx, &mut a, WINDOW, 10, |_| false);
    assert_eq!(a.ui.pictures.counts(), (3, 3), "read and tried again");
    let _ = std::fs::remove_dir_all(dir);
}

/// A picture bigger than all the room there is lets go of no other: the last message's
/// thumbnails are kept (all of them went, for a picture then thrown away).
#[test]
fn a_picture_too_big_for_the_budget_evicts_nothing() {
    let (mut a, dir) = app("too-big");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let small = with_files(&mut a, 9016, pictures(3));
    let big = with_files(&mut a, 9017, vec![("poster.png".into(), "image/png", png(400, 400))]);
    a.run("list.open", json!({"key": small.to_id()}));
    let (frames, _) = pump(&ctx, &mut a, WINDOW, 20, |a| a.ui.pictures.ready() == 3);
    assert!(frames < 20, "the three thumbnails");
    // Room for the three, not for a thumbnail 160 pixels square.
    a.ui.pictures.set_budget(64 << 10);
    a.run("list.open", json!({"key": big.to_id()}));
    pump(&ctx, &mut a, WINDOW, 10, |_| false);
    assert_eq!(a.ui.pictures.counts().1, 4, "the big one decoded");
    assert_eq!(a.ui.pictures.ready(), 3, "and the three kept");
    let _ = std::fs::remove_dir_all(dir);
}

/// Older mail read on the server comes for showing and is never kept here: the picture its HTML
/// shows by `cid:` is taken out of the source held with it while it's open, and drawn, as a
/// kept message's is (it stayed an empty space: the pictures read the cache only).
#[test]
fn the_pictures_of_older_mail_not_kept_are_drawn() {
    let (mut a, dir) = app("not-kept-pictures");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let inline = vec![InlinePicture { cid: "harbour@x".into(), name: "harbour.png".into(), mime: "image/png".into(), data: png(30, 30) }];
    let html = "<p>The harbour</p><img src=\"cid:harbour@x\" width=\"30\" height=\"30\">".to_string();
    let k = with_files_and_html(&mut a, 9300, vec![("notes.txt".into(), "text/plain", b"Quay seven, berth two.".to_vec())], Some(html), inline);
    // Read on the server: in no folder's list here, its source not kept.
    let store = a.session.store.clone().unwrap();
    let raw = store.read_body(&k).unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let at = inbox.messages.iter().position(|m| m.key == k).unwrap();
    let older = inbox.messages.remove(at);
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.remove_body(&k);
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    a.session.apply(MailEvent::Older { account: "sample".into(), folder: "INBOX".into(), items: vec![older], more: false, error: None });
    a.run("list.open", json!({"key": k.to_id()}));
    a.session.apply(MailEvent::Body { key: k.clone(), raw });
    assert!(a.session.reading.as_ref().is_some_and(|r| r.key == k && r.parsed.is_some()), "shown");
    pump(&ctx, &mut a, WINDOW, 60, |a| a.ui.pictures.of(&k).1 > 0);
    assert!(a.ui.pictures.of(&k).1 > 0, "its picture is drawn: {:?}", a.ui.pictures.of(&k));
    assert!(!store.has_body(&k), "nothing of it is written to the cache");
    let _ = std::fs::remove_dir_all(dir);
}
