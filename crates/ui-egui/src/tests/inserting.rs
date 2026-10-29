//! UI tests: what Insert puts in a message.

use super::*;

/// Insert › Emoji puts the emoji clicked at the cursor, in colour as the rest of the text draws
/// them, keeps the picker open for the next one, and the Recent row starts with it (the owner
/// found no way to put one in, 29/10).
#[test]
fn an_emoji_is_put_in_at_the_cursor() {
    let (mut a, dir) = app("emoji");
    let ctx = egui::Context::default();
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ann@x.example", "subject": "Hi", "body": "Hello"}));
    let size = egui::vec2(1280.0, 800.0);
    let tab = settled_text_at(&ctx, &mut a, "Insert", size).expect("the ribbon's Insert tab");
    click_at(&ctx, &mut a, tab, 1, size);
    let button = settled_text_at(&ctx, &mut a, "Emoji", size).expect("Insert › Emoji");
    click_at(&ctx, &mut a, button, 1, size);
    let first = settled_text_at(&ctx, &mut a, "😀", size).expect("the picker's smileys");
    click_at(&ctx, &mut a, first, 1, size);
    frame_sized(&ctx, &mut a, vec![], size);
    let body = |a: &MailApp| a.session.compose.as_ref().map(|c| c.body.clone()).unwrap_or_default();
    assert!(body(&a).contains('😀'), "{:?}", body(&a));
    assert_eq!(a.session.prefs.recent_emoji.first().map(String::as_str), Some("😀"));
    // Still open: the next goes in after it.
    let next = settled_text_at(&ctx, &mut a, "😃", size).expect("the picker, still open");
    click_at(&ctx, &mut a, next, 1, size);
    frame_sized(&ctx, &mut a, vec![], size);
    assert!(body(&a).contains("😀😃"), "{:?}", body(&a));
    assert_eq!(a.session.prefs.recent_emoji, vec!["😃".to_string(), "😀".to_string()]);
    let _ = std::fs::remove_dir_all(dir);
}
