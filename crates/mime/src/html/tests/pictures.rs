// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Pictures in an HTML body: their addresses read whole, however the tag is written, and the
//! ones that track the read counted where they are.

use super::*;

/// An unquoted address keeps its trailing `/` in every pass that reads it: the list of pictures
/// and the picture span in the text are the same address.
#[test]
fn an_unquoted_addresses_trailing_slash_is_kept_in_every_pass() {
    let html = "<p>Hi</p><img src=https://x.example/c.png/>";
    let images = crate::html_images(html);
    let r = html_to_rich(html);
    let span_images: Vec<&str> = r.iter().flatten().filter_map(|sp| sp.image.as_deref()).collect();
    assert_eq!(images, ["https://x.example/c.png/"]);
    assert_eq!(span_images, images, "{span_images:?}");
}

/// A `>` inside a quoted attribute doesn't end the tag: the picture whose `alt` holds one is
/// found, and counted as a tracking picture where it is one.
#[test]
fn a_picture_with_a_quoted_gt_in_its_alt_is_found() {
    let html = "<p>Hi</p><img alt=\"5 > 4\" src=\"https://pictures.example/a.png\"><img src=\"https://pictures.example/b.png\">";
    assert_eq!(crate::html_images(html), ["https://pictures.example/a.png", "https://pictures.example/b.png"]);
    let tracker = "<img alt=\"a>b\" src=\"https://t.mailtrack.example/open?id=1\" width=\"1\" height=\"1\">";
    assert_eq!(crate::trackers::trackers(tracker).pictures.len(), 1);

    // An attribute inside another's quoted value isn't one, and a tab around the `=` doesn't
    // hide the real `src`.
    let html = "<img alt=\"x src=https://evil.example/\" src=\"https://x.example/a.png\"><img\tsrc\t=\t\"https://x.example/b.png\">";
    assert_eq!(crate::html_images(html), ["https://x.example/a.png", "https://x.example/b.png"]);
}

/// A picture's size is the one CSS picks: the last declaration marked `!important` over a
/// later plain one (`width:600px !important; width:1px` is 600 wide, not 1).
#[test]
fn a_pictures_size_is_the_one_css_picks() {
    let html = r#"<img src="https://x.example/a.png" style="width:600px !important; width:1px"><img src="https://x.example/b.png" style="width:1px; width:600px">"#;
    let sizes = picture_sizes(html);
    assert_eq!(sizes[0].1.width, Some(layout::Width::Px(600)), "{sizes:?}");
    assert_eq!(sizes[1].1.width, Some(layout::Width::Px(600)), "{sizes:?}");
}
