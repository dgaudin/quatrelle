// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the thumbnails of a message's attachments, kept while they're shown.

use super::*;

const PIXEL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// More pictures in a message than thumbnails are kept: once it shows, no frame decodes one
/// again (the thumbnails were all let go at the 65th, then every one decoded again each frame).
#[test]
fn pictures_past_those_kept_are_not_decoded_each_frame() {
    let (mut a, dir) = app("many-pictures");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let mut raw = String::from(
        "From: Studio Shop <shop@studio.example>\r\nTo: riley@studio.example\r\nSubject: Seventy pictures\r\n\
         Date: Wed, 22 Oct 2026 10:00:00 +0200\r\nMessage-ID: <seventy@x>\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n--b1\r\nContent-Type: text/plain\r\n\r\nThe pictures.\r\n",
    );
    for i in 0..70 {
        raw.push_str(&format!(
            "--b1\r\nContent-Type: image/png\r\nContent-Transfer-Encoding: base64\r\n\
             Content-Disposition: attachment; filename=\"p{i}.png\"\r\n\r\n{PIXEL}\r\n"
        ));
    }
    raw.push_str("--b1--\r\n");
    let store = a.session.store.clone().unwrap();
    let mut st = store.load_folder("sample", "INBOX").unwrap();
    let uid = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1;
    let key = quatrelle_engine::model::MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: st.validity.max(1), uid };
    let raw = raw.into_bytes();
    st.messages.insert(0, quatrelle_engine::mime::summary_from_full(&raw, key.clone(), Default::default(), raw.len() as u32, NOW));
    store.write_body(&key, &raw).unwrap();
    store.save_folder("sample", "INBOX", &st).unwrap();
    a.session.apply(quatrelle_engine::model::Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    frame(&ctx, &mut a, vec![]);
    a.run("list.open", json!({"key": key.to_id()}));
    let input = || RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    for _ in 0..10 {
        ctx.run_ui(input(), |ui| a.ui(ui)).textures_delta.clear();
    }
    let reads = store.reads();
    for _ in 0..5 {
        let out = ctx.run_ui(input(), |ui| a.ui(ui));
        assert!(out.textures_delta.set.is_empty(), "{} textures made", out.textures_delta.set.len());
    }
    assert_eq!(store.reads(), reads, "no source read again for a thumbnail");
    let _ = std::fs::remove_dir_all(dir);
}
