// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What an Outlook data file keeps of a message comes through the import: files made by the
//! crate's own writer, read back by `import`.

use outlook_pst::messaging::named_prop::PS_PUBLIC_STRINGS;
use outlook_pst::{UnicodePstFile as Writer, UnicodePstMessage, UnicodePstName, UnicodePstProperty, UnicodePstValue};

use super::*;

/// A data file holding `message` alone, imported: its summary and its source.
fn import_one(name: &str, properties: &[UnicodePstProperty<'_>]) -> (quatrelle_model::Summary, Vec<u8>) {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-fidelity-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("mail.pst");
    let message = UnicodePstMessage {
        subject: "Plan",
        sender_name: "Ana",
        sender_email: "ana@example.com",
        recipients: &[],
        body: "Hello.",
        html_body: None,
        message_id: "<plan@example.com>",
        // 2024-01-01T00:00:00Z
        delivery_time: 133_485_408_000_000_000,
        message_class: "IPM.Note",
        properties,
    };
    drop(Writer::create(&file, &message).unwrap());
    let store = Store::open(dir.join("cache")).unwrap();
    let done = import(&file, &store, "a1", &mut |_| {}).unwrap();
    assert_eq!(done.messages, 1, "{done:?}");
    let s = done.folders.iter().find_map(|f| store.load_folder("a1", &f.path).ok().and_then(|s| s.messages.into_iter().next())).unwrap();
    let raw = store.read_body(&s.key).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    (s, raw)
}

/// Outlook's categories (the string-named `Keywords`) come in as the message's keywords.
#[test]
fn categories_come_in_as_keywords() {
    let names = ["Work", "To do"];
    let categories = UnicodePstProperty::named(PS_PUBLIC_STRINGS, UnicodePstName::String("Keywords"), UnicodePstValue::MultipleUnicode(&names));
    let (s, _) = import_one("categories", &[categories]);
    assert_eq!(s.flags.keywords, ["work", "to_do"]);
}

/// A FILETIME (100 ns since 1601) for Unix seconds.
fn filetime(unix: i64) -> i64 {
    (unix + 11_644_473_600) * 10_000_000
}

/// A follow-up Outlook marked complete (`PidTagFlagStatus` 1) comes in complete, with the time it
/// was (`PidTagFlagCompleteTime`), as Exchange's does: not flagged, `$FollowUpComplete`.
#[test]
fn a_completed_follow_up_comes_in_complete_with_its_time() {
    let done = 1_762_340_000;
    let status = UnicodePstProperty::tagged(0x1090, UnicodePstValue::Integer32(1));
    let at = UnicodePstProperty::tagged(0x1091, UnicodePstValue::Time(filetime(done)));
    let (s, _) = import_one("complete", &[status, at]);
    assert!(s.flags.follow_up_complete() && !s.flags.flagged, "{:?}", s.flags);
    assert_eq!(s.flags.completed, done);
    // Without its time, complete all the same; a time before 1970 is none.
    let (s, _) = import_one("complete-no-time", &[status]);
    assert!(s.flags.follow_up_complete() && s.flags.completed == 0, "{:?}", s.flags);
    let early = UnicodePstProperty::tagged(0x1091, UnicodePstValue::Time(filetime(-86_400)));
    let (s, _) = import_one("complete-early", &[status, early]);
    assert!(s.flags.follow_up_complete() && s.flags.completed == 0, "{:?}", s.flags);
    // Flagged (2): flagged, no time kept; a time with no complete state is none.
    let (s, _) = import_one("flagged", &[UnicodePstProperty::tagged(0x1090, UnicodePstValue::Integer32(2)), at]);
    assert!(s.flags.flagged && !s.flags.follow_up_complete() && s.flags.completed == 0, "{:?}", s.flags);
}

/// What Outlook did last with a message (`PidTagLastVerbExecuted`: 102 replied, 103 replied to
/// all, 104 forwarded) and when (`PidTagLastVerbExecutionTime`) come in as its arrow and its time,
/// as Exchange's do; another verb (a vote) is neither, and MAPI's "none" (4501) is no time.
#[test]
fn the_last_reply_or_forward_comes_in_with_its_time() {
    let when = 1_762_340_000;
    let at = UnicodePstProperty::tagged(0x1082, UnicodePstValue::Time(filetime(when)));
    let verb = |n| UnicodePstProperty::tagged(0x1081, UnicodePstValue::Integer32(n));
    for (name, n, answered, forwarded) in [("reply", 102, true, false), ("reply-all", 103, true, false), ("forward", 104, false, true)] {
        let (s, _) = import_one(name, &[verb(n), at]);
        assert_eq!((s.flags.answered, s.flags.forwarded, s.flags.acted), (answered, forwarded, when), "{name}");
    }
    let (s, _) = import_one("vote", &[verb(261), at]);
    assert_eq!((s.flags.answered, s.flags.forwarded, s.flags.acted), (false, false, 0));
    let none = quatrelle_model::dates::days_from_civil(4501, 1, 1) * 86_400;
    let (s, _) = import_one("reply-none", &[verb(102), UnicodePstProperty::tagged(0x1082, UnicodePstValue::Time(filetime(none)))]);
    assert_eq!((s.flags.answered, s.flags.acted), (true, 0));
    let (s, _) = import_one("reply-no-time", &[verb(102)]);
    assert_eq!((s.flags.answered, s.flags.acted), (true, 0));
}
