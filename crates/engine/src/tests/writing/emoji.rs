//! Engine tests: Insert › Emoji's Recent row.

use super::*;

/// The emoji put in last come first in Insert › Emoji's Recent row, each once, 24 at most; what
/// isn't one (empty, with a space or a control character, longer than any sequence) is refused.
#[test]
fn the_emoji_put_in_last_come_first() {
    let mut s = sample_session("recent-emoji");
    for e in ["😀", "👍", "😀", "1️⃣", "👨‍👩‍👧"] {
        s.run("emoji.used", &json!({ "emoji": e })).unwrap();
    }
    assert_eq!(s.prefs.recent_emoji, ["👨‍👩‍👧", "1️⃣", "😀", "👍"]);
    for bad in ["", "a b", "\u{7}", &"😀".repeat(20)] {
        assert!(s.run("emoji.used", &json!({ "emoji": bad })).is_err(), "{bad:?}");
    }
    for n in 0..40u32 {
        let e = char::from_u32(0x1F400 + n).unwrap().to_string();
        s.run("emoji.used", &json!({ "emoji": e })).unwrap();
    }
    assert_eq!(s.prefs.recent_emoji.len(), crate::RECENT_EMOJI);
    done(s);
}
