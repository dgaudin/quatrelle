//! Pictures in the message being written: put in at the width asked (a page's, for a wide one),
//! on a line of their own, and resized (from upstream 865a712).

use super::*;
use quatrelle_mime::rich::PICTURE;

#[test]
fn a_picture_goes_in_at_its_width_and_is_resized() {
    let mut s = sample_session("picture-width");
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Poster", "body": "See this one"})).unwrap();
    let png = crate::sample::sample_png();
    // A screenshot mid-sentence, on a line of its own, at a page's width.
    let r = s.run("compose.insertPicture", &json!({"name": "shot.png", "data": png, "at": 3, "width": 624, "line": true})).unwrap();
    let cid = r["cid"].as_str().unwrap().to_string();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.body, format!("See\n{PICTURE}\n this one"));
    assert_eq!(r["end"], json!(5));
    let at = c.body.chars().position(|ch| ch == PICTURE).unwrap();
    assert_eq!(c.styles.style(at).picture_width, Some(624));
    assert!(quatrelle_mime::rich::to_html(&c.body, &c.styles).contains("width=\"624\""));
    // Made smaller, then back to its own width; nothing below the narrowest.
    s.run("compose.resizePicture", &json!({"cid": cid, "width": 300})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().styles.style(at).picture_width, Some(300));
    s.run("compose.resizePicture", &json!({"cid": cid, "width": 3})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().styles.style(at).picture_width, Some(16));
    s.run("compose.resizePicture", &json!({"cid": cid})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().styles.style(at).picture_width, None);
    assert!(s.run("compose.resizePicture", &json!({"cid": "nothing@here"})).is_err());
    // Typed right after it: plain text, no width carried on.
    let before = s.compose.as_ref().unwrap().styles.style(at).for_typing();
    assert_eq!(before.picture_width, None);
    done(s);
}
