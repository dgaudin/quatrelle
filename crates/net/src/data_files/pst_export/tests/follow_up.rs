// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A follow-up marked complete, and the last reply or forward, through a data file and back, as
//! Outlook keeps them.

use super::*;

/// An Inbox of messages with these flags, exported with `marks` and imported back: their flags
/// by subject.
fn round_trip(label: &str, messages: &[(&str, Flags)], marks: &ExportMarks) -> Vec<(String, Flags)> {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-export-follow-up-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(dir.join("cache")).unwrap();
    let mut summaries = Vec::new();
    for (uid, (subject, flags)) in (1..).zip(messages) {
        let d = mime::Draft { subject: (*subject).into(), body: "Hello.".into(), ..Default::default() };
        let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, &format!("m{uid}@example.com")).unwrap();
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        store.write_body(&key, &raw).unwrap();
        summaries.push(mime::summary_from_full(&raw, key, flags.clone(), 10, 0));
    }
    store.save_folder("a1", "INBOX", &FolderState { validity: 1, uid_next: 100, messages: summaries, ..Default::default() }).unwrap();
    let inbox =
        quatrelle_model::Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    store.save_folders("a1", &[inbox]).unwrap();
    let file = dir.join("Out.pst");
    export(&store, "a1", "Riley's mail", &file, marks, &mut |_| {}).unwrap();
    let back = Store::open(dir.join("back")).unwrap();
    crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap();
    let mut out: Vec<(String, Flags)> =
        back.load_folder("b1", "INBOX").unwrap().messages.iter().map(|m| (m.subject.clone(), m.flags.clone())).collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// A follow-up marked complete comes back complete: with the time Exchange said it was done, or
/// with none when that isn't known; one flagged again elsewhere, its keyword left behind, flagged.
#[test]
fn a_completed_follow_up_comes_back_complete() {
    let done_at = 1_790_000_000;
    let complete = |completed| Flags { keywords: vec![quatrelle_model::FOLLOW_UP_COMPLETE.into()], completed, ..Default::default() };
    let again = Flags { flagged: true, ..complete(0) };
    let got = round_trip("complete", &[("Done", complete(done_at)), ("Done, no day", complete(0)), ("Flagged again", again)], NONE);
    let flags = |s: &str| got.iter().find(|(subject, _)| subject == s).map(|(_, f)| f.clone()).unwrap();
    let done = flags("Done");
    assert!(done.follow_up_complete() && done.completed == done_at, "{done:?}");
    let no_day = flags("Done, no day");
    assert!(no_day.follow_up_complete() && no_day.completed == 0, "{no_day:?}");
    let again = flags("Flagged again");
    assert!(again.flagged && !again.follow_up_complete(), "{again:?}");
}
