//! What a crafted `winmail.dat` can cost: the output, the memory and the passes over it stay
//! bounded however it's built (measured on crafted inputs, review of 2026-10-31).

use super::*;

/// A link around text whose formatting changes again and again: the link is written once.
#[test]
fn a_link_stays_open_while_its_formatting_changes() {
    let rtf = format!(
        "{{\\rtf1{{\\field{{\\*\\fldinst HYPERLINK \"http://x.example/{}\"}}{{\\fldrslt {}}}}}}}",
        "a".repeat(4000),
        "\\b x\\b0 y".repeat(2000)
    );
    let TnefBody::Html(h) = rtf_body(rtf.as_bytes()) else { panic!("no HTML") };
    assert_eq!(h.matches("<a href=").count(), 1);
    assert_eq!(h.matches("<b>x</b>").count(), 2000);
    // Before: the 4 KB link written for each of the 4000 changes, 16 MB.
    assert!(h.len() < 64 << 10, "{} bytes", h.len());
}

/// The pictures a body shows, found in one pass: by `src`, by a style's `url()`, never by a
/// longer reference that starts like one.
#[test]
fn the_pictures_a_body_shows_are_found_in_one_pass() {
    let body = TnefBody::Html("<img src=\"cid:one@x\"><div style=\"background:url(cid:two@x)\"></div><img src='cid:three@xy'>".into());
    let shown = shown_cids(&body, ["one@x", "two@x", "three@x", "four@x"]);
    let mut shown: Vec<_> = shown.into_iter().collect();
    shown.sort();
    assert_eq!(shown, ["one@x", "two@x"]);
    assert!(shown_cids(&TnefBody::Text("cid:one@x".into()), ["one@x"]).is_empty());
    // References one after the other with nothing between: each read no further than the longest
    // Content-ID (before: to the end of the body each time).
    let html = TnefBody::Html("cid:".repeat(4 << 20));
    let started = std::time::Instant::now();
    assert!(shown_cids(&html, ["a@x"]).is_empty());
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
}

/// The review's case: a big body (16 MB of HTML from 2.8 KB of RTF there, 8 MB of HTML here)
/// and 500 files with Outlook-length Content-IDs the body never shows. Each file looked through
/// the whole body (13 s a pass in the review's release build); the body is now read once.
#[test]
fn many_pictures_and_a_big_body_read_in_one_pass() {
    let cids: Vec<String> = (0..500).map(|i| format!("{}{i}", "a".repeat(40))).collect();
    let mut a = Vec::new();
    attr(&mut a, 1, ATT_MSG_PROPS, &props(&[(0x0102, PR_HTML, binary(format!("<p>{}</p>", "x".repeat(8 << 20)).as_bytes()))]));
    for c in &cids {
        attr(&mut a, 2, ATT_REND_DATA, &[]);
        attr(&mut a, 2, ATT_ATTACHMENT, &props(&[(0x001E, PR_ATTACH_CONTENT_ID, str8(c))]));
        attr(&mut a, 2, ATT_DATA, b"x");
    }
    let raw = outlook_message(stream(&a));
    let started = std::time::Instant::now();
    let data = crate::attachment_data(&raw, 499);
    assert_eq!(data.as_deref(), Some(&b"x"[..]));
    assert!(started.elapsed() < std::time::Duration::from_secs(2), "{:?}", started.elapsed());
}
