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
