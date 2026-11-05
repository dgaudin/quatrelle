// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Pictures the HTML shows go out under their names.

use super::*;

/// A picture the HTML shows goes out under its name (`filename=` and `name=`, as Outlook writes
/// them; one pasted in, `local3`, with its type's extension, and one in another script encoded),
/// so another client and a saved copy have it, and it stays inline with no paperclip; shown by
/// nothing, the same picture is a file. One without a name is written as before.
#[test]
fn a_named_inline_picture_keeps_its_name_and_no_paperclip() {
    let png = b"\x89PNG\r\n\x1a\nfake".to_vec();
    let pic = |cid: &str, name: &str| InlinePicture { cid: cid.into(), name: name.into(), mime: "image/png".into(), data: png.clone() };
    let d = Draft {
        to: vec![EmailAddress::new("", "ann@studio.example")],
        subject: "Logo".into(),
        body: "Hi".into(),
        html: Some("<p>Hi</p><img src=\"cid:a@x\"><img src=\"cid:b@x\"><img src=\"cid:c@x\">".into()),
        inline: vec![pic("a@x", "logo.png"), pic("b@x", "local3"), pic("c@x", "café.png")],
        ..Default::default()
    };
    let raw = build(&d, &EmailAddress::new("", "z@studio.example"), 0, "n@x").unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("filename=\"logo.png\"") && text.contains("name=\"logo.png\""), "{text}");
    let p = parse(&raw).unwrap();
    let parts: Vec<_> = p.attachments.iter().map(|a| (a.name.as_str(), a.inline, a.content_id.as_deref().unwrap_or_default())).collect();
    assert_eq!(parts, [("logo.png", true, "a@x"), ("local3.png", true, "b@x"), ("café.png", true, "c@x")]);
    assert!(!summary_from_full(&raw, key(), Flags::default(), 0, 0).has_attachments);

    let unshown = Draft { html: Some("<p>Hi</p>".into()), ..d.clone() };
    let raw = build(&unshown, &EmailAddress::new("", "z@studio.example"), 0, "n@x").unwrap();
    assert!(summary_from_full(&raw, key(), Flags::default(), 0, 0).has_attachments);

    let unnamed = Draft { inline: vec![pic("a@x", " ")], ..d };
    let raw = build(&unnamed, &EmailAddress::new("", "z@studio.example"), 0, "n@x").unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("Content-Disposition: inline\r\n") && !text.contains("filename"), "{text}");
}
